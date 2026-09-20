# 设计文档：workspace-context-lock

## 概述

本功能不再尝试用 `default_cwd` 模拟 Workspace 切换。网关在 pin 时创建一个以活动 Git worktree 为根的新 `ToolContext`，并原子替换当前对话 `Binding` 中的 context 与锁键。现有工具调度和文件边界由此自然落在同一个活动根。

**对应需求:** FR-1, FR-2, FR-3, FR-4, FR-5, FR-6, NFR-1, NFR-2, NFR-3, NFR-4

---

## 技术方案

### 技术选型

| 类别 | 选择 | 理由 | 关联需求 |
|------|------|------|----------|
| 会话所有权 | 复用 `GatewayRouter.bindings` | 已按 OpenAI/MCP 会话隔离并有 TTL 与配置撤销 | FR-1, FR-2 |
| 活动根切换 | 原子替换 `Binding.context` | 避免逐个工具修补 cwd；Workspace/Harness/History 一次切换 | FR-2 |
| Git 身份 | `git rev-parse` / `symbolic-ref` / `merge-base --is-ancestor` | 与 linked worktree 语义一致，不解析 `.git` 文本格式 | FR-1, FR-3 |
| 失败策略 | fail closed | 漂移后停止，禁止静默回退父仓库 | FR-3 |
| 状态存储 | 内存 `Binding.context_lock` | 与现有会话绑定生命周期一致 | NFR-2 |

### 架构设计

```text
ChatGPT conversation
  -> Gateway Binding
       configured workspace profile
       context_lock: Option<WorkspaceContextPin>
       context: Arc<ToolContext(active worktree)>
       lock_key: canonical active root
  -> preflight validate pin
  -> existing handle_request / call_tool
```

`pin_workspace_context`、`get_workspace_context`、`unpin_workspace_context` 属于网关控制工具，由 `GatewayRouter` 在进入原 `handle_request` 前处理。其他工具继续沿用原调度内核。

---

## 数据模型

```rust
struct WorkspaceContextPin {
    configured_root: PathBuf,
    active_root: PathBuf,
    git_dir: PathBuf,
    git_common_dir: PathBuf,
    branch: String,
    initial_head: String,
    last_head: String,
    source: SessionKeySource,
    pinned_at: Instant,
    expires_at: Instant,
}
```

`Binding` 新增 `context_lock: Option<Arc<Mutex<WorkspaceContextPin>>>`。共享锁用于在请求前校验并安全更新 `last_head`；Pin 不序列化，profile 指纹或 allowlist 变化仍立即撤销整个 binding。

---

## API 设计

| 工具 | 入参 | 关键出参 | 关联需求 |
|------|------|----------|----------|
| `pin_workspace_context` | `path`, `expected_branch?`, `expires_in_minutes?`, `allow_protected_branch?`, `confirm?` | `locked`, `configured_root`, `active_root`, `branch`, `initial_head`, `expires_at` | FR-1 |
| `get_workspace_context` | `{}` | `locked`, 根目录、Git 身份、当前 HEAD、`status`, `remaining_seconds` | FR-3, FR-4 |
| `unpin_workspace_context` | `confirm` | `locked=false`, 恢复后的 `active_root` | FR-4 |

主要错误码：`WORKSPACE_CONTEXT_PATH_INVALID`、`WORKTREE_ROOT_REQUIRED`、`PROTECTED_BRANCH_REQUIRES_CONFIRMATION`、`WORKSPACE_CONTEXT_MISMATCH`、`WORKSPACE_CONTEXT_EXPIRED`、`WORKSPACE_CONTEXT_BUSY`。

---

## 文件结构

- 新增 `src-tauri/src/mcp/workspace_context.rs`：Git worktree 探测、Pin 数据和漂移校验。
- 修改 `src-tauri/src/mcp/gateway.rs`：Binding 状态、上下文重建、请求前校验和响应摘要。
- 新增 `src-tauri/src/mcp/gateway/workspace_context_gateway.rs`：pin/get/unpin 控制工具与 schema。
- 新增 `src-tauri/src/mcp/workspace_context_tests.rs`：临时 Git worktree 测试夹具与漂移回归。
- 修改 `src-tauri/src/mcp/mod.rs` 与 `src-tauri/src/mcp/server.rs`：模块注册与初始化指令。
- 修改 `src-tauri/src/tools/patch.rs`：提交后的磁盘复核。
- 视 SessionStore 现有 API，最小修改 `src-tauri/src/tools/session.rs` 增加运行中会话查询。

---

## 设计决策

### 决策 1: 替换 ToolContext，而不是继续扩展 default cwd（关联需求: FR-2）

**问题**: `Workspace`、Harness、History 和 Git 工具拥有不同根来源。
**选项**: 为每个工具增加 active root 参数；把 Workspace 变成全局可变对象；在网关 Binding 中替换完整 ToolContext。
**决策**: 选择 Binding 级 ToolContext 替换，保持工具内核签名和路径安全模型不变。

### 决策 2: 漂移或过期后失败关闭（关联需求: FR-3）

**问题**: 自动恢复父仓库会重新引入误写 main 的风险。
**决策**: 所有普通项目工具返回结构化错误；只有 get/unpin 可继续，用于诊断和显式恢复。

### 决策 3: 允许同分支 HEAD 快进（关联需求: FR-3）

**问题**: 正常 commit 会改变 HEAD。
**决策**: 允许当前 HEAD 是最近 HEAD 的后代并更新观测值；拒绝非快进、分支切换和 Git worktree 身份变化。

### 决策 4: 首期只支持网关会话（关联需求: NFR-3）

**问题**: 非网关 listener 目前共享单一 ToolContext，不具备每对话状态所有权。
**决策**: 控制工具仅加入网关工具目录；单工作区模式保持现有契约，后续可抽取通用 SessionRouter。

---

## 风险评估

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 修改 `GatewayRouter` 高影响 | 高 | 保持 `call_tool` 不变，新增控制工具在路由层包装；完整网关回归 |
| Pin/Unpin 与运行命令竞争 | 高 | 检测活动 SessionStore，忙时拒绝切换；Binding 更新原子化 |
| Git 命令在 Windows 路径格式差异 | 中 | 对 Git 返回路径 canonicalize 后比较 |
| HEAD 正常提交被误判 | 中 | 使用祖先关系允许快进并更新 last_head |
| Patch 写入后验证失败 | 中 | 保留现有事务回滚语义并返回明确错误 |

# 设计文档：workspace-execution-context

## 概述

本功能在现有 `workspace-context-lock` 之上补齐执行根的类型化表达。`Workspace` 继续负责 active root 的文件边界，新增 repository root 元数据；`ToolContext` 额外持有 `Mutex<ExecutionContext>`，所有入口先校验它，再进入既有 `call_tool` 分发内核。网关 pin 时一次性构造带 Git 身份的上下文，避免只修改 `default_cwd` 造成工具之间根目录分裂。

**对应需求:** FR-1, FR-2, FR-3, FR-4, FR-5, FR-6, NFR-1, NFR-2, NFR-3, NFR-4

---

## 技术方案

### 技术选型

| 类别 | 选择 | 理由 | 关联需求 |
|------|------|------|----------|
| 根模型 | `Workspace { repository_root, active_root }` | 保留现有 `root()` 调用，同时显式记录仓库和活动目录 | FR-1, FR-3, NFR-3 |
| 会话状态 | `ToolContext.execution: Mutex<ExecutionContext>` | 与 default cwd、SessionStore 同属会话上下文，支持 HEAD 观测值更新 | FR-1, FR-4 |
| Git 校验 | `rev-parse`、`symbolic-ref`、`merge-base` | 复用 Git worktree 语义，不解析 `.git` 文本格式 | FR-4 |
| 调度保护 | `call_tool` 单一前置校验 | MCP、Actions 和内部测试共同经过同一边界，避免入口分叉 | FR-3, FR-5 |
| 网关接入 | Binding 原子替换 Workspace + ExecutionContext | 保留已有会话隔离、Harness、History 和锁实现 | FR-5 |

### 架构设计

```text
MCP / Actions request
  -> Gateway route (if enabled)
  -> ToolContext
       Workspace(repository_root, active_root)
       ExecutionContext(repository_root, execution_root, Git identity)
       default_cwd inside execution_root
       Harness / History / SessionStore rooted at active_root
  -> validate_execution_context()
  -> apply_default_cwd()
  -> policy + existing call_tool dispatch
```

`Workspace::root()` 在兼容层返回 `active_root`；新代码优先调用 `Workspace::active_root()` 或 `ToolContext::execution_root()`。这样不需要改动所有工具的公共参数类型，也不会让 Git、Harness、History 继续依赖配置父目录。

---

## 数据模型

```rust
pub struct ExecutionContext {
    repository_root: PathBuf,
    execution_root: PathBuf,
    git_dir: Option<PathBuf>,
    git_common_dir: Option<PathBuf>,
    branch: Option<String>,
    observed_head: Option<String>,
}
```

约束：两个根在创建时 canonicalize；`execution_root` 必须位于 `repository_root` 内；无 Git 元数据时允许普通目录；带 Git 元数据时每次项目工具前验证身份。`observed_head` 允许快进更新，非快进变化会失败关闭。

`Workspace` 结构改为：

```rust
pub struct Workspace {
    repository_root: PathBuf,
    active_root: PathBuf,
}
```

`root()` 保留为 `active_root` 的兼容别名；增加 `repository_root()` 与 `active_root()` 只读访问器，以及 `new_with_roots(repository_root, active_root)` 构造函数。

---

## API 设计

| 方法/函数 | 路径/签名 | 入参 | 出参 | 关联需求 |
|-----------|-----------|------|------|----------|
| `ExecutionContext::for_workspace` | `tools/execution_context.rs` | `repository_root`, `execution_root` | 无 Git 锁的上下文 | FR-1, FR-4 |
| `ExecutionContext::with_git_identity` | `tools/execution_context.rs` | git dir/common dir、branch、HEAD | 带身份锁的上下文 | FR-1, FR-4 |
| `ToolContext::execution_root` | `tools/context.rs` | 无 | `PathBuf` | FR-1, FR-3 |
| `ToolContext::validate_execution_context` | `tools/context.rs` | 无 | `WorkspaceResult<()>` | FR-3, FR-4 |
| `ToolContext::set_default_cwd` | `tools/context.rs` | 候选绝对路径 | `WorkspaceResult<()>` | FR-2 |
| `Workspace::new_with_roots` | `tools/workspace.rs` | repository/active 根 | `WorkspaceResult<Workspace>` | FR-1, FR-5 |
| `GatewayRouter::build_context_at_root` | `mcp/gateway.rs` | host、target、active root、Git identity | 带执行上下文的 `SharedState` | FR-5 |

错误使用现有结构化 `WorkspaceError::ToolDetails`，新增 `WORKSPACE_CONTEXT_MISMATCH`、`WORKSPACE_CONTEXT_EXPIRED` 仅在上下文校验阶段产生；details 仅返回规范化路径、branch 和 HEAD 前缀摘要。

---

## 文件结构

```text
src-tauri/src/tools/
├── execution_context.rs       # 新增：根模型、Git 身份和漂移校验
├── context.rs                 # 修改：ToolContext 持有 execution 状态
├── workspace.rs               # 修改：repository_root / active_root 双根模型
├── dispatch.rs                # 修改：入口 preflight、cwd 推导和 set_default_cwd
├── exec.rs                    # 修改：命令解析与默认 cwd 使用 execution_root
├── patch.rs                   # 修改：写入前执行上下文校验
├── git.rs                     # 修改：Git cwd 使用 active execution root
└── history/                   # 修改：显示与扫描根统一
src-tauri/src/mcp/
├── gateway.rs                 # 修改：pin 上下文传入双根和 Git 身份
└── workspace_context.rs       # 修改：复用 ExecutionContext 的身份校验
```

---

## 设计决策

### 决策 1: 保留 `Workspace::root()` 作为 active root（关联需求: FR-1, NFR-3）

**问题**: 直接把 `root()` 改成 repository root 会让现有文件边界、Git 和 Harness 全部回到父目录。

**选项**:
1. 让 `root()` 返回 repository root，并逐个工具增加 active root 参数。
2. 让 `root()` 保持 active root，新增 `repository_root()` 表达仓库归属。

**决策**: 选择选项 2。

**理由**: 兼容现有工具签名和 Workspace 路径防护，同时用新 ExecutionContext 显式表达两种根；后续工具可逐步迁移到命名访问器。

### 决策 2: 在 `call_tool` 前置校验，而不是每个工具复制校验（关联需求: FR-3, FR-4）

**问题**: 在 Patch、Git、exec、History 各自加校验会造成行为分叉，遗漏一个入口就会重新引入漂移。

**决策**: `call_tool` 作为唯一入口统一调用 `validate_execution_context()`；网关控制工具仍由网关路由层单独处理。

**理由**: MCP 与 Actions 都已经通过 `call_tool`，能以最小修改覆盖全部项目工具；原有工具函数继续负责各自参数和文件边界。

### 决策 3: Pin 身份复用，避免两套 Git 观测值（关联需求: FR-4, FR-5）

**问题**: 现有 `WorkspaceContextPin` 已记录 Git dir、branch 和 HEAD，ExecutionContext 再独立探测会产生重复实现和不同步风险。

**决策**: 抽取可复用的 Git identity/ancestry 校验到 `ExecutionContext`，让 `WorkspaceContextPin` 保留过期和网关协议字段但委托同一校验函数；pin 构造的 `ToolContext` 直接携带该 identity。

**理由**: 保持既有网关公共行为和错误码，同时使 Actions、单工作区和网关共享同一个身份算法。

### 决策 4: 非 Git 目录保持兼容（关联需求: FR-4, NFR-3）

**问题**: 大量工具测试和用户临时目录并不是 Git 仓库。

**决策**: ExecutionContext 的 Git 字段可选；只有 pin 或显式带身份的上下文执行 Git 校验，普通目录只做存在性和路径边界校验。

**理由**: 不把新增防漂移能力变成普通文件工具的强制 Git 依赖。

---

## 测试策略

- 单元测试：根 canonicalize、active root 越界、default cwd 越界、非 Git 目录、branch/HEAD 漂移、快进更新和非快进拒绝。
- MCP 集成测试：pin 后 Patch/exec/Git/History/Harness 都指向 worktree；pin 失败保持原 context；两个会话绑定不同 worktree 不互相覆盖。
- 契约回归：单工作区工具目录、Actions 调用、`Workspace::root()` 兼容、现有 `call_tool` 返回结构和 Patch `verified=true` 不变。
- 门禁：`npm run check`、`npm run build`、`cargo test --manifest-path src-tauri/Cargo.toml`、`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`。

---

## 风险评估

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| `ToolContext` 和 `Workspace` 是共享核心结构 | 高 | 保留旧构造函数和 `root()` 别名，先加测试再迁移入口 |
| Git 校验与现有 `WorkspaceContextPin` 重复 | 高 | 抽取统一 identity/ancestry helper，禁止两套算法长期并存 |
| call_tool 每次校验增加延迟 | 中 | 非 Git 上下文不运行 Git CLI；Git CLI 使用 5 秒超时并保持错误可重试 |
| default cwd 显示路径兼容性 | 中 | 同时返回相对 display 和绝对 resolved path，保留旧字段名 |
| Actions 与网关上下文生命周期不同 | 中 | Actions 使用单一 ToolContext，网关继续按 Binding 原子替换并跑双路径回归 |

---

## 检查清单

- [x] 技术方案与现有 Rust/Tauri 架构一致。
- [x] requirements.md 的 FR-1 至 FR-6 和 NFR-1 至 NFR-4 已全部覆盖。
- [x] 文件结构只列出当前源码中可定位的模块和本次新增模块。
- [x] 数据模型、兼容 API、错误契约和超时约束清晰。
- [x] 关键设计决策已记录并回链需求。
- [x] 测试策略覆盖审计指出的根因和普通模式回归。


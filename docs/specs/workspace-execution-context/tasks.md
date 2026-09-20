# 任务清单：workspace-execution-context

## 概述

实现会话级统一 ExecutionContext，修复 repository root、active worktree 和 default cwd 的隐式混用。每条任务回链需求与设计，生产新增模块不超过 500 行。

## 交付物清单（Scope-lock）

- **预计新建文件数**: 1 个生产模块、1 个单元测试模块、3 个规格文件。
- **预计修改文件数**: 9 个 Rust 源文件。
- **预计新增/修改函数数**: 约 30 个函数或方法。
- **交付物逐项列举**:
  1. `src-tauri/src/tools/execution_context.rs`：ExecutionContext 数据模型、Git 身份探测、快进和漂移校验。
  2. `src-tauri/src/tools/context.rs`：ToolContext execution 状态与兼容构造入口。
  3. `src-tauri/src/tools/workspace.rs`：repository/active 双根和路径边界 API。
  4. `src-tauri/src/tools/dispatch.rs`：统一 preflight、execution root cwd 推导和 set_default_cwd 保护。
  5. `src-tauri/src/tools/exec.rs`、`git.rs`、`patch.rs`、`history/`：执行根接入。
  6. `src-tauri/src/mcp/gateway.rs` 与 `workspace_context.rs`：网关 pin 复用统一上下文。
  7. Rust 单元/集成测试、规格文档和全量门禁证据。

---

## 任务列表

### 阶段 1: 准备与影响面

- [x] 1.1 校验本规格并完成共享入口影响分析，确认 `ToolContext`、`Workspace`、`call_tool`、网关 pin 和 Actions 的真实调用边界。
  - **证据块**: 当前 `src-tauri/src/tools/context.rs:10-19` 只有 `Workspace`、`default_cwd` 和 `SessionStore`；`src-tauri/src/tools/dispatch.rs:52-63` 是唯一策略/分发入口；`src-tauri/src/mcp/gateway.rs:681-700` 在 pin 时重建 `ToolContext`。
  - **涉及文件**: `docs/specs/workspace-execution-context/*.md`；不修改生产代码。
  - _需求: FR-1, FR-3, FR-5_ ｜ _设计: 技术方案、架构设计_

### 阶段 2: 根模型与校验

- [x] 2.1 新建 `execution_context.rs`，实现 repository/active 双根、可选 Git identity、5 秒有界探测、快进 HEAD 更新和结构化漂移错误。
  - **证据块**: `src-tauri/src/mcp/workspace_context.rs:77-214` 已有 pin 字段与 validate 逻辑；`src-tauri/src/mcp/workspace_context.rs:254-318` 已有 Git rev-parse 和 merge-base 需求。
  - **涉及文件**: `src-tauri/src/tools/execution_context.rs`（≤500 行）、`src-tauri/src/tools/mod.rs`（≤5 行新增）、`src-tauri/src/tools/execution_context_tests.rs`（≤220 行）。
  - _需求: FR-1, FR-4, FR-6_ ｜ _设计: 数据模型、决策 3-4_

- [x] 2.2 扩展 `Workspace` 支持 repository root 与 active root，并保持 `root()` 返回 active root 的兼容语义。
  - **证据块**: `src-tauri/src/tools/workspace.rs:142-167` 当前只有私有 `root` 字段，`root()`/`root_display()` 被文件、Git、History 和 Harness 共享。
  - **涉及文件**: `src-tauri/src/tools/workspace.rs`（新增 ≤80 行）、`src-tauri/src/tools/workspace_tests.rs`（如已有测试模块则新增 ≤80 行）。
  - _需求: FR-1, FR-2, FR-3_ ｜ _设计: 数据模型、决策 1_

### 阶段 3: ToolContext 与入口接入

- [x] 3.1 扩展 `ToolContext` 持有 `Mutex<ExecutionContext>`，增加执行根读取、Git identity 注入、preflight 校验和受保护的 default cwd 设置；旧构造函数默认双根相同。
  - **证据块**: `src-tauri/src/tools/context.rs:39-75` 由 Workspace root 初始化 default cwd，`set_default_cwd` 在 `:102-108` 当前无返回值或边界检查。
  - **涉及文件**: `src-tauri/src/tools/context.rs`（新增 ≤130 行）、`src-tauri/src/tools/dispatch.rs`（新增 ≤70 行）。
  - _需求: FR-1, FR-2, FR-4_ ｜ _设计: API 设计、决策 2_

- [x] 3.2 在 `call_tool` 分发前执行统一 execution context preflight，并将 cwd 推导从 `default_cwd`/`workspace.root()` 改为 execution root；保持 Gateway 控制工具和现有 `call_tool` 签名不变。
  - **证据块**: `src-tauri/src/tools/dispatch.rs:54-63` 当前先 `apply_default_cwd` 再校验策略；`src-tauri/src/tools/dispatch.rs:233-281` 以 `ctx.workspace.root()` 判断默认根。
  - **涉及文件**: `src-tauri/src/tools/dispatch.rs`（新增 ≤90 行）、`src-tauri/src/tools/policy.rs`（仅必要兼容调整 ≤20 行）。
  - _需求: FR-2, FR-3, FR-4, FR-5_ ｜ _设计: 架构设计、决策 2_

- [x] 3.3 让 exec、Git、Patch、Harness 和 History 的命令 cwd、路径显示与写入边界统一使用 active execution root。
  - **证据块**: `src-tauri/src/tools/exec.rs:26,236,365`、`src-tauri/src/tools/patch.rs:12`、`src-tauri/src/tools/history/mod.rs:26,143` 当前分别从 `ctx.workspace` 获取根。
  - **涉及文件**: `src-tauri/src/tools/exec.rs`（≤35 行新增）、`src-tauri/src/tools/git.rs`（≤25 行新增）、`src-tauri/src/tools/patch.rs`（≤20 行新增）、`src-tauri/src/tools/history/mod.rs`（≤35 行新增）。
  - _需求: FR-3, FR-5_ ｜ _设计: 架构设计、测试策略_

### 阶段 4: 网关整合

- [x] 4.1 将现有 `WorkspaceContextPin` 的 Git identity/ancestry 校验委托给 ExecutionContext，并在 `GatewayRouter::build_context_at_root` 原子注入 repository root、active root 和 identity。
  - **证据块**: `src-tauri/src/mcp/gateway/workspace_context_gateway.rs:50-90` 当前创建 pin 后只传 active root 给 `ToolContext::from_workspace`；`src-tauri/src/mcp/gateway.rs:681-700` 当前用单 root 构造 Workspace。
  - **涉及文件**: `src-tauri/src/mcp/workspace_context.rs`（重构 ≤80 行净增）、`src-tauri/src/mcp/gateway.rs`（新增 ≤55 行）、`src-tauri/src/mcp/gateway/workspace_context_gateway.rs`（新增 ≤25 行）。
  - _需求: FR-4, FR-5_ ｜ _设计: 决策 3、文件结构_

### 阶段 5: 集成测试与门禁

- [x] 5.1 覆盖普通目录、Git worktree、default cwd 越界、branch/HEAD 漂移、快进和非快进拒绝。
  - **证据块**: `src-tauri/src/mcp/workspace_context_tests.rs:55-113` 已覆盖 pin、主分支、过期、branch 和路径边界；`src-tauri/src/tools/execution_context_tests.rs` 同时验证 ExecutionContext、detached Git、嵌套 Git 和 call_tool 未执行内核。
  - **涉及文件**: `src-tauri/src/tools/execution_context_tests.rs`、`src-tauri/src/mcp/workspace_context_tests.rs`（新增 ≤220 行）。
  - _需求: FR-2, FR-4, FR-6_ ｜ _设计: 测试策略_

- [x] 5.2 验证两个会话的 Patch、exec、Git、Harness、History 和命令 session 不串联，并验证网关关闭/单工作区回归。
  - **证据块**: `src-tauri/src/mcp/gateway.rs:1626-1747` 已有双 worktree context 隔离测试；`src-tauri/tests/call_tool_contract.rs` 与 `harness_tool_contract.rs` 提供既有工具契约。
  - **涉及文件**: `src-tauri/src/mcp/gateway.rs` 测试模块（新增 ≤100 行）、`src-tauri/tests/call_tool_contract.rs`（新增 ≤80 行）。
  - _需求: FR-3, FR-5, FR-6_ ｜ _设计: 架构设计、测试策略_

- [x] 5.3 执行 `npm run check`、`npm run build`、`cargo test --manifest-path src-tauri/Cargo.toml` 和全目标 Clippy，回读任务清单并记录完整 revision 证据。
  - **证据块**: `npm run check` 通过（0 errors、0 warnings）；`npm run build` 通过（Vite/SvelteKit production bundle）；`cargo test --manifest-path src-tauri/Cargo.toml` 通过（145 个库测试、22 个 call_tool 契约测试、24 个安全测试、4 个 Harness 状态测试、9 个 Harness 契约测试、16 个 History 测试，全部 0 failed）；`cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` 通过。新增 detached Git 与嵌套 detached Git 回归测试均通过。
  - **涉及文件**: `docs/specs/workspace-execution-context/tasks.md`；不修改生产逻辑。
  - _需求: FR-6, NFR-3_ ｜ _设计: 测试策略_

---

## 检查点

- [x] 阶段 1 完成后：`check_spec` 与 GitNexus impact 通过，且文件/符号范围已记录。
- [x] 阶段 2 完成后：ExecutionContext/Workspace 单元测试通过，非 Git 目录兼容。
- [x] 阶段 3 完成后：call_tool preflight、default cwd、exec/Git/Patch/History 根路径回归通过。
- [x] 阶段 4 完成后：网关 pin 与两会话隔离测试通过。
- [x] 阶段 5 完成后：四个项目门禁通过，code_review 无高优先级问题。

---

## 需求覆盖矩阵

| 需求 ID | 设计章节 | 任务编号 | 状态 |
|---------|----------|----------|------|
| FR-1 | 数据模型、API 设计 | 2.1, 2.2, 3.1, 4.1 | 已完成 |
| FR-2 | API 设计、决策 1 | 2.2, 3.1, 3.2, 5.1 | 已完成 |
| FR-3 | 架构设计、决策 2 | 3.2, 3.3, 5.2 | 已完成 |
| FR-4 | 数据模型、决策 3-4 | 2.1, 3.1, 3.2, 4.1, 5.1 | 已完成 |
| FR-5 | 架构设计、文件结构 | 3.2, 3.3, 4.1, 5.2 | 已完成 |
| FR-6 | 测试策略 | 2.1, 5.1, 5.2, 5.3 | 已完成 |

---

## 文件变更清单

| 文件 | 操作 | 行数预算 | 说明 |
|------|------|----------|------|
| `src-tauri/src/tools/execution_context.rs` | 新建 | ≤500 | ExecutionContext、Git identity、漂移校验 |
| `src-tauri/src/tools/execution_context_tests.rs` | 新建 | ≤220 | 非 Git、路径、分支和 HEAD 回归 |
| `src-tauri/src/tools/context.rs` | 修改 | ≤130 新增 | ToolContext execution 状态与 API |
| `src-tauri/src/tools/workspace.rs` | 修改 | ≤80 新增 | 双根模型和兼容访问器 |
| `src-tauri/src/tools/dispatch.rs` | 修改 | ≤160 新增 | preflight 与 execution root cwd |
| `src-tauri/src/tools/exec.rs` | 修改 | ≤35 新增 | 命令 cwd 和解析根 |
| `src-tauri/src/tools/git.rs` | 修改 | ≤25 新增 | Git cwd |
| `src-tauri/src/tools/patch.rs` | 修改 | ≤20 新增 | 写入前上下文保护 |
| `src-tauri/src/tools/history/mod.rs` | 修改 | ≤35 新增 | History active root |
| `src-tauri/src/mcp/workspace_context.rs` | 修改 | ≤80 净增 | 复用 Git identity 校验 |
| `src-tauri/src/mcp/gateway.rs` | 修改 | ≤155 新增 | 注入上下文和测试 |
| `src-tauri/src/mcp/gateway/workspace_context_gateway.rs` | 修改 | ≤25 新增 | pin 构造接入 |
| `src-tauri/src/tools/mod.rs` | 修改 | ≤5 新增 | 注册 ExecutionContext |
| `src-tauri/tests/call_tool_contract.rs` | 修改 | ≤80 新增 | 根路径和 preflight 契约 |

---

## 检查清单

- [x] 交付物清单已填，实现后数量逐项核对。
- [x] 每条任务标题均为动词、对象和约束的具体描述。
- [x] 每条任务含当前代码证据块和文件预算。
- [x] 任务按准备、核心、网关、测试分阶段，粒度可在单次提交内完成。
- [x] 每条任务都回链到 FR 与 design 章节。
- [x] 需求覆盖矩阵无遗漏。
- [x] 阶段 5 包含对照验收标准核验和四个门禁。
- [x] 全文占位检查已完成，规格内容均为可执行描述。

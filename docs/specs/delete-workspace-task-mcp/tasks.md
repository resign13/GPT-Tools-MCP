# 任务清单：delete-workspace-task-mcp

## 概述

在独立 `feature/delete-workspace-task` worktree 中补齐 MCP 网关任务删除能力，保持当前运行桌面程序不受影响。

---

## 交付物清单（Scope-lock）

- **预计新建文件数**: 3 个规格文件
- **预计修改文件数**: 2 个生产代码文件
- **预计新增/修改函数数**: 约 3 个（`DataStore::remove`、一个 Rust 单测、一个 Svelte 删除处理函数）
- **交付物逐项列举**:
  1. `src-tauri/src/data/store.rs`
  2. `src/routes/gateway/+page.svelte`
  3. `docs/specs/delete-workspace-task-mcp/requirements.md`
  4. `docs/specs/delete-workspace-task-mcp/design.md`
  5. `docs/specs/delete-workspace-task-mcp/tasks.md`

---

## 任务列表

### 阶段 1: 数据一致性

- [x] 1.1 扩展 `DataStore::remove`，在单次保存内清理已删除 ID 的所有 Gateway allowlist 引用
  - **证据块**: `src-tauri/src/data/store.rs:108-117` 当前只执行 `profiles.remove(index)`、`workspace_secrets.remove(id)` 和 `save()`；`src-tauri/src/commands/workspace.rs:123-140` 表明生产删除链路由 `delete_workspace` 调用 `store.remove(&id)`。
  - **涉及文件**: `src-tauri/src/data/store.rs`，预计新增 30-45 行（含测试）。
  - _需求: FR-2_ ｜ _设计: 数据模型、设计决策 1_

---

### 阶段 2: 网关任务删除交互

- [x] 2.1 在网关任务卡片增加非宿主删除按钮，使用原生二次确认并防止重复提交
  - **证据块**: `src/routes/gateway/+page.svelte:29-36` 的 `load()` 已负责重新加载并同步全局 store；`src/lib/api/workspaces.ts:23-25` 已存在 `deleteWorkspace(id)`；当前任务列表 `src/routes/gateway/+page.svelte:158-177` 只有整行选择按钮，没有删除入口。
  - **涉及文件**: `src/routes/gateway/+page.svelte`，预计修改 45-70 行；现文件 185 行，无需拆分。
  - _需求: FR-1, FR-3_ ｜ _设计: 技术方案、设计决策 2_

- [x] 2.2 删除成功后刷新列表并恢复有效选中项，失败时保留页面并显示 Toast
  - **证据块**: 当前 `selectedId` 直接读取 URL query；删除当前项后若不主动导航，URL 会继续包含已删除 ID。
  - **涉及文件**: `src/routes/gateway/+page.svelte`，与 2.1 同一函数内完成，不新增模块。
  - _需求: FR-4_ ｜ _设计: 架构设计_

---

### 阶段 3: 回归验证

- [x] 3.1 对照 FR-1 至 FR-4 执行 Rust 单测、Svelte 检查和生产构建，不启动或停止桌面程序
  - **证据块**: `package.json` 已提供 `check`、`build`；Rust 核心单测可使用 `cargo test --lib` 避免覆盖当前运行的 `coding-tools-mcp-desktop.exe`。
  - **涉及文件**: 不新增测试基础设施；测试代码位于 `src-tauri/src/data/store.rs`。
  - _需求: FR-1, FR-2, FR-3, FR-4, NFR-4_ ｜ _设计: 测试策略_

---

## 检查点

- [x] 阶段 1 完成后：Rust 测试证明删除 target 会同时清理 gateway allowlist。
- [x] 阶段 2 完成后：`npm run check` 通过，删除交互不存在嵌套 button 和类型错误。
- [x] 阶段 3 完成后：`npm run build` 与 `cargo test --lib` 通过，主工作区仍保持 `main` 干净且当前程序未被操作。

---

## 需求覆盖矩阵

| 需求 ID | 设计章节 | 任务编号 | 状态 |
|---------|----------|----------|------|
| FR-1 | 技术方案 | 2.1, 3.1 | 已完成 |
| FR-2 | 数据模型、设计决策 1 | 1.1, 3.1 | 已完成 |
| FR-3 | 设计决策 2 | 2.1, 3.1 | 已完成 |
| FR-4 | 架构设计 | 2.2, 3.1 | 已完成 |

---

## 文件变更清单

| 文件 | 操作 | 行数预算 | 说明 |
|------|------|----------|------|
| `src-tauri/src/data/store.rs` | 修改 | 30-45 行 | 删除一致性和 Rust 单元测试 |
| `src/routes/gateway/+page.svelte` | 修改 | 45-70 行 | 删除按钮、确认、加载状态、删除后导航 |
| `docs/specs/delete-workspace-task-mcp/requirements.md` | 新建 | 约 100 行 | 需求和验收标准 |
| `docs/specs/delete-workspace-task-mcp/design.md` | 新建 | 约 120 行 | 技术方案和测试策略 |
| `docs/specs/delete-workspace-task-mcp/tasks.md` | 新建 | 约 90 行 | 实施清单和覆盖矩阵 |

---

## 检查清单

- [x] 交付物清单已锁定
- [x] 每条任务包含源码证据和文件预算
- [x] 每条任务回链到 FR 与设计章节
- [x] 需求覆盖矩阵无遗漏
- [x] 集成验证明确不影响当前运行程序
- [x] 文档无模板占位符

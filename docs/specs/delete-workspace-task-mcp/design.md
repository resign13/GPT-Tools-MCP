# 设计文档：delete-workspace-task-mcp

## 概述

本功能在现有 MCP 网关任务页补充删除入口，并把删除后的 Gateway allowlist 一致性下沉到 `DataStore::remove()`。前端只负责用户交互、宿主保护和删除后的页面状态；后端数据层负责一次保存内清理 profile、workspace secrets 与所有 allowlist 引用。

**对应需求:** FR-1, FR-2, FR-3, FR-4, NFR-1, NFR-2, NFR-3, NFR-4

---

## 技术方案

### 技术选型

| 类别 | 选择 | 理由 | 关联需求 |
|------|------|------|----------|
| 删除确认 | `@tauri-apps/plugin-dialog` 的 `confirm` | 项目已集成 dialog 插件，使用桌面原生确认体验，不新增依赖 | FR-1 |
| 删除调用 | 复用 `deleteWorkspace(id)` | 已映射到后端 `delete_workspace`，避免新增 IPC | FR-1, NFR-3 |
| 数据一致性 | 扩展 `DataStore::remove()` | profile 删除和 allowlist 清理可以在一次 `save()` 中完成，避免前端两次调用的中间态 | FR-2 |
| 宿主保护 | UI 只为非宿主任务展示删除按钮 | 避免普通任务清理功能意外中断当前单连接器网关 | FR-3 |

### 架构设计

```text
gateway/+page.svelte
  ├─ 用户点击非宿主任务的删除按钮
  ├─ native confirm
  └─ deleteWorkspace(id)
        ↓
Tauri delete_workspace
  ├─ drop_tunnel_workspace(id)
  ├─ runtime.drop_workspace(profile)
  └─ DataStore::remove(id)
        ├─ remove profile
        ├─ remove workspace secrets
        ├─ prune id from every gateway.workspace_ids
        └─ save once
        ↓
前端 load() → workspaces store → goto(fallback selection)
```

---

## 数据模型

不新增字段。只加强现有 `WorkspaceProfile.gateway.workspace_ids: Vec<String>` 的删除一致性约束：任何已被 `DataStore::remove(id)` 删除的 profile ID 不得继续存在于剩余 profile 的 allowlist 中。

---

## API 设计

| 方法/函数 | 路径/签名 | 入参 | 出参 | 关联需求 |
|-----------|-----------|------|------|----------|
| `deleteWorkspace` | `src/lib/api/workspaces.ts` | `id: string` | `Promise<void>` | FR-1 |
| `delete_workspace` | Tauri command | `id: String` | `AppResult<()>` | FR-1, FR-2 |
| `DataStore::remove` | `remove(&mut self, id: &str)` | Workspace ID | `AppResult<Option<WorkspaceProfile>>` | FR-2 |

现有 API 签名保持不变。

---

## 文件结构

```text
src/
└── routes/gateway/+page.svelte                 # 增加删除交互、确认和选中状态恢复
src-tauri/
└── src/data/store.rs                           # 删除时清理 gateway.workspace_ids + 单元测试
├── requirements.md
├── design.md
└── tasks.md
```

---

## 设计决策

### 决策 1: allowlist 清理由数据层保证（关联需求: FR-2）

**问题**: 删除任务需要同时移除宿主 allowlist 引用。如果前端先 `updateWorkspace(host)` 再 `deleteWorkspace(id)`，任一步失败都会产生中间态。

**选项**:
1. 前端串联更新宿主和删除 profile。
2. 在 `DataStore::remove()` 中统一清理 profile、secrets 和所有 allowlist 引用。

**决策**: 选择选项 2。

**理由**: 数据不变量应由持久化层保证；该函数生产代码只有 `delete_workspace` 一个直接调用方，影响面低，并能一次保存完成一致性修改。

### 决策 2: 宿主不在任务列表直接删除（关联需求: FR-3）

**问题**: 删除当前宿主会停止 MCP/隧道并让所有会话入口失效。

**选项**:
1. 所有任务都显示删除按钮。
2. 只允许删除非宿主任务，宿主继续通过工作区配置管理。

**决策**: 选择选项 2。

**理由**: 当前需求是清理工作区任务，而不是设计宿主迁移流程；先保护在线网关，避免误操作。

### 决策 3: 不启动桌面端进行验证（关联需求: NFR-4）

使用 `npm run check`、`npm run build` 和 Rust 单元测试验证代码，不执行 `tauri dev`、不停止当前运行进程。

---

## 测试策略

- Rust 单元测试：构造 host + target profile，调用 `DataStore::remove(target)` 后断言 target 被删除、target secrets 被删除、host `gateway.workspace_ids` 不再包含 target。
- 前端静态检查：`npm run check`，确保 Svelte/TypeScript 无错误。
- 前端生产构建：`npm run build`，验证页面和 `@tauri-apps/plugin-dialog` 引入正确。
- Rust 核心回归：`cargo test --lib`，不运行/覆盖当前桌面 executable。
- 手工代码审查：确认 delete 按钮不会嵌套在任务选择 `<button>` 内，避免无效 HTML 和点击冒泡。

---

## 风险评估

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 删除按钮嵌套在现有整行 `<button>` | 中 | 将任务卡片改为容器，选择按钮和删除按钮作为同级交互元素 |
| 删除后 URL 仍指向不存在的 workspace | 中 | 删除成功后根据剩余列表显式 `goto` 到宿主/首项/空列表 |
| allowlist 残留已删除 ID | 高 | 在 `DataStore::remove()` 同一保存事务内 prune |
| 误删网关宿主导致在线 MCP 中断 | 高 | 页面不向宿主提供删除入口 |
| 开发时影响当前运行程序 | 中 | 独立 Git worktree；不启动、不重启、不停止桌面程序 |

---

## 检查清单

- [x] 技术方案复用现有 API 和架构
- [x] 每条 FR 都有实现路径
- [x] 不新增数据模型和 IPC 契约
- [x] 明确删除数据边界和宿主保护
- [x] 测试方案无需干扰当前运行程序

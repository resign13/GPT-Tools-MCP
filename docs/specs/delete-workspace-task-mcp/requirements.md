# 需求文档：delete-workspace-task-mcp

## 功能概述

为 MCP 网关的“工作区任务”列表增加安全删除能力。用户可以从列表删除不再需要的普通工作区任务，删除前必须二次确认；删除只移除 Coding Tools MCP 保存的工作区配置、运行时关联与密钥记录，不删除本地项目目录。当前网关宿主不允许从任务列表直接删除，避免误操作导致正在使用的单连接器网关失效。

## 历史经验与坑

- **可复用经验**: 前端已经存在 `deleteWorkspace(id)` Tauri API，后端已有 `delete_workspace` 命令负责停止目标工作区相关运行时/隧道并删除配置，可直接复用。
- **必须规避的坑**: `DataStore::remove()` 当前只删除 profile 和 workspace secrets，不会清理其他 profile 的 `gateway.workspace_ids`，如果只在前端加按钮会留下失效 allowlist。

## 术语定义

- **工作区任务**: 网关页面中可被 ChatGPT 会话绑定的 WorkspaceProfile。
- **网关宿主**: `gateway.enabled=true`、提供单一 MCP listener 的 WorkspaceProfile。

---

## 范围边界

**In Scope（本次要做）**
- 在 `/gateway` 工作区任务列表为非宿主任务增加删除入口。
- 删除前展示包含任务名称的原生二次确认对话框。
- 删除过程中防止重复点击，并在失败时显示错误 Toast。
- 删除成功后刷新本页和全局 workspace store，并自动选择剩余任务。
- `DataStore::remove()` 删除 profile 时同步从所有剩余 profile 的 `gateway.workspace_ids` 中移除该 ID，并与 profile 删除在同一次保存中持久化。
- 保持现有 `delete_workspace` 对运行时、隧道和工作区 secrets 的清理行为。

**Out of Scope（本次不做）**
- 删除磁盘上的真实项目目录或项目文件。
- 在网关任务列表直接删除当前网关宿主。
- 自动迁移或重新选择新的网关宿主。
- 修改 MCP Gateway 的 session binding 协议。

---

## 需求列表

### FR-1: 删除普通工作区任务

**优先级:** Must
**用户故事:** 作为 Coding Tools MCP 用户，我想从工作区任务列表删除不再使用的普通任务，以便保持任务列表整洁。

#### 验收标准（EARS）

1. WHEN 用户查看非宿主工作区任务 THEN 系统 SHALL 提供清晰的删除入口。
2. WHEN 用户触发删除 THEN 系统 SHALL 显示包含任务名称且明确说明“不会删除本地目录”的二次确认。
3. WHEN 用户取消确认 THEN 系统 SHALL 不修改任何工作区配置。
4. WHEN 删除正在进行 THEN 系统 SHALL 阻止同一删除操作重复触发。

### FR-2: 保持网关 allowlist 一致

**优先级:** Must
**用户故事:** 作为网关用户，我希望删除任务后网关配置同步更新，以便不会出现已删除 ID 仍被授权的脏数据。

#### 验收标准（EARS）

1. WHEN `DataStore::remove(id)` 成功删除一个 profile THEN 系统 SHALL 从所有剩余 profile 的 `gateway.workspace_ids` 中移除该 ID。
2. WHEN profile、workspace secrets 与 gateway allowlist 清理完成 THEN 系统 SHALL 只执行一次最终持久化保存。
3. IF 目标 ID 不存在 THEN `DataStore::remove(id)` SHALL 保持现有行为并返回 `None`，且不改写数据。

### FR-3: 保护当前网关宿主

**优先级:** Must
**用户故事:** 作为正在使用单连接器网关的用户，我希望任务列表避免误删宿主，以便删除普通任务不会中断网关服务。

#### 验收标准（EARS）

1. WHILE 某个任务是当前网关宿主 THEN 网关任务列表 SHALL 不提供可执行的删除按钮。
2. WHEN 用户查看宿主任务 THEN 系统 SHALL 保留“宿主”标识和现有配置入口。

### FR-4: 删除后的列表与选中状态

**优先级:** Must
**用户故事:** 作为用户，我希望删除完成后界面立即反映真实状态，以便继续操作其他任务。

#### 验收标准（EARS）

1. WHEN 删除成功 THEN 系统 SHALL 重新加载工作区列表并同步全局 `workspaces` store。
2. WHEN 被删除任务是当前选中任务 THEN 系统 SHALL 导航到宿主任务；若无宿主，则选择第一个剩余任务；若无任务，则回到 `/gateway` 空状态。
3. IF 删除失败 THEN 系统 SHALL 保持当前页面可用并显示错误 Toast。

---

## 非功能需求

- **NFR-1（性能）**: 删除操作不引入额外后台服务；列表刷新只调用现有 workspace/runtime API。
- **NFR-2（安全）**: 删除功能不得删除 Workspace 路径对应的本地目录；宿主不得通过网关任务列表直接删除。
- **NFR-3（兼容性）**: 不改变现有 `delete_workspace` Tauri IPC 名称和前端 API 签名，不改变 Gateway MCP 对外协议。
- **NFR-4（运行影响）**: 功能开发和验证不得停止、重启用户当前正在运行的 Coding Tools MCP 桌面程序。

---

## 依赖关系

- 依赖 `src/lib/api/workspaces.ts::deleteWorkspace`。
- 依赖 `src-tauri/src/commands/workspace.rs::delete_workspace` 的现有运行时/隧道清理链路。
- 数据一致性依赖 `src-tauri/src/data/store.rs::DataStore::remove`。
- UI 位于 `src/routes/gateway/+page.svelte`。

---

## 检查清单

- [x] 已确认现有删除 API 和后端命令可复用
- [x] 已识别 gateway allowlist 脏引用风险
- [x] 需求覆盖确认、取消、失败、宿主保护和删除后选中状态
- [x] 每条需求具有稳定 FR ID 并可独立验收
- [x] 范围明确排除真实目录删除和宿主迁移

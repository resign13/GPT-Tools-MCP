# 任务清单：app-update-and-repo-link

## 概述

实现侧边栏仓库入口、通用设置关于区块，以及基于 GitHub Releases 的手动检查更新。每条任务回链 FR 与设计章节。

> **二元禁令（零容忍）**：禁止出现未替换占位符、`TODO`、省略实现。

---

## 交付物清单（Scope-lock）

- **预计新建文件数**: 4 个
- **预计修改文件数**: 7 个
- **预计新增/修改函数数**: 约 12 个
- **交付物逐项列举**:
  1. `src-tauri/src/update/mod.rs`（新建）
  2. `src-tauri/src/commands/app_info.rs`（新建）
  3. `src/lib/app-links.ts`（新建）
  4. `src/lib/api/app-info.ts`（新建）
  5. `src-tauri/src/platform/open.rs`（修改，增加 open_url）
  6. `src-tauri/src/platform/mod.rs`（修改，导出）
  7. `src-tauri/src/commands/mod.rs`（修改，注册）
  8. `src-tauri/src/lib.rs`（修改，模块与 handler）
  9. `src/lib/components/AppShell.svelte`（修改，仓库入口）
  10. `src/routes/settings/general/+page.svelte`（修改，关于卡）
  11. `src/app.css`（修改，footer 链接样式）

---

## 任务列表

### 阶段 1: 后端能力与测试

- [x] 1.1 扩展 `platform/open.rs` 增加仅允许 http/https 的 `open_url`，并导出
  - **证据块**: 现状 `open_path_in_file_manager` 用 explorer/open/xdg-open 打开目录（`src-tauri/src/platform/open.rs`）。
  - **涉及文件**: `src-tauri/src/platform/open.rs`（+40 行）、`src-tauri/src/platform/mod.rs`（+1 行）
  - _需求: FR-4_ ｜ _设计: API 设计 / 决策 4_

- [x] 1.2 新建 `update` 模块：常量、tag 规范化、版本比较、JSON 解析与 `check_app_update` 核心逻辑，含单元测试
  - **证据块**: `tunnel/download.rs` 已有 `build_client` 与代理模式；本次独立短超时客户端，复用 `AppSettings.download` 代理字段。
  - **涉及文件**: `src-tauri/src/update/mod.rs`（新建，约 220 行，单文件不超 500）
  - _需求: FR-3, NFR-1, NFR-2_ ｜ _设计: 技术方案 / 数据模型_

- [x] 1.3 新建 `commands/app_info.rs` 暴露 `open_url` 与 `check_app_update`，并在 `commands/mod.rs`、`lib.rs` 注册
  - **证据块**: `lib.rs` 的 `invoke_handler` 与 `commands/mod.rs` 导出模式；`open_workspace_directory` 为同类薄封装。
  - **涉及文件**: `src-tauri/src/commands/app_info.rs`（新建，约 40 行）、`commands/mod.rs`、`lib.rs`
  - _需求: FR-1, FR-2, FR-3, FR-4_ ｜ _设计: API 设计_

### 阶段 2: 前端接入

- [x] 2.1 新增 `app-links.ts` 与 `api/app-info.ts`，封装仓库 URL 常量与 invoke
  - **证据块**: `src/lib/api/workspaces.ts` 使用 `invoke`；`src/lib/app-version.ts` 读取版本。
  - **涉及文件**: `src/lib/app-links.ts`、`src/lib/api/app-info.ts`（各约 40 行）
  - _需求: FR-1, FR-2, FR-3_ ｜ _设计: 文件结构_

- [x] 2.2 修改 `AppShell.svelte` 与 `app.css`：版本旁增加仓库按钮，失败时 dialog 提示
  - **证据块**: `AppShell.svelte` footer 已渲染版本号。
  - **涉及文件**: `src/lib/components/AppShell.svelte`、`src/app.css`
  - _需求: FR-1, NFR-4_ ｜ _设计: 决策 3_

- [x] 2.3 在 `settings/general/+page.svelte` 增加关于卡片：当前版本、打开仓库/Releases、检查更新与进行中状态
  - **证据块**: 通用页现仅有代理表单；复用 `@tauri-apps/plugin-dialog` 的 `message`/`ask`。
  - **涉及文件**: `src/routes/settings/general/+page.svelte`（+约 80 行）
  - _需求: FR-2, FR-3_ ｜ _设计: 决策 3_

### 阶段 3: 验证

- [x] 3.1 运行 update 模块单测与相关 `cargo test`，对照 FR-3/FR-4 验收标准核验
  - **证据块**: 新增 `#[cfg(test)]` 覆盖 normalize、compare、allowed url、JSON 解析。
  - **涉及文件**: `src-tauri/src/update/mod.rs`
  - _需求: FR-3, FR-4_ ｜ _设计: 测试策略_

- [x] 3.2 前端 `npm run check`（或项目既有 check）通过，确认无类型错误
  - **证据块**: 既有 Svelte check 流程。
  - **涉及文件**: 前端改动文件
  - _需求: FR-1, FR-2, FR-3_ ｜ _设计: 测试策略_

---

## 检查点

- [x] 阶段 1 完成后：`open_url` 拒绝非 http(s)；`cargo test` 中 update 相关用例通过。
- [x] 阶段 2 完成后：侧边栏可见仓库入口；通用页可检查更新并打开链接。
- [x] 阶段 3 完成后：对照 FR-1 至 FR-4 验收标准全部可演示或由测试覆盖。

---

## 需求覆盖矩阵

| 需求 ID | 设计章节 | 任务编号 | 状态 |
|---------|----------|----------|------|
| FR-1 | 决策 3 / API | 1.3, 2.1, 2.2 | 已完成 |
| FR-2 | 决策 3 / 文件结构 | 1.3, 2.1, 2.3 | 已完成 |
| FR-3 | 技术方案 / 决策 1-2 | 1.2, 1.3, 2.3, 3.1 | 已完成 |
| FR-4 | 决策 4 | 1.1, 3.1 | 已完成 |
| NFR-1 | 技术方案 | 1.2 | 已完成 |
| NFR-2 | 技术方案 | 1.1, 1.2 | 已完成 |
| NFR-3 | 技术方案 | 1.1, 1.2 | 已完成 |
| NFR-4 | 技术选型 | 2.2, 2.3 | 已完成 |

---

## 文件变更清单

| 文件 | 操作 | 行数预算 | 说明 |
|------|------|----------|------|
| `src-tauri/src/update/mod.rs` | 新建 | 220 | 检查更新核心 |
| `src-tauri/src/commands/app_info.rs` | 新建 | 40 | Tauri 命令 |
| `src/lib/app-links.ts` | 新建 | 20 | URL 常量 |
| `src/lib/api/app-info.ts` | 新建 | 40 | invoke 封装 |
| `src-tauri/src/platform/open.rs` | 修改 | +40 | open_url |
| `src-tauri/src/platform/mod.rs` | 修改 | +2 | 导出 |
| `src-tauri/src/commands/mod.rs` | 修改 | +5 | 注册 |
| `src-tauri/src/lib.rs` | 修改 | +10 | 模块与 handler |
| `src/lib/components/AppShell.svelte` | 修改 | +25 | 仓库入口 |
| `src/routes/settings/general/+page.svelte` | 修改 | +80 | 关于卡 |
| `src/app.css` | 修改 | +20 | footer 链接样式 |

---

## 检查清单

- [x] 交付物清单（Scope-lock）已填，实现后数量已逐项核对
- [x] 每条任务标题是「动词+对象+约束」的具体描述，无宽泛标题
- [x] 每条任务含证据块（先读后写）
- [x] 每条任务标注涉及文件与行数预算，超 500 行的有拆分方案
- [x] 任务分阶段合理，粒度可在单次提交内完成
- [x] 每条任务都回链到 FR 与 design 章节
- [x] 需求覆盖矩阵已填，无遗漏的 FR
- [x] 阶段 3 包含「对照验收标准核验」
- [x] 全文无未替换占位符 / TODO / 省略号占位（二元禁令）

# 任务清单：context-management-optimization

## 概述

实现基于 OpenViking 分层思想的本地 synopsis 导航：L0 为 manifest synopsis，L1 为有界 state/search，L2 为原文分页。每条任务回链到需求和设计，实施不得改变数字 Markdown 事实源或既有 MCP 工具表面。

## 交付物清单（Scope-lock）

- **预计新建文件数**: 3 个规格文档
- **预计修改文件数**: 4 个实现/测试文件
- **预计新增/修改函数数**: 约 8 个函数或方法
- **交付物逐项列举**:
  1. `docs/specs/context-management-optimization/requirements.md`
  2. `docs/specs/context-management-optimization/design.md`
  3. `docs/specs/context-management-optimization/tasks.md`
  4. `src-tauri/src/tools/history/model.rs` 的可选 synopsis 字段
  5. `src-tauri/src/tools/history/storage.rs` 的确定性 synopsis 派生和旧 manifest 兼容
  6. `src-tauri/src/tools/history/mod.rs` 的 L1 投影、搜索评分和可选输出字段
  7. `src-tauri/tests/history_session.rs` 的分层上下文回归测试

## 任务列表

### 阶段 1: 规格与影响边界

- [x] 1.1 确认 OpenViking 借鉴边界和本项目事实源约束
  - **证据块**: `docs/project-context.md` 将 Markdown/history 模块列为当前上下文入口；`docs/specs/history-session-archive/design.md` 规定数字 Markdown 是事实源、state/manifest 可重建；当前 `src-tauri/src/tools/history/storage.rs` 已有 `build_manifest`、`build_state`、`tokenize`。
  - **涉及文件**: `docs/specs/context-management-optimization/*.md`；规格文档总计约 320 行。
  - _需求: FR-1, FR-2, FR-3, FR-4, FR-5, NFR-1 至 NFR-4_ ｜ _设计: 技术方案、设计决策_

- [x] 1.2 固化 L0/L1/L2 数据与兼容契约
  - **证据块**: `src-tauri/src/tools/history/model.rs` 已有 `ManifestEntry`、`MemoryReference`、`SearchHit`；`src-tauri/src/tools/history/mod.rs` 的 bootstrap 有 64 KiB 预算，read 有 hash/cursor 契约。
  - **涉及文件**: `docs/specs/context-management-optimization/design.md`；约 170 行。
  - _需求: FR-2, FR-3, FR-5_ ｜ _设计: 数据模型、API 设计_

### 阶段 2: 核心实现

- [x] 2.1 扩展历史模型并保持旧 JSON 可反序列化
  - **证据块**: 修改前已确认 `ManifestEntry`、`MemoryReference` 和 `SearchHit` 为历史派生输出的唯一 Rust 模型入口；新增字段必须使用 serde default。
  - **涉及文件**: `src-tauri/src/tools/history/model.rs`，当前约 170 行，修改后预算不超过 210 行。
  - _需求: FR-1, FR-3, FR-4_ ｜ _设计: 数据模型_

- [x] 2.2 从结构化档案字段生成有界且脱敏的 synopsis
  - **证据块**: 修改前已确认 `storage::build_manifest` 扫描 `HistoryDocument`，`markdown::parse_initial_input_records`、`parse_checkpoint_records` 和 `redact_text` 已可复用。
  - **涉及文件**: `src-tauri/src/tools/history/storage.rs`，当前约 590 行；不新增文件，synopsis 辅助函数保持在现有模块并控制在 500 行约束内，必要时复用现有函数。
  - _需求: FR-1, FR-2, FR-5_ ｜ _设计: Synopsis 生成规则_

- [x] 2.3 将 synopsis 投影到 state references 并识别旧 manifest
  - **证据块**: 修改前已确认 `build_state` 由 report/manifest 生成 references，`history::search` 当前仅按 archive revision 复用 manifest。
  - **涉及文件**: `src-tauri/src/tools/history/storage.rs`、`src-tauri/src/tools/history/mod.rs`，合计约 850 行；不引入新模块。
  - _需求: FR-2, FR-3, FR-5_ ｜ _设计: 架构设计、API 设计、决策 3_

- [x] 2.4 提升 search 的字段权重并返回导航摘要
  - **证据块**: 修改前已确认 `search_score` 当前权重为 title 16、keywords 10、content 4，`SearchHit` 当前返回 snippet 和 sha256。
  - **涉及文件**: `src-tauri/src/tools/history/mod.rs`，当前约 800 行，修改后预算不超过 850 行。
  - _需求: FR-4, NFR-3_ ｜ _设计: 搜索排序、决策 4_

### 阶段 3: 集成测试与验证

- [x] 3.1 增加 synopsis、兼容、脱敏和排序回归测试
  - **证据块**: 修改前已确认 `src-tauri/tests/history_session.rs` 已覆盖 bootstrap、checkpoint、search、read、validate、路径安全、并发编号和 64 KiB 响应。
  - **涉及文件**: `src-tauri/tests/history_session.rs`，当前约 660 行有效测试主体；新增测试保持单文件约 900 行以内。
  - _需求: FR-1 至 FR-5, NFR-1 至 NFR-4_ ｜ _设计: 测试策略_

- [x] 3.2 执行格式化、受影响测试和完整静态检查
  - **证据块**: 修改前已确认仓库测试约定使用 `cargo test`、`cargo check` 和前端 `npm run check`；历史模块测试位于 `src-tauri/tests/history_session.rs`。
  - **涉及文件**: 不新增源文件；测试命令输出作为交付证据。
  - **验证结果**: `cargo check` 通过；完整 `cargo test` 通过 241 项测试；`cargo fmt` 和 `cargo clippy` 因本机未安装对应 Rust toolchain 组件未执行；`npm run check` 因未安装 `node_modules` 未执行。
  - _需求: FR-2, FR-3, FR-5, NFR-1, NFR-3_ ｜ _设计: 测试策略、风险评估_

- [x] 3.3 复核 Git diff 和实际符号影响范围
  - **证据块**: 修改前基线为 `dee2aa4`；提交前必须运行 GitNexus `detect_changes` 或其项目降级路径，确认变更仅落在 history 模块、测试和规格文档。
  - **涉及文件**: `git diff`、GitNexus 影响报告和最终 review 结果。
  - **验证结果**: GitNexus `detect-changes --scope all` 报告 6 个文件、26 个符号、4 条执行流、medium 风险；未发现计划外业务文件。
  - _需求: NFR-2, NFR-3, NFR-4_ ｜ _设计: 架构设计、风险评估_

## 检查点

- [x] 阶段 1 完成后：规格通过 `check_spec`，并确认实现前无未决范围问题。
- [x] 阶段 2 完成后：旧 manifest 可读、新 manifest 含 synopsis、state/search 暴露 synopsis，数字 Markdown hash 不变。
- [x] 阶段 3 完成后：受影响测试、完整检查、GitNexus 变更检测和代码审查均有真实输出。

## 需求覆盖矩阵

| 需求 ID | 设计章节 | 任务编号 | 状态 |
|---------|----------|----------|------|
| FR-1 | Synopsis 生成规则、数据模型 | 1.2, 2.1, 2.2, 3.1 | 已完成 |
| FR-2 | 架构设计、决策 2、测试策略 | 1.1, 1.2, 2.2, 2.3, 3.1, 3.2 | 已完成 |
| FR-3 | 数据模型、决策 3 | 1.2, 2.1, 2.3, 3.1, 3.2 | 已完成 |
| FR-4 | 搜索排序、API 设计 | 1.2, 2.1, 2.4, 3.1 | 已完成 |
| FR-5 | Synopsis 生成规则、风险评估 | 2.2, 2.3, 3.1, 3.2 | 已完成 |
| NFR-1 | 技术选型、测试策略 | 2.2, 3.1, 3.2 | 已完成 |
| NFR-2 | 架构设计、风险评估 | 2.2, 3.3 | 已完成 |
| NFR-3 | API 设计、决策 4 | 2.4, 3.1, 3.2, 3.3 | 已完成 |
| NFR-4 | 技术选型、决策 1 | 2.1, 2.2, 3.3 | 已完成 |

## 文件变更清单

| 文件 | 操作 | 行数预算 | 说明 |
|------|------|----------|------|
| `src-tauri/src/tools/history/model.rs` | 修改 | 210 | 增加 synopsis 可选字段 |
| `src-tauri/src/tools/history/storage.rs` | 修改 | 500 以内 | 派生 synopsis、manifest 版本、state 投影 |
| `src-tauri/src/tools/history/mod.rs` | 修改 | 850 | manifest 复用条件、search score/result |
| `src-tauri/tests/history_session.rs` | 修改 | 900 以内 | 分层导航和兼容回归 |
| `docs/specs/context-management-optimization/requirements.md` | 新建 | 220 | 需求和验收标准 |
| `docs/specs/context-management-optimization/design.md` | 新建 | 260 | 技术设计和风险 |
| `docs/specs/context-management-optimization/tasks.md` | 新建 | 220 | 实施任务和覆盖矩阵 |

## 检查清单

- [x] 交付物清单已锁定实现、测试和规格文件。
- [x] 每条任务标题均包含动作、对象和验收约束。
- [x] 每条任务含修改前证据块、涉及文件预算和需求/设计回链。
- [x] 任务按规格、实现、测试和审查阶段组织。
- [x] 需求覆盖矩阵覆盖 FR-1 至 FR-5 和 NFR-1 至 NFR-4。
- [x] 阶段 3 包含对照验收标准核验和实际变更检测。
- [x] 全文没有未完成的模板内容或待办占位。

# 需求文档：context-management-optimization

## 功能概述

本功能借鉴 OpenViking 的分层上下文思想，为现有 `history_session_*` 增加本地、确定性的上下文导航。目标用户是使用 ChatGPT/MCP 连接器持续开发项目的开发者和编程代理；目标是在新会话恢复时先获得小而有用的档案概览，只在确有需要时读取完整 Markdown 原文。

## 历史经验与坑

- **可复用经验**: 现有历史模块已经将数字 Markdown 作为事实源，将 `memory/state.json`、`memory/manifest.json` 和 `index.json` 作为可重建派生数据；bootstrap、search、read 均有响应或页大小上限。
- **必须规避的坑**: 不能把所有档案正文或递归摘要重新拼入 bootstrap；不能把远程 ChatGPT 未作为工具参数传入的转录当作已保存；不能为了语义检索越过 Workspace 边界或引入无界外部依赖。

## 术语定义

- **L0 synopsis**: 从单个档案已有标题、首次输入和最新 checkpoint 结构化字段确定性生成的短摘要，存储在 manifest 并随导航结果返回。
- **L1 state**: 现有有界 `MemoryState` 及搜索结果，用于决定需要查看哪些档案。
- **L2 detail**: `history_session_read` 返回的数字 Markdown 原文分页，保持事实完整性。
- **事实源**: 用户可审阅、可版本控制的数字 Markdown 档案；任何派生摘要都不能替代它。

---

## 范围边界

**In Scope（本次要做）**

- 为 manifest 档案增加有界、可脱敏、可重建的 `synopsis`。
- 在 `MemoryState.references` 和 `history_session_search` 命中项中返回 synopsis。
- 将 synopsis 纳入确定性搜索匹配和评分，并保持标题匹配优先。
- 让旧版缺少 synopsis 的 manifest 自动回退到当前档案扫描并在下次派生写入时升级。
- 补充序列化兼容、长度、脱敏、搜索导航和现有工具行为回归测试。

**Out of Scope（本次不做）**

- OpenViking 的 AGPL 代码、Python 服务、AGFS、SQLite、向量数据库、Embedding、LLM 或外部 HTTP 服务。
- 自动读取 ChatGPT 完整转录、后台会话捕获、UI 重构、OAuth/FRP/tunnel 改造。
- Workspace 权限模型、执行目录语义、既有 MCP 工具名称和必填参数的重构。

---

## 需求列表

### FR-1: 生成分层档案 synopsis

**优先级:** Must

**用户故事:** 作为跨会话恢复的编程代理，我想先获得每个历史档案的短 synopsis，以便判断是否需要读取完整原文。

#### 验收标准（EARS）

1. WHEN 系统扫描数字 Markdown 档案并构建 manifest THEN 系统 SHALL 从标题、最新有效 checkpoint 和最新首次输入记录中确定性生成 synopsis。
2. WHILE synopsis 被写入 manifest、state reference 或搜索结果 THEN 单条 synopsis SHALL 不超过 512 个 UTF-8 字节，并保持完整字符边界。
3. IF 档案没有结构化输入记录 THEN 系统 SHALL 使用安全的标题或首个正文片段作为回退 synopsis，而不是失败或读取工作区外文件。

### FR-2: 分层导航不改变事实源

**优先级:** Must

**用户故事:** 作为项目开发者，我想要摘要只是导航视图，以便精确恢复仍然可以读取未改写的历史原文。

#### 验收标准（EARS）

1. WHEN bootstrap 或 search 返回上下文导航 THEN 系统 SHALL 返回有界 state、引用和 synopsis，不返回全部档案正文。
2. WHEN 客户端需要精确细节 THEN 系统 SHALL 继续使用 `history_session_read` 按页读取原始 Markdown，并校验已有 content hash/cursor 契约。
3. IF 重建 manifest 或 state THEN 系统 SHALL 不修改已有数字 Markdown 的字节内容。

### FR-3: 兼容旧派生数据

**优先级:** Must

**用户故事:** 作为已有项目用户，我想升级后继续使用旧的 `memory/manifest.json`，以便历史归档无需手工迁移。

#### 验收标准（EARS）

1. WHEN 读取缺少 synopsis 的旧 manifest 或旧 state reference THEN 反序列化 SHALL 成功，并将缺失字段视为空值。
2. WHEN 旧 manifest 的 archive revision 与当前档案匹配但缺少 synopsis THEN search SHALL 使用当前扫描结果构建带 synopsis 的 manifest 视图。
3. WHEN bootstrap、checkpoint 或 validate repair 写入派生数据 THEN 系统 SHALL 写入当前 manifest/state 版本和 synopsis。

### FR-4: 改进确定性检索导航

**优先级:** Should

**用户故事:** 作为需要恢复历史决定的编程代理，我想按标题、摘要和关键词定位档案，以便减少无关的 L2 原文读取。

#### 验收标准（EARS）

1. WHEN `history_session_search` 收到非空 query THEN 系统 SHALL 对标题、synopsis、manifest keywords 和档案正文执行大小写无关的确定性匹配。
2. WHEN 同一 query 同时命中多个字段 THEN 标题匹配 SHALL 高于 synopsis 匹配，synopsis SHALL 高于普通正文匹配。
3. WHEN search 返回命中项 THEN 每个命中项 SHALL 返回稳定的 number、path、sha256、score、snippet 和 synopsis，并继续遵守 limit/cursor 上限。

### FR-5: 安全和有界性

**优先级:** Must

**用户故事:** 作为工作区管理员，我想让摘要生成和检索沿用现有安全边界，以便上下文优化不会扩大数据泄露或响应膨胀风险。

#### 验收标准（EARS）

1. WHEN synopsis 从用户输入或 checkpoint 字段生成 THEN 系统 SHALL 应用现有敏感信息脱敏规则，并且 synopsis 不得包含被识别的秘密值。
2. WHEN bootstrap 返回含有 synopsis 的 state THEN 序列化结果 SHALL 继续小于 64 KiB；超预算时 SHALL 按现有收紧策略报告截断而不丢失事实档案。
3. IF history_dir、number、path、cursor 或 session target 非法 THEN 系统 SHALL 继续返回结构化错误并 fail-closed。

## 非功能需求

- **NFR-1（性能）**: synopsis 只基于一次档案扫描和已有结构化解析生成；不启动外部进程，不执行网络请求；普通历史目录的新增 CPU/内存开销应与档案总字节数线性相关。
- **NFR-2（安全）**: 所有派生文件和结果继续位于当前 Workspace active root；旧 synopsis 缺失时只能从已扫描档案重建。
- **NFR-3（兼容性）**: 不改变五个 `history_session_*` 工具名称、现有必填参数、`tool_ok` envelope、读取分页和 session/workspace 校验。
- **NFR-4（可维护性）**: synopsis 生成必须是纯本地确定性逻辑，字段有 serde 默认值，避免引入新的运行时服务或用户配置。

## 依赖关系

- 依赖 `src-tauri/src/tools/history/` 的扫描、Markdown 解析、脱敏、派生文件原子写入和 UTF-8 分页能力。
- 依赖 `history_session_bootstrap`、`history_session_checkpoint`、`history_session_search`、`history_session_read` 和 `history_session_validate` 的现有契约测试。
- 参考 OpenViking 的 L0/L1/L2 分层设计，但不复制其代码和 AGPL-3.0 实现。

## 检查清单

- [x] 已消化现有历史归档的无损事实源、派生状态和有界响应经验。
- [x] 需求覆盖新档案、旧 manifest、搜索、读取、安全和失败路径。
- [x] 每条需求有唯一 ID，并将在 design.md / tasks.md 中被引用。
- [x] 验收标准使用可执行的 EARS 表述。
- [x] 已标注优先级、范围边界、非功能需求和依赖关系。

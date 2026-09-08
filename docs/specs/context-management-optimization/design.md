# 设计文档：context-management-optimization

## 概述

本设计在现有 `tools/history` 内增加一个确定性的档案 synopsis 层，借鉴 OpenViking 的 L0/L1/L2 加载顺序，但保持本项目的 Markdown 事实源、Workspace 安全边界和无外部服务约束。L0 是每个档案的有界 synopsis，L1 是现有有界 `MemoryState`、references 和 search 命中，L2 是 `history_session_read` 的原文分页。

**对应需求:** FR-1, FR-2, FR-3, FR-4, FR-5, NFR-1, NFR-2, NFR-3, NFR-4

## 技术方案

### 技术选型

| 类别 | 选择 | 理由 | 关联需求 |
|------|------|------|----------|
| 事实存储 | 现有数字 Markdown | 可审阅、可版本控制、重建不丢原文 | FR-2, NFR-2 |
| L0 派生索引 | `MemoryManifest.entries[].synopsis` | 与已有 title/keywords 同处，search 无需新存储 | FR-1, FR-3, FR-4 |
| L1 导航 | `MemoryState` 与 `SearchHit` | 保持 bootstrap 有界并将读取决策交给调用方 | FR-2, FR-4, FR-5 |
| L2 精读 | 既有 `history_session_read` | 保留 hash、cursor 和 UTF-8 安全分页 | FR-2, NFR-3 |
| 摘要算法 | Rust 本地结构化字段拼接 + UTF-8 字节截断 | 无模型成本、结果可测试、离线可用 | FR-1, NFR-1, NFR-4 |

### 架构设计

```text
history_session_bootstrap/checkpoint/search
                |
                v
       history::storage::scan
                |
                +--> markdown records --> bounded synopsis (L0)
                |
                +--> manifest/state references (L1)
                |
                +--> history_session_read -> original Markdown pages (L2)
```

`storage::build_manifest` 从已扫描 `HistoryDocument` 生成 synopsis。`MemoryReference` 和 `SearchHit` 只携带这个有界字段；不会把 synopsis 写回数字 Markdown，也不会将前序档案摘要复制到当前档案。manifest 版本升级用于识别旧派生文件，但 serde 默认字段保证旧 JSON 可读。

### Synopsis 生成规则

按以下顺序选择少量结构化信息，并在最终输出前脱敏和按 512 UTF-8 字节截断：

1. 档案标题作为稳定前缀。
2. 最新 revision 的首次用户输入作为“目标”。
3. 最新有效 checkpoint 的 `user_intent`、第一条 `findings` 或 `decisions` 作为“进展”。
4. 第一条 `remaining_issues` 或 `next_actions` 作为“待办”。
5. 没有结构化记录时，使用第一个安全正文片段回退。

仅保留非空字段，重复字段不重复追加。字段来自旧档案时也经过 `markdown::redact_text`，避免新增摘要出口暴露已存在的疑似秘密。

### 搜索排序

搜索 token 继续使用现有确定性 tokenizer。每个 token 的权重顺序为：完整/包含标题 24 分、synopsis 14 分、keywords 10 分、正文 4 分。总分相同则按更新时间和编号稳定排序。返回 synopsis 后，调用方可在 L1 决策是否使用 read 获取 L2。

## 数据模型

| 实体/字段 | 类型 | 约束 | 说明 |
|-----------|------|------|------|
| `ManifestEntry.synopsis` | `String` | serde default；最多 512 UTF-8 字节 | 单档案 L0 导航摘要 |
| `MemoryReference.synopsis` | `String` | serde default；由 manifest 投影 | state 的有界 L1 引用摘要 |
| `SearchHit.synopsis` | `String` | 最多 512 UTF-8 字节 | search 命中导航摘要 |
| `MemoryManifest.version` | `u32` | 当前写入版本为 3 | 让旧缺 synopsis manifest 在内存中回退重建 |
| `MemoryState.version` | `u32` | 保持现有版本兼容 | 新字段通过 serde default 读取旧 state |

manifest 的 `archive_revision` 继续由数字档案内容 hash 计算，不把派生 synopsis 作为事实版本；这样同一份档案的 synopsis 算法演进仍可重建。search 只有在 manifest 版本和 archive revision 都匹配时才复用持久化 manifest，否则使用当前扫描生成的 manifest。

## API 设计

| 方法/函数 | 路径/签名 | 入参 | 出参 | 关联需求 |
|-----------|-----------|------|------|----------|
| `storage::build_manifest` | `fn build_manifest(report: &ScanReport) -> MemoryManifest` | 扫描报告 | 含 synopsis 的 manifest | FR-1, FR-3 |
| `storage::build_state` | `fn build_state(report, manifest, current_number, timestamp, state_revision) -> MemoryState` | report、manifest、current number、时间戳和 revision | references 含 synopsis | FR-2 |
| `history::search` | `history_session_search` | 既有 query/limit/cursor | `SearchHit` 含 synopsis | FR-4 |
| `history::bootstrap` | `history_session_bootstrap` | 既有参数 | 有界 state 和现有 envelope | FR-2, FR-5 |

不新增 MCP 工具、不新增必填参数、不改变 `history_session_read` 的输入输出分页契约。

## 文件结构

```text
D:/Documents/ChatGPT/gpt-mcp-fork/
├── docs/specs/context-management-optimization/requirements.md
├── docs/specs/context-management-optimization/design.md
├── docs/specs/context-management-optimization/tasks.md
├── src-tauri/src/tools/history/model.rs       # 修改：synopsis 字段
├── src-tauri/src/tools/history/storage.rs     # 修改：生成/投影/版本兼容
├── src-tauri/src/tools/history/mod.rs         # 修改：manifest 复用条件和搜索评分/输出
└── src-tauri/tests/history_session.rs         # 修改：新分层契约回归
```

## 设计决策

### 决策 1: 采用确定性 synopsis，不引入语义基础设施（关联需求: FR-1, NFR-1, NFR-4）

**问题**: OpenViking 的语义检索和模型摘要需要独立服务、模型和索引生命周期，和桌面单体应用的本地、可审计目标不一致。

**选项**:

1. 结构化字段拼接并严格截断：离线、可复现、无需配置。
2. 接入 Embedding/LLM/向量数据库：语义能力更强，但增加部署、许可证、隐私和失败面。

**决策**: 选择选项 1。

**理由**: 当前首要收益是减少无意义的 L2 读取，不是替代语义检索；标题、用户意图、checkpoint 和关键词已经提供足够的确定性导航信号。

### 决策 2: synopsis 放在派生 manifest，不写入事实档案（关联需求: FR-2, FR-3）

**问题**: 把摘要写入当前 Markdown 会造成历史文件噪声和跨档案递归膨胀。

**决策**: 仅在 `MemoryManifest` 中保存，并从档案随时重建。

**理由**: 保持原文 hash 和用户审阅内容稳定；旧 manifest 缺字段时可透明重建。

### 决策 3: 版本化 manifest，字段 serde default（关联需求: FR-3）

**问题**: 旧 manifest 可能与新 archive revision 相同但没有 synopsis。

**决策**: 当前 manifest 写入版本为 3，复用条件同时检查版本和 archive revision；JSON 缺字段仍反序列化成功。

**理由**: 不破坏用户已有文件，且能确保新 search 不长期使用空 synopsis。

### 决策 4: 保持现有 MCP 表面（关联需求: FR-2, FR-4, NFR-3）

**问题**: 新增工具或修改 envelope 会放大远程连接器兼容风险。

**决策**: 只增加响应中的可选字段，沿用五个工具和原有必填参数。

**理由**: 旧客户端忽略新字段即可工作；新客户端获得 L0 导航，L2 读取仍明确可控。

## 测试策略

- 单元/集成测试验证 synopsis 从 title、initial input、checkpoint、fallback 的优先级和 512 字节 UTF-8 上限。
- 旧 manifest/state JSON 缺少 synopsis 时验证反序列化、search 重建和 repair 写入版本 3。
- checkpoint 含 Bearer、token、password 和私钥模式时验证 synopsis 不包含秘密值。
- search 验证标题优先于 synopsis、synopsis 优先于正文，结果字段和分页不变。
- bootstrap 验证 state references 带 synopsis、结果仍小于 64 KiB，已有数字档案 hash 不变。
- 执行既有 `cargo test --manifest-path src-tauri/Cargo.toml --test history_session` 和完整 Rust/前端检查。

## 风险评估

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 旧档案正文含未被旧规则识别的秘密 | 中 | synopsis 只取结构化少量字段并复用现有脱敏；不扩大 read 的权限范围 |
| synopsis 算法截断中文或扩展大小写导致越界 | 中 | 按 UTF-8 字节边界截断并用固定上限测试 |
| 新字段让 bootstrap 变大 | 中 | 单条 512 字节、references 既有数量上限、沿用 64 KiB 最终预算 |
| 旧 manifest 被错误视为有效 | 中 | manifest 版本必须为 3 且 archive revision 必须匹配，否则从扫描报告重建 |
| search 分数改变旧排序 | 低 | 仅增加 synopsis 权重，保留 score/update/number 稳定排序并补回归测试 |

## 检查清单

- [x] 技术方案与现有 Rust/Tauri 历史模块一致。
- [x] requirements.md 的 FR-1 至 FR-5 和 NFR-1 至 NFR-4 均有设计覆盖。
- [x] 文件结构使用当前真实文件路径。
- [x] 数据模型、兼容策略和 API 可测且不改变必填契约。
- [x] 关键决策均说明了 OpenViking 借鉴边界。
- [x] 测试策略覆盖正常、兼容、安全和有界性场景。

# 设计文档：app-update-and-repo-link

## 概述

在侧边栏与「设置 → 通用」暴露官方仓库入口，并提供基于 GitHub Releases API 的手动版本检测。打开链接走跨平台系统浏览器；更新检测走 reqwest，比较语义化版本后由前端对话框引导用户。

**对应需求:** FR-1, FR-2, FR-3, FR-4, NFR-1, NFR-2, NFR-3, NFR-4

---

## 技术方案

### 技术选型

| 类别 | 选择 | 理由 | 关联需求 |
|------|------|------|----------|
| 打开链接 | 扩展 `platform/open.rs`（explorer/open/xdg-open） | 与现有打开目录一致，不引入 opener 插件 | FR-1, FR-2, FR-4 |
| 版本检测 | reqwest GET Releases latest | 仓库已有 reqwest；无需 updater 插件 | FR-3 |
| 版本比较 | 自研轻量 semver（主.次.补丁） | 依赖面小，易单测；应用版本均为 x.y.z | FR-3 |
| 前端展示 | AppShell footer + settings/general 关于卡 | 无 About 对话框先例，贴合现有导航 | FR-1, FR-2 |
| 图标 | Lucide ExternalLink / Github / RefreshCw | 符合设计系统 | NFR-4 |

### 架构设计

```text
侧边栏「仓库」 / 通用「打开仓库|Releases|检查更新」
        │
        ▼
  src/lib/api/app-info.ts  (invoke)
        │
   ┌────┴────┐
   ▼         ▼
open_url   check_app_update
   │         │
   ▼         ▼
platform::open_url    update::fetch_latest_release
   │                  ├─ 读 CARGO_PKG_VERSION
   │                  ├─ GET api.github.com/.../releases/latest
   │                  └─ compare_versions → UpdateCheckResult
   ▼
系统默认浏览器
```

---

## 数据模型

不新增持久化存储。命令返回 DTO：

| 实体/字段 | 类型 | 约束 | 说明 |
|-----------|------|------|------|
| `UpdateCheckResult.current_version` | string | 非空 | 本地 `CARGO_PKG_VERSION` |
| `UpdateCheckResult.latest_version` | string | 去 `v` 后的 tag | 来自 `tag_name` |
| `UpdateCheckResult.update_available` | bool | — | latest > current |
| `UpdateCheckResult.release_url` | string | https URL | 优先 `html_url`，否则 Releases latest |
| `UpdateCheckResult.latest_tag` | string | 原始 tag | 展示用 |

常量集中在 `src-tauri/src/update/mod.rs` 与前端 `src/lib/app-links.ts`，两端 URL 保持一致。

---

## API 设计

| 方法/函数 | 路径/签名 | 入参 | 出参 | 关联需求 |
|-----------|-----------|------|------|----------|
| `open_url` | Tauri command | `url: String` | `AppResult<()>` | FR-1, FR-2, FR-4 |
| `check_app_update` | Tauri command | 无（读 AppState settings） | `AppResult<UpdateCheckResult>` | FR-3 |
| `open_url` (platform) | `fn open_url(url: &str) -> AppResult<()>` | http/https | `()` | FR-4 |
| `compare_versions` | `fn compare_versions(a, b) -> Option<Ordering>` | 版本字符串 | 比较结果 | FR-3 |
| `normalize_tag` | `fn normalize_tag(tag) -> String` | 可含 `v` | 规范化版本 | FR-3 |

---

## 文件结构

```text
docs/specs/app-update-and-repo-link/
├── requirements.md
├── design.md
└── tasks.md
src-tauri/src/
├── platform/open.rs              新增 open_url
├── platform/mod.rs               导出 open_url
├── update/mod.rs                 新增：常量、比较、HTTP 检查
├── commands/app_info.rs          新增：open_url / check_app_update
├── commands/mod.rs               注册导出
└── lib.rs                        invoke_handler 注册
src/
├── lib/app-links.ts              新增：仓库与 Releases URL 常量
├── lib/api/app-info.ts           新增：前端 invoke 封装
├── lib/components/AppShell.svelte  侧边栏仓库入口
├── routes/settings/general/+page.svelte  关于卡片
└── app.css                       footer 链接样式微调
```

---

## 设计决策

### 决策 1: 不做自动安装更新器（关联需求: FR-3）

**问题**: 是否用 tauri-plugin-updater 下载并安装？

**选项**:
1. 完整 updater：体验好但签名、渠道、CI 成本高。
2. 仅检测 + 打开浏览器下载：满足“找得到仓库/新版本”的用户反馈。

**决策**: 选择 2

**理由**: 用户痛点是找不到仓库与新版本入口；当前 Releases 已有安装包；避免扩大范围。

### 决策 2: 后端比较版本，前端只展示（关联需求: FR-3）

**问题**: 版本比较放前端还是后端？

**选项**:
1. 前端请求 GitHub：CORS/混合内容与代理不一致。
2. 后端 reqwest + 下载代理：与软件下载路径一致，可单测。

**决策**: 选择 2

**理由**: 已有代理配置；单元测试不依赖浏览器。

### 决策 3: 侧边栏轻入口 + 通用页完整操作（关联需求: FR-1, FR-2）

**问题**: 所有操作是否塞进 footer？

**选项**:
1. 全部放 footer：拥挤，难放检查更新状态。
2. footer 仅仓库链接；检查更新放通用设置关于卡。

**决策**: 选择 2

**理由**: 符合现有设置分区；footer 保持低密度。

### 决策 4: URL 白名单协议（关联需求: FR-4）

**问题**: 是否允许任意字符串交给系统打开？

**选项**:
1. 任意字符串。
2. 仅 http/https，拒绝其他协议。

**决策**: 选择 2

**理由**: 降低 `file:`、`javascript:` 等误用风险。

---

## 测试策略

- Rust 单测：`normalize_tag`、`compare_versions`（含 `v` 前缀、相等、更高、非法）、`is_allowed_url`（http/https 通过，其他拒绝）。
- Rust 单测：对 `parse_latest_release` 用固定 JSON fixture，不依赖真实网络。
- 手测：点击侧边栏与通用页链接；断网时检查更新错误可读；本地版本与伪造更高 tag 的比较逻辑由单测覆盖。

---

## 风险评估

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| GitHub API 速率限制或国内网络不通 | 中 | 复用下载代理；错误信息提示可手动打开 Releases |
| 前端与后端版本字符串短暂不一致 | 低 | 发布流程要求三处版本同步；比较以 Rust 包版本为准 |
| 系统无默认浏览器 | 低 | 返回明确错误文案 |

---

## 检查清单

- [x] 技术方案与现有架构一致
- [x] requirements.md 中每条 FR 都被本设计覆盖
- [x] 文件结构对照真实代码库，路径可定位
- [x] 数据模型 / 接口契约清晰（含类型与约束）
- [x] 关键设计决策已记录并关联需求
- [x] 测试策略可验证验收标准

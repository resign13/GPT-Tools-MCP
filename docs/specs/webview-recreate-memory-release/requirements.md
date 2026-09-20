# 需求文档：webview-recreate-memory-release

## 功能概述

长时间运行后 WebView2（HOST/renderer）内存可涨到数 GB。现有「释放界面内存」仅用 `window.location.reload()`，不会重建 `msedgewebview2` 进程，释放后仍可残留约 2GB。本功能改为在 Rust 侧 `destroy` 主窗口 WebView 并按配置重建，且不停止 MCP / Actions / FRP。超过约 2GB 时在冷却期内自动重建（不再只弹 toast）。

## 历史经验与坑（来自记忆库）

- 本次自动注入的记忆均为跨项目 UI/注册表问题，与 WebView2 进程重建无关；以本仓库实测为准：`location.reload()` 不换 WebView PID。
- **必须规避**: 重建窗口时不得调用 `stop_runtime` / `stop_tunnel`；Windows 上创建窗口须用 async command，避免 WebView2 死锁。

## 术语定义

- **UI 重建**: 销毁当前主 `WebviewWindow` 后按 `tauri.conf.json` 窗口配置新建同标签窗口。
- **冷却**: 两次 UI 重建间隔至少 1 小时（前端 localStorage）。

---

## 范围边界

**In Scope**
- Rust 异步命令 `recreate_ui_webview`：destroy → 短暂等待 → from_config 重建，尽量保留位置/尺寸/最大化。
- 前端所有释放路径（设置按钮、静默 hidden、阈值）改为 invoke 该命令；失败时回退 `location.reload()`。
- 可见且 WebView 工作集 > ~2GB 且冷却通过时自动重建。
- 版本升至 0.1.30 并打 Windows NSIS。

**Out of Scope**
- 修改 MCP/FRP 生命周期或杀主进程重启整个应用。
- 清理 WebView 用户数据目录 / `clear_all_browsing_data`（保留 localStorage 等）。
- 可见闲置（无操作）自动回收。

---

## 需求列表

### FR-1: Rust 重建主 WebView

**优先级:** Must

#### 验收标准（EARS）

1. WHEN 调用 `recreate_ui_webview` THEN 系统 SHALL destroy 当前主 WebviewWindow 并新建窗口，且不得停止 MCP/Actions/FRP。
2. WHEN 在 Windows 上执行该命令 THEN 系统 SHALL 使用异步命令路径，避免同步创建窗口死锁。
3. IF 重建成功 THEN 新的 `msedgewebview2` 进程启动时间 SHALL 晚于调用时刻（进程被替换）。

### FR-2: 前端统一走重建命令

**优先级:** Must

#### 验收标准（EARS）

1. WHEN 用户点击「释放界面内存」THEN 系统 SHALL invoke `recreate_ui_webview` 而非仅 `location.reload()`。
2. WHEN 静默条件满足（最小化/hidden ≥约 50 分钟且冷却通过）THEN 系统 SHALL 调用同一重建路径。
3. IF invoke 失败 THEN 系统 SHALL 回退为 `location.reload()`。

### FR-3: 高内存自动重建

**优先级:** Must

#### 验收标准（EARS）

1. WHEN 窗口可见且采样 WebView 工作集 ≥约 2048MB 且冷却通过 THEN 系统 SHALL 自动触发 UI 重建。
2. WHEN 冷却未过 THEN 系统 SHALL 不重复自动重建。

---

## 非功能需求

- 重建不得中断已运行的 MCP / Actions / FRP（进程与端口保持）。
- 重建后 UI 应在数秒内可交互；几何状态尽量保留。
- 冷却默认 1 小时，避免频繁闪窗。

## 依赖关系

- 依赖 Tauri 2 `WebviewWindow` / `WebviewWindowBuilder`。
- 依赖现有 `get_webview_memory_sample` 与 `ui-memory-guard` 采样/定时逻辑。
- 不依赖新的外部服务或插件。

## 覆盖矩阵

| FR | design | tasks |
|----|--------|-------|
| FR-1 | D-1 | T-1 |
| FR-2 | D-2 | T-2 |
| FR-3 | D-2 | T-2 |

# UI 内存保护机制导致桌面窗口闪退问题分析

## 1. 问题摘要

桌面程序出现“运行一段时间后自动闪退”的现象。当前证据表明，消失的是 Tauri/WebView2 主窗口，MCP、Actions 和隧道后台服务并没有同步崩溃。

主要故障链为：

```text
Windows 用户配置文件临时化
        +
UI 内存保护机制自动触发 WebView 重建
        ↓
WebView2 用户数据/窗口重建失败
        ↓
主窗口被销毁且没有恢复
        ↓
用户看到“程序闪退”，后台 MCP 可能仍在运行
```

这不是权限模式、Full Access、MCP listener 或 Cloudflare 隧道本身导致的崩溃。

## 2. 当前状态

| 项目 | 状态 |
| --- | --- |
| 分析分支 | `codex/permission-development` |
| 分析时间 | 2026-09-09 |
| 主要影响 | Tauri/WebView2 UI 窗口消失 |
| MCP 后台服务 | 证据显示仍可响应 |
| 自动 UI 内存保护 | 当前代码仍启用 |
| 权限重构 | 与本问题无直接因果关系 |
| 修复状态 | 已定位，尚未在本文件对应的分析时点完成代码修复 |

## 3. 复现和日志证据

### 3.1 最近一次窗口销毁

- 进程 PID：`49884`
- 进程启动：`2026-09-09 14:18:05`
- 生命周期日志中的 `window-destroyed`：`2026-09-09 15:12:07`
- 同一秒出现 Windows 用户配置文件事件：
  - `Microsoft-Windows-User Profiles Service` 事件 `1511`
  - `Microsoft-Windows-User Profiles Service` 事件 `1515`
  - `Kernel-General` 事件 `16`，涉及 `C:\Users\TEMP\ntuser.dat`

### 3.2 重复样本

旧进程 PID `49428` 在以下时间出现窗口销毁：

- `12:50:26`
- `13:50:27`

两个时间点同样伴随 User Profiles Service `1511/1515` 事件。该重复模式说明问题不是一次性的网络或 MCP 请求失败。

### 3.3 已排除的崩溃类型

- 未发现 `coding-tools-mcp-desktop.exe` 对应的 Windows WER `APPCRASH`。
- 未发现对应的 `Application Error` 记录。
- `lifecycle.log` 记录的是 `window-destroyed`，没有对应的 `exit-requested`。
- 窗口销毁后 MCP listener 仍可返回 HTTP `200`。

因此当前最强结论是“UI 窗口重建失败”，而不是 Rust 进程主动退出。

## 4. 相关代码路径

### 4.1 自动保护入口

文件：[`src/routes/+layout.svelte`](../src/routes/+layout.svelte)

根布局在 `onMount` 中调用 `startUiMemoryGuard()`。这会在整个桌面会话期间注册后台定时器。

### 4.2 自动触发条件

文件：[`src/lib/ui-memory-guard.ts`](../src/lib/ui-memory-guard.ts)

当前阈值如下：

| 条件 | 当前值 |
| --- | --- |
| 隐藏/最小化自动重建 | 持续约 50 分钟 |
| WebView 内存阈值 | 约 2048 MB |
| 采样周期 | 5 分钟 |
| 首次采样 | 启动约 60 秒后 |
| 两次重建冷却时间 | 60 分钟 |

满足隐藏时长或内存阈值后，`maybeSilentReload()` / `maybeAutoRecreateHighMemory()` 会调用 `reloadUiOnly()`。

### 4.3 前端重建流程

`reloadUiOnly()` 调用 Tauri 命令 `recreate_ui_webview`。如果调用失败，代码还会尝试 `window.location.reload()`；但如果原生窗口已经被销毁，前端上下文可能已经不存在，回退刷新无法恢复主窗口。

### 4.4 Rust 原生重建流程

文件：[`src-tauri/src/commands/ui_memory.rs`](../src-tauri/src/commands/ui_memory.rs)

流程为：

1. 设置重建中的保护状态。
2. 创建隐藏 keepalive 窗口，避免销毁主窗口时触发“最后一个窗口关闭”退出。
3. 销毁主 WebView。
4. 等待 WebView2 子进程退出。
5. 按 Tauri 配置重建窗口。
6. 配置构建失败时尝试生成恢复窗口标签。
7. 恢复位置、尺寸、最大化和隐藏状态。

文件：[`src-tauri/src/lib.rs`](../src-tauri/src/lib.rs) 和 [`src-tauri/src/commands/window_chrome.rs`](../src-tauri/src/commands/window_chrome.rs) 还会读取 `should_prevent_exit()`，用于防止重建期间被当成用户关闭。

## 5. 根因分析

### 5.1 环境层根因

Windows User Profiles Service 在窗口销毁时将当前用户切换到了临时配置文件，路径涉及 `C:\Users\TEMP`。这会影响依赖用户配置目录的桌面组件，尤其是 WebView2 的用户数据目录、锁文件和注册表配置。

### 5.2 代码层触发点

UI 内存保护机制会主动销毁并重建主 WebView。该操作是破坏性操作：一旦重建阶段无法打开 WebView2 用户数据目录、原窗口标签仍被占用，或 Tauri 窗口创建失败，主 UI 就可能消失。

### 5.3 综合因果结论

**Windows 用户配置文件临时化使 WebView2/Tauri 重建依赖失效；自动 UI 内存保护随后销毁主窗口并在恢复阶段失败，最终表现为桌面程序闪退。**

置信度：高。环境事件、生命周期日志、重复时间模式和代码触发链能够相互印证。

## 6. 与权限版本的关系

权限版本主要影响：

- `Ask` / `Auto Approve` / `Full Access` 的审批策略；
- 命令和网络的软权限门；
- Workspace、Git worktree 和 Sandbox 的硬边界。

这些逻辑不会修复 Windows 用户配置文件，也不会改变 WebView2 的用户数据目录。因此即使启用 `Full Access`，UI 内存保护问题仍可能发生。

## 7. 建议修复方案

### 7.1 第一阶段：立即停止自动重建

在 [`src/routes/+layout.svelte`](../src/routes/+layout.svelte) 中移除根布局对 `startUiMemoryGuard()` 的自动启动。

这样可以停止：

- 隐藏/最小化 50 分钟后的自动 WebView 重建；
- WebView 内存超过阈值后的自动 WebView 重建；
- 因自动重建失败导致的主窗口销毁。

设置页中的“刷新占用”和“释放界面内存”可以暂时保留为用户主动操作；如需彻底禁用所有 WebView 重建，再单独移除该手动入口和对应 Tauri command。

### 7.2 第二阶段：修复环境和重建兜底

如果以后要重新启用自动机制，应先完成：

1. 检查 Windows 用户配置文件是否正常加载，不允许在临时用户配置下自动重建。
2. 为 WebView2 用户数据目录增加可诊断的读写检查。
3. 重建失败时先恢复原窗口或创建可见恢复窗口，再释放 keepalive。
4. 记录明确的失败阶段：`profile`、`destroy`、`build`、`restore`。
5. 重建失败不能触发 `window.location.reload()` 的无限重试。
6. 只有用户明确点击时才允许执行破坏性重建。

## 8. 最小验证方案

停止自动机制后，验证范围控制如下：

1. `npm run check`
2. `cargo check --manifest-path src-tauri/Cargo.toml`
3. 启动桌面程序，确认 MCP、Actions 和隧道状态正常。
4. 检查日志中不再出现自动触发的 `ui-memory-guard` 重建记录。
5. 确认窗口隐藏或内存采样不会自动调用 `recreate_ui_webview`。
6. 手动入口若保留，只验证用户点击后仍有明确确认，并且失败不会导致后台服务退出。

不需要重复运行完整权限矩阵或跨语言 Sandbox 矩阵，因为它们不覆盖本问题的根因。

## 9. 回滚和注意事项

- 本问题修复不应回滚现有权限、Gateway 或 Workspace Execution Context 改动。
- 不要删除 `output/` 中的运行时诊断资料。
- 不要用 `git clean -fdx` 清理工作区。
- 停止自动内存保护后，长时间运行时 WebView 内存可能继续增长；这是稳定性和内存占用之间的明确取舍。
- 手动 WebView 重建仍属于高风险操作，直到 Windows 用户配置文件问题修复前不建议自动触发。

## 10. 验收标准

- 程序运行超过原自动触发周期时，主窗口不会因为内存保护机制被销毁。
- MCP、Actions 和 Cloudflare/FRP 隧道保持运行。
- 日志不再出现自动 `window-destroyed` 重建链路。
- 用户配置文件恢复正常后，手动重建失败能够显示明确错误并保留可用窗口。

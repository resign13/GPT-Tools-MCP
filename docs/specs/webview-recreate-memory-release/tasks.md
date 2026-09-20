# 任务列表：webview-recreate-memory-release

## T-1: Rust 命令与注册

对应需求：FR-1。

- [ ] 实现 `recreate_ui_webview`（async destroy + rebuild + 几何恢复 + 回退构建）
- [ ] 在 `commands/mod.rs` 与 `lib.rs` invoke_handler 注册
- [ ] `cargo check` 通过

## T-2: 前端与版本

对应需求：FR-2、FR-3。

- [ ] `ui-memory-guard.ts` / API / 设置页走重建命令；阈值自动重建
- [ ] 版本升至 0.1.30（package.json、Cargo.toml、tauri.conf.json）
- [ ] `svelte-check` 通过；打 NSIS 安装包

## 需求覆盖矩阵

| FR | 任务 | 验收要点 |
|----|------|----------|
| FR-1 | T-1 | destroy+rebuild；服务不停；新 WebView PID |
| FR-2 | T-2 | 设置/静默路径 invoke；失败回退 reload |
| FR-3 | T-2 | ≥2GB 且冷却通过自动重建 |

## 文件变更清单

- `src-tauri/src/commands/ui_memory.rs`
- `src-tauri/src/commands/mod.rs`
- `src-tauri/src/lib.rs`
- `src/lib/api/ui-memory.ts`
- `src/lib/ui-memory-guard.ts`
- `src/routes/settings/general/+page.svelte`（文案，可选）
- `package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json`

## 交付物清单

- 可用的 `recreate_ui_webview` 命令与前端接线
- 规格目录 `docs/specs/webview-recreate-memory-release/`
- Windows 安装包 `Coding Tools MCP_0.1.30_x64-setup.exe`

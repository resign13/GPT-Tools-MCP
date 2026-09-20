# Coding Tools MCP 启动配置

本文档对应当前 Rust + Tauri 版本，适用于本地开发、MCP 服务验证和严格 Windows AppContainer 执行环境。

版本说明（2026-09-07）：本文包含 `codex/permission-development` 分支的开发配置，远程 `main` 不一定已包含这些功能。当前 Git 仓库命令的 AppContainer 兼容修复尚未完成；以下验收结果是目标，不代表已经通过。

## 1. 环境准备

- Windows 10/11，当前用户可以读写项目目录。
- Node.js、npm、Rust toolchain、Cargo 已加入 `PATH`。
- Git for Windows 已安装；Python 和 Node 运行时路径可被 AppContainer 读取和执行。
- 项目位于本地 NTFS 目录。严格模式不支持把网络盘、不可写卷或不支持 ACL 的目录作为 Git Workspace。

检查版本：

```powershell
node --version
npm --version
rustc --version
cargo --version
git --version
```

## 2. 首次安装依赖

在项目根目录 `D:\it\codex-harness` 执行：

```powershell
npm install
cargo fetch --manifest-path src-tauri/Cargo.toml
```

## 3. 启动桌面程序

开发模式：

```powershell
npm run tauri dev
```

仅检查前端：

```powershell
npm run check
```

构建生产包：

```powershell
npm run build
npm run tauri build
```

启动后保持桌面程序运行，不要同时启动同一 Workspace 的旧 MCP 实例。

## 4. 配置一个 MCP Workspace

1. 在桌面程序中打开目标 Workspace。
2. 确认 Workspace 路径是目标 Git 仓库或目标 Git worktree。
3. 在 Runtime 配置中选择权限模式：
   - `Ask`：高风险能力返回审批要求。
   - `Auto Approve`：常规 Workspace 开发操作自动执行。
   - `Full Access`：跳过软权限门，但仍保留 Workspace、当前 worktree、受保护 `.git/.github`、资源上限和 AppContainer Sandbox。
4. 点击启动 MCP，确认状态为 `Running`。
5. 记录 MCP 地址，默认路径为：

```text
https://YOUR_HOST/mcp
```

本地联调也可以使用：

```text
http://127.0.0.1:PORT/mcp
```

## 5. Cloudflare 公网隧道

ChatGPT 网页连接器需要公网 HTTPS 地址。Cloudflare Tunnel 只需要域名和已登录的 `cloudflared`，不需要额外服务器。

1. 在 Cloudflare Zero Trust 创建或选择 Tunnel。
2. 在本机运行 `cloudflared`，复制 Tunnel Token 到桌面程序的隧道配置。
3. Public Hostname 使用未被其他网站占用的子域名，例如 `mcp.gingtto.store`。
4. Service 类型选择 `HTTP`，URL 填 MCP 本地监听地址，例如 `http://127.0.0.1:PORT`。
5. 启动 Tunnel，浏览器访问 `https://mcp.gingtto.store/mcp`，确认能收到 MCP 端点响应。
6. ChatGPT 连接器 URL 填完整的 `https://mcp.gingtto.store/mcp`，按 OAuth 页面完成授权。

不要修改仍用于其他网站的 `chensheng.space` DNS 记录。

## 6. ChatGPT 网页端使用

1. 在 ChatGPT 设置中打开连接器配置，添加 MCP HTTPS 地址。
2. 完成 OAuth 授权后新建对话。
3. 每个新对话首句明确目标任务或工作区任务 ID；网关会按会话绑定对应任务。
4. 同一对话生命周期内保持任务绑定，不要在对话中途切换目录；需要切换时新建对话。

## 7. 严格沙箱验证

以下测试文件目前仅存在于本地未提交修复中；待该修复交付后，在项目根目录执行最小运行时回归：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --test runtime_compatibility installed_toolchains_execute_in_strict_sandbox -- --nocapture
```

期望结果：

- `cmd`、`whoami`、Python、Node 和 Git 的 `sandbox_enforced` 为 `true`。
- Python/Node 可以在 Workspace 内写入测试文件。
- Git 操作只使用当前 execution root，不访问 sibling worktree。
- AppContainer 无法建立时返回 `EXEC_SANDBOX_UNAVAILABLE`，不回退到 direct execution。

## 8. 常见故障处理

### 程序闪退或端口占用

```powershell
Get-Process | Where-Object { $_.ProcessName -match 'codex|coding|mcp|cloudflared|frpc' }
Get-NetTCPConnection -State Listen | Sort-Object LocalPort
```

退出旧实例后只保留一份 MCP 和一份隧道进程，再重新启动桌面程序。

### `EXEC_SANDBOX_UNAVAILABLE`

确认 Workspace 位于本地 NTFS、当前用户有 ACL 修改权限，并检查 Python、Node、Git 安装目录可读可执行。Full Access 不会关闭 Sandbox，也不会允许失败后裸执行。

### `WORKSPACE_CONTEXT_MISMATCH`

确认当前任务绑定的 active worktree 与桌面程序显示路径一致。不要把父仓库根目录和 sibling worktree 混用；必要时重新打开任务或新建对话。

### Git 报 `Unable to read current working directory`

先确认使用的是最新分支代码和最新构建，再重试运行时回归。不要通过关闭 Sandbox 或手动放宽 sibling worktree ACL 来绕过该错误。

## 9. 停止流程

1. 在桌面程序中停止 MCP。
2. 停止 Cloudflare Tunnel。
3. 关闭桌面程序。
4. 如需确认无残留进程，再执行：

```powershell
Get-Process | Where-Object { $_.ProcessName -match 'codex|coding|mcp|cloudflared|frpc' }
```

## 10. 当前开发分支

当前修复在 `codex/permission-development` 分支进行。提交前需保留工作区未提交改动，执行最小测试和 GitNexus `detect_changes`；未经确认不要把功能分支直接合并到 `main`。

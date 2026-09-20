```powershell
npm run desktop:watch
```
`npm run dev` 仅启动前端开发服务器，不等同于完整桌面 MCP 服务。
## 第一步：配置本地 MCP 与工作区任务
1. 打开桌面端的网关管理页面，设置宿主 MCP。
2. 确认监听端口，例如 `28766`；后续所有示例均以此端口说明。
3. 在认证配置中启用 OAuth，准备界面提供的 Client ID、Client Secret 和授权口令。
4. 保存配置并启动 MCP。
5. 在“工作区任务”中新建任务，选择实际项目目录。
6. 复制任务 ID，供网页对话绑定使用。
在网关任务界面创建的任务会自动加入可绑定列表。删除任务只移除配置，不会删除对应本地目录。首次验证建议准备两个不同目录的任务。
如果沿用旧版独立工作区配置，需要确认已启用网关且目标任务在可访问列表内。
## 第二步：配置 Cloudflare 公网入口
### 选择哪种模式
| 模式 | 域名要求 | 地址特点 | 适用场景 |
| --- | --- | --- | --- |
| Named Tunnel | 自有域名已在 Cloudflare 激活 | 固定 HTTPS 地址 | 长期使用，推荐 |
| Quick Tunnel | 不需要自有域名 | 临时地址，重启可能变化 | 首次体验、临时测试 |
两种模式都不需要额外购买 VPS，也不要求家庭网络具有公网 IP。本地电脑、桌面 MCP 服务和隧道需要保持运行；电脑关机或休眠时远程调用会中断。
### 方案 A：Named Tunnel，长期使用
#### 1. 接入域名
1. 在 Cloudflare 添加域名。
2. 在域名注册商处，将 Nameserver 修改为 Cloudflare 为该域名分配的两个地址。
3. 等待 Cloudflare 域名状态显示 `Active`。
4. 为 MCP 选择独立子域名，例如 `mcp.example.com`。
已有网站可以继续使用其他主机名。为 MCP 创建新子域名即可，无需修改网站原有记录。
#### 2. 创建隧道并取得 Token
1. 进入 Cloudflare Zero Trust 控制台。
2. 在网络相关菜单中找到 Tunnels；不同界面可能显示在 Networks、Connectors 等入口下。
3. 创建隧道，连接器类型选择 `cloudflared`。
4. 为隧道命名，例如 `gpt-tools-mcp`。
5. 在安装连接器页面找到运行命令，复制其中的 Tunnel Token。
命令一般包含类似 `--token <TOKEN>` 或 `service install <TOKEN>` 的内容。桌面 Token 字段仅填 Token 本身，不粘贴整条命令。由桌面端管理隧道时，按桌面流程启动即可。
#### 3. 填写桌面端隧道配置
| 配置项 | 填写内容 |
| --- | --- |
| 隧道类型 | Cloudflare |
| 模式 | Named Tunnel |
| Tunnel Token | 上一步取得的 Token |
| 公网 URL | `https://mcp.example.com` |
| 网络代理 | 按本机网络情况选择；启用时使用全局代理配置 |
点击“保存配置”，再启动隧道。回到 Cloudflare，确认连接器在线后继续设置公开路由。
公网 URL 填根地址，不附加 `/mcp`。应用会为 MCP 和 OAuth 等接口分别生成完整地址。保存后配置和 Token 由应用持久化，下次使用同一应用配置无需重复填写。
#### 4. 配置公开主机名
在该隧道的 Public Hostname 或 Published application routes 页面添加：
| 字段 | 示例 |
| --- | --- |
| Subdomain | `mcp` |
| Domain | `example.com` |
| Path | 留空 |
| Service Type | HTTP |
| Service URL | `127.0.0.1:28766` |
保存路由后，最终 MCP 地址为：
```text
https://mcp.example.com/mcp
```
本地端口应与桌面 MCP 的实际监听端口一致。Path 留空，让 MCP、OAuth 授权和元数据路由都到达同一个本地服务。外部使用 HTTPS，本地服务使用 HTTP。

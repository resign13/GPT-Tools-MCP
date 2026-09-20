# 需求文档：Full Access + 宿主完全访问

## 功能概述

2026-09-18 用户批准的 host 方案取代旧 compatibility policy-only 实现方向。分支为 `codex/compatibility-full-access`，基线为 `77fd09a`。保留全部已有未提交改动，不提交、推送或替换运行服务。

## 范围边界

权限预设 default/full_access 与执行隔离 strict/compatibility/host 独立。最高组合为明确保存的 version=1、full_access、host。兼容 ACL 后端延期，旧 compatibility 不转换为 host。Windows 优先；没有实现的系统明确返回不支持。

## 需求列表

### FR-1 显式配置与权限交集

- WHEN 桌面保存 host THEN 系统 SHALL 必须同时为 full_access 和新配置版本 1，否则返回配置错误。
- WHEN 读取缺失字段或旧配置 THEN 系统 SHALL 保留 strict 默认值及原迁移限制，不自动提权。
- WHEN Gateway 合并策略 THEN 系统 SHALL host 和 target 都启用 host 才有宿主访问；隔离顺序 strict > compatibility > host，其他策略分别取交集。
- WHEN 保存新策略 THEN 系统 SHALL 当前 ToolContext 不就地升级；提示重启服务并重新绑定。

### FR-2 宿主执行与文件访问

- WHEN 有效策略为 host THEN 系统 SHALL 使用当前 Windows 用户令牌，不创建 AppContainer/受限令牌，不改 ACL，不自动 UAC 提权。
- WHEN 启动子进程 THEN 系统 SHALL 先挂入 kill-on-close Job 再恢复线程；失败关闭句柄并返回错误，不失败降级。
- WHEN host 调用文件/Patch/exec THEN 系统 SHALL 允许绝对路径、其他盘、父目录、sibling worktree、.git/.github；相对路径使用会话默认目录。
- WHEN exec 指定外部 cwd 或环境 THEN 系统 SHALL 仅影响该次子进程；任务绑定、历史与 Harness 存储不迁移。
- 保留工具可见范围、参数校验、超时、输出和进程管理；Patch 事务、预检和读回验证不变。

### FR-3 固定任务身份与可变 Git 状态

- WHEN host checkout/reset/rebase/detach THEN 系统 SHALL 分支及 HEAD 可刷新，命令 session 指纹不随其改变。
- WHEN 根目录或 Git 元数据对象被替换/移走 THEN 系统 SHALL 返回 WORKSPACE_CONTEXT_MISMATCH。
- WHEN 本会话命令完成 THEN 系统 SHALL 刷新 Harness 预期 Git 状态；后续外部变更继续按原基线规则报告。
- 原生 Git 查询固定绑定仓库；其他仓库用带明确 cwd 的 exec。

### FR-4 公共响应与桌面展示

- server_info、permission_status、check_exec_environment 共用隔离能力报告。
- host 返回 host_process、sandbox_enforced=false、sandbox_bypass=true、fallback_allowed=false 和实际 process_elevated。
- filesystem_scope 为 workspace/host，省略使用有效策略；显式不匹配返回错误，不用参数切换后端。
- Runtime/Actions 支持 host；Gateway 任务提供独立策略表单，显示最近活动上下文实际后端及待重启状态。

## 非功能需求

- NFR-1 保持受限路径与 strict 后端行为，不改变 OAuth、Cloudflare、域名、任务绑定协议。
- NFR-2 SessionStore、默认目录和历史按对话区分；host 文件系统共享，不宣称跨会话文件隔离。
- NFR-3 测试限于 permission、host_access 和构建/类型检查，不运行跨语言逃逸矩阵。

## 验收标准

- [x] 最终 cargo check、permission、host_access、npm check 通过。
- [x] 外部读写、自定义环境、Git 分支状态、双会话及进程树清理通过临时目录测试。
- [x] 构建桌面验证版本。
- [ ] 实际 Tauri 环境与两个真实 GPT 对话完成验收，单独记录，不以单测代替。

## 依赖关系

现有 ExecutionContext、Gateway、SessionStore、Windows Job 和 Patch 事务链路；前端依赖 Tauri IPC。无新增外部网络服务。

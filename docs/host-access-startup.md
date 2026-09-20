# 宿主完全访问：配置与启动

适用分支：`codex/compatibility-full-access`。本次开发未替换当前运行服务。

## 开启

1. 在宿主 MCP 的「配置 → 策略」选择 Full Access 和「宿主完全访问」，保存。
2. 在 Gateway「当前任务权限与执行隔离」为需要访问的目标任务分别保存相同组合。
3. 重启宿主 MCP。已有命令进程由服务 shutdown 清理；旧会话重新绑定任务。
4. GPT 新对话首句写明任务 ID 或名称，调用 bind_workspace，再初始化历史。
5. 调用 permission_status 或 check_exec_environment，确认有效状态：

```json
{"permission_mode":"full_access","isolation_policy":"host","execution_isolation_mode":"host","isolation_backend":"host_process","sandbox_enforced":false,"sandbox_bypass":true,"fallback_allowed":false}
```

两边必须都开启。任一方 strict，最终仍是受限后端；旧 compatibility 不会转为 host。桌面显示最近活动上下文，不代表所有会话的独立快照。

## 使用

exec_command 可提供外部绝对 workdir 和 env；省略 cwd 使用当前会话默认目录。PATH/PATHEXT 仅在该子进程环境中合并。Windows 命令使用 cmd 语义；PowerShell 脚本显式使用 `powershell -File` 或 `pwsh -File`。

filesystem_scope 省略即可；显式 host 只在有效 host 策略下通过。host 下请求 workspace 隔离会返回能力不匹配。跨仓库 Git 操作用带 workdir 的 exec，原生 git_status 等仍查询绑定仓库。

可以访问当前 Windows 用户有权限的外部目录和 .git/.github；没有额外 UAC 或 SYSTEM 提权。实际令牌状态以 process_elevated 为准。两个对话的默认目录和命令 session 分离，但宿主文件系统共享。

分支、HEAD 和 detached HEAD 可变；任务根目录及 Git directory/common directory 对象不可替换。目录身份变化需重新建立任务上下文。

## 恢复受限运行

将宿主或目标隔离保存为 strict，重启宿主服务并重新绑定。strict 后端失败返回原错误，不转为宿主执行。

## 本地构建

```powershell
npm run tauri -- build --debug --no-bundle
```

构建只产出验证程序；启动前先退出需要替换的旧程序，避免端口占用。真实 GPT 验收在临时目录完成，不对业务仓库做 reset 或删除操作。

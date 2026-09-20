# 设计文档：Full Access + Host

## 概述

对应 FR-1–FR-4、NFR-1–NFR-3。host 是显式独立后端；原 compatibility ACL 实验延期。沿用现有工具调度和 Gateway 绑定。

## 技术方案与模块

| 模块 | 实现 | 需求 |
|---|---|---|
| security/permission.rs | Host 枚举、配置版本校验、restrictive 交集、host_access 判定 | FR-1 |
| security/exec_sandbox | Host 模式及真实状态；Windows 共享句柄/Job 基础设施但跳过 AppContainer 和 ACL | FR-2 |
| tools/context、execution_context | 固定执行根与 Git 目录；卷号/文件索引检测目录替换；分支/HEAD 可刷新 | FR-2、FR-3 |
| tools/exec、session | cmd shell 接收本次合并环境 PATH/PATHEXT；后台 session/超时/整树清理 | FR-2 |
| tools/policy、patch | 仅 host 放开外部路径及受保护资产；保留参数、大小与事务验证 | FR-2 |
| mcp/gateway、workspace_context | Host/Target 交集；host pin 允许分支变化，身份和 TTL 仍校验 | FR-1、FR-3 |
| harness/state | host 命令完成时刷新预期 Git 状态，保留执行前 baseline 检查 | FR-3 |
| tools/dispatch、registry | 公共能力报告、scope schema、模式不匹配结构化错误 | FR-4 |
| runtime/diagnostics | Weak<ToolContext> 只读引用，用真实活动上下文提供桌面诊断，不维护另一份策略 | FR-4 |
| RuntimePolicyForm、ActionsPolicyForm、Gateway 页面 | host 选项、目标任务配置、活动后端及待重启展示 | FR-1、FR-4 |

## 执行流程

配置解析 → Gateway 逐维交集 → 新建 ToolContext → 校验固定执行身份 → 策略与参数检查 → 显式选后端 → Windows suspended process → Job 绑定 → resume → 收集输出/维护 session → 完成与清理。

不因 strict 初始化失败选择 host。host 可跨目录访问，但默认目录和 Git 查询对象仍属于绑定任务。不存在路径沿已有父目录解析，symlink 解析后再根据有效模式检查。

## API 设计

权限状态保留既有字段，增加/统一 isolation_policy、execution_isolation_mode、isolation_backend、sandbox_bypass、fallback_allowed、workspace_exec_available、process_elevated、isolation_capabilities。host 的 read/write/network 均为 host 且 enforced=false。

exec_command.filesystem_scope 无固定 schema 默认值。host 请求 workspace 返回 ISOLATION_CAPABILITY_UNSATISFIED；受限上下文请求 host 返回 EXTERNAL_EXECUTION_NOT_ALLOWED。命令结果提供实际 scope、backend 和 resolved_cwd。

桌面 get_execution_policy_status 只读；返回最近活动上下文状态、当前配置的交集及 restart_required，不向 MCP 暴露提权接口。

## 生命周期

ToolContext 持有稳定策略快照。诊断只保存弱引用，不延长服务生命。后台命令由 SessionStore/Job 持有；超时、kill 和服务 shutdown 终止进程树。本会话命令完成后按固定身份重新验证再更新 Harness 基线。任意外部进程与运行中命令同时修改文件的归属不由本功能推断。

## 测试策略

permission：旧值、配置、权限交集。host_access：外部 Patch/读取、自定义 PATH/HOME、Git/Python/Node/npm、.git/.github、分支/HEAD、替换目录、双会话、超时和服务关闭。Gateway 单测补充真实构造上下文的交集与 pin/detach。

## 风险与回退

Host 使用当前账户能力，不代表管理员。目录对象身份检查不是任意子进程的路径沙箱。恢复时将宿主或目标隔离保存 strict，重启宿主并重新绑定；compatibility 不转换。桌面与真实 GPT 验收须在临时任务进行，现有运行服务本轮不替换。

## 文件结构

后端位于 `src-tauri/src/security/exec_sandbox/`、`tools/`、`mcp/`、`harness/`。新增 `runtime/diagnostics.rs` 与 `tests/host_access.rs`；前端新增 `src/lib/components/ExecutionPolicyStatus.svelte`，复用两份策略表单；配置文档位于 `docs/host-access-startup.md`。

# 需求文档：workspace-context-lock

## 功能概述

为单 MCP 网关增加会话级 Developer Session / Workspace Context Lock。ChatGPT 对话绑定工作区后，可进一步锁定该工作区内部的一个 Git worktree；锁定期间所有项目工具必须使用同一活动根、分支和 Git worktree 身份，发现上下文漂移时失败关闭，不得回退父仓库继续执行。

## 历史经验与坑

- **可复用经验**: 网关已为每个对话维护独立 `Binding` 和 `ToolContext`，适合作为活动 worktree 的所有权边界。
- **必须规避的坑**: 仅给路径参数增加 `default_cwd` 前缀并不等于切换 Workspace；`git_diff`、Harness 和历史工具仍可能固定使用配置工作区根。

---

## 范围边界

- **In Scope**: 网关模式的会话级 worktree 锁；pin/get/unpin MCP 工具；活动 ToolContext 重建；Git 身份校验；主分支保护；Patch 落盘复核；初始化指令与自动化测试。
- **Out of Scope**: UI 管理页、跨重启持久化、自动创建/删除 Git worktree、GPT Actions 路由、单工作区非网关模式、权限等级功能。

---

## 需求列表

### FR-1: 锁定活动 Git worktree

**优先级:** Must
**用户故事:** 作为通过 ChatGPT 使用 MCP 的开发者，我想把当前对话锁定到明确的 Git worktree，以便后续操作不会落到父仓库。

#### 验收标准（EARS）

1. WHEN 已绑定工作区的会话调用 `pin_workspace_context` THEN 系统 SHALL 解析工作区内的现有目录并确认其等于 Git toplevel。
2. WHEN pin 成功 THEN 系统 SHALL 记录配置工作区根、活动根、Git common dir、Git dir、分支、初始 HEAD、最近 HEAD、会话来源与过期时间。
3. IF 路径位于工作区外、不是 Git worktree 根、处于 detached HEAD 或预期分支不匹配 THEN 系统 SHALL 返回结构化错误且保持原上下文不变。
4. IF 分支为 `main` 或 `master` 且未同时提供 `allow_protected_branch=true` 与 `confirm=true` THEN 系统 SHALL 拒绝 pin。

### FR-2: 统一工具执行根

**优先级:** Must
**用户故事:** 作为开发者，我想让所有项目工具共享锁定后的活动根，以便读写、Git、命令和项目状态一致。

#### 验收标准（EARS）

1. WHEN pin 成功 THEN 系统 SHALL 用活动 worktree 创建新的 `ToolContext`，包括 Workspace、default cwd、SessionStore、Harness 和历史环境。
2. WHILE 上下文已锁定 THEN read/list/search/view、Patch、exec、Git、命令 session、Harness 和历史工具 SHALL 仅使用活动 worktree 根。
3. IF 工具参数尝试访问活动根之外的写路径 THEN 系统 SHALL 沿用 Workspace 路径边界拒绝执行。
4. WHEN 多个对话分别 pin 不同 worktree THEN 系统 SHALL 保持上下文、命令 session、Harness 和工作区锁相互隔离。

### FR-3: 每次调用检测上下文漂移

**优先级:** Must
**用户故事:** 作为开发者，我想在工具执行前发现目录或 Git 身份漂移，以便系统停止而不是修改错误目录。

#### 验收标准（EARS）

1. BEFORE 已锁定会话执行项目工具 THEN 系统 SHALL 校验活动目录仍存在、Git toplevel/Git dir/common dir/分支仍匹配。
2. IF 最近 HEAD 发生变化且新 HEAD 是最近 HEAD 的后代 THEN 系统 SHALL 更新最近 HEAD 并继续，以支持正常提交。
3. IF HEAD 非快进变化、分支变化、worktree 被移除/替换或锁过期 THEN 系统 SHALL 返回 `WORKSPACE_CONTEXT_MISMATCH` 或 `WORKSPACE_CONTEXT_EXPIRED`，且不调用工具内核。
4. WHEN 配置工作区被删除、移出 allowlist 或配置指纹变化 THEN 系统 SHALL 继续沿用现有网关撤销规则。

### FR-4: 查询与解除锁定

**优先级:** Must
**用户故事:** 作为开发者，我想查询真实执行上下文并显式结束 Developer Session，以便完成前核验和受控恢复。

#### 验收标准（EARS）

1. WHEN 调用 `get_workspace_context` THEN 系统 SHALL 返回锁定状态、配置根、活动根、分支、初始/当前 HEAD、剩余有效期和漂移状态。
2. WHEN 调用 `unpin_workspace_context` 且 `confirm=true`、无运行中命令 session THEN 系统 SHALL 重建配置工作区根上下文并解除锁。
3. IF `confirm` 缺失或存在运行中命令 session THEN 系统 SHALL 拒绝解除且保持活动上下文。
4. WHEN锁过期或已漂移 THEN `get_workspace_context` 和 `unpin_workspace_context` SHALL 仍可用于诊断和恢复。

### FR-5: 写入结果可验证

**优先级:** Must
**用户故事:** 作为开发者，我想让 Patch 成功响应证明文件已经落盘，以便避免只根据解析结果误报完成。

#### 验收标准（EARS）

1. AFTER 非 dry-run Patch 写入 THEN 系统 SHALL 重新读取或检查每个目标，并与事务期望内容/删除状态一致。
2. IF 任一目标复核失败 THEN 系统 SHALL 返回 Patch 错误，不得返回 `ok=true` 的 `affected_files`。
3. WHEN复核成功 THEN 响应 SHALL 包含 `verified=true`，并由网关附带当前锁定上下文摘要。

### FR-6: 引导模型完成上下文核验

**优先级:** Should
**用户故事:** 作为 ChatGPT 用户，我想让模型在 worktree 开发前后主动核验上下文，以减少错误操作和阶段性误报。

#### 验收标准（EARS）

1. WHEN 网关初始化 THEN instructions SHALL 要求 worktree 任务先 pin，再执行实现。
2. BEFORE 模型报告任务完成 THEN instructions SHALL 要求调用 `get_workspace_context`、`git_status` 和 `git_diff`。
3. IF 只有 `affected_files` 而没有回读/Git 证据 THEN instructions SHALL 明确不得声称落盘成功。

---

## 非功能需求

- **NFR-1**: 每次 Git 身份校验最长 5 秒；不得在日志中记录原始会话密钥或认证信息。
- **NFR-2**: 锁状态仅存内存，默认 120 分钟，允许 5-480 分钟；应用重启或网关绑定过期后消失。
- **NFR-3**: 网关关闭时现有工具清单和单工作区行为保持不变。
- **NFR-4**: 不修改现有 `call_tool` 工具调度内核的公共签名。

---

## 依赖关系

- 依赖现有网关 `Binding`、配置指纹、会话别名、并发锁和 `ToolContext` 创建流程。
- 依赖本机 Git CLI 提供 worktree 身份与祖先关系校验。

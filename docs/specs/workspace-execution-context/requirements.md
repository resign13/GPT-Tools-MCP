# 需求文档：workspace-execution-context

## 功能概述

为每个 `ToolContext` 增加明确的仓库根、执行根和 Git 身份模型，消除 `Workspace.root()`、活动 worktree 与 `default_cwd` 的隐式混用。网关 pin、普通 MCP、Actions 和测试上下文都通过同一套执行上下文入口运行；当路径、分支或 Git worktree 身份漂移时，工具在调度内核前失败关闭，避免请求落回父仓库。

## 历史经验与坑

- **可复用经验**: `workspace-context-lock` 已证明网关会话应在活动 worktree 上重建完整 `ToolContext`，并在每次路由前校验 Git 身份；该能力作为本功能的上游输入。
- **必须规避的坑**: 仅设置 `default_cwd` 会让文件工具看似位于子目录，但 Git、Harness、History 或命令解析仍可能使用 `Workspace.root()`；执行根必须成为独立状态，并在统一入口校验。

## 术语定义

- **repository_root**: 当前 `ToolContext` 所属仓库或配置工作区的稳定根，用于标识仓库边界和显示归属。
- **execution_root**: 当前请求实际允许读写、执行命令和运行 Git 的活动目录；在 linked worktree 场景下可以不同于 `repository_root`。
- **ExecutionContext**: 保存两个根及可选 Git dir、common dir、branch、HEAD 观测值的会话级状态。
- **上下文漂移**: execution root 不存在、Git toplevel/dir/common dir/branch 不匹配、HEAD 非快进变化或 default cwd 越出 execution root。

---

## 范围边界

**In Scope（本次要做）**
- 新增 `src-tauri/src/tools/execution_context.rs`，定义可验证的 `ExecutionContext`。
- 扩展 `Workspace` 保留 repository root 与 active root，并保持 `root()` 兼容为 active root。
- 扩展 `ToolContext`，统一提供 execution root、Git 身份和前置校验 API。
- 让 dispatch、exec、Git、Patch、Harness/History 使用同一执行根语义；限制 `set_default_cwd` 不得越界。
- 将网关 pin 的 Git 身份传入新上下文，保留现有 pin/get/unpin 和会话隔离行为。
- 增加路径、Git 漂移、双会话和完整回归测试，并同步设计/任务文档。

**Out of Scope（本次不做）**
- 不新增 UI、Cloudflare、FRP、OAuth、GPT Actions 路由或权限等级。
- 不自动创建、删除或迁移 Git worktree。
- 不重构密钥存储、Harness 数据格式或 `call_tool` 公共函数签名。

---

## 需求列表

### FR-1: 建立统一执行上下文

**优先级:** Must
**用户故事:** 作为 MCP/Actions 调用方，我想知道请求所属的仓库根和真正执行根，以便同一个会话中的所有工具使用一致目录。

#### 验收标准（EARS）

1. WHEN 创建 `ToolContext` THEN 系统 SHALL 同时初始化 `repository_root` 与 `execution_root`，普通工作区默认两者相同。
2. WHEN 使用 linked worktree 创建上下文 THEN 系统 SHALL 保留配置仓库根、活动 worktree 根、Git dir、Git common dir、branch 和 HEAD 观测值。
3. WHEN 既有代码调用 `Workspace::root()` THEN 系统 SHALL 继续返回活动执行根，既有工具契约不因字段拆分失效。

### FR-2: 约束 default cwd 与路径边界

**优先级:** Must
**用户故事:** 作为开发者，我想让默认目录只能在当前执行根内变化，以便一次对话不能把后续工具切回另一个 worktree。

#### 验收标准（EARS）

1. WHEN 调用 `set_default_cwd` THEN 系统 SHALL 使用 execution root 解析目录，并拒绝绝对路径、路径穿越、符号链接逃逸和另一个 worktree 的路径。
2. WHEN 读取或显示 default cwd THEN 系统 SHALL 返回相对于 execution root 的稳定显示值，并同时提供解析后的绝对路径。
3. IF 旧调用通过内部 API 直接设置越界 cwd THEN 系统 SHALL 返回 `WORKSPACE_CONTEXT_MISMATCH`，不得静默写入状态。

### FR-3: 所有工具统一消费 execution root

**优先级:** Must
**用户故事:** 作为开发者，我想让文件、Patch、命令、Git、Harness 和 History 共享同一个执行根，以便不会出现“读取 worktree、写入父仓库”的分裂行为。

#### 验收标准（EARS）

1. BEFORE `call_tool` 分发项目工具 THEN 系统 SHALL 对 `ExecutionContext` 做前置校验，再应用默认 cwd 和策略参数。
2. WHILE execution root 已锁定 THEN read/list/search/view、Patch、exec、命令 session、Git、Harness 和 History SHALL 只使用该根及其允许子路径。
3. WHEN Git 工具执行 `status`、`diff`、`log`、`show` 或 `blame` THEN 系统 SHALL 以 execution root 作为 Git cwd。
4. WHEN `exec_command` 未显式传 workdir THEN 系统 SHALL 使用 execution root；命令解析的根路径也 SHALL 使用 execution root。

### FR-4: Git 身份漂移失败关闭

**优先级:** Must
**用户故事:** 作为开发者，我想在目录或分支发生漂移时停止执行，以便系统不会在 main 或其他 worktree 上继续修改。

#### 验收标准（EARS）

1. BEFORE 带 Git 身份的上下文执行工具 THEN 系统 SHALL 校验 execution root 存在、Git toplevel、Git dir、Git common dir 和 branch 均匹配。
2. IF HEAD 是上次观测 HEAD 的后代 THEN 系统 SHALL 更新观测值并继续；IF HEAD 非快进、branch 变化或 worktree 被替换 THEN 系统 SHALL 返回 `WORKSPACE_CONTEXT_MISMATCH`。
3. IF 校验失败 THEN 系统 SHALL 在调用工具内核前返回结构化错误，包含安全的 expected/actual 摘要且不得泄漏会话密钥。
4. IF 上下文没有 Git 元数据（例如普通非 Git 临时目录）THEN 系统 SHALL 保持现有非 Git 工具行为，同时继续执行路径边界校验。

### FR-5: 网关会话和现有模式兼容

**优先级:** Must
**用户故事:** 作为多对话网关用户，我想让每个对话的 active worktree 独立绑定，同时不破坏现有单工作区和网关能力。

#### 验收标准（EARS）

1. WHEN 网关 pin 一个 worktree THEN 系统 SHALL 原子替换该 Binding 的 Workspace 与 ExecutionContext，Harness、History、SessionStore 和 default cwd 使用同一 active root。
2. WHEN 两个会话绑定不同 worktree THEN 系统 SHALL 保持执行根、命令 session、Harness、History 和并发锁相互隔离。
3. WHEN 网关关闭或使用普通单工作区模式 THEN 系统 SHALL 保持既有工具目录、OAuth、Actions、Cloudflare 和 `call_tool` 签名不变。
4. IF profile、allowlist 或活动根配置变化 THEN 系统 SHALL 沿用现有 binding 撤销和 fail-closed 行为。

### FR-6: 可核验结果与回归证据

**优先级:** Should
**用户故事:** 作为维护者，我想通过测试证明执行根不会漂移，以便后续功能不会重新引入隐式目录状态。

#### 验收标准（EARS）

1. WHEN 在 worktree 上调用 Patch、exec、Git、Harness 和 History THEN 测试 SHALL 证明所有落盘和命令 cwd 均位于该 worktree。
2. WHEN 强制改变 branch、HEAD、Git dir 或 default cwd THEN 测试 SHALL 证明工具内核未被调用并返回 `WORKSPACE_CONTEXT_MISMATCH`。
3. WHEN 两个会话同时执行读写任务 THEN 测试 SHALL 证明不同 worktree 可并行且同一 worktree 的写操作仍受现有锁控制。
4. WHEN 执行项目门禁 THEN `npm run check`、`npm run build`、`cargo test` 与全目标 Clippy SHALL 通过。

---

## 非功能需求

- **NFR-1（性能）**: execution context 的路径校验不超过 5 秒；Git 校验使用有界超时，不在无界等待中阻塞 MCP 请求。
- **NFR-2（安全）**: 漂移失败关闭；错误和日志只包含工具名、会话哈希、工作区/根摘要，不包含原始会话密钥或认证信息。
- **NFR-3（兼容性）**: 保持 `Workspace::root()`、`ToolContext` 构造辅助函数、工具 schema 和 `call_tool(&ToolContext, ...)` 签名兼容。
- **NFR-4（可维护性）**: Git 身份和执行根逻辑集中在 `ExecutionContext`，避免在每个工具内复制 cwd 推导；新增生产模块不超过 500 行。

---

## 依赖关系

- 依赖 `workspace-context-lock` 已提供的网关 Binding、pin/get/unpin、ToolContext 重建和并发锁。
- 依赖 `Workspace` 的现有 canonicalize、路径边界和 symlink 防护。
- 依赖本机 Git CLI 的 `rev-parse`、`symbolic-ref` 和 `merge-base --is-ancestor`。

---

## 检查清单

- [x] 已消化上一轮审计与 workspace-context-lock 的历史经验，并明确避免 default cwd 漂移。
- [x] 需求覆盖普通 MCP、Actions、网关、Git worktree 和非 Git 临时目录。
- [x] 每条需求有唯一 ID（FR-1 至 FR-6），将在 design.md / tasks.md 中被引用。
- [x] 验收标准使用 EARS 格式且可测试。
- [x] 已标注优先级（MoSCoW）。
- [x] 范围边界（In/Out of Scope）明确。
- [x] 非功能需求可量化并包含兼容性约束。
- [x] 依赖关系完整。


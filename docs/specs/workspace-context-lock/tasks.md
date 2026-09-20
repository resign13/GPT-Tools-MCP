# 任务清单：workspace-context-lock

## 概述

实现网关会话级 Workspace Context Lock；每条任务回链需求与设计。

> 交付物中零容忍占位符、TODO 或省略实现。生产源码新增模块不超过 500 行。

---

## 交付物清单（Scope-lock）

- 预计规格文件数: 3
- 实际生产代码文件数: 7
- 预计任务数: 7

---

## 任务列表

### 阶段 1: 规格与影响面

- [x] 1.1 校验规格并分析 GatewayRouter/ToolContext/Patch 影响面
  - 证据块: `src-tauri/src/mcp/gateway.rs:60`, `src-tauri/src/tools/context.rs:10`, `src-tauri/src/tools/patch.rs:11`
  - 文件: `docs/specs/workspace-context-lock/*.md`
  - _需求: FR-1 至 FR-6_ · _设计: 技术方案、风险评估_

### 阶段 2: 核心实现

- [x] 2.1 新建 workspace_context.rs 实现 Git worktree 探测与漂移校验，≤500 行
  - 证据块: `src-tauri/src/mcp/gateway.rs:60-97`, `src-tauri/src/tools/git.rs:519-529`
  - 文件: `src-tauri/src/mcp/workspace_context.rs`（上限 ≤500 行）
  - _需求: FR-1, FR-3, FR-4_ · _设计: 数据模型、决策 2-3_
- [x] 2.2 扩展 Gateway Binding 接入 pin/get/unpin 和 ToolContext 原子替换
  - 证据块: `src-tauri/src/mcp/gateway.rs:383-668`, `src-tauri/src/mcp/gateway.rs:933-1068`
  - 文件: `src-tauri/src/mcp/gateway.rs`（新增预算 ≤220 行）
  - _需求: FR-1, FR-2, FR-4_ · _设计: 架构设计、决策 1_
- [x] 2.3 注册网关工具 schema 并更新初始化指令
  - 证据块: `src-tauri/src/mcp/gateway.rs:884-929`, `src-tauri/src/mcp/server.rs:42-55`
  - 文件: `src-tauri/src/mcp/mod.rs`, `src-tauri/src/mcp/server.rs`（新增预算 ≤45 行）
  - _需求: FR-4, FR-6_ · _设计: API 设计_
- [x] 2.4 增加 SessionStore 忙状态与 Patch 落盘复核
  - 证据块: `src-tauri/src/tools/session.rs`, `src-tauri/src/tools/patch.rs:51-113`
  - 文件: `src-tauri/src/tools/session.rs`, `src-tauri/src/tools/patch.rs`（新增预算 ≤80 行）
  - _需求: FR-4, FR-5_ · _设计: 风险评估_

### 阶段 3: 集成测试

- [x] 3.1 覆盖 pin、主分支拒绝、分支漂移、HEAD 快进、过期和 unpin
  - 证据块: `src-tauri/src/mcp/gateway.rs` 现有双会话与绑定撤销测试，`src-tauri/src/mcp/workspace_context.rs` 新增单元测试
  - 验收点: FR-1、FR-3、FR-4 全部验收标准
  - _需求: FR-1, FR-3, FR-4_
- [x] 3.2 验证两个会话锁定不同 worktree，Patch/Git/Harness 不回退父仓库
  - 证据块: `src-tauri/src/mcp/gateway.rs:1144` 双会话隔离测试，`src-tauri/tests/call_tool_contract.rs` 工具契约测试
  - 验收点: FR-2、FR-5、NFR-3
  - _需求: FR-2, FR-5_

---

## 检查点

- [x] 阶段 1 完成后：`check_spec` 通过，GitNexus impact 已记录并处理风险。
- [x] 阶段 2 完成后：相关 Rust 测试通过，所有控制工具返回结构化上下文。
- [x] 阶段 3 完成后：`npm run check`、`npm run build`、`cargo test`、`cargo clippy --all-targets -- -D warnings` 通过。

---

## 需求覆盖矩阵

| 需求 ID | 设计章节 | 任务编号 | 状态 |
|---------|----------|----------|------|
| FR-1 | 数据模型、API 设计 | 2.1, 2.2, 3.1 | 已完成 |
| FR-2 | 架构设计、决策 1 | 2.2, 3.2 | 已完成 |
| FR-3 | 决策 2-3 | 2.1, 3.1 | 已完成 |
| FR-4 | API 设计 | 2.1, 2.2, 2.4, 3.1 | 已完成 |
| FR-5 | 风险评估 | 2.4, 3.2 | 已完成 |
| FR-6 | API 设计 | 2.3 | 已完成 |

---

## 文件变更清单

| 文件 | 操作 | 行数预算 | 说明 |
|------|------|----------|------|
| `src-tauri/src/mcp/workspace_context.rs` | 新建 | ≤500 | Pin 数据、Git 探测和校验 |
| `src-tauri/src/mcp/gateway/workspace_context_gateway.rs` | 新建 | ≤300 | 控制工具、schema 和上下文原子替换 |
| `src-tauri/src/mcp/workspace_context_tests.rs` | 新建 | ≤150 | Git worktree 测试夹具与漂移回归 |
| `src-tauri/src/mcp/gateway.rs` | 修改 | ≤220 新增 | Binding、请求前校验和响应摘要 |
| `src-tauri/src/mcp/mod.rs` | 修改 | ≤5 新增 | 注册模块 |
| `src-tauri/src/mcp/server.rs` | 修改 | ≤40 新增 | 初始化指令与测试 |
| `src-tauri/src/tools/session.rs` | 修改 | ≤25 新增 | 活动命令查询 |
| `src-tauri/src/tools/patch.rs` | 修改 | ≤55 新增 | 落盘复核与测试 |

---

## 交付前自检

- [x] 无占位符、TODO 或省略注释
- [x] 生产源码新增模块 ≤500 行
- [x] 网关关闭时既有工具清单不变
- [x] 每个文件和任务均回链 FR

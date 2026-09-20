# 任务列表：Full Access + Host

## 交付物清单

2026-09-18 批准范围覆盖需求 FR-1–FR-4。旧八文件 compatibility 预算已由本方案取代。修改限于策略、执行后端、路径/Git 上下文、Gateway pin、Harness 完成基线、UI/诊断和文档；保留原脏工作区。

## 阶段 1：基线和规格

- [x] 1.1 记录分支 codex/compatibility-full-access、HEAD 77fd09a 和原有改动。
  - **证据块**：对应 design 模块表与 `docs/verification/host-access.md` 的检查结果；未完成项保留未勾选。
- [x] 1.2 更新 requirements/design/tasks 并校验规格。
  - **证据块**：对应 design 模块表与 `docs/verification/host-access.md` 的检查结果；未完成项保留未勾选。
- [x] 1.3 对策略、路径、进程、Patch 和 Gateway 进行 impact；UNKNOWN 以源码补查，CRITICAL Git 校验保持受限入口行为。
  - **证据块**：对应 design 模块表与 `docs/verification/host-access.md` 的检查结果；未完成项保留未勾选。

## 阶段 2：实现

- [x] 2.1 增加显式 Host 解析和 Gateway 交集（FR-1；permission、gateway）。
  - **证据块**：对应 design 模块表与 `docs/verification/host-access.md` 的检查结果；未完成项保留未勾选。
- [x] 2.2 接入 Windows 宿主 Job 执行、环境、输出与进程树清理（FR-2；exec_sandbox、exec、session）。
  - **证据块**：对应 design 模块表与 `docs/verification/host-access.md` 的检查结果；未完成项保留未勾选。
- [x] 2.3 统一外部文件/Patch/cwd 路径能力（FR-2；context、policy、patch）。
  - **证据块**：对应 design 模块表与 `docs/verification/host-access.md` 的检查结果；未完成项保留未勾选。
- [x] 2.4 固定目录对象身份，允许分支/HEAD 变化，刷新已完成任务状态（FR-3；execution_context、workspace_context、harness）。
  - **证据块**：对应 design 模块表与 `docs/verification/host-access.md` 的检查结果；未完成项保留未勾选。
- [x] 2.5 统一能力报告、scope schema 和 Runtime/Actions/Gateway 配置 UI（FR-4；dispatch、registry、runtime/diagnostics、Svelte）。
  - **证据块**：对应 design 模块表与 `docs/verification/host-access.md` 的检查结果；未完成项保留未勾选。
- [x] 2.6 新增启动配置文档和开发进度（FR-4）。
  - **证据块**：对应 design 模块表与 `docs/verification/host-access.md` 的检查结果；未完成项保留未勾选。

## 阶段 3：验证与交付

- [x] 3.1 最终 cargo check、permission、host_access、npm check；只在改动/失败后重跑。
  - **证据块**：对应 design 模块表与 `docs/verification/host-access.md` 的检查结果；未完成项保留未勾选。
- [x] 3.2 手工审查、code_review、GitNexus detect_changes。
  - **证据块**：对应 design 模块表与 `docs/verification/host-access.md` 的检查结果；未完成项保留未勾选。
- [x] 3.3 构建桌面验证程序，不替换运行服务。
  - **证据块**：对应 design 模块表与 `docs/verification/host-access.md` 的检查结果；未完成项保留未勾选。
- [ ] 3.4 实际 Tauri 与两个真实 GPT 对话验收，临时任务操作。
  - **证据块**：对应 design 模块表与 `docs/verification/host-access.md` 的检查结果；未完成项保留未勾选。
- [ ] 3.5 记录验收证据和未完成项，converge；不自动提交。
  - **证据块**：对应 design 模块表与 `docs/verification/host-access.md` 的检查结果；未完成项保留未勾选。

## 需求覆盖矩阵

| 需求 | 设计 | 任务 | 验证 |
|---|---|---|---|
| FR-1 | 配置与交集 | 2.1 | permission/host_access |
| FR-2 | 后端和路径 | 2.2、2.3 | host_access |
| FR-3 | Git/生命周期 | 2.4 | host_access + Gateway pin |
| FR-4 | API/UI | 2.5、2.6 | npm check + 状态一致性 |

## 文件变更清单

以 design 模块表及 git diff 为准；新增运行时诊断、状态组件、host_access 测试、配置与验证文档。

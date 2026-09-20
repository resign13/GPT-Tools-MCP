# Host Access 验证记录

日期：2026-09-18。分支 `codex/compatibility-full-access`，基线 `77fd09a2f8f9d05f4e8446b8c54582dd4f90b59e`。保留原未提交改动；本次没有提交、推送或更换运行服务。

## 检查结果

| 检查 | 结果 |
|---|---|
| check_spec | 4 条需求，0 error / warning |
| cargo check --manifest-path src-tauri/Cargo.toml | 通过，9 项既有 dead_code 提示 |
| cargo test --manifest-path src-tauri/Cargo.toml permission | 通过；旧网络测试改为断言 NETWORK_NOT_ALLOWED 错误前缀，网络仍拒绝 |
| cargo test --manifest-path src-tauri/Cargo.toml host_access | 1 项 Gateway + 9 项集成测试通过 |
| 后续定向 parent-exit 回归 | 通过；新增第 10 项集成测试，主进程退出后不残留持有管道的后代 |
| npm run check | exit 0，Svelte 0 errors / warnings；扫描 output 下参考项目时存在缺依赖/tsconfig 提示 |
| git diff --check | 通过 |
| architecture drift | 已提供真实源码/测试/设计证据，阶段门禁通过 |
| GitNexus detect_changes | 已执行并以 limit=1000 重跑；28 个 tracked 文件、116 符号、162 流程，CRITICAL |

GitNexus 统计包含本次开始前已有改动；CRITICAL 来自执行/诊断公共链路，不能等同于低风险或全仓正确证明。CLI 默认展示省略部分清单，新文件没有旧图谱节点，已另读源码审查。没有为本功能运行全套 Sandbox 或跨语言逃逸矩阵。

## 实际运行证据

临时目录中的宿主 runner 已执行外部 Patch/读取、绝对 Python 路径、自定义 HOME/PATH/PATHEXT、Git 查询、Python 子进程、Node 捕获子进程输出、npm build，以及 cmd 管道/引号语法。验证了 .git/.github host 可写与 strict 拒绝、branch/commit/reset/detached HEAD 后继续原生 Git 查询和 session 读取、目录对象替换拒绝、跨对话 session 拒绝、超时与服务 SessionStore shutdown 清理后代。

Gateway 测试使用真实配置合并构造上下文：单方 host 不获宿主访问；双方 host 生效；两个任务目录和 SessionStore 分离；pin 后 detached HEAD 可继续路由。

## 审查修正

- 保留原受限 Git probe，host 专用探针接受 detached HEAD。
- 用卷号/文件索引检测相同路径的目录替换，而非只比较路径字符串。
- Gateway pin 与 ToolContext 使用一致的 host 分支语义。
- 命令完成时刷新 Harness 预期分支/HEAD；下一次调用先收集已退出的 host sessions。
- host shell 不再使用 POSIX shell_words 阻断 cmd 合法单引号；环境结构和资源检查仍保留。
- 三个状态接口和 exec 错误结果显示实际 scope/backend，修复嵌套 host_scope_available 误报。
- 宿主 Job 关闭、启动失败、超时和主进程提前退出均清理其后代/句柄。
- 桌面诊断只保存 Weak<ToolContext>；不产生另一份运行时权限源。

## 交付与未验收项

桌面构建命令：`npm run tauri -- build --debug --no-bundle`。产物路径：`src-tauri/target/debug/coding-tools-mcp-desktop.exe`，最终哈希见 `output/host-access-artifact.json`。

下列验收保留为待完成，不使用集成测试替代：

- 从实际 Tauri 窗口启动服务后的 Git/Python/Node/npm 烟测。
- 两个真实 GPT 新对话绑定两个临时任务后的双向配置与并行验证。

本轮按约定不替换当前服务；上述两项需要启动验证版服务、明确保存双方 host 配置和重新绑定。启动步骤见 `docs/host-access-startup.md`。

## 证据文件

- `output/host-access-permission-tests.log`
- `output/host-access-tests.log`
- `output/host-access-parent-exit-test.log`
- `output/host-access-desktop-build.log`
- `output/host-access-detect-changes.txt`
- `output/host-access-architecture.json`

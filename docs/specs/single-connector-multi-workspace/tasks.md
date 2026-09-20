# Tasks: single-connector-multi-workspace

## 交付物清单

- Gateway model and migration-safe persistence.
- Session router, bounded binding cache, and isolated contexts.
- Gateway tools, authorization intersection, and redacted logging.
- `GatewayConfigForm` and authorization projection.
- Regression/security tests and a v0.3.0 build.

## 任务列表

- [ ] 1.1 Add defaulted gateway model and validate host allowlist. (FR-1)
  - 证据块: inspect `WorkspaceProfile` and persistence tests before editing.
  - Evidence: model diff and migration test result.
- [ ] 2.1 Add listener-level session router and isolated context cache. (FR-2)
  - 证据块: inspect listener/server call graph before editing.
  - Evidence: concurrent-session and expiry test result.
- [ ] 3.1 Add gateway tools and profile/policy intersection checks. (FR-3, FR-4)
  - 证据块: inspect tool registry and policy evaluator before editing.
  - Evidence: authorization, traversal, and redaction test result.
- [ ] 4.1 Add `GatewayConfigForm`, authorization projection, and restart hints. (FR-5)
  - 证据块: inspect existing workspace forms and API stores before editing.
  - Evidence: frontend check and component test result.
- [ ] 5.1 Add invalidation, isolation, compatibility, and security tests. (FR-2, FR-3, FR-4, FR-6)
  - 证据块: inspect existing MCP contract tests before editing.
  - Evidence: Rust and frontend test results.
- [ ] 6.1 Synchronize versions to 0.3.0 and run all release gates. (FR-6)
  - 证据块: inspect all five version sources before editing.
  - Evidence: check, build, test, clippy, and package results.

Subspec task compatibility-tests-release/1.2 is included in the release gate.

## 需求覆盖矩阵

| FR ID | 子规格 | 任务引用 | 状态 |
|---|---|---|---|
| FR-1 | gateway-model-ui | gateway-model-ui/1.1 | pending |
| FR-2 | mcp-session-routing | mcp-session-routing/1.1, mcp-session-routing/1.2 | pending |
| FR-3 | gateway-tools-security | gateway-tools-security/1.1 | pending |
| FR-4 | gateway-tools-security | gateway-tools-security/1.2 | pending |
| FR-5 | gateway-model-ui | gateway-model-ui/1.2 | pending |
| FR-6 | compatibility-tests-release | compatibility-tests-release/1.1 | pending |

## 文件变更清单

- `src-tauri/src/workspace/model.rs`, `src-tauri/src/mcp/`, and `src-tauri/src/tools/`.
- `src/lib/components/` and existing frontend API/store modules.
- Rust/frontend tests and the five version source files.

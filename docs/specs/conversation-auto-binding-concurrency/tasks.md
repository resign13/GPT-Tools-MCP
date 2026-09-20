# Tasks: conversation-auto-binding-concurrency

## 交付物清单

- Parent and four child specifications.
- Session identity and binding implementation.
- Per-workspace concurrency coordination.
- Prompt, compatibility, and release evidence.

## 任务列表

1. `session-identity-routing/1.1` and `1.2`
2. `gateway-binding-contract/2.1` and `2.2`
3. `workspace-concurrency-isolation/3.1` and `3.2`
4. `compatibility-tests-docs/4.1` and `4.2`

## 需求覆盖矩阵

| FR | Subspec | Task references | Status |
| --- | --- | --- | --- |
| FR-1 | session-identity-routing | session-identity-routing/1.1, session-identity-routing/1.2 | pending |
| FR-2 | gateway-binding-contract | gateway-binding-contract/2.1, gateway-binding-contract/2.2 | pending |
| FR-3 | workspace-concurrency-isolation | workspace-concurrency-isolation/3.1, workspace-concurrency-isolation/3.2 | pending |
| FR-4 | compatibility-tests-docs | compatibility-tests-docs/4.1, compatibility-tests-docs/4.2 | pending |

## Cross-Module Contracts

- `listener.rs` owns identity extraction and transport headers.
- `gateway.rs` owns binding, validation, routing, locks, and gateway tool
  schemas.
- `server.rs` and `ChatGptSessionPrompt.svelte` own client instructions.
- Existing tool dispatch remains unchanged.

## 文件变更清单

- `src-tauri/src/mcp/listener.rs`
- `src-tauri/src/mcp/gateway.rs`
- `src-tauri/src/mcp/server.rs`
- `src/lib/components/ChatGptSessionPrompt.svelte`
- Focused Rust and frontend tests.

## Release Gates

- `check_spec` passes before implementation.
- `npm run check` and `npm run build` pass.
- `cargo test --manifest-path src-tauri/Cargo.toml` passes.
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` passes.
- GitNexus impact is recorded before symbol edits and `detect_changes` passes
  before commit.

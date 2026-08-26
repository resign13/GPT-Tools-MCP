# Tasks: Gateway bind API and strict session lock

- [ ] 2.1 Add `bind_workspace` matching and public response contract.
  - 证据块: exact-match, not-found, ambiguous, and allowlist tests.
  - Files: `src-tauri/src/mcp/gateway.rs`
  - Requirement: FR-2
- [ ] 2.2 Enforce strict lock and update initialize/server metadata instructions.
  - 证据块: same-target idempotency and different-target lock tests.
  - Files: `src-tauri/src/mcp/gateway.rs`, `src-tauri/src/mcp/server.rs`
  - Requirement: FR-2

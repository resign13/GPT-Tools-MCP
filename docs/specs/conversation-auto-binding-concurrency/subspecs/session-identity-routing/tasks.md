# Tasks: Conversation identity and automatic workspace binding

- [ ] 1.1 Implement logical/session transport identity resolution and bounded aliases.
  - 证据块: listener and gateway call sites inspected; unit tests cover source precedence and conflicts.
  - Files: `src-tauri/src/mcp/listener.rs`, `src-tauri/src/mcp/gateway.rs`
  - Requirement: FR-1
- [ ] 1.2 Preserve redacted session logging and restart/TTL behavior.
  - 证据块: log assertions and cache expiry tests.
  - Files: `src-tauri/src/mcp/listener.rs`, `src-tauri/src/mcp/gateway.rs`
  - Requirement: FR-1

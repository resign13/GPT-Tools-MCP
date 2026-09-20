# Tasks: Concurrent routing and per-workspace serialization

- [ ] 3.1 Add per-canonical-workspace read/write locks and global eight-request gate.
  - 证据块: parallel different-workspace and queued same-workspace tests.
  - Files: `src-tauri/src/mcp/gateway.rs`
  - Requirement: FR-3
- [ ] 3.2 Route isolated contexts and classify history/Harness/command writes.
  - 证据块: cwd, command session, history, and write isolation tests.
  - Files: `src-tauri/src/mcp/gateway.rs`, `src-tauri/src/mcp/listener.rs`
  - Requirement: FR-3

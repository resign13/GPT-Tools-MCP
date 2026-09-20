# Tasks: Compatibility, prompts, tests, and release gates

- [ ] 4.1 Update initialize/session prompts and gateway response metadata.
  - 证据块: prompt tests assert bind-before-bootstrap and no-host-fallback rules.
  - Files: `src-tauri/src/mcp/server.rs`, `src/lib/components/ChatGptSessionPrompt.svelte`
  - Requirement: FR-4
- [ ] 4.2 Run compatibility, security, concurrency, frontend, and release gates.
  - 证据块: exact command output, code review, and two-chat acceptance record.
  - Files: Rust tests and frontend test locations.
  - Requirement: FR-4

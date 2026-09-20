# Subspec: Compatibility, prompts, tests, and release gates

## 范围

Keep single-workspace behavior unchanged, update client instructions, and
validate the gateway through automated and real connector scenarios.

## 需求回链

- FR-4

## 验收标准

1. WHEN gateway mode is disabled THEN the system SHALL use the original
   handler and tool catalog.
2. WHEN gateway mode is enabled THEN initialize instructions SHALL describe
   bind-before-history order.
3. Existing list/select/get gateway tools SHALL remain available.
4. Rust, frontend, clippy, and build gates SHALL pass.
5. Two real ChatGPT conversations SHALL run different workspace tasks at once.

## 涉及文件

Keep Cloudflare, OAuth, Actions, and persisted gateway fields untouched. Update
only the prompt copy and focused regression coverage.

- `src-tauri/src/mcp/server.rs`
- `src/lib/components/ChatGptSessionPrompt.svelte`
- `src-tauri/src/mcp/gateway.rs`

## 不做项

- Cloudflare DNS or OAuth secret changes.
- GPT Actions multi-workspace routing.

## 设计要点

Keep Cloudflare, OAuth, Actions, and persisted gateway fields untouched. Update
only the prompt copy and focused regression coverage.

- Add compatibility tests, prompt assertions, and release/e2e verification.

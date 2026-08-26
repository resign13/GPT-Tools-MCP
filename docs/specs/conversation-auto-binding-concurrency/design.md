# Design: conversation-auto-binding-concurrency

## 概述

The listener remains the HTTP/auth boundary. Gateway routing wraps the current
`handle_request` kernel and never changes `call_tool`.

```text
HTTP request
  -> auth
  -> session identity resolver
  -> GatewayRouter
       -> binding cache
       -> allowlist/fingerprint validation
       -> concurrency permit
       -> target ToolContext
  -> existing handle_request / call_tool
```

Gateway mode stays opt-in. Disabled mode bypasses every gateway branch.

## 技术方案

The design covers FR-1, FR-2, FR-3, and FR-4.

### Session Identity

The listener extracts OpenAI metadata from the root and `params` objects. A
logical key is selected in this order: OpenAI conversation ID, known alias for
the transport ID, raw `Mcp-Session-Id`, then a server-generated ID during
initialize. Transport IDs are returned only as the `Mcp-Session-Id` response
header. An alias table maps transport IDs to logical keys and is bounded by the
same TTL and cache limits as bindings. Conflicting known aliases return
`session_identity_conflict`.

The binding cache stores the target ID, fingerprint, session source, context,
and last-used timestamp. It is memory-only and is cleared on process restart.

### Binding Contract

`bind_workspace` accepts one required `workspace_hint`. Matching is deterministic
and allowlist-only: exact ID, case-insensitive name, normalized full path, or
normalized path basename. The result contains public workspace metadata,
`locked: true`, the session source, a short session hash, and the next history
bootstrap action. Ambiguous and missing matches return structured errors with
safe candidates. A second workspace cannot replace an existing binding.

`select_workspace` remains a compatibility alias for explicit ID selection and
uses the same lock rules. `get_selected_workspace` reports the current binding
or `workspace_not_selected`.

### Routing and Contexts

After binding, the router validates host availability, allowlist membership,
target existence, canonical path, and the host/target policy fingerprint on
every project call. It constructs one `ToolContext` per binding, so default cwd,
command sessions, Harness state, and history calls cannot cross conversation
boundaries. History requests receive the logical conversation key in their
metadata injection.

### Concurrency

The router owns a bounded global request gate with a default limit of eight.
Each canonical workspace path owns a read/write lock. Read-only tools acquire a
read permit. `apply_patch`, `exec_command`, `write_stdin`, `kill_session`,
history bootstrap/checkpoint/validate, and Harness task/event writes acquire an
exclusive permit. Different workspace locks do not contend. Waiting beyond the
request window returns retryable `gateway_busy`.

### Errors and Logging

Gateway errors use the existing MCP structured tool-result wrapper. New error
codes are `workspace_hint_required`, `workspace_not_found`,
`workspace_ambiguous`, `workspace_locked`, `session_identity_conflict`, and
`gateway_busy`. The listener logs only method, tool, session hash, and target
workspace ID.

### Client Instructions

The gateway initialize prompt tells the model to extract a workspace hint from
the user's first request, call `bind_workspace`, list and ask when no hint is
present, and call history bootstrap only after binding. The desktop session
prompt mirrors this order. No Cloudflare or OAuth UI changes are required.

### Compatibility

The public gateway config remains `{ enabled, workspace_ids, prompt }`. Existing
gateway tools remain in the exposed catalog. Existing single-workspace listeners
continue to call `handle_request` directly.

## 文件结构

- `src-tauri/src/mcp/listener.rs`: identity extraction and transport headers.
- `src-tauri/src/mcp/gateway.rs`: binding, routing, permits, and gateway tools.
- `src-tauri/src/mcp/server.rs`: initialization instructions and dispatch boundary.
- `src/lib/components/ChatGptSessionPrompt.svelte`: client-side session prompt.

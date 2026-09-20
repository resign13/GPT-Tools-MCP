# Subspec: Gateway bind API and strict session lock

## 范围

Expose deterministic `bind_workspace`, preserve legacy gateway tools, and lock
each logical conversation to its first selected workspace.

## 需求回链

- FR-2

## 验收标准

1. WHEN a hint exactly matches an allowlisted ID, name, full path, or basename
   THEN the gateway SHALL bind that target.
2. WHEN a hint is missing or ambiguous THEN the gateway SHALL return a
   structured safe error.
3. WHEN no hint is supplied THEN the gateway SHALL not select the host.
4. WHEN the same target is bound twice THEN the gateway SHALL be idempotent;
   selecting another target SHALL return `workspace_locked`.
5. WHEN target state changes THEN the gateway SHALL revoke its binding.

## 涉及文件

Add `bind_workspace` to the gateway-only tool catalog. Keep `select_workspace`
as an explicit ID compatibility path and use the existing fingerprint and
policy intersection checks for every routed tool call.

- `src-tauri/src/mcp/gateway.rs`
- `src-tauri/src/mcp/server.rs`

## 不做项

- Fuzzy or semantic matching.
- In-conversation workspace switching.

## 设计要点

Add `bind_workspace` to the gateway-only tool catalog. Keep `select_workspace`
as an explicit ID compatibility path and use the existing fingerprint and
policy intersection checks for every routed tool call.

- Add matching, binding response fields, lock errors, schemas, instructions,
  and API tests.

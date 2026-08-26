# Subspec: mcp-session-routing

## 范围

Listener-level session routing and isolated context lifecycle.

## 需求回链

- FR-2

## 验收标准

WHEN a request has `Mcp-Session-Id` THEN the router SHALL prefer it.
WHEN both identifiers are absent THEN the system SHALL return `session_required`.
WHEN a binding is stale or changed THEN the system SHALL revoke it and SHALL require selection.

## 涉及文件

- `src-tauri/src/mcp/listener.rs`
- `src-tauri/src/mcp/server.rs`
- `src-tauri/src/mcp/gateway.rs`

## 不做项

- Persistent bindings across application restart.
- Changes to `tools::dispatch::call_tool`.

# Design: single-connector-multi-workspace

## 概述

The gateway is an optional listener-layer wrapper. Existing request handling and
tool dispatch remain the execution kernel. FR-1 through FR-6 are covered by the
model, router, security, UI, and compatibility layers below.

## 对应需求

FR-1, FR-2, FR-3, FR-4, FR-5, and FR-6.

## 技术方案

## Architecture

The listener adds a gateway router before the existing `handle_request`. The
router resolves a session key, loads or validates a binding, and constructs a
workspace-specific `ToolContext`. Existing `tools::dispatch::call_tool` and
the request handler remain unchanged. Disabled gateway mode follows the old
single-workspace path.

```text
MCP listener -> gateway router -> session binding -> ToolContext -> handle_request
                                      |                 |
                              host allowlist      target policy/profile
```

## Data model

`WorkspaceProfile` receives `#[serde(default)] gateway: GatewayConfig`:

```text
GatewayConfig { enabled: bool, workspace_ids: Vec<String> }
```

The host profile supplies the listener, OAuth, tunnel, public tool catalog and
logs. A target profile supplies path, Harness, history, and target policy. A
binding stores session hash, target ID, fingerprint, context, created time,
and last-used time in an in-memory LRU/TTL cache (256 entries, 24 hours).

## Routing and security

The server-issued `Mcp-Session-Id` is preferred. `_meta["openai/session"]` is
accepted only as a compatibility fallback. Missing identifiers produce
`session_required`; there is no process-global fallback. Every call rechecks
allowlist, profile existence, path canonicalization, policy fingerprint, and
the host/target permission intersection before dispatch.

Gateway tools are registered in the host catalog. Their handlers operate on
the current session binding and never accept a filesystem path. The selected
workspace response is sanitized and excludes credentials.

## UI and operations

`GatewayConfigForm.svelte` is independent of the existing workspace forms and
uses the existing Tauri API/store conventions. It shows the stable `/mcp`
connection URL and restart hints. Authorization rendering consumes the same
allowlist projection as `list_workspaces`.

## Failure behavior

Use structured errors: `session_required`, `workspace_not_selected`,
`workspace_not_allowed`, `workspace_unavailable`, `workspace_changed`, and
`gateway_disabled`. Errors do not reveal secrets or raw session identifiers.

## 文件结构

- `src-tauri/src/workspace/model.rs`: defaulted gateway configuration.
- `src-tauri/src/mcp/listener.rs` and `src-tauri/src/mcp/server.rs`: router boundary.
- `src-tauri/src/mcp/gateway.rs`: session bindings and gateway tools.
- `src-tauri/src/tools/`: existing dispatch kernel, unchanged.
- `src/lib/components/GatewayConfigForm.svelte`: independent UI form.
- `src-tauri/tests/` and `src/**/*.test.ts`: isolation and compatibility tests.

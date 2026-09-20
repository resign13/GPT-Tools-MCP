# Requirements: conversation-auto-binding-concurrency

## 功能概述

Allow several ChatGPT web conversations to use one MCP gateway while each
conversation remains bound to one explicitly named local workspace.

## Scope Boundaries

In scope: MCP session identity, automatic binding from a first-message hint,
strict session locking, isolated contexts, concurrent routing, same-workspace
write serialization, gateway tool schemas, prompts, and compatibility tests.

Out of scope: GPT Actions, Cloudflare DNS changes, OAuth redesign, persistent
conversation bindings, fuzzy or semantic workspace classification, and target
MCP processes.

## 需求列表

### FR-1 Conversation Identity

1. WHEN `_meta["openai/session"]` is present THEN the gateway SHALL use it as
   the logical conversation key.
2. WHEN the OpenAI session value is absent and `Mcp-Session-Id` is present THEN
   the gateway SHALL use the MCP session ID as the logical key.
3. WHEN both values are absent during `initialize` THEN the gateway SHALL
   generate a transport session ID and return it in `Mcp-Session-Id`.
4. WHEN both values are present but map to conflicting logical sessions THEN
   the gateway SHALL return `session_identity_conflict` and SHALL NOT route a
   tool call.
5. WHEN a binding is older than 24 hours or the cache exceeds 256 entries THEN
   the gateway SHALL evict it and require a new binding.

### FR-2 Automatic Binding and Locking

1. WHEN a bound gateway conversation calls `bind_workspace` with an exact
   allowlisted ID, case-insensitive name, full path, or path basename THEN the
   gateway SHALL bind that workspace and return its public metadata.
2. WHEN the hint matches no allowlisted workspace THEN the gateway SHALL return
   `workspace_not_found` without selecting the host workspace.
3. WHEN the hint matches multiple allowlisted workspaces THEN the gateway SHALL
   return `workspace_ambiguous` with safe candidate metadata.
4. WHEN no hint is available THEN the gateway SHALL return
   `workspace_hint_required` or expose `list_workspaces` so the client can ask
   the user to choose.
5. WHEN a conversation is already bound to a different workspace THEN
   `bind_workspace` and `select_workspace` SHALL return `workspace_locked`.
6. WHEN a target is removed, deleted, or its path or policy fingerprint
   changes THEN the gateway SHALL revoke the binding before the next project
   tool call.

### FR-3 Concurrent Isolation

1. WHEN two conversations are bound to different canonical workspace paths
   THEN their tool calls SHALL be allowed to execute concurrently.
2. WHEN two conversations are bound to the same canonical path THEN read-only
   calls MAY run concurrently, while filesystem, command, history, and Harness
   writes SHALL execute under an exclusive per-workspace queue.
3. WHEN more than eight gateway requests are active THEN additional requests
   SHALL wait within the configured window or return retryable `gateway_busy`.
4. EACH binding SHALL own an independent `ToolContext`, default cwd,
   `SessionStore`, Harness context, and history session environment.
5. Logs SHALL contain only a session hash, target workspace ID, and tool name;
   raw session identifiers, hints, and credentials SHALL never be logged.

### FR-4 Compatibility

1. WHEN gateway mode is disabled THEN MCP routing, tools/list, OAuth, Actions,
   and all existing schemas SHALL behave exactly as the single-workspace path.
2. WHEN gateway mode is enabled THEN `list_workspaces`, `select_workspace`,
   and `get_selected_workspace` SHALL remain available for existing clients.
3. `initialize.instructions` SHALL require binding before history bootstrap or
   project tools and SHALL explain the no-hint selection path.
4. Gateway configuration changes SHALL not change Cloudflare DNS, OAuth
   secrets, or target workspace runtime processes.

## 非功能需求

- No process-global workspace fallback in gateway mode.
- No persistent binding storage.
- Public responses never include secrets or raw session keys.
- Existing dispatch and policy enforcement remain the final execution layer.
- Windows, Rust, Svelte, and the current MCP protocol version remain supported.

## 依赖关系

- `session-identity-routing` provides the logical session key.
- `gateway-binding-contract` depends on identity resolution.
- `workspace-concurrency-isolation` depends on a validated binding.
- `compatibility-tests-docs` depends on all previous subspecs.

## Acceptance Summary

Two real ChatGPT conversations can bind `codex-harness` and `cs-data-platfrom`
independently, run simultaneous commands, and report different default
directories without starting a second MCP or tunnel.

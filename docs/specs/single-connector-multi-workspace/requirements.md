# Requirements: single-connector-multi-workspace

## 功能概述

Add an opt-in MCP gateway to the current host workspace so one ChatGPT
connector can access multiple explicitly allowlisted local workspaces.

## 范围边界

- In Scope: gateway configuration, session routing, gateway tools, isolation,
  security, UI, compatibility tests, and v0.3.0 release gates.
- Out of Scope: GPT Actions multi-workspace routing, persistent bindings, and
  automatic Cloudflare provisioning.

## 需求列表

### FR-1 Gateway configuration and migration

1. WHEN a workspace profile is loaded without `gateway` THEN the system SHALL
   deserialize `gateway.enabled=false` and `gateway.workspace_ids=[]`.
2. WHEN gateway configuration is saved THEN the host workspace SHALL be present
   in the allowlist and unknown or deleted IDs SHALL be rejected.
3. WHEN gateway is disabled THEN existing single-workspace MCP behavior SHALL
   remain unchanged.

### FR-2 Session-scoped workspace selection

1. WHEN an MCP request has a server `Mcp-Session-Id` THEN the router SHALL use
   it as the primary binding key.
2. WHEN that header is absent and `_meta["openai/session"]` is present THEN the
   router SHALL use that value as a compatibility key.
3. WHEN both identifiers are absent in gateway mode THEN the system SHALL return
   structured `session_required` and SHALL NOT use a global workspace.
4. WHEN a session selects a workspace THEN all subsequent calls in that session
   SHALL use an isolated `ToolContext` for that workspace.
5. WHEN a binding is older than 24 hours or the cache exceeds 256 entries THEN
   it SHALL expire using bounded eviction and require selection again.

### FR-3 Gateway tools and public contract

1. `list_workspaces {}` SHALL return only allowlisted workspace `id`, `name`,
   `path`, and `available` fields.
2. `select_workspace { workspace_id }` SHALL validate the allowlist and return
   selected workspace information plus `history_session_bootstrap`.
3. `get_selected_workspace {}` SHALL return selected state or structured
   `workspace_not_selected`.
4. Gateway `initialize.instructions` SHALL require listing and selecting a
   workspace before history initialization.
5. `server_info` SHALL retain existing fields and add gateway enabled, host ID,
   and selected workspace fields.

### FR-4 Authorization and invalidation

1. EVERY gateway call SHALL validate host allowlist membership, target existence,
   and a configuration fingerprint covering path and policy.
2. WHEN a target is deleted, removed from the allowlist, or its path/policy
   changes THEN its binding SHALL be revoked immediately.
3. Effective tool permission SHALL be the intersection of host and target tool
   profiles and policies.
4. Path traversal, forged IDs, and stale bindings SHALL be rejected without
   exposing secrets.
5. Logs SHALL contain a session hash, target workspace ID, and tool name only;
   raw session keys and authentication material SHALL never be logged.

### FR-5 UI and operations

1. The workspace UI SHALL expose a separate `GatewayConfigForm` with enable
   toggle, workspace multi-select, connection URL, and gateway prompt.
2. Gateway configuration changes SHALL not alter Cloudflare DNS.
3. Changes to host port, OAuth, or host tool catalog SHALL prompt MCP restart.
4. The authorization page SHALL show names of gateway-accessible workspaces.

### FR-6 Compatibility and release

1. Single-workspace tool listing, OAuth, Actions, and existing MCP schemas SHALL
   remain compatible when gateway is disabled.
2. Only the host MCP and Cloudflare tunnel must run for gateway access; target
   workspaces need not start independent services.
3. The five version sources SHALL be synchronized to `0.3.0` before packaging.
4. The release SHALL pass frontend check/build, Rust tests, and clippy with
   warnings denied.

## 非功能需求

- Gateway routing SHALL preserve the existing dispatch kernel and single-workspace contract.
- Session bindings SHALL be bounded, expire after 24 hours, and never persist across restart.
- Errors and logs SHALL be structured and SHALL not expose raw session keys or credentials.

## 依赖关系

- `mcp-session-routing` depends on `gateway-model-ui`.
- `gateway-tools-security` depends on `mcp-session-routing`.
- `compatibility-tests-release` depends on all implementation subspecs.

# Design

## 概述
Two user presets with independently enforced isolation and desktop-only one-shot decisions.

## 技术方案
Typed policy and bounded in-memory approvals are introduced before transport/UI integration. Existing strict execution remains unchanged during foundation work.

## Policy model (FR-1, FR-2, FR-3)
PermissionPreset controls soft capabilities only. ExecutionPolicy retains a legacy restriction marker, network permission and resource/command constraints. IsolationPolicy is independent. Old safe/restricted/ask disable network, old trusted/developer/auto_approve preserve their network allowance, old dangerous/admin/full_access preserve their legacy command restrictions. A desktop save explicitly creates a current-generation policy. Unknown values fail parsing rather than upgrade.

## Approval model (FR-4, FR-5)
An in-memory bounded ApprovalStore owns opaque UUID request ids and immutable fingerprints: context nonce, task, canonical execution root, tool, sorted JSON arguments and policy revision. TTL is 600 seconds using monotonic time. A desktop-only decision changes pending to approved or denied; execution atomically consumes approved. MCP status is scoped to the originating context. Request metadata for display excludes secrets. No broad grant or self-approval API exists. Store capacity 256; exhausted capacity returns busy without evicting a live approval.

## Dispatch and backend (FR-2, FR-3, FR-6, FR-8)
Validate execution context and structural constraints before approval lookup. Every retry repeats validation and backend matching before consumption. Do not hold global gateway/workspace permits while waiting for a human. Preserve original arguments for execution. No string matching on error messages to authorize commands.
Backend reports read/write/network capabilities separately. Strict policy requires AppContainer; compatibility requires explicit host and target agreement. A WRITE_RESTRICTED prototype uses active-root SID, exact-ACE checks with per-path locking, per-context private temp SID, checked Win32 calls, Job Object ownership and no direct runner fallback. Known upstream piped grandchild and console limitations are release gates, not silently bypassed errors.

## UI and migration (FR-1, FR-7)
Runtime and Actions share the two-option control. Backend selection is separate and compatibility remains disabled until verified. Desktop approval list shows task, tool, safe request summary, age and decision. Requests expire on restart; policy edits require service restart and never mutate running contexts implicitly.

## 文件结构
New security policy/approval modules; later adapters in tools/context.rs, tools/dispatch.rs, tools/policy.rs, mcp/gateway.rs, commands and shared Svelte controls. Keep independent modules below 500 lines. High-risk dispatch and sandbox edits require GitNexus impact first. Preserve existing configuration and source modifications.

## Test strategy
Focused Rust tests for FR-1 through FR-5, backend smoke for FR-6, frontend check for FR-7, response assertions for FR-8. Real ChatGPT two-conversation acceptance remains explicit and must not be inferred from unit tests.

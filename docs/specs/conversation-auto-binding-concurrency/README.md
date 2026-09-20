# Conversation Auto-Binding and Concurrent Multi-Workspace Gateway

This feature extends the existing opt-in MCP gateway so one authenticated
ChatGPT connector can serve several local workspaces at the same time.

## 功能概述

In scope: conversation-scoped identity, first-message workspace binding,
strict binding locks, isolated tool contexts, per-workspace concurrency
coordination, structured errors, prompt updates, and regression coverage.

Out of scope: GPT Actions routing, Cloudflare or OAuth provisioning,
persistent bindings, fuzzy project classification, and starting MCP services
for target workspaces.

## 原则

- Conversation identity is the routing boundary.
- No implicit host fallback in gateway mode.
- Existing dispatch and disabled-gateway behavior remain authoritative.

## Requirements

| FR | Requirement | Subspec |
| --- | --- | --- |
| FR-1 | Prefer the ChatGPT conversation identity and keep transport aliases isolated. | session-identity-routing |
| FR-2 | Bind a conversation from an explicit workspace hint and lock it. | gateway-binding-contract |
| FR-3 | Run different workspaces concurrently and serialize same-workspace writes. | workspace-concurrency-isolation |
| FR-4 | Preserve disabled-gateway compatibility and validate the complete contract. | compatibility-tests-docs |

## 子规格索引

- `session-identity-routing`: FR-1 identity and transport aliasing.
- `gateway-binding-contract`: FR-2 deterministic binding and strict locking.
- `workspace-concurrency-isolation`: FR-3 parallel routing and write queues.
- `compatibility-tests-docs`: FR-4 prompts, compatibility, and release gates.

## 依赖关系

Identity resolution precedes binding; binding precedes concurrency routing;
compatibility and release validation depend on all three implementation layers.

## 里程碑

1. Complete and validate the parent/child specifications.
2. Implement identity, binding, and concurrency in dependency order.
3. Run regression, frontend, Rust, clippy, and connector acceptance gates.

## Release Constraints

- Preserve all existing uncommitted changes.
- Keep `call_tool` and the existing dispatch kernel unchanged.
- Keep the current host MCP URL, OAuth configuration, Cloudflare tunnel, and
  `GatewayConfig` JSON shape backward compatible.
- Bindings remain in memory, capped at 256 entries and 24 hours.

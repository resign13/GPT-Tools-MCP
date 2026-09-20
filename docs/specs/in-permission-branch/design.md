# Design

## 概述

Keep `ExecutionPolicy` as the source of approval semantics. Its soft-permission predicate becomes unconditional automatic approval for both presets and legacy settings. Preset parsing, network policy and isolation intersection remain intact. This intentionally supersedes the old default-preset desktop approval behavior.

## 技术方案

对应需求：FR-1、FR-2、FR-3、FR-4、FR-5。

- `security/permission.rs`: central automatic-approval predicate and migration/intersection regressions.
- `tools/policy.rs`: structural validation continues before the automatic-approval predicate. Capability analysis remains diagnostic, not an OS isolation guarantee.
- `tools/dispatch.rs`: apply automatic policy before consulting old approval IDs. Keep `AuthorizedInvocation`, validation and actual execution unchanged. Concrete `request_permissions` calls grant automatically; diagnostics expose `desktop_approval_required: false`.
- `tools/registry.rs` and shared permission UI: align current descriptions with automatic behavior without changing tool schemas or stored settings.
- Existing approval store and lookup: retain compatibility and accurate old record statuses; no simulated desktop clicks or fabricated decisions.

## 文件结构

Production files remain in `src-tauri/src/security`, `src-tauri/src/tools` and `src/lib/components`. Contract tests remain in `src-tauri/tests`; specifications remain in this directory.

## Non-Goals

No sandbox backend changes, ACL grants, network hard-policy expansion, task/session protocol changes, configuration rewrite, approval-store redesign or automatic commit/restart.

## Verification

Focused policy and contract tests exercise actual successful local execution where supported, validate stale-ID retries and hard denials, and assert no pending approval records. Run `cargo check`, targeted tests and frontend type checking only if frontend text changes.

# Tasks

## 交付物清单
Central automatic approval, compatible diagnostics, updated permission text and focused regression evidence.

## 任务列表

- [x] Inspect current branch, existing changes and central approval paths.
- [x] Validate specifications and perform GitNexus impact analysis.
- [x] Enable centralized automatic soft approval, including stale-ID retry handling.
- [x] Align diagnostics, tool descriptions and permission labels.
- [x] Update focused regression tests and run compiler/frontend checks.
- [x] Review diff and record results without committing or restarting services.

## 需求覆盖矩阵
| Requirement | Implementation | Validation |
| --- | --- | --- |
| FR-1 | Policy predicate and dispatch | Default/full/legacy tests |
| FR-2 | Concrete request validation | Granted and invalid-request contracts |
| FR-3 | Dispatch retry handling | Stale-ID command execution |
| FR-4 | Existing structural checks | Hard rejection regressions |
| FR-5 | Status, registry, shared UI | Status contracts and frontend check |

## 文件变更清单
`security/permission.rs`, `tools/policy.rs`, `tools/dispatch.rs`, `tools/registry.rs`, focused tests and existing shared permission component; no sandbox implementation edits.

## Verification Results (2026-09-14)

- `cargo check --locked --manifest-path src-tauri/Cargo.toml`: passed; 9 existing dead-code warnings.
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- permission automatic_approval`: 14 passed.
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib tools::policy::tests`: 14 passed (overlaps existing permission tests).
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --test call_tool_contract -- permission automatic_approval check_exec_environment full_access_removes`: 8 passed. Actual Python child execution succeeded in both presets, with and without a stale approval ID.
- `npm run check`: exit 0, Svelte reports 0 errors and 0 warnings. Existing nested reference/worktree configuration loading messages remain; those unrelated projects were not changed.
- `cargo build --locked --manifest-path src-tauri/Cargo.toml`: passed.
- `git diff --check`: passed.

An initial command fixture using `cmd /d /c` was rejected by the existing external-path validator. The fixture now uses Python; no path-validation behavior was changed to make a test pass. This change does not resolve unrelated hard-policy or runtime failures.

## Review And Delivery

Reviewed the actual diff and the five requirements. Hard validation remains before automatic approval; old IDs do not influence execution authorization; historical approval lookups remain truthful. Gateway intersection and the sandbox backend are unchanged. No new dependency, secret, saved-configuration rewrite, commit or service restart.

GitNexus reports HIGH impact for the central predicate/dispatcher and CRITICAL aggregate change risk across shared tool flows. `detect-changes --scope all` was run twice (second with limit 200); its CLI prints a condensed list, not proof of complete graph coverage. Existing dirty agent metadata remains outside this feature. Source review and focused tests provide the verification evidence.

Live ChatGPT and the full OS sandbox matrix were not run. The built code takes effect after restarting the desktop program and its MCP/Actions services.

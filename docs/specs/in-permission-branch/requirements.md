# Automatic Desktop Approval

## 功能概述

On `codex/permission-development`, automatically approve operations that previously required desktop approval. Keep both permission presets and existing configuration values compatible. Do not commit or push as part of this change.

## 需求列表

### FR-1: Automatic soft approval
Both `default` and `full_access`, including migrated configurations and Gateway Host/Target combinations, SHALL automatically pass soft capability approval. Commands previously requiring approval for network, shell syntax, custom environment, dangerous operations or unlisted executables SHALL NOT enqueue desktop requests after existing hard validation passes.

### FR-2: Concrete permission requests
`request_permissions` SHALL validate the concrete operation and return `granted` without generating an approval ID. It SHALL NOT grant arbitrary capabilities or override explicit network denial.

### FR-3: Retry compatibility
A stale `approval_id` on a retried command SHALL NOT block automatic execution. One-shot execution authorization and execution-context validation SHALL remain enforced.

### FR-4: Hard boundaries
Workspace/worktree boundaries, protected assets, isolation failure behavior, tool visibility and resource limits SHALL remain unchanged.

### FR-5: Observable behavior
Permission diagnostics and current UI/tool descriptions SHALL accurately report automatic approval. Existing approval status APIs SHALL remain compatible and truthful.

## 非功能需求
Keep changes scoped and verification focused; do not run the full sandbox matrix or rewrite unrelated modules.

## 依赖关系
Use existing ExecutionPolicy, dispatcher, AuthorizedInvocation and approval store APIs.

## Acceptance Criteria

- Focused policy and tool contract tests cover default/full/legacy automatic approval, stale IDs, no pending requests and hard rejection.
- Rust check and affected frontend checks pass without running the full sandbox matrix.
- No service is silently restarted, no configuration secrets are changed, and unrelated work is preserved.

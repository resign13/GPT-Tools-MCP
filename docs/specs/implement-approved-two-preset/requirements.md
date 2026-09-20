# Two permission presets and independent isolation

## 功能概述
Implement the approved default/full_access design on codex/permission-development.
Preserve uncommitted gateway changes and backups. Do not commit automatically.

## 需求列表
- FR-1: canonical presets default/full_access; legacy values retain their original execution restrictions until explicit desktop save.
- FR-2: ToolContext owns one immutable effective typed policy; host/target capabilities intersect and strict isolation wins.
- FR-3: default automatically permits configured development operations. Additional capabilities require desktop approval. Full access automatically permits soft capabilities, never disables isolation.
- FR-4: approvals bind context/session, task, execution root, tool, normalized arguments and policy revision. Expire after 600 seconds, consume atomically once, never authorize through MCP.
- FR-5: approval requests return immediately. Pending, denied, expired, consumed and invalidated are distinguishable; restart clears approvals.
- FR-6: independent strict AppContainer and explicitly selected compatibility ACL backend. No direct fallback. Compatibility reports partial, ambient reads/network and documented write limitations.
- FR-7: shared Runtime/Actions preset UI and desktop pending approvals. Saving requires runtime restart.
- FR-8: report actual read/write/network enforcement, backend and reason separately. Keep OAuth, Cloudflare, task binding and existing file tool schemas.

## 非功能需求
The system SHALL return approval_required without waiting for user interaction. The system SHALL reject approval reuse and SHALL retain configured isolation when a backend fails.
Unit tests cover migration, intersections, approval expiry/replay/isolation and hard-boundary preservation. Windows validation covers actual desktop launch, child output capture, npm build, D/E volumes, linked worktrees and private temp. Compatibility stays experimental if these checks fail. Run cargo check, targeted tests and npm run check once at integration completion.

## Non-goals
No history-system rewrite, unrestricted execution, automatic backend downgrade or restoration of the previous AppContainer/restricted-token hybrid experiments.

## 依赖关系
Existing ExecutionContext, gateway bindings, Tauri commands and Windows process ownership. Reference backend revision c389f96. No new external runtime required for the production Rust policy modules.

# Implementation tasks

## 交付物清单
Initial additions: security/permission.rs (under 250 lines), security/approval.rs (under 400 lines), three spec documents. Integration touches security/mod.rs, tools/context.rs, tools/policy.rs, tools/dispatch.rs, mcp/gateway.rs, workspace configuration, registry, desktop commands and shared UI. Exact integration inventory is recorded before that stage; the stages below are not claimed complete by creation of this document.

## 任务列表
- [ ] T1: Add typed presets, isolation and conservative legacy migration with tests. Evidence: tools/context.rs:28 `pub permission_mode: String`; tools/policy.rs:62 duplicates the mode. FR-1/2; design Policy model. Budget 250 new lines plus module registration.
- [ ] T2: Add bounded one-shot approval state machine with expiry and context fingerprints. Evidence: tools/dispatch.rs:140 request_permissions returns an unscoped grant or unsupported. FR-4/5; design Approval model. Budget 400 new lines.
- [ ] T3: Wire one effective snapshot into dispatch, gateway intersections and policy checks; separate soft gates from hard checks. Evidence: tools/policy.rs:203 mixes syntax and path validation. FR-2/3/8; design Dispatch and backend. Split new capability evaluator into its own module rather than expanding dispatch.
- [ ] T4: Validate independent ACL prototype from desktop execution environment, preserving existing AppContainer. Evidence: exec_sandbox/mod.rs:95 chooses strict solely from Git identity. FR-6; design Dispatch and backend. New backend and grants modules each under 500 lines; release gated on child-output and npm build smoke.
- [ ] T5: Add desktop approval commands/list, MCP status and shared preset/isolation UI with explicit migration. FR-1/4/5/7/8; design UI and migration. New UI components under 250 lines each, adapters in existing forms.
- [ ] T6: Run cargo check, targeted Rust tests, npm run check, review and two-conversation acceptance. FR-1 through FR-8; design Test strategy. No full language escape matrix.

## Completion
Tasks stay unchecked until corresponding production behavior and evidence exist. Backend compatibility failures keep the backend experimental. No auto commit or stash restore.

## 需求覆盖矩阵
| Requirement | Tasks |
|---|---|
| FR-1 | T1, T5, T6 |
| FR-2 | T1, T3, T6 |
| FR-3 | T3, T6 |
| FR-4 | T2, T5, T6 |
| FR-5 | T2, T5, T6 |
| FR-6 | T4, T6 |
| FR-7 | T5, T6 |
| FR-8 | T3, T5, T6 |

## 文件变更清单
security/permission.rs and security/approval.rs: new typed foundation modules. security/mod.rs: registration. Remaining integration adapters are listed in the scope inventory and remain pending until their implementation stage.

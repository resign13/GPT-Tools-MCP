use serde_json::{json, Value};

use crate::tools::context::ToolContext;
use crate::tools::policy::{validate_tool_arguments_for_workspace, PolicyError};
use crate::tools::workspace::{tool_err, tool_err_code, tool_ok, WorkspaceError};
use crate::tools::{exec, file, git, history, image_tool, patch, session};

fn policy_tool_err(err: PolicyError) -> Value {
    let dangerous = err
        .0
        .strip_prefix("DANGEROUS_OPERATION_REQUIRES_CONFIRMATION: ");
    let protected = err.0.strip_prefix("PROTECTED_REPOSITORY_ASSET: ");
    let code = if protected.is_some() {
        "PROTECTED_REPOSITORY_ASSET"
    } else if dangerous.is_some() {
        "DANGEROUS_OPERATION_REQUIRES_CONFIRMATION"
    } else {
        "POLICY_REJECTED"
    };
    let message = protected.or(dangerous).unwrap_or(&err.0).to_string();
    let (reason, suggestion) = if dangerous.is_some() {
        (
            "confirmation_required",
            "为危险操作补充 confirm=true，确认后再重试",
        )
    } else if message.contains("allowlisted") {
        ("command_rejected", "改用允许的命令，或调整工作区命令白名单")
    } else if message.contains("Shell chaining") {
        (
            "shell_syntax_rejected",
            "移除未加引号的 shell 操作符；引号内的程序参数可以保留",
        )
    } else {
        ("policy_rejected", "根据错误信息修正参数后重试")
    };
    tool_err(WorkspaceError::ToolDetails {
        code,
        message,
        category: "policy",
        retryable: false,
        details: json!({
            "stage": "policy",
            "reason": reason,
            "recoverable": reason != "confirmation_required",
            "suggestion": suggestion
        }),
    })
}

/// **唯一工具执行入口**。MCP `tools/call` 与 Actions `POST /actions/{tool}` 必须且只能调用此函数。
/// 策略校验、分发、错误格式在此统一，两路传输层不得另做执行前校验（Actions 仅允许额外的暴露层 `validate_actions_exposure`）。
pub fn call_tool(ctx: &ToolContext, name: &str, args: &Value) -> Value {
    if let Err(error) = ctx.validate_execution_context() {
        return tool_err(error);
    }
    // Keep the original arguments intact. Each tool resolves paths through the
    // execution-aware context; policy inspection only needs an implicit command
    // cwd when the caller omitted one.
    let policy_args = policy_arguments(ctx, name, args);
    if let Err(e) =
        validate_tool_arguments_for_workspace(name, &policy_args, &ctx.policy, Some(&ctx.workspace))
    {
        return policy_tool_err(e);
    }

    if crate::harness::tools::TOOL_NAMES.contains(&name) {
        return match crate::harness::tools::call(ctx, name, args) {
            Ok(value) => value,
            Err(error) => attach_harness_status(ctx, tool_err(error), false),
        };
    }

    let task_id = if requires_write_baseline(name, args) {
        let task = ctx.harness.current_task().ok().flatten();
        if let Some(task) = task {
            if let Err(error) = ctx.harness.check_baseline(&task.id) {
                return attach_harness_status(
                    ctx,
                    tool_err_code(error.code(), error.to_string(), "permission"),
                    false,
                );
            }
            let _ = ctx.harness.record_event(
                &task.id,
                "operation_started",
                Some(name),
                operation_input(args),
                json!({"ok": true, "tracking": "task"}),
            );
            Some(task.id)
        } else {
            None
        }
    } else {
        None
    };

    let operation = if should_log_operation(name) {
        ctx.harness
            .record_operation(
                None,
                task_id.as_deref(),
                name,
                "started",
                json!({"arguments_present": !args.is_null()}),
                json!({"ok": true}),
            )
            .ok()
    } else {
        None
    };

    let result = match name {
        "history_session_bootstrap" => history::bootstrap(ctx, args),
        "history_session_checkpoint" => history::checkpoint(ctx, args),
        "history_session_validate" => history::validate(ctx, args),
        "history_session_search" => history::search(ctx, args),
        "history_session_read" => history::read(ctx, args),
        "server_info" => server_info(ctx),
        "check_exec_environment" => check_exec_environment(ctx),
        "exec_health_check" => exec::exec_health_check(ctx),
        "get_default_cwd" => get_default_cwd(ctx),
        "set_default_cwd" => set_default_cwd(ctx, args),
        "read_file" => file::read_file(ctx, args),
        "list_dir" => file::list_dir(ctx, args),
        "list_files" => file::list_files(ctx, args),
        "search_text" | "grep_text" | "grep" => file::search_text(ctx, args),
        "patch_check" => patch::patch_check(ctx, args),
        "apply_patch" => patch::apply_patch(ctx, args),
        "exec_command" => exec::exec_command(ctx, args),
        "read_output" => session::read_output(ctx, args),
        "write_stdin" => session::write_stdin(ctx, args),
        "kill_session" => session::kill_session(ctx, args),
        "git_status" => git::git_status(ctx, args),
        "git_diff" => git::git_diff(ctx, args),
        "git_log" => git::git_log(ctx, args),
        "git_show" => git::git_show(ctx, args),
        "git_blame" => git::git_blame(ctx, args),
        "view_image" => image_tool::view_image(ctx, args),
        "request_permissions" => {
            if ctx.policy.skip_permission_gates() {
                Ok(tool_ok(json!({
                    "ok": true,
                    "status": "granted",
                    "grant_id": "dangerously-skip-all-permissions",
                    "expires_at": null,
                    "constraints": {
                        "mode": "dangerous",
                        "workspace": ctx.workspace.root_display(),
                        "requested": args
                    },
                    "warnings": [
                        "dangerous permission mode is enabled; permission-gated operations are auto-granted"
                    ]
                })))
            } else {
                Ok(tool_ok(json!({
                    "ok": false,
                    "status": "unsupported",
                    "grant_id": null,
                    "expires_at": null,
                    "next_actions": [],
                    "error": {
                        "code": "ELICITATION_UNSUPPORTED",
                        "message": "Permission elicitation is not available for this client.",
                        "category": "permission",
                        "retryable": false,
                        "details": { "requested": args }
                    }
                })))
            }
        }
        _ => {
            return tool_err_code(
                "INVALID_ARGUMENT",
                format!("Unknown tool: {name}"),
                "validation",
            )
        }
    };
    let mut output = match result {
        Ok(v) => v,
        Err(e) => tool_err(e),
    };
    if task_id.is_none()
        && standalone_operation(name)
        && output.get("ok") == Some(&Value::Bool(true))
    {
        attach_standalone_metadata(
            &mut output,
            "当前操作已在 standalone 模式完成；如需继续，直接调用下一个开发工具。",
        );
    }
    if let Some(operation) = operation.as_ref() {
        if let Some(object) = output.as_object_mut() {
            object.insert("operation_id".into(), Value::String(operation.id.clone()));
        }
    }
    if output.get("ok").and_then(Value::as_bool) == Some(false) {
        output = attach_harness_status(ctx, output, task_id.is_none());
    }
    if let Some(task_id) = task_id.as_deref() {
        let succeeded = output.get("ok").and_then(Value::as_bool) == Some(true);
        let _ = ctx.harness.record_event(
            task_id,
            "operation_finished",
            Some(name),
            operation_input(args),
            json!({"ok": succeeded, "tool": name}),
        );
        if succeeded {
            let _ = ctx.harness.refresh_expected_state(task_id);
        }
    }
    if let Some(operation) = operation {
        let succeeded = output.get("ok").and_then(Value::as_bool) == Some(true);
        let _ = ctx.harness.record_operation(
            Some(&operation.id),
            task_id.as_deref(),
            name,
            if succeeded { "completed" } else { "failed" },
            operation_input(args),
            json!({
                "ok": succeeded,
                "tool": name,
                "affected_files": output.get("affected_files")
            }),
        );
    }
    output
}

fn policy_arguments(ctx: &ToolContext, name: &str, args: &Value) -> Value {
    if name != "exec_command" || args.get("workdir").is_some() || args.get("cwd").is_some() {
        return args.clone();
    }
    let mut effective = args.clone();
    let base = if ctx.default_cwd_path() == ctx.execution_root() {
        ".".to_string()
    } else {
        ctx.default_cwd_display()
    };
    effective["workdir"] = Value::String(base);
    effective
}

fn requires_write_baseline(name: &str, args: &Value) -> bool {
    match name {
        "exec_command" => true,
        "apply_patch" => !args
            .get("dry_run")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        _ => false,
    }
}

fn standalone_operation(name: &str) -> bool {
    matches!(name, "patch_check" | "apply_patch" | "exec_command")
}

fn should_log_operation(name: &str) -> bool {
    standalone_operation(name)
        || matches!(
            name,
            "git_status" | "git_diff" | "git_log" | "git_show" | "git_blame"
        )
}

fn operation_input(args: &Value) -> Value {
    json!({
        "arguments_present": !args.is_null(),
        "reason": args.get("reason")
    })
}

fn attach_harness_status(ctx: &ToolContext, mut output: Value, standalone: bool) -> Value {
    if let Ok(mut status) = ctx.harness.status() {
        if standalone && status.task_id.is_none() {
            status.next_actions.clear();
        }
        status.next_actions = filter_exposed_actions(ctx, status.next_actions);
        if let Some(object) = output.as_object_mut() {
            object.insert(
                "harness".into(),
                serde_json::to_value(status).unwrap_or_else(|_| {
                    json!({
                        "status": "unavailable",
                        "reason": "无法序列化 Harness 状态"
                    })
                }),
            );
            if standalone {
                attach_standalone_metadata(
                    &mut output,
                    "命令未成功；请检查 stderr、exit_code 或调整参数后重试。",
                );
            }
        }
    }
    output
}

fn attach_standalone_metadata(output: &mut Value, recovery_hint: &str) {
    if let Some(object) = output.as_object_mut() {
        object.insert("harness_mode".into(), Value::String("standalone".into()));
        object.insert("task_required".into(), Value::Bool(false));
        object.insert("next_actions".into(), json!([]));
        object.insert(
            "recovery_hint".into(),
            Value::String(recovery_hint.to_string()),
        );
    }
}

fn filter_exposed_actions(ctx: &ToolContext, actions: Vec<String>) -> Vec<String> {
    let exposed = crate::tools::registry::exposed_tool_names(&ctx.tool_profile);
    actions
        .into_iter()
        .filter(|action| exposed.contains(&action.as_str()))
        .collect()
}

pub fn server_info(ctx: &ToolContext) -> Result<Value, WorkspaceError> {
    let tools = crate::tools::registry::exposed_tool_names(&ctx.tool_profile);
    let sandbox = ctx.exec_sandbox_status();
    Ok(tool_ok(json!({
        "server": "coding-tools-mcp",
        "title": "Coding Tools MCP",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": "2025-06-18",
        "workspace": ctx.workspace.root_display(),
        "repository_root": ctx.repository_root_display(),
        "execution_root": ctx.execution_root_display(),
        "permission_mode": ctx.permission_mode,
        "default_cwd": ctx.default_cwd_display(),
        "network_allowed": ctx.policy.network_allowed(),
        "execution_isolation_mode": ctx.execution_isolation_mode().as_str(),
        "workspace_exec_boundary": sandbox.boundary(),
        "workspace_exec_sandbox_enforced": sandbox.enforced,
        "tool_profile": ctx.tool_profile,
        "auth_enabled": ctx.auth.auth_enabled(),
        "auth_type": ctx.auth.auth_type,
        "endpoint_path": "/mcp",
        "tools": tools,
        "tool_count": tools.len()
    })))
}

pub fn check_exec_environment(ctx: &ToolContext) -> Result<Value, WorkspaceError> {
    let sandbox = ctx.exec_sandbox_status();
    let strict = ctx.requires_strict_exec_isolation();
    let warnings = if strict {
        vec!["Generic child processes are disabled until an OS filesystem sandbox is available"]
    } else {
        vec!["Workspace child processes use policy-only execution; no OS filesystem sandbox is enforced"]
    };
    Ok(tool_ok(json!({
        "workspace": ctx.workspace.root_display(),
        "repository_root": ctx.repository_root_display(),
        "execution_root": ctx.execution_root_display(),
        "permission_mode": ctx.permission_mode,
        "network_allowed": ctx.policy.network_allowed(),
        "execution_isolation_mode": ctx.execution_isolation_mode().as_str(),
        "landlock_enabled": false,
        "filesystem_sandbox": {
            "available": sandbox.available,
            "enforced": sandbox.enforced,
            "implementation": sandbox.implementation,
            "fallback_allowed": sandbox.fallback_allowed,
            "default_scope": "workspace",
            "host_scope_available": false
        },
        "global_tmp_write": if ctx.permission_mode == "dangerous" { "allowed" } else { "tmp-prefix" },
        "workspace_exec_available": sandbox.available,
        "workspace_exec_sandbox_enforced": sandbox.enforced,
        "workspace_exec_boundary": sandbox.boundary(),
        "system_command_allowlist": ctx.policy.allowed_commands.iter().cloned().collect::<Vec<_>>(),
        "workspace_local_entries": {
            "enabled": ctx.policy.workspace_local_entries,
            "script_extensions": ctx.policy.workspace_script_extensions.iter().cloned().collect::<Vec<_>>(),
            "resolution": "workdir_first"
        },
        // Backward-compatible alias for older MCP clients.
        "allowed_commands": ctx.policy.allowed_commands.iter().cloned().collect::<Vec<_>>(),
        "warnings": warnings
    })))
}

pub fn get_default_cwd(ctx: &ToolContext) -> Result<Value, WorkspaceError> {
    Ok(tool_ok(json!({
        "workspace": ctx.workspace.root_display(),
        "execution_root": ctx.execution_root_display(),
        "default_cwd": ctx.default_cwd_display(),
        "resolved_cwd": ctx.default_cwd_path().display().to_string()
    })))
}

pub fn set_default_cwd(ctx: &ToolContext, args: &Value) -> Result<Value, WorkspaceError> {
    let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let resolved = ctx.resolve_from_execution_root(path, crate::tools::PathIntent::CommandCwd)?;
    if !resolved.path.is_dir() {
        return Err(WorkspaceError::not_a_directory(
            "Default cwd must be a directory",
        ));
    }
    ctx.set_default_cwd(resolved.path.clone())?;
    Ok(tool_ok(json!({
        "workspace": ctx.workspace.root_display(),
        "execution_root": ctx.execution_root_display(),
        "default_cwd": resolved.display,
        "resolved_cwd": resolved.path.display().to_string()
    })))
}

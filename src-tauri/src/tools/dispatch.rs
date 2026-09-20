use serde_json::{json, Value};

use crate::security::approval::{ApprovalScope, ApprovalStatus};
use crate::tools::authorized_invocation::AuthorizedInvocation;
use crate::tools::context::ToolContext;
use crate::tools::policy::{
    assess_command_for_workspace, validate_tool_arguments_for_workspace,
    CommandCapability, PolicyError,
};
use crate::tools::workspace::{tool_err, tool_err_code, tool_ok, WorkspaceError};
use crate::tools::{exec, file, git, history, image_tool, patch, session};

fn policy_tool_err(err: PolicyError) -> Value {
    let dangerous = err
        .0
        .strip_prefix("DANGEROUS_OPERATION_REQUIRES_CONFIRMATION: ");
    let protected = err.0.strip_prefix("PROTECTED_REPOSITORY_ASSET: ");
    let network = err.0.strip_prefix("NETWORK_NOT_ALLOWED: ");
    let code = if err.0.starts_with("ISOLATION_CAPABILITY_UNSATISFIED:") {
        "ISOLATION_CAPABILITY_UNSATISFIED"
    } else if err.0.starts_with("EXTERNAL_EXECUTION_NOT_ALLOWED:") {
        "EXTERNAL_EXECUTION_NOT_ALLOWED"
    } else if protected.is_some() {
        "PROTECTED_REPOSITORY_ASSET"
    } else if network.is_some() {
        "NETWORK_NOT_ALLOWED"
    } else if dangerous.is_some() {
        "DANGEROUS_OPERATION_REQUIRES_CONFIRMATION"
    } else {
        "POLICY_REJECTED"
    };
    let message = protected
        .or(network)
        .or(dangerous)
        .unwrap_or(&err.0)
        .to_string();
    let (reason, suggestion) = if dangerous.is_some() {
        (
            "confirmation_required",
            "为危险操作补充 confirm=true，确认后再重试",
        )
    } else if network.is_some() {
        (
            "network_disabled",
            "工作区网络策略已禁用该能力，桌面审批不能解除此硬限制",
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
            "recoverable": reason != "confirmation_required" && reason != "network_disabled",
            "suggestion": suggestion
        }),
    })
}

/// **唯一工具执行入口**。MCP `tools/call` 与 Actions `POST /actions/{tool}` 必须且只能调用此函数。
/// 策略校验、分发、错误格式在此统一，两路传输层不得另做执行前校验（Actions 仅允许额外的暴露层 `validate_actions_exposure`）。
pub fn call_tool(ctx: &ToolContext, name: &str, args: &Value) -> Value {
    call_tool_with_identity(ctx, name, args, None)
}

/// Execute a tool with the trusted logical conversation identity supplied by
/// the transport. Direct callers retain the legacy wrapper above and use a
/// local context for tests and desktop-only invocations.
pub fn call_tool_with_identity(
    ctx: &ToolContext,
    name: &str,
    args: &Value,
    session_identity: Option<&str>,
) -> Value {
    if ctx.host_access() {
        tauri::async_runtime::block_on(ctx.sessions.refresh_host_completions());
    }
    if let Err(error) = ctx.validate_execution_context() {
        return tool_err(error);
    }
    // Keep the original arguments intact. Each tool resolves paths through the
    // execution-aware context; policy inspection only needs an implicit command
    // cwd when the caller omitted one.
    let policy_args = policy_arguments(ctx, name, args);
    let soft_capabilities = if name == "exec_command" {
        match assess_command_for_workspace(&policy_args, &ctx.policy, Some(&ctx.workspace)) {
            Ok(capabilities) => capabilities,
            Err(error) => return policy_tool_err(error),
        }
    } else {
        if let Err(e) = validate_tool_arguments_for_workspace(
            name,
            &policy_args,
            &ctx.policy,
            Some(&ctx.workspace),
        ) {
            return policy_tool_err(e);
        }
        Vec::new()
    };
    let mut consumed_approval: Option<(String, ApprovalScope, Value)> = None;
    // Automatic policy takes precedence over stale approval IDs on retries.
    // Hard validation above and the one-shot runner handoff below still apply.
    if !soft_capabilities.is_empty() && !ctx.policy.auto_approves_permissions() {
        let scope = approval_scope(ctx, session_identity);
        let effect_args = approval_effect_args(&policy_args);
        if let Some(approval_id) = args
            .get("approval_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if let Err(code) = ctx
                .approval_store()
                .consume(approval_id, &scope, name, &effect_args)
            {
                return approval_decision_error(code);
            }
            consumed_approval = Some((approval_id.to_string(), scope, effect_args));
        } else {
            let request_id = args
                .get("client_request_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            match ctx
                .approval_store()
                .request_with_details(
                    scope,
                    name,
                    &effect_args,
                    request_id,
                    capability_labels(&soft_capabilities),
                    safe_operation_summary(name, &policy_args),
                    ctx.execution_isolation_mode().as_str().to_string(),
                )
            {
                Ok(status) => return approval_required(ctx, name, &soft_capabilities, &status),
                Err(code) => return approval_decision_error(code),
            }
        }
    }

    // Create the runner handoff only after all approval decisions have
    // succeeded. The private token binds the exact effect and current policy
    // revision to this one dispatch attempt.
    // Keep the runner handoff tied to the caller's actual arguments. The
    // policy copy may contain an implicit `workdir` used only for capability
    // assessment; passing that synthetic field to the runner would resolve it
    // relative to the already-selected default cwd a second time.
    let authorized_invocation =
        (name == "exec_command").then(|| AuthorizedInvocation::new(ctx, name, args));

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
        "exec_command" => authorized_invocation
            .as_ref()
            .ok_or_else(|| WorkspaceError::invalid_argument("exec authorization is missing"))
            .and_then(|invocation| exec::exec_command_authorized(ctx, args, invocation)),
        "read_output" => session::read_output(ctx, args),
        "write_stdin" => session::write_stdin(ctx, args),
        "kill_session" => session::kill_session(ctx, args),
        "git_status" => git::git_status(ctx, args),
        "git_diff" => git::git_diff(ctx, args),
        "git_log" => git::git_log(ctx, args),
        "git_show" => git::git_show(ctx, args),
        "git_blame" => git::git_blame(ctx, args),
        "view_image" => image_tool::view_image(ctx, args),
        "permission_status" => permission_status(ctx),
        "request_permissions" => request_permissions(ctx, args, session_identity),
        "get_approval_status" => get_approval_status(ctx, args, session_identity),
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
    if let Some((approval_id, scope, effect_args)) = consumed_approval.as_ref() {
        if name == "exec_command" && launch_failed_output(&output) {
            let _ = ctx
                .approval_store()
                .mark_launch_failed(approval_id, scope, name, effect_args);
        }
    }
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

fn approval_scope(ctx: &ToolContext, session_identity: Option<&str>) -> ApprovalScope {
    let context = session_identity
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("local:{}", ctx.execution_fingerprint()));
    let task = ctx
        .harness
        .current_task()
        .ok()
        .flatten()
        .map(|task| task.id)
        .unwrap_or_default();
    ApprovalScope {
        context,
        task,
        execution_root: ctx.execution_root_display(),
        policy_revision: ctx.permission_revision(),
    }
}

fn approval_effect_args(args: &Value) -> Value {
    let Some(object) = args.as_object() else {
        return args.clone();
    };
    let mut effect = object.clone();
    effect.remove("approval_id");
    effect.remove("client_request_id");
    Value::Object(effect)
}

fn capability_labels(capabilities: &[CommandCapability]) -> Vec<String> {
    capabilities
        .iter()
        .map(|capability| match capability {
            CommandCapability::ShellSyntax => "shell_syntax",
            CommandCapability::DangerousOperation => "dangerous_operation",
            CommandCapability::Network => "network",
            CommandCapability::UnlistedExecutable => "unlisted_executable",
            CommandCapability::CustomEnvironment => "custom_environment",
        })
        .map(str::to_string)
        .collect()
}

/// Return only a bounded executable label and argument count. The raw command
/// may contain tokens, URLs, or secrets and must never enter the approval DTO.
fn safe_operation_summary(tool: &str, args: &Value) -> String {
    if tool != "exec_command" {
        return tool.to_string();
    }
    let Some(command) = args.get("cmd").and_then(Value::as_str) else {
        return "exec_command".into();
    };
    let Ok(parts) = shell_words::split(command) else {
        return "exec_command".into();
    };
    let Some(executable) = parts.first() else {
        return "exec_command".into();
    };
    let label = executable
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(executable)
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
        .take(48)
        .collect::<String>();
    let label = if label.is_empty() { "program" } else { &label };
    format!("exec_command: {label} (+{} args)", parts.len().saturating_sub(1))
}

fn approval_required(
    ctx: &ToolContext,
    tool: &str,
    capabilities: &[CommandCapability],
    status: &ApprovalStatus,
) -> Value {
    let mut result = tool_err_code_with_details(
        "PERMISSION_APPROVAL_REQUIRED",
        "该操作需要桌面确认后才能执行。",
        "permission",
        json!({
            "status": "approval_required",
            "approval_id": status.approval_id,
            "approval": status,
            "tool": tool,
            "capabilities": capabilities,
            "workspace": ctx.workspace.root_display(),
            "execution_root": ctx.execution_root_display(),
            "retryable": true,
            "suggestion": "在桌面批准后，使用相同参数重试；不要自动循环申请。"
        }),
    );
    result["approval_id"] = Value::String(status.approval_id.clone());
    result
}

fn approval_decision_error(code: &str) -> Value {
    let (status, message, retryable) = match code {
        "approval_required" => (
            "approval_required",
            "该操作尚未获得桌面批准。",
            true,
        ),
        "approval_expired" => ("approval_expired", "该审批已过期，请重新发起请求。", true),
        "approval_denied" => ("approval_denied", "该操作被用户拒绝。", false),
        "approval_consumed" => ("approval_consumed", "该审批已经消费，不能重复执行。", false),
        "approval_launch_failed" => (
            "launch_failed",
            "该审批对应的进程未能启动，请重新评估并申请一次新的审批。",
            true,
        ),
        "approval_invalidated" => (
            "approval_invalidated",
            "审批绑定的工作区、策略或参数已变化。",
            true,
        ),
        "approval_request_conflict" => (
            "approval_request_conflict",
            "client_request_id 已绑定到不同的操作。",
            false,
        ),
        "approval_queue_full" => (
            "gateway_busy",
            "审批队列已满，请稍后重试。",
            true,
        ),
        _ => ("approval_unavailable", "审批状态暂时不可用。", true),
    };
    json!({
        "ok": false,
        "status": status,
        "summary": message,
        "error": {
            "code": code,
            "message": message,
            "category": "permission",
            "retryable": retryable,
            "details": {}
        }
    })
}

fn launch_failed_output(output: &Value) -> bool {
    let error_code = output
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str);
    if matches!(
        error_code,
        Some(
            "COMMAND_REJECTED"
                | "COMMAND_SPAWN_FAILED"
                | "EXEC_SANDBOX_UNAVAILABLE"
                | "EXEC_SANDBOX_INIT_FAILED"
                | "GIT_RUNTIME_STAGING_FAILED"
        )
    ) {
        return true;
    }
    let status = output.get("status").and_then(Value::as_str);
    let termination_reason = output
        .get("termination_reason")
        .and_then(Value::as_str)
        .or_else(|| {
            output
                .get("error")
                .and_then(|error| error.get("details"))
                .and_then(|details| details.get("termination_reason"))
                .and_then(Value::as_str)
        });
    status == Some("spawn_failed") || termination_reason == Some("spawn_failed")
}

fn request_permissions(
    ctx: &ToolContext,
    args: &Value,
    session_identity: Option<&str>,
) -> Result<Value, WorkspaceError> {
    let tool_name = args
        .get("tool_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| WorkspaceError::invalid_argument("tool_name is required"))?;
    let requested_permission = args
        .get("permission")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| WorkspaceError::invalid_argument("permission is required"))?;
    let operation_args = args
        .get("arguments")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| WorkspaceError::invalid_argument("arguments must be an object"))?;
    if args
        .get("scope")
        .and_then(Value::as_str)
        .is_some_and(|scope| scope != "once")
    {
        return Err(WorkspaceError::invalid_argument(
            "permission scope must be once",
        ));
    }
    if args
        .get("ttl_seconds")
        .and_then(Value::as_u64)
        .is_some_and(|ttl| ttl != 600)
    {
        return Err(WorkspaceError::invalid_argument(
            "permission ttl_seconds is fixed at 600 seconds",
        ));
    }

    // Validate the concrete operation before deciding whether a soft gate can
    // be approved. Automatic approval never removes hard scope or
    // argument validation.
    let policy_args = policy_arguments(ctx, tool_name, &operation_args);
    let capabilities = match tool_name {
        "exec_command" => assess_command_for_workspace(
            &policy_args,
            &ctx.policy,
            Some(&ctx.workspace),
        )
        .map_err(policy_tool_err_to_workspace_error)?,
        _ => {
            return Err(WorkspaceError::invalid_argument(
                "permission requests only support exec_command",
            ));
        }
    };

    if !is_supported_permission(requested_permission) {
        return Err(WorkspaceError::invalid_argument(
            "permission is not a supported one-shot capability",
        ));
    }

    // A request is always tied to a concrete capability observed in the
    // operation. Automatic policy removes the human approval step, but must not
    // turn this endpoint into a generic permission grant for unrelated work.
    if !capability_requested(requested_permission, &capabilities) {
        return Err(WorkspaceError::ToolDetails {
            code: "PERMISSION_CAPABILITY_MISMATCH",
            message: "Requested permission is not present in the evaluated operation.".into(),
            category: "permission",
            retryable: false,
            details: json!({
                "requested_permission": requested_permission,
                "capabilities": capability_labels(&capabilities),
            }),
        });
    }

    if ctx.policy.auto_approves_permissions() {
        return Ok(tool_ok(json!({
            "ok": true,
            "status": "granted",
            "mode": ctx.permission_mode(),
            "authority": "configured_workspace_policy",
            "desktop_approval_required": false,
            "sandbox_bypass": ctx.host_access(),
            "isolation_policy": ctx.policy.permissions.isolation,
            "fallback_allowed": ctx.exec_sandbox_status().fallback_allowed,
            "process_elevated": process_elevated(),
            "workspace": ctx.workspace.root_display()
        })));
    }

    let client_request_id = args
        .get("client_request_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let effect_args = approval_effect_args(&policy_args);
    let status = ctx
        .approval_store()
        .request_with_details(
            approval_scope(ctx, session_identity),
            tool_name,
            &effect_args,
            client_request_id,
            capability_labels(&capabilities),
            safe_operation_summary(tool_name, &policy_args),
            ctx.execution_isolation_mode().as_str().to_string(),
        )
        .map_err(|code| WorkspaceError::ToolDetails {
            code: "APPROVAL_UNAVAILABLE",
            message: format!("Permission approval could not be created: {code}"),
            category: "permission",
            retryable: true,
            details: json!({"reason": code}),
        })?;
    Ok(approval_required(ctx, tool_name, &capabilities, &status))
}

fn capability_requested(permission: &str, capabilities: &[CommandCapability]) -> bool {
    let requested = match permission {
        "network" => "network",
        "destructive_command" => "dangerous_operation",
        "shell_expansion" | "inline_script" => "shell_syntax",
        "privileged_executable" => "unlisted_executable",
        "sensitive_env" => "custom_environment",
        _ => return false,
    };
    capability_labels(capabilities)
        .iter()
        .any(|capability| capability == requested)
}

fn is_supported_permission(permission: &str) -> bool {
    matches!(
        permission,
        "network"
            | "destructive_command"
            | "shell_expansion"
            | "inline_script"
            | "privileged_executable"
            | "sensitive_env"
    )
}

fn policy_tool_err_to_workspace_error(err: PolicyError) -> WorkspaceError {
    let message = err.0;
    let code = if message.starts_with("PROTECTED_REPOSITORY_ASSET:") {
        "PROTECTED_REPOSITORY_ASSET"
    } else if message.starts_with("NETWORK_NOT_ALLOWED:") {
        "NETWORK_NOT_ALLOWED"
    } else {
        "POLICY_REJECTED"
    };
    WorkspaceError::ToolDetails {
        code,
        message,
        category: "policy",
        retryable: false,
        details: json!({"stage": "permission_assessment"}),
    }
}

fn get_approval_status(
    ctx: &ToolContext,
    args: &Value,
    session_identity: Option<&str>,
) -> Result<Value, WorkspaceError> {
    let approval_id = args
        .get("approval_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| WorkspaceError::invalid_argument("approval_id is required"))?;
    let context = approval_scope(ctx, session_identity).context;
    let Some(status) = ctx.approval_store().status(&context, approval_id) else {
        return Ok(json!({
            "ok": false,
            "status": "not_found",
            "summary": "审批不存在、已过期清理或不属于当前会话。",
            "error": {
                "code": "APPROVAL_NOT_FOUND",
                "message": "Approval is not available for this conversation.",
                "category": "permission",
                "retryable": false,
                "details": {}
            }
        }));
    };
    Ok(tool_ok(json!({
        "ok": true,
        "status": status.decision,
        "approval": status
    })))
}

fn tool_err_code_with_details(
    code: &'static str,
    message: impl Into<String>,
    category: &'static str,
    details: Value,
) -> Value {
    let message = message.into();
    json!({
        "ok": false,
        "status": "approval_required",
        "summary": message.clone(),
        "error": {
            "code": code,
            "message": message,
            "category": category,
            "retryable": true,
            "details": details
        }
    })
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

fn process_elevated() -> Option<bool> {
    #[cfg(windows)]
    { crate::security::exec_sandbox::process_elevated() }
    #[cfg(not(windows))]
    { None }
}

fn isolation_failure_reason(ctx: &ToolContext, sandbox: &crate::security::exec_sandbox::SandboxStatus) -> Option<&'static str> {
    if ctx.host_access() && !sandbox.available { Some("HOST_BACKEND_UNSUPPORTED") } else { (ctx.requires_strict_exec_isolation() && !sandbox.enforced).then_some("EXEC_SANDBOX_UNAVAILABLE") }
}

fn isolation_capabilities(
    ctx: &ToolContext,
    sandbox: &crate::security::exec_sandbox::SandboxStatus,
) -> Value {
    let filesystem_mode = if ctx.host_access() {
        "host"
    } else if sandbox.enforced {
        "workspace"
    } else if ctx.requires_strict_exec_isolation() {
        "unavailable"
    } else {
        "policy_only"
    };
    let network_allowed = ctx.policy.network_allowed();
    json!({
        "backend": sandbox.implementation.clone(),
        "requested_mode": ctx.policy.permission_snapshot().isolation,
        "execution_mode": ctx.execution_isolation_mode().as_str(),
        "read": {
            "mode": filesystem_mode,
            "enforced": sandbox.enforced
        },
        "write": {
            "mode": filesystem_mode,
            "enforced": sandbox.enforced
        },
        "network": {
            "mode": if ctx.host_access() { "host" } else if network_allowed { "policy_only" } else { "disabled" },
            "allowed": network_allowed,
            "enforced": false
        },
        "fallback_allowed": sandbox.fallback_allowed,
        "failure_reason": isolation_failure_reason(ctx, sandbox)
    })
}

pub fn permission_status(ctx: &ToolContext) -> Result<Value, WorkspaceError> {
    let sandbox = ctx.exec_sandbox_status();
    let failure_reason = isolation_failure_reason(ctx, &sandbox);
    let backend = sandbox.implementation.clone();
    let isolation = isolation_capabilities(ctx, &sandbox);
    Ok(tool_ok(json!({
        "permission_mode": ctx.permission_mode(),
        "execution_isolation_mode": ctx.execution_isolation_mode().as_str(),
        "configured_permission_mode": ctx.permission_mode(),
        "permission_policy": ctx.policy.permission_snapshot(),
        "desktop_approval_required": !ctx.policy.auto_approves_permissions(),
        "sandbox_enforced": sandbox.enforced,
        "sandbox_bypass": ctx.host_access(),
            "isolation_policy": ctx.policy.permissions.isolation,
            "fallback_allowed": ctx.exec_sandbox_status().fallback_allowed,
            "process_elevated": process_elevated(),
        "isolation_backend": backend,
        "workspace_exec_available": sandbox.available,
        "isolation": isolation.clone(),
        "isolation_capabilities": isolation,
        "isolation_failure_reason": failure_reason,
        "network_allowed": ctx.policy.network_allowed(),
        "filesystem_sandbox": {
            "available": sandbox.available,
            "enforced": sandbox.enforced,
            "implementation": sandbox.implementation.clone(),
            "fallback_allowed": sandbox.fallback_allowed,
            "failure_reason": failure_reason
        },
        "workspace": ctx.workspace.root_display(),
        "repository_root": ctx.repository_root_display(),
        "execution_root": ctx.execution_root_display()
    })))
}

pub fn server_info(ctx: &ToolContext) -> Result<Value, WorkspaceError> {
    let tools = crate::tools::registry::exposed_tool_names(&ctx.tool_profile);
    let sandbox = ctx.exec_sandbox_status();
    let failure_reason = isolation_failure_reason(ctx, &sandbox);
    let backend = sandbox.implementation.clone();
    let isolation = isolation_capabilities(ctx, &sandbox);
    Ok(tool_ok(json!({
        "server": "coding-tools-mcp",
        "title": "Coding Tools MCP",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": "2025-06-18",
        "workspace": ctx.workspace.root_display(),
        "repository_root": ctx.repository_root_display(),
        "execution_root": ctx.execution_root_display(),
        "permission_mode": ctx.permission_mode(),
        "configured_permission_mode": ctx.permission_mode(),
        "permission_policy": ctx.policy.permission_snapshot(),
        "desktop_approval_required": !ctx.policy.auto_approves_permissions(),
        "sandbox_enforced": sandbox.enforced,
        "sandbox_bypass": ctx.host_access(),
            "isolation_policy": ctx.policy.permissions.isolation,
            "fallback_allowed": ctx.exec_sandbox_status().fallback_allowed,
            "process_elevated": process_elevated(),
        "default_cwd": ctx.default_cwd_display(),
        "network_allowed": ctx.policy.network_allowed(),
        "execution_isolation_mode": ctx.execution_isolation_mode().as_str(),
        "isolation_backend": backend,
        "workspace_exec_available": sandbox.available,
        "isolation": isolation.clone(),
        "isolation_capabilities": isolation,
        "isolation_failure_reason": failure_reason,
        "filesystem_sandbox": {
            "available": sandbox.available,
            "enforced": sandbox.enforced,
            "implementation": sandbox.implementation.clone(),
            "fallback_allowed": sandbox.fallback_allowed,
            "failure_reason": failure_reason
        },
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
    let failure_reason = isolation_failure_reason(ctx, &sandbox);
    let backend = sandbox.implementation.clone();
    let isolation = isolation_capabilities(ctx, &sandbox);
    let strict = ctx.requires_strict_exec_isolation();
    let warnings = if strict && !sandbox.enforced {
        vec!["Generic child processes are disabled until an OS filesystem sandbox is available"]
    } else if ctx.host_access() {
        vec!["Host execution uses the current Windows account without an OS sandbox"]
    } else if !strict {
        vec!["Workspace child processes use policy-only execution; no OS filesystem sandbox is enforced"]
    } else {
        Vec::new()
    };
    Ok(tool_ok(json!({
        "workspace": ctx.workspace.root_display(),
        "repository_root": ctx.repository_root_display(),
        "execution_root": ctx.execution_root_display(),
        "permission_mode": ctx.permission_mode(),
        "configured_permission_mode": ctx.permission_mode(),
        "permission_policy": ctx.policy.permission_snapshot(),
        "desktop_approval_required": !ctx.policy.auto_approves_permissions(),
        "sandbox_enforced": sandbox.enforced,
        "sandbox_bypass": ctx.host_access(),
            "isolation_policy": ctx.policy.permissions.isolation,
            "fallback_allowed": ctx.exec_sandbox_status().fallback_allowed,
            "process_elevated": process_elevated(),
        "network_allowed": ctx.policy.network_allowed(),
        "execution_isolation_mode": ctx.execution_isolation_mode().as_str(),
        "isolation_backend": backend,
        "workspace_exec_available": sandbox.available,
        "isolation": isolation.clone(),
        "isolation_capabilities": isolation,
        "isolation_failure_reason": failure_reason,
        "landlock_enabled": false,
        "filesystem_sandbox": {
            "available": sandbox.available,
            "enforced": sandbox.enforced,
            "implementation": sandbox.implementation.clone(),
            "fallback_allowed": sandbox.fallback_allowed,
            "failure_reason": failure_reason,
            "default_scope": if ctx.host_access() { "host" } else { "workspace" },
            "host_scope_available": ctx.host_access() && sandbox.available
        },
        "global_tmp_write": if ctx.host_access() { "host" } else { "tmp-prefix" },
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

#[cfg(test)]
mod automatic_approval_tests {
    use super::*;

    #[test]
    fn automatic_approval_does_not_enqueue_or_consume_desktop_requests() {
        let temp = tempfile::tempdir().unwrap();
        let ctx = ToolContext::new(temp.path().to_path_buf()).unwrap();
        for (permission, arguments) in [
            ("network", json!({"cmd": "curl https://example.com"})),
            ("shell_expansion", json!({"cmd": "echo a && echo b"})),
            ("destructive_command", json!({"cmd": "git reset --hard"})),
            ("privileged_executable", json!({"cmd": "custom-build-tool"})),
            ("sensitive_env", json!({"cmd": "echo ok", "env": {"BUILD_MODE": "test"}})),
        ] {
            let result = call_tool(&ctx, "request_permissions", &json!({
                "tool_name": "exec_command", "permission": permission,
                "reason": "automatic approval regression", "arguments": arguments
            }));
            assert_eq!(result["status"], "granted", "{result}");
        }
        assert!(ctx.approval_store().pending().is_empty());

        let args = json!({"cmd": "echo approval-ok", "env": {"BUILD_MODE": "test"}});
        let scope = approval_scope(&ctx, Some("conversation-a"));
        let pending = ctx.approval_store().request_with_details(
            scope.clone(), "exec_command", &args, None,
            vec!["custom_environment".into()], "exec_command: echo".into(),
            ctx.execution_isolation_mode().as_str().into(),
        ).unwrap();
        ctx.approval_store().decide(&pending.approval_id, false).unwrap();
        let mut retry = args;
        retry["approval_id"] = json!(pending.approval_id);
        let result = call_tool_with_identity(&ctx, "exec_command", &retry, Some("conversation-a"));
        assert_eq!(result["command_ok"], true, "{result}");
        let status = get_approval_status(&ctx, &json!({"approval_id": pending.approval_id}), Some("conversation-a")).unwrap();
        assert_eq!(status["status"], "denied");
        assert!(ctx.approval_store().pending().is_empty());
    }

    #[test]
    fn automatic_approval_keeps_hard_policy_rejections() {
        let temp = tempfile::tempdir().unwrap();
        for preset in ["default", "full_access"] {
            let mut ctx = ToolContext::new(temp.path().to_path_buf()).unwrap();
            ctx.policy.permissions = crate::tools::policy::ExecutionPolicy::from_config(
                preset, 1, Default::default(),
            ).unwrap();
            for args in [
                json!({"cmd": "echo ok", "workdir": "../sibling"}),
                json!({"cmd": "echo ok", "filesystem_scope": "host"}),
                json!({"cmd": "echo bad > .git/config"}),
                json!({"cmd": "echo bad > .github/workflows/ci.yml"}),
                json!({"cmd": "echo ok", "timeout_ms": 600001}),
                json!({"cmd": "echo ok", "env": {"VALUE": 12}}),
            ] {
                let result = call_tool(&ctx, "exec_command", &args);
                assert_eq!(result["ok"], false, "{result}");
                assert_ne!(result["status"], "approval_required", "{result}");
                assert_eq!(result["error"]["category"], "policy");
            }
            ctx.policy.permissions.network_allowed = false;
            let result = call_tool(&ctx, "request_permissions", &json!({
                "tool_name": "exec_command", "permission": "network",
                "arguments": {"cmd": "curl https://example.com"}
            }));
            assert_eq!(result["ok"], false, "{result}");
            assert_eq!(result["error"]["code"], "NETWORK_NOT_ALLOWED");
            assert!(ctx.approval_store().pending().is_empty());
        }
    }
}

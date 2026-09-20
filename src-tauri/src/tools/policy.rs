use std::collections::HashSet;
use std::path::{Component, Path};

use serde_json::Value;

use crate::tools::workspace::Workspace;
use crate::workspace::ActionsConfig;
pub use crate::security::permission::{ExecutionPolicy, IsolationPolicy};

use super::registry::is_allowed_tool;

static NETWORK_COMMAND_PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
static DANGEROUS_COMMAND_PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
static INTERPRETER_MUTATION_PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

const BASIC_READ_ONLY_COMMANDS: &[&str] = &[
    "pwd", "ls", "dir", "cat", "head", "tail", "grep", "find", "which", "echo",
];

const DEFAULT_ALLOWED_COMMANDS: &[&str] = &[
    "pytest",
    "python",
    "python3",
    "npm",
    "npx",
    "node",
    "pnpm",
    "yarn",
    "make",
    "mvn",
    "mvnw",
    "gradle",
    "gradlew",
    "cargo",
    "go",
    "ruff",
    "mypy",
    "eslint",
    "tsc",
    "msbuild",
    "dotnet",
    "deno",
    "bun",
    "ruby",
    "java",
    "javac",
    "cmake",
    "clang",
    "gcc",
    "g++",
    "git",
    "cmd",
    "powershell",
    "pwsh",
];

#[derive(Debug, Clone)]
pub struct PolicySettings {
    pub allowed_commands: HashSet<String>,
    pub workspace_local_entries: bool,
    pub workspace_script_extensions: HashSet<String>,
    pub max_patch_bytes: usize,
    pub permissions: ExecutionPolicy,
}

impl Default for PolicySettings {
    fn default() -> Self {
        Self {
            allowed_commands: default_allowed_command_set(),
            workspace_local_entries: true,
            workspace_script_extensions: default_workspace_script_extension_set(),
            max_patch_bytes: 200_000,
            permissions: ExecutionPolicy::from_config("default", 1, IsolationPolicy::Strict)
                .expect("built-in policy"),
        }
    }
}

impl PolicySettings {
    pub fn permission_snapshot(&self) -> &ExecutionPolicy {
        &self.permissions
    }

    pub fn from_runtime(runtime: &crate::workspace::RuntimeConfig) -> Result<Self, String> {
        Ok(Self {
            allowed_commands: merge_default_allowed_commands(&runtime.allowed_commands),
            workspace_local_entries: runtime.workspace_local_entries,
            workspace_script_extensions: parse_workspace_script_extensions(
                &runtime.workspace_script_extensions,
            ),
            max_patch_bytes: 200_000,
            permissions: ExecutionPolicy::from_config(&runtime.permission_mode,
                runtime.permission_policy_version, runtime.isolation_policy)?,
        })
    }

    pub fn from_actions_config(actions: &ActionsConfig) -> Result<Self, String> {
        Ok(Self {
            allowed_commands: merge_default_allowed_commands(&actions.allowed_commands),
            workspace_local_entries: true,
            workspace_script_extensions: default_workspace_script_extension_set(),
            max_patch_bytes: actions.max_patch_bytes as usize,
            permissions: ExecutionPolicy::from_config(&actions.permission_mode,
                actions.permission_policy_version, actions.isolation_policy)?,
        })
    }

    pub fn network_allowed(&self) -> bool {
        self.permissions.network_allowed
    }

    /// Approval policy applies only to soft capabilities. Hard workspace,
    /// network and sandbox checks are never skipped by this method.
    pub fn auto_approves_permissions(&self) -> bool {
        self.permissions.auto_approves_soft_permissions()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct PolicyError(pub String);

pub fn parse_allowed_commands(configured: &str) -> HashSet<String> {
    let trimmed = configured.trim();
    if trimmed.is_empty() {
        return default_allowed_command_set();
    }
    let mut commands: HashSet<String> = trimmed
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    // 基础诊断命令是工作区可用性的最低保障，不应因 Actions 配置遗漏而失效。
    commands.extend(BASIC_READ_ONLY_COMMANDS.iter().map(|s| s.to_string()));
    commands
}

pub fn parse_workspace_script_extensions(configured: &str) -> HashSet<String> {
    let mut extensions = configured
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.starts_with('.') {
                value.to_ascii_lowercase()
            } else {
                format!(".{}", value.to_ascii_lowercase())
            }
        })
        .collect::<HashSet<_>>();
    if extensions.is_empty() {
        extensions = default_workspace_script_extension_set();
    }
    extensions
}

fn default_allowed_command_set() -> HashSet<String> {
    DEFAULT_ALLOWED_COMMANDS
        .iter()
        .map(|s| s.to_string())
        .chain(BASIC_READ_ONLY_COMMANDS.iter().map(|s| s.to_string()))
        .collect()
}

fn merge_default_allowed_commands(configured: &str) -> HashSet<String> {
    let mut commands = default_allowed_command_set();
    commands.extend(parse_allowed_commands(configured));
    commands
}

fn default_workspace_script_extension_set() -> HashSet<String> {
    [".exe", ".bat", ".cmd", ".ps1"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

pub fn validate_tool_arguments(
    tool_name: &str,
    arguments: &Value,
    policy: &PolicySettings,
) -> Result<(), PolicyError> {
    validate_tool_arguments_for_workspace(tool_name, arguments, policy, None)
}

pub fn validate_tool_arguments_for_workspace(
    tool_name: &str,
    arguments: &Value,
    policy: &PolicySettings,
    workspace: Option<&Workspace>,
) -> Result<(), PolicyError> {
    match tool_name {
        "exec_command" => validate_command_for_workspace(arguments, policy, workspace),
        "apply_patch" | "patch_check" => validate_patch(arguments, policy),
        _ => Ok(()),
    }
}

/// Actions OpenAPI 暴露层校验：仅限制「能否调用」，不参与执行逻辑。
pub fn validate_actions_exposure(tool_name: &str) -> Result<(), PolicyError> {
    if is_allowed_tool(tool_name) {
        Ok(())
    } else {
        Err(PolicyError(format!("Tool is not exposed: {tool_name}")))
    }
}

pub fn validate_command(arguments: &Value, policy: &PolicySettings) -> Result<(), PolicyError> {
    validate_command_for_workspace(arguments, policy, None)
}

pub fn validate_command_for_workspace(
    arguments: &Value,
    policy: &PolicySettings,
    workspace: Option<&Workspace>,
) -> Result<(), PolicyError> {
    let capabilities = assess_command_for_workspace(arguments, policy, workspace)?;
    if policy.auto_approves_permissions() {
        return Ok(());
    }
    if let Some(capability) = capabilities.first() {
        return Err(PolicyError(capability.legacy_message().into()));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandCapability {
    ShellSyntax,
    DangerousOperation,
    Network,
    UnlistedExecutable,
    CustomEnvironment,
}

impl CommandCapability {
    fn legacy_message(self) -> &'static str {
        match self {
            Self::ShellSyntax => "Shell chaining, redirection and expansion are not allowed",
            Self::DangerousOperation => "DANGEROUS_OPERATION_REQUIRES_CONFIRMATION: dangerous command requires confirm=true",
            Self::Network => "Network-looking commands are blocked in safe permission mode",
            Self::UnlistedExecutable => "Command is not allowlisted",
            Self::CustomEnvironment => "Environment variables cannot be supplied by GPT",
        }
    }
}

/// Structural failures are errors; soft capabilities are data, never inferred
/// from an error message. The dispatcher owns approval of the returned set.
pub fn assess_command_for_workspace(
    arguments: &Value,
    policy: &PolicySettings,
    workspace: Option<&Workspace>,
) -> Result<Vec<CommandCapability>, PolicyError> {
    let mut capabilities = Vec::new();
    let command = arguments
        .get("cmd")
        .and_then(Value::as_str)
        .ok_or_else(|| PolicyError("exec_command requires a non-empty cmd".into()))?;
    if command.trim().is_empty() || command.contains('\0') {
        return Err(PolicyError("exec_command requires a non-empty cmd".into()));
    }
    if command.len() > 4_000 {
        return Err(PolicyError("Command is too long".into()));
    }
    let host_access = policy.permissions.host_access();
    let filesystem_scope = arguments
        .get("filesystem_scope")
        .and_then(Value::as_str)
        .unwrap_or(if host_access { "host" } else { "workspace" });
    if filesystem_scope != if host_access { "host" } else { "workspace" } {
        return Err(PolicyError(
            if host_access { "ISOLATION_CAPABILITY_UNSATISFIED: host execution does not enforce workspace isolation" } else { "EXTERNAL_EXECUTION_NOT_ALLOWED: host scope requires configured host execution" }.into(),
        ));
    }
    for key in ["workdir", "cwd"] {
        if let Some(workdir) = arguments.get(key).and_then(Value::as_str) {
            let path = Path::new(workdir);
            if !host_access && (path.is_absolute() || path.components().any(|part| part == Component::ParentDir)) {
                return Err(PolicyError(
                    "workdir must stay inside the configured workspace".into(),
                ));
            }
        }
    }
    // Child processes inherit the workspace boundary, so path-like command
    // arguments must not be allowed to name an absolute path or parent escape.
    // This check applies before soft capability evaluation, including Full
    // Access, because the permission preset cannot widen the execution root.
    if !host_access && command_contains_external_path(command) {
        return Err(PolicyError(
            "WORKSPACE_PATH_PROTECTED: command references a path outside the active workspace"
                .into(),
        ));
    }
    if has_forbidden_shell_syntax(command) {
        if !host_access && command_targets_protected_repository_asset(command) {
            return Err(PolicyError(
                "PROTECTED_REPOSITORY_ASSET: shell command cannot modify .git/.github"
                    .into(),
            ));
        }
        capabilities.push(CommandCapability::ShellSyntax);
    }
    if !host_access && (dangerous_command_pattern().is_match(command)
        || interpreter_mutation_pattern().is_match(command))
        && command_targets_protected_repository_asset(command)
    {
        return Err(PolicyError(
            "PROTECTED_REPOSITORY_ASSET: 禁止删除或递归清空 .git/.github".into(),
        ));
    }
    if !host_access && interpreter_mutation_pattern().is_match(command) && command_contains_external_path(command) {
        return Err(PolicyError(
            "WORKSPACE_PATH_PROTECTED: workspace scope 禁止通过子进程写入 Workspace 外部路径"
                .into(),
        ));
    }
    if dangerous_command_pattern().is_match(command)
        && (policy.permissions.legacy.is_none() || !arguments
            .get("confirm")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    {
        capabilities.push(CommandCapability::DangerousOperation);
    }
    if network_command_pattern().is_match(command) {
        // Network policy is a hard capability boundary. A desktop approval
        // cannot turn a workspace that explicitly disabled network access back
        // on. When network is allowed by configuration, its soft approval
        // follows the same automatic policy as other capabilities.
        if !policy.network_allowed() {
            return Err(PolicyError(
                "NETWORK_NOT_ALLOWED: network access is disabled by workspace policy".into(),
            ));
        }
        capabilities.push(CommandCapability::Network);
    }

    if !host_access {
    let parts =
        shell_words::split(command).map_err(|_| PolicyError("Invalid command syntax".into()))?;
    if parts.is_empty() {
        return Err(PolicyError("Empty command".into()));
    }

    let executable = parts[0].trim_start_matches("./");
    let base_name = executable.rsplit(['/', '\\']).next().unwrap_or(executable);
    let stem = base_name
        .strip_suffix(".exe")
        .or_else(|| base_name.strip_suffix(".cmd"))
        .or_else(|| base_name.strip_suffix(".bat"))
        .unwrap_or(base_name);

    let workspace_entry_candidate = workspace_local_entry_exists(workspace, arguments, executable)
        || executable.contains(['/', '\\'])
        || policy
            .workspace_script_extensions
            .iter()
            .any(|extension| base_name.to_ascii_lowercase().ends_with(extension));
    if !(policy.allowed_commands.contains(stem)
        || (policy.workspace_local_entries && workspace_entry_candidate))
    {
        capabilities.push(CommandCapability::UnlistedExecutable);
    }

    }

    if arguments.get("env").is_some() {
        let env = arguments["env"].as_object()
            .ok_or_else(|| PolicyError("env must be an object of strings".into()))?;
        if env.iter().any(|(key, value)| key.is_empty() || key.contains(['=', '\0'])
            || value.as_str().is_none_or(|value| value.contains('\0'))) {
            return Err(PolicyError("Invalid environment variable name or value".into()));
        }
        capabilities.push(CommandCapability::CustomEnvironment);
    }

    if let Some(timeout_ms) = arguments.get("timeout_ms").and_then(Value::as_u64) {
        if timeout_ms > 600_000 {
            return Err(PolicyError("Command timeout exceeds 10 minutes".into()));
        }
    }

    Ok(capabilities)
}

fn workspace_local_entry_exists(
    workspace: Option<&Workspace>,
    arguments: &Value,
    executable: &str,
) -> bool {
    let Some(workspace) = workspace else {
        return false;
    };
    let workdir = arguments
        .get("workdir")
        .or_else(|| arguments.get("cwd"))
        .and_then(Value::as_str)
        .unwrap_or(".");
    let Ok(base) = workspace.resolve_existing(workdir) else {
        return false;
    };
    let candidate = if Path::new(executable).is_absolute() {
        Path::new(executable).to_path_buf()
    } else {
        base.path.join(executable)
    };
    candidate
        .canonicalize()
        .map(|path| path.is_file() && path.starts_with(workspace.root()))
        .unwrap_or(false)
}

pub fn validate_patch(arguments: &Value, policy: &PolicySettings) -> Result<(), PolicyError> {
    let patch = arguments
        .get("patch")
        .and_then(Value::as_str)
        .ok_or_else(|| PolicyError("apply_patch requires a patch".into()))?;
    if patch.trim().is_empty() {
        return Err(PolicyError("apply_patch requires a patch".into()));
    }

    if patch.len() > policy.max_patch_bytes {
        return Err(PolicyError("Patch is too large".into()));
    }

    Ok(())
}

fn has_forbidden_shell_syntax(command: &str) -> bool {
    if command.contains(['\r', '\n']) {
        return true;
    }

    let chars: Vec<char> = command.chars().collect();
    let mut quote = None;
    let mut escaped = false;
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }

        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                }
            }
            Some('"') => {
                if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    quote = None;
                }
            }
            Some(_) => {}
            None => {
                if ch == '\\' {
                    escaped = true;
                } else if ch == '\'' || ch == '"' {
                    quote = Some(ch);
                } else if matches!(ch, ';' | '&' | '|' | '>' | '<' | '`')
                    || (ch == '$'
                        && chars
                            .get(index + 1)
                            .is_some_and(|next| *next == '(' || *next == '{'))
                {
                    return true;
                }
            }
        }
        index += 1;
    }
    false
}

/// Exposed to the runner so native diagnostics never swallow a command that
/// contains real shell operators such as redirection or chaining.
pub(crate) fn command_has_shell_syntax(command: &str) -> bool {
    has_forbidden_shell_syntax(command)
}

fn network_command_pattern() -> &'static regex::Regex {
    NETWORK_COMMAND_PATTERN.get_or_init(|| {
        regex::Regex::new(
            r"(?i)(https?://|urllib\.request|requests\.|http\.client|\bcurl\b|\bwget\b|\bssh\b|\bscp\b|\bftp\b)",
        )
        .expect("valid regex")
    })
}

fn dangerous_command_pattern() -> &'static regex::Regex {
    DANGEROUS_COMMAND_PATTERN.get_or_init(|| {
        regex::Regex::new(
            r"(?i)(git\s+reset\s+--hard|git\s+clean\s+-[^\r\n]*f|git\s+checkout\s+--\s+\.|(^|\s)rm\s+(-[^\r\n]*r[^\r\n]*f|--recursive)|remove-item\s+[^\r\n]*-recurse|(^|\s)(rmdir|del)\s+/s\b)",
        )
        .expect("valid regex")
    })
}

fn interpreter_mutation_pattern() -> &'static regex::Regex {
    INTERPRETER_MUTATION_PATTERN.get_or_init(|| {
        regex::Regex::new(
            r#"(?i)(shutil\.(rmtree|move)|os\.(remove|unlink|rmdir)|pathlib\.[^\s;]+\.(unlink|rename)|write_text|write_bytes|fs\.(writefile|writefilesync|unlink|rm)|set-content|out-file|new-item|files?\.(write|delete)|open\([^)]*['\"]w)"#,
        )
        .expect("valid regex")
    })
}

fn command_contains_external_path(command: &str) -> bool {
    let normalized = command.replace('\\', "/");
    normalized.contains("../")
        || normalized.contains("..\\")
        || regex::Regex::new(r#"(?i)(^|["'\s])/[^"]"#)
            .expect("valid regex")
            .is_match(&normalized)
        || regex::Regex::new(r"(?i)\b[A-Z]:/")
            .expect("valid regex")
            .is_match(&normalized)
}

fn command_targets_protected_repository_asset(command: &str) -> bool {
    let normalized_command = command.to_ascii_lowercase().replace('\\', "/");
    let references_protected_asset =
        normalized_command.contains(".git") || normalized_command.contains(".github");
    if !references_protected_asset {
        return false;
    }

    let mutating_operation = [
        "rm ",
        "remove-item",
        "rmdir",
        "del ",
        "unlink",
        "rmtree",
        "write_text",
        "writefile",
        "rename",
        "move",
        "checkout",
        "clean ",
    ]
    .iter()
    .any(|needle| normalized_command.contains(needle));
    if mutating_operation {
        return true;
    }

    command.split_whitespace().any(|part| {
        let token = part
            .trim_matches(|ch: char| matches!(ch, '\'' | '"' | '`' | ',' | ';'))
            .replace('\\', "/");
        let token = token.strip_prefix("./").unwrap_or(&token);
        token == ".git"
            || token.starts_with(".git/")
            || token == ".github"
            || token.starts_with(".github/")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn permission_assessment_separates_soft_capabilities_from_hard_limits() {
        let mut policy = PolicySettings::default();
        policy.permissions = ExecutionPolicy::from_config("full_access", 1, IsolationPolicy::Strict).unwrap();
        let capabilities = assess_command_for_workspace(
            &json!({"cmd":"custom-build && echo done", "env":{"BUILD_MODE":"release"}}),
            &policy, None,
        ).unwrap();
        assert!(capabilities.contains(&CommandCapability::ShellSyntax));
        assert!(capabilities.contains(&CommandCapability::UnlistedExecutable));
        assert!(capabilities.contains(&CommandCapability::CustomEnvironment));
        for args in [
            json!({"cmd":"custom-build", "filesystem_scope":"host"}),
            json!({"cmd":"custom-build", "cwd":"../sibling"}),
            json!({"cmd":"custom-build", "timeout_ms":600001}),
            json!({"cmd":"custom-build", "env":{"A":12}}),
        ] {
            assert!(assess_command_for_workspace(&args, &policy, None).is_err());
        }
    }

    #[test]
    fn full_access_rejects_external_command_arguments() {
        let mut policy = PolicySettings::default();
        policy.permissions =
            ExecutionPolicy::from_config("full_access", 1, IsolationPolicy::Strict).unwrap();
        for command in [
            "python ../outside.py",
            r#"python C:\outside\script.py"#,
            r#"cmd /d /c "type C:\outside\secret.txt""#,
        ] {
            let error = assess_command_for_workspace(&json!({"cmd": command}), &policy, None)
                .expect_err("external command paths must remain a hard boundary");
            assert!(error.0.starts_with("WORKSPACE_PATH_PROTECTED:"), "{error}");
        }
    }

    #[test]
    fn permission_new_policy_does_not_treat_agent_confirmation_as_approval() {
        let mut policy = PolicySettings::default();
        policy.permissions = ExecutionPolicy::from_config("default", 1, IsolationPolicy::Strict).unwrap();
        let args = json!({"cmd":"git reset --hard", "confirm":true});
        let capabilities = assess_command_for_workspace(&args, &policy, None).unwrap();
        assert!(capabilities.contains(&CommandCapability::DangerousOperation));
    }

    #[test]
    fn default_network_is_soft_approval_when_policy_allows_it() {
        let mut policy = PolicySettings::default();
        policy.permissions = ExecutionPolicy::from_config("default", 1, IsolationPolicy::Strict)
            .expect("default policy");
        let capabilities = assess_command_for_workspace(
            &json!({"cmd": "curl https://example.com"}),
            &policy,
            None,
        )
        .expect("network assessment");
        assert!(capabilities.contains(&CommandCapability::Network));
    }

    #[test]
    fn disabled_network_is_hard_rejected_before_approval() {
        let policy = ExecutionPolicy::from_config("safe", 0, IsolationPolicy::Strict)
            .expect("legacy restricted policy");
        let policy = PolicySettings {
            permissions: policy,
            ..PolicySettings::default()
        };
        let error = assess_command_for_workspace(
            &json!({"cmd": "curl https://example.com"}),
            &policy,
            None,
        )
        .expect_err("disabled network must be hard rejected");
        assert!(error.0.starts_with("NETWORK_NOT_ALLOWED:"));
    }

    #[test]
    fn permission_config_missing_version_preserves_old_runtime_limits() {
        let runtime: crate::workspace::RuntimeConfig = serde_json::from_value(json!({
            "permission_mode": "dangerous"
        })).unwrap();
        let policy = PolicySettings::from_runtime(&runtime).unwrap();
        assert_eq!(runtime.permission_policy_version, 0);
        assert_eq!(policy.permissions.isolation, IsolationPolicy::Strict);
        assert!(policy.permissions.auto_approves_soft_permissions());
        assert!(validate_command(&json!({"cmd":"echo a > result.txt"}), &policy).is_ok());
        assert!(validate_command(&json!({"cmd":"echo a > ../result.txt"}), &policy).is_err());
    }

    #[test]
    fn permission_config_explicit_version_and_unknown_values_are_distinct() {
        let mut runtime: crate::workspace::RuntimeConfig = serde_json::from_value(json!({
            "permission_mode":"full_access", "permission_policy_version":1,
            "isolation_policy":"compatibility"
        })).unwrap();
        let policy = PolicySettings::from_runtime(&runtime).unwrap();
        assert!(policy.permissions.auto_approves_soft_permissions());
        assert_eq!(policy.permissions.isolation, IsolationPolicy::Compatibility);
        runtime.permission_policy_version = 2;
        assert!(PolicySettings::from_runtime(&runtime).is_err());
        runtime.permission_policy_version = 1;
        runtime.permission_mode = "dangerous".into();
        assert!(PolicySettings::from_runtime(&runtime).is_err());
    }

    #[test]
    fn workspace_allowed_commands_override_defaults() {
        let actions = ActionsConfig {
            allowed_commands: "cargo,go".into(),
            ..ActionsConfig::default()
        };
        let policy = PolicySettings::from_actions_config(&actions).unwrap();
        assert!(policy.allowed_commands.contains("cargo"));
        assert!(policy.allowed_commands.contains("pytest"));
    }

    #[test]
    fn trusted_mode_accepts_any_configured_workspace_script_extension() {
        let policy = PolicySettings {
            workspace_local_entries: true,
            workspace_script_extensions: parse_workspace_script_extensions(".cmd,.launcher"),
            ..PolicySettings::default()
        };
        assert!(
            validate_command(&serde_json::json!({ "cmd": "anything.launcher" }), &policy).is_ok()
        );
        assert!(validate_command(
            &serde_json::json!({ "cmd": "scripts/another-name.cmd" }),
            &policy
        )
        .is_ok());
    }

    #[test]
    fn trusted_mode_accepts_an_extensionless_workspace_entry() {
        let dir = tempfile::tempdir().expect("workspace");
        std::fs::write(dir.path().join("project-entry"), "#!/bin/sh\necho ok\n").expect("entry");
        let workspace = Workspace::new(dir.path().to_path_buf()).expect("workspace");
        assert!(validate_command_for_workspace(
            &serde_json::json!({ "cmd": "project-entry", "workdir": "." }),
            &PolicySettings::default(),
            Some(&workspace),
        )
        .is_ok());
    }

    #[test]
    fn patch_size_uses_workspace_limit() {
        let actions = ActionsConfig {
            max_patch_bytes: 10,
            ..ActionsConfig::default()
        };
        let policy = PolicySettings::from_actions_config(&actions).unwrap();
        let err = validate_patch(&json!({ "patch": "01234567890" }), &policy).unwrap_err();
        assert!(err.0.contains("too large"));
    }

    #[test]
    fn basic_diagnostic_commands_are_allowed() {
        let policy = PolicySettings::default();
        for command in BASIC_READ_ONLY_COMMANDS {
            validate_command(&json!({"cmd": command}), &policy)
                .unwrap_or_else(|err| panic!("{command} should be allowed: {err}"));
        }
    }

    #[test]
    fn configured_commands_keep_basic_diagnostics() {
        let actions = ActionsConfig {
            allowed_commands: "cargo,go".into(),
            ..ActionsConfig::default()
        };
        let policy = PolicySettings::from_actions_config(&actions).unwrap();
        assert!(validate_command(&json!({"cmd": "pwd"}), &policy).is_ok());
        assert!(validate_command(&json!({"cmd": "pytest"}), &policy).is_ok());
    }

    #[test]
    fn quoted_python_code_is_not_treated_as_shell_chaining() {
        let policy = PolicySettings::default();
        assert!(validate_command(
            &json!({"cmd": "python -c \"import os; print(os.getcwd())\""}),
            &policy
        )
        .is_ok());
        assert!(validate_command(
            &json!({"cmd": "python -c \"print(1)\" && echo nope"}),
            &policy
        )
        .is_ok());
        assert!(validate_command(&json!({"cmd": "echo hello > output.txt"}), &policy).is_ok());
    }
}

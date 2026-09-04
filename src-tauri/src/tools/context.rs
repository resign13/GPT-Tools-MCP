use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::harness::{Harness, HarnessError};
#[cfg(windows)]
use crate::security::exec_sandbox::SandboxChild;
use crate::security::exec_sandbox::{
    ExecSandboxManager, ExecutionIsolationMode, SandboxPolicy, SandboxStatus,
};
use crate::tools::execution_context::{ExecutionContext, GitIdentity, PathIntent};
use crate::tools::policy::PolicySettings;
use crate::tools::session::SessionStore;
use crate::tools::workspace::{
    relative_display, ResolvedPath, Workspace, WorkspaceError, WorkspaceResult,
};
use crate::workspace::AuthConfig;

pub struct ToolContext {
    pub workspace: Workspace,
    pub auth: AuthConfig,
    pub policy: PolicySettings,
    pub tool_profile: String,
    pub permission_mode: String,
    pub harness: Harness,
    execution: Mutex<ExecutionContext>,
    exec_sandbox: ExecSandboxManager,
    pub sessions: Arc<SessionStore>,
}

pub type SharedToolContext = Arc<ToolContext>;

impl ToolContext {
    pub fn new(workspace_path: PathBuf) -> Result<Self, String> {
        let workspace = Workspace::new(workspace_path).map_err(|e| e.message())?;
        let auth = AuthConfig {
            auth_type: "noauth".into(),
            ..AuthConfig::default()
        };
        Ok(Self::from_workspace(
            workspace,
            auth,
            PolicySettings::default(),
            "full".into(),
            "trusted".into(),
        ))
    }

    pub fn from_workspace(
        workspace: Workspace,
        auth: AuthConfig,
        policy: PolicySettings,
        tool_profile: String,
        permission_mode: String,
    ) -> Self {
        Self::try_from_workspace(workspace, auth, policy, tool_profile, permission_mode)
            .expect("无法初始化工作区工具上下文")
    }

    pub fn try_from_workspace(
        workspace: Workspace,
        auth: AuthConfig,
        policy: PolicySettings,
        tool_profile: String,
        permission_mode: String,
    ) -> WorkspaceResult<Self> {
        let harness_root = Harness::default_root().map_err(workspace_harness_error)?;
        Self::try_from_workspace_with_harness_root_and_identity(
            workspace,
            auth,
            policy,
            tool_profile,
            permission_mode,
            harness_root,
            None,
        )
    }

    pub fn from_workspace_with_harness_root(
        workspace: Workspace,
        auth: AuthConfig,
        policy: PolicySettings,
        tool_profile: String,
        permission_mode: String,
        harness_root: PathBuf,
    ) -> Self {
        Self::try_from_workspace_with_harness_root_and_identity(
            workspace,
            auth,
            policy,
            tool_profile,
            permission_mode,
            harness_root,
            None,
        )
        .expect("无法初始化工作区工具上下文")
    }

    pub fn from_workspace_with_identity(
        workspace: Workspace,
        auth: AuthConfig,
        policy: PolicySettings,
        tool_profile: String,
        permission_mode: String,
        identity: GitIdentity,
    ) -> Self {
        Self::try_from_workspace_with_identity(
            workspace,
            auth,
            policy,
            tool_profile,
            permission_mode,
            identity,
        )
        .expect("无法初始化工作区工具上下文")
    }

    pub fn try_from_workspace_with_identity(
        workspace: Workspace,
        auth: AuthConfig,
        policy: PolicySettings,
        tool_profile: String,
        permission_mode: String,
        identity: GitIdentity,
    ) -> WorkspaceResult<Self> {
        let harness_root = Harness::default_root().map_err(workspace_harness_error)?;
        Self::try_from_workspace_with_harness_root_and_identity(
            workspace,
            auth,
            policy,
            tool_profile,
            permission_mode,
            harness_root,
            Some(identity),
        )
    }

    pub fn from_workspace_with_harness_root_and_identity(
        workspace: Workspace,
        auth: AuthConfig,
        policy: PolicySettings,
        tool_profile: String,
        permission_mode: String,
        harness_root: PathBuf,
        identity: Option<GitIdentity>,
    ) -> Self {
        Self::try_from_workspace_with_harness_root_and_identity(
            workspace,
            auth,
            policy,
            tool_profile,
            permission_mode,
            harness_root,
            identity,
        )
        .expect("无法初始化工作区工具上下文")
    }

    pub fn try_from_workspace_with_harness_root_and_identity(
        workspace: Workspace,
        auth: AuthConfig,
        policy: PolicySettings,
        tool_profile: String,
        permission_mode: String,
        harness_root: PathBuf,
        identity: Option<GitIdentity>,
    ) -> WorkspaceResult<Self> {
        let execution = identity
            .map(|identity| {
                ExecutionContext::with_git_identity(
                    workspace.repository_root().to_path_buf(),
                    workspace.active_root().to_path_buf(),
                    identity,
                )
            })
            .unwrap_or_else(|| {
                ExecutionContext::for_workspace(
                    workspace.repository_root().to_path_buf(),
                    workspace.active_root().to_path_buf(),
                )
            })?;
        let root = execution.execution_root().to_path_buf();
        let exec_sandbox = ExecSandboxManager::for_execution_context(&execution);
        let harness = Harness::new(root, harness_root).map_err(workspace_harness_error)?;
        Ok(Self {
            workspace,
            auth,
            policy,
            tool_profile: crate::tools::registry::normalize_tool_profile(&tool_profile).into(),
            permission_mode,
            harness,
            execution: Mutex::new(execution),
            exec_sandbox,
            sessions: Arc::new(SessionStore::new()),
        })
    }

    pub fn for_test(workspace_path: PathBuf, harness_root: PathBuf) -> Result<Self, String> {
        let workspace = Workspace::new(workspace_path).map_err(|e| e.message())?;
        Ok(Self::from_workspace_with_harness_root(
            workspace,
            AuthConfig {
                auth_type: "noauth".into(),
                ..AuthConfig::default()
            },
            PolicySettings::default(),
            "full".into(),
            "trusted".into(),
            harness_root,
        ))
    }

    pub fn workspace_path(&self) -> String {
        self.workspace.root_display()
    }

    pub fn repository_root_path(&self) -> PathBuf {
        self.execution
            .lock()
            .expect("execution context lock")
            .repository_root()
            .to_path_buf()
    }

    pub fn repository_root_display(&self) -> String {
        self.repository_root_path().display().to_string()
    }

    pub fn execution_root(&self) -> PathBuf {
        self.execution
            .lock()
            .expect("execution context lock")
            .execution_root()
            .to_path_buf()
    }

    pub fn execution_root_display(&self) -> String {
        self.execution_root().display().to_string()
    }

    pub fn git_identity(&self) -> Option<GitIdentity> {
        self.execution
            .lock()
            .expect("execution context lock")
            .git_identity()
    }

    pub fn execution_fingerprint(&self) -> String {
        self.execution
            .lock()
            .expect("execution context lock")
            .fingerprint()
    }

    pub fn validate_execution_context(&self) -> WorkspaceResult<()> {
        self.execution
            .lock()
            .expect("execution context lock")
            .validate()
    }

    pub fn execution_isolation_mode(&self) -> ExecutionIsolationMode {
        self.exec_sandbox.mode()
    }

    pub fn requires_strict_exec_isolation(&self) -> bool {
        self.execution_isolation_mode() == ExecutionIsolationMode::Strict
    }

    pub fn exec_sandbox_status(&self) -> SandboxStatus {
        self.exec_sandbox.status(&self.execution_root())
    }

    pub fn require_exec_sandbox(&self) -> WorkspaceResult<()> {
        self.exec_sandbox.require_enforced(&self.execution_root())
    }

    pub fn exec_sandbox_unavailable(&self) -> WorkspaceError {
        self.exec_sandbox.unavailable_error(&self.execution_root())
    }

    pub fn sandbox_policy(
        &self,
        working_directory: PathBuf,
        readonly_roots: Vec<PathBuf>,
    ) -> SandboxPolicy {
        let execution_root = self.execution_root();
        SandboxPolicy {
            repository_root: self.repository_root_path(),
            execution_root: execution_root.clone(),
            startup_directory: working_directory.clone(),
            working_directory,
            writable_roots: vec![execution_root.clone()],
            readonly_roots,
            temp_root: self.exec_sandbox.temp_root(&execution_root),
            network_allowed: self.policy.network_allowed(),
            allow_child_processes: true,
        }
    }

    #[cfg(windows)]
    pub(crate) fn spawn_sandbox(
        &self,
        policy: &SandboxPolicy,
        program: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> WorkspaceResult<SandboxChild> {
        self.exec_sandbox.spawn(policy, program, args, env)
    }

    pub fn default_cwd_display(&self) -> String {
        self.execution
            .lock()
            .expect("execution context lock")
            .default_cwd_display()
    }

    pub fn set_default_cwd(&self, path: PathBuf) -> WorkspaceResult<()> {
        self.execution
            .lock()
            .expect("execution context lock")
            .set_default_cwd(path)
    }

    pub fn default_cwd_path(&self) -> PathBuf {
        self.execution
            .lock()
            .expect("execution context lock")
            .default_cwd()
            .to_path_buf()
    }

    pub fn resolve_existing_from_default_cwd(
        &self,
        raw_path: &str,
    ) -> WorkspaceResult<ResolvedPath> {
        self.resolve_path_from_default_cwd(raw_path, PathIntent::Read, false)
    }

    pub fn resolve_read_from_default_cwd(&self, raw_path: &str) -> WorkspaceResult<ResolvedPath> {
        self.resolve_path_from_default_cwd(raw_path, PathIntent::Read, false)
    }

    pub fn resolve_for_write_from_default_cwd(
        &self,
        raw_path: &str,
    ) -> WorkspaceResult<ResolvedPath> {
        self.resolve_path_from_default_cwd(raw_path, PathIntent::Write, true)
    }

    pub fn resolve_command_cwd(&self, raw_path: &str) -> WorkspaceResult<ResolvedPath> {
        self.resolve_path_from_default_cwd(raw_path, PathIntent::CommandCwd, false)
    }

    /// Resolve a path relative to an already validated command working directory.
    /// This keeps native `ls`/`dir` diagnostics consistent with the child process
    /// cwd instead of accidentally resolving their argument from the workspace root.
    pub fn resolve_command_path(
        &self,
        base: &Path,
        raw_path: &str,
    ) -> WorkspaceResult<ResolvedPath> {
        self.resolve_path_from_base(base, raw_path, PathIntent::CommandCwd, false)
    }

    /// Resolve a read path relative to an explicit base directory. Absolute paths
    /// retain the existing read-only escape hatch, while relative paths are still
    /// checked against the active execution context and Git worktree identity.
    pub fn resolve_read_from(&self, base: &Path, raw_path: &str) -> WorkspaceResult<ResolvedPath> {
        self.resolve_path_from_base(base, raw_path, PathIntent::Read, false)
    }

    /// Resolve a path relative to the execution root. This is used by
    /// `set_default_cwd`, whose argument has always been workspace-root relative.
    pub fn resolve_from_execution_root(
        &self,
        raw_path: &str,
        intent: PathIntent,
    ) -> WorkspaceResult<ResolvedPath> {
        let base = self.execution_root();
        self.resolve_path_from_base(&base, raw_path, intent, false)
    }

    /// Resolve a Git pathspec while keeping the actual Git cwd fixed at the
    /// execution root. Globs may refer to paths that do not exist yet, but they
    /// may never contain an absolute path, parent traversal, or Git magic prefix.
    pub fn resolve_git_pathspec(&self, raw_path: &str) -> WorkspaceResult<String> {
        let raw = if raw_path.is_empty() { "." } else { raw_path };
        if raw.starts_with(':') {
            return Err(crate::tools::workspace::WorkspaceError::invalid_argument(
                "Git pathspec magic is not supported",
            ));
        }
        let resolved = self.resolve_path_from_default_cwd(raw, PathIntent::GitTarget, true)?;
        Ok(if resolved.display.is_empty() {
            ".".into()
        } else {
            resolved.display
        })
    }

    pub fn validate_path(&self, path: &Path, intent: PathIntent) -> WorkspaceResult<PathBuf> {
        self.execution
            .lock()
            .expect("execution context lock")
            .validate_resolved_path(path, intent)
    }

    pub fn validate_read_path(&self, path: &Path) -> WorkspaceResult<PathBuf> {
        self.validate_path(path, PathIntent::Read)
    }

    pub fn display_path(&self, path: &Path) -> String {
        relative_display(&self.execution_root(), path)
    }

    pub fn resolve_history_dir(
        &self,
        workspace_root: Option<&str>,
        history_dir: Option<&str>,
    ) -> WorkspaceResult<PathBuf> {
        let candidate = crate::tools::history::storage::resolve_history_dir(
            &self.workspace,
            workspace_root,
            history_dir,
        )?;
        let resolved = self.validate_path(&candidate, PathIntent::History)?;
        if resolved.exists() && !resolved.is_dir() {
            return Err(crate::tools::workspace::WorkspaceError::not_a_directory(
                "history_dir must be a directory",
            ));
        }
        Ok(resolved)
    }

    fn resolve_path_from_default_cwd(
        &self,
        raw_path: &str,
        intent: PathIntent,
        allow_missing: bool,
    ) -> WorkspaceResult<ResolvedPath> {
        let base = self.default_cwd_path();
        self.resolve_path_from_base(&base, raw_path, intent, allow_missing)
    }

    fn resolve_path_from_base(
        &self,
        base: &Path,
        raw_path: &str,
        intent: PathIntent,
        allow_missing: bool,
    ) -> WorkspaceResult<ResolvedPath> {
        let raw = if raw_path.is_empty() { "." } else { raw_path };
        if raw.contains('\0') {
            return Err(crate::tools::workspace::WorkspaceError::invalid_argument(
                "Path contains a NUL byte",
            ));
        }
        if !matches!(intent, PathIntent::Read) {
            self.workspace.reject_unsafe_text(raw)?;
        }
        let input = Path::new(raw);
        let candidate = if input.is_absolute() {
            input.to_path_buf()
        } else {
            base.join(raw.replace('/', std::path::MAIN_SEPARATOR_STR))
        };
        let existed = candidate.exists() || candidate.is_symlink();
        if !allow_missing && !existed {
            return Err(crate::tools::workspace::WorkspaceError::not_found(format!(
                "Path not found: {raw}"
            )));
        }
        if matches!(intent, PathIntent::Write) && candidate.is_symlink() {
            return Err(crate::tools::workspace::WorkspaceError::symlink_escape());
        }
        let validated = self.validate_path(&candidate, intent)?;
        if matches!(intent, PathIntent::CommandCwd) && !validated.is_dir() {
            return Err(crate::tools::workspace::WorkspaceError::not_a_directory(
                "Command cwd must be a directory",
            ));
        }
        Ok(ResolvedPath {
            display: self.display_path(&validated),
            path: validated,
            existed,
        })
    }
}

fn workspace_harness_error(error: HarnessError) -> WorkspaceError {
    WorkspaceError::Tool {
        code: "WORKSPACE_UNAVAILABLE",
        message: format!("Unable to initialize workspace harness: {error}"),
        category: "workspace",
        retryable: true,
    }
}

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::tools::execution_context::ExecutionContext;
use crate::tools::workspace::{WorkspaceError, WorkspaceResult};

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub(crate) use windows::{SandboxChild, SandboxProcess};

/// Controls whether a generic child process may use the legacy direct runner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionIsolationMode {
    /// Compatibility mode for non-Git temporary directories.
    LegacyPolicyOnly,
    /// A Git workspace/worktree requires an enforced OS boundary.
    Strict,
}

impl ExecutionIsolationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LegacyPolicyOnly => "legacy_policy_only",
            Self::Strict => "strict",
        }
    }
}

/// Filesystem and process capabilities granted to one child process tree.
#[derive(Clone, Debug)]
pub struct SandboxPolicy {
    pub repository_root: PathBuf,
    pub execution_root: PathBuf,
    /// Directory passed to CreateProcessW as the initial current directory.
    /// Git for Windows resolves this directory through a handle during its
    /// startup; keeping that lookup inside the private sandbox temp root
    /// avoids requiring traverse ACLs on arbitrary user parent directories.
    pub startup_directory: PathBuf,
    pub working_directory: PathBuf,
    pub writable_roots: Vec<PathBuf>,
    pub readonly_roots: Vec<PathBuf>,
    pub temp_root: PathBuf,
    pub network_allowed: bool,
    pub allow_child_processes: bool,
}

#[derive(Clone, Debug)]
pub struct SandboxStatus {
    pub available: bool,
    pub enforced: bool,
    pub implementation: String,
    pub execution_root: PathBuf,
    pub fallback_allowed: bool,
    pub mode: ExecutionIsolationMode,
}

impl SandboxStatus {
    pub fn boundary(&self) -> &'static str {
        if self.enforced {
            "windows_appcontainer"
        } else if self.mode == ExecutionIsolationMode::Strict {
            "unavailable"
        } else {
            "policy_only"
        }
    }
}

/// Platform sandbox facade. Strict callers fail closed until the platform
/// backend reports a usable and enforced filesystem boundary.
#[derive(Clone, Debug)]
pub struct ExecSandboxManager {
    mode: ExecutionIsolationMode,
}

impl ExecSandboxManager {
    pub fn for_execution_context(context: &ExecutionContext) -> Self {
        let mode = if context.git_identity().is_some() {
            ExecutionIsolationMode::Strict
        } else {
            ExecutionIsolationMode::LegacyPolicyOnly
        };
        Self { mode }
    }

    pub fn mode(&self) -> ExecutionIsolationMode {
        self.mode
    }

    pub fn status(&self, execution_root: &Path) -> SandboxStatus {
        #[cfg(windows)]
        let available = windows::is_available_for_path(execution_root);
        #[cfg(not(windows))]
        let available = false;
        SandboxStatus {
            available,
            // `windows::spawn` is the only strict execution path.  When the
            // API probe succeeds, every generic child is created with the
            // AppContainer attribute and a Job Object before it is resumed.
            enforced: available,
            implementation: if cfg!(windows) {
                "windows_appcontainer".into()
            } else {
                "unsupported".into()
            },
            execution_root: execution_root.to_path_buf(),
            fallback_allowed: self.mode == ExecutionIsolationMode::LegacyPolicyOnly,
            mode: self.mode,
        }
    }

    /// Return the private per-context temporary directory used by sandboxed
    /// toolchains. The path contains only a stable hash, never the workspace
    /// path itself.
    pub fn temp_root(&self, execution_root: &Path) -> PathBuf {
        use sha2::{Digest, Sha256};
        let mut digest = Sha256::new();
        digest.update(execution_root.to_string_lossy().as_bytes());
        let hash = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        // Keep the parent directory traversable by AppContainer.  Windows'
        // system temp directory already grants AppContainer traversal, while
        // the per-context hash below still makes the actual grant private.
        // Falling back to the user temp directory keeps non-standard Windows
        // installations functional; its ancestors are handled by the
        // minimal traversal grants in the sandbox backend.
        let base = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .map(|root| root.join("Temp"))
            .filter(|path| path.is_dir())
            .unwrap_or_else(std::env::temp_dir);
        base.join(format!("CodingToolsMCP-sandbox-{hash}"))
    }

    pub fn unavailable_error(&self, execution_root: &Path) -> WorkspaceError {
        let status = self.status(execution_root);
        WorkspaceError::ToolDetails {
            code: "EXEC_SANDBOX_UNAVAILABLE",
            message: "The command was not started because strict workspace isolation requires an OS filesystem sandbox.".into(),
            category: "security",
            retryable: false,
            details: json!({
                "execution_root": execution_root.display().to_string(),
                "sandbox_required": true,
                "sandbox_available": status.available,
                "sandbox_enforced": status.enforced,
                "fallback_allowed": false,
                "implementation": status.implementation,
                "isolation_mode": self.mode.as_str()
            }),
        }
    }

    pub fn require_enforced(&self, execution_root: &Path) -> WorkspaceResult<()> {
        let status = self.status(execution_root);
        if self.mode == ExecutionIsolationMode::Strict && (!status.available || !status.enforced) {
            return Err(self.unavailable_error(execution_root));
        }
        Ok(())
    }

    #[cfg(windows)]
    pub fn spawn(
        &self,
        policy: &SandboxPolicy,
        program: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> WorkspaceResult<SandboxChild> {
        if self.mode != ExecutionIsolationMode::Strict {
            return Err(WorkspaceError::Tool {
                code: "EXEC_SANDBOX_UNAVAILABLE",
                message: "Sandboxed execution is only required for strict contexts.".into(),
                category: "security",
                retryable: false,
            });
        }
        windows::spawn(policy, program, args, env).map_err(|error| WorkspaceError::ToolDetails {
            code: "EXEC_SANDBOX_INIT_FAILED",
            message: format!("Failed to create the Windows AppContainer sandbox: {error}"),
            category: "security",
            retryable: true,
            details: json!({
                "execution_root": policy.execution_root.display().to_string(),
                "sandbox_required": true,
                "sandbox_available": true,
                "sandbox_enforced": false,
                "fallback_allowed": false,
                "implementation": "windows_appcontainer"
            }),
        })
    }
}

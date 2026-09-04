use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::tools::execution_context::{
    canonical_directory as execution_canonical_directory, is_ancestor as execution_is_ancestor,
    path_is_within, path_key, probe_git_identity, GitIdentity,
};
use crate::tools::workspace::tool_err_code;

const GIT_TIMEOUT: Duration = crate::tools::execution_context::GIT_TIMEOUT;
const DEFAULT_TTL_MINUTES: u64 = 120;
const MIN_TTL_MINUTES: u64 = 5;
const MAX_TTL_MINUTES: u64 = 480;

#[derive(Clone, Debug)]
pub struct PinOptions {
    pub path: PathBuf,
    pub expected_branch: Option<String>,
    pub ttl: Duration,
    pub allow_protected_branch: bool,
    pub confirm: bool,
}

impl PinOptions {
    pub fn from_value(args: &Value) -> Result<Self, ContextError> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ContextError::new(
                    "WORKSPACE_CONTEXT_PATH_INVALID",
                    "path must identify an existing Git worktree directory.",
                )
            })?;
        let ttl_minutes = args
            .get("expires_in_minutes")
            .map(|value| {
                value.as_u64().ok_or_else(|| {
                    ContextError::new(
                        "WORKSPACE_CONTEXT_PATH_INVALID",
                        "expires_in_minutes must be an integer between 5 and 480.",
                    )
                })
            })
            .transpose()?
            .unwrap_or(DEFAULT_TTL_MINUTES);
        if !(MIN_TTL_MINUTES..=MAX_TTL_MINUTES).contains(&ttl_minutes) {
            return Err(ContextError::new(
                "WORKSPACE_CONTEXT_PATH_INVALID",
                "expires_in_minutes must be between 5 and 480.",
            ));
        }
        Ok(Self {
            path: PathBuf::from(path),
            expected_branch: args
                .get("expected_branch")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            ttl: Duration::from_secs(ttl_minutes * 60),
            allow_protected_branch: args
                .get("allow_protected_branch")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            confirm: args
                .get("confirm")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceContextPin {
    pub configured_root: PathBuf,
    pub active_root: PathBuf,
    pub git_dir: PathBuf,
    pub git_common_dir: PathBuf,
    pub branch: String,
    pub initial_head: String,
    pub last_head: String,
    pub source: String,
    pinned_at: Instant,
    expires_at: Instant,
    pinned_at_unix: u64,
    expires_at_unix: u64,
}

impl WorkspaceContextPin {
    pub fn create(
        configured_root: &Path,
        options: &PinOptions,
        source: &str,
    ) -> Result<Self, ContextError> {
        let configured_root = canonical_directory(configured_root)?;
        let candidate = if options.path.is_absolute() {
            options.path.clone()
        } else {
            configured_root.join(&options.path)
        };
        let active_root = canonical_directory(&candidate)?;
        if !path_is_within(&configured_root, &active_root) {
            return Err(ContextError::new(
                "WORKSPACE_CONTEXT_PATH_INVALID",
                "The active worktree must be inside the configured workspace.",
            ));
        }

        let deadline = Instant::now() + GIT_TIMEOUT;
        let identity = probe_identity(&active_root, deadline)?;
        if path_key(&identity.root) != path_key(&active_root) {
            return Err(ContextError::new(
                "WORKTREE_ROOT_REQUIRED",
                "path must equal the Git worktree top-level directory.",
            ));
        }
        if let Some(expected) = &options.expected_branch {
            if expected != &identity.branch {
                return Err(ContextError::new(
                    "WORKSPACE_CONTEXT_MISMATCH",
                    format!(
                        "Expected branch {expected}, but the worktree is on {}.",
                        identity.branch
                    ),
                ));
            }
        }
        if is_protected_branch(&identity.branch)
            && !(options.allow_protected_branch && options.confirm)
        {
            return Err(ContextError::new(
                "PROTECTED_BRANCH_REQUIRES_CONFIRMATION",
                "Pinning main or master requires allow_protected_branch=true and confirm=true.",
            ));
        }

        let now = Instant::now();
        let unix_now = unix_seconds();
        Ok(Self {
            configured_root,
            active_root,
            git_dir: identity.git_dir,
            git_common_dir: identity.git_common_dir,
            branch: identity.branch,
            initial_head: identity.head.clone(),
            last_head: identity.head,
            source: source.to_string(),
            pinned_at: now,
            expires_at: now + options.ttl,
            pinned_at_unix: unix_now,
            expires_at_unix: unix_now + options.ttl.as_secs(),
        })
    }

    pub fn validate(&mut self) -> Result<GitIdentity, ContextError> {
        if Instant::now() >= self.expires_at {
            return Err(ContextError::new(
                "WORKSPACE_CONTEXT_EXPIRED",
                "The workspace context lock has expired. Unpin it before continuing.",
            ));
        }
        let active_root = canonical_directory(&self.active_root).map_err(|_| {
            ContextError::new(
                "WORKSPACE_CONTEXT_MISMATCH",
                "The pinned worktree directory is unavailable.",
            )
        })?;
        let deadline = Instant::now() + GIT_TIMEOUT;
        let identity = probe_identity(&active_root, deadline)?;
        if path_key(&identity.root) != path_key(&self.active_root)
            || path_key(&identity.git_dir) != path_key(&self.git_dir)
            || path_key(&identity.git_common_dir) != path_key(&self.git_common_dir)
            || identity.branch != self.branch
        {
            return Err(ContextError::new(
                "WORKSPACE_CONTEXT_MISMATCH",
                "The pinned path, Git worktree identity, or branch has changed.",
            ));
        }
        if identity.head != self.last_head {
            if !is_ancestor(&self.active_root, &self.last_head, &identity.head, deadline)? {
                return Err(ContextError::new(
                    "WORKSPACE_CONTEXT_MISMATCH",
                    "HEAD moved non-fast-forward from the last verified commit.",
                ));
            }
            self.last_head = identity.head.clone();
        }
        Ok(identity)
    }

    pub fn git_identity(&self) -> GitIdentity {
        GitIdentity {
            root: self.active_root.clone(),
            git_dir: self.git_dir.clone(),
            git_common_dir: self.git_common_dir.clone(),
            branch: self.branch.clone(),
            head: self.last_head.clone(),
        }
    }

    pub fn snapshot(&self, status: &str, message: Option<&str>) -> Value {
        json!({
            "locked": true,
            "status": status,
            "configured_root": self.configured_root.to_string_lossy(),
            "active_root": self.active_root.to_string_lossy(),
            "git_dir": self.git_dir.to_string_lossy(),
            "git_common_dir": self.git_common_dir.to_string_lossy(),
            "branch": self.branch,
            "initial_head": self.initial_head,
            "current_head": self.last_head,
            "session_key_source": self.source,
            "pinned_at_unix": self.pinned_at_unix,
            "expires_at_unix": self.expires_at_unix,
            "remaining_seconds": self.expires_at.saturating_duration_since(Instant::now()).as_secs(),
            "age_seconds": Instant::now().saturating_duration_since(self.pinned_at).as_secs(),
            "message": message
        })
    }

    #[cfg(test)]
    pub(crate) fn expire_for_test(&mut self) {
        self.expires_at = Instant::now();
    }
}

#[derive(Clone, Debug)]
pub struct ContextError {
    pub code: &'static str,
    pub message: String,
}

impl ContextError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn tool_value(&self, pin: Option<&WorkspaceContextPin>) -> Value {
        let mut value = tool_err_code(self.code, self.message.clone(), "workspace_context");
        if let Some(pin) = pin {
            value["error"]["details"] = pin.snapshot("invalid", Some(&self.message));
        }
        value
    }
}

fn probe_identity(path: &Path, deadline: Instant) -> Result<GitIdentity, ContextError> {
    probe_git_identity(path, deadline).map_err(|error| ContextError::new(error.code, error.message))
}

fn is_ancestor(
    root: &Path,
    ancestor: &str,
    descendant: &str,
    deadline: Instant,
) -> Result<bool, ContextError> {
    execution_is_ancestor(root, ancestor, descendant, deadline)
        .map_err(|error| ContextError::new(error.code, error.message))
}

fn canonical_directory(path: &Path) -> Result<PathBuf, ContextError> {
    execution_canonical_directory(path)
        .map_err(|error| ContextError::new(error.code, error.message))
}

fn is_protected_branch(branch: &str) -> bool {
    branch.eq_ignore_ascii_case("main") || branch.eq_ignore_ascii_case("master")
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod path_tests {
    use super::path_key;
    use std::path::Path;

    #[test]
    fn path_keys_follow_platform_case_semantics() {
        let key = path_key(Path::new("CaseSensitive/Worktree"));
        let expected = if cfg!(windows) {
            "casesensitive/worktree"
        } else {
            "CaseSensitive/Worktree"
        };
        assert_eq!(key, expected);
    }
}

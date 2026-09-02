use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::tools::workspace::tool_err_code;

const GIT_TIMEOUT: Duration = Duration::from_secs(5);
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
pub struct GitIdentity {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub git_common_dir: PathBuf,
    pub branch: String,
    pub head: String,
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
    let output = run_git(
        path,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--show-toplevel",
            "--absolute-git-dir",
            "--git-common-dir",
            "--verify",
            "HEAD",
        ],
        deadline,
    )?;
    require_success(&output, "The path is not a usable Git worktree.")?;
    let lines = output.stdout.lines().map(str::trim).collect::<Vec<_>>();
    if lines.len() != 4 {
        return Err(ContextError::new(
            "WORKTREE_ROOT_REQUIRED",
            "Git returned an incomplete worktree identity.",
        ));
    }
    let branch = run_git(
        path,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        deadline,
    )?;
    if !branch.status.success() {
        return Err(ContextError::new(
            "WORKSPACE_CONTEXT_MISMATCH",
            "Detached HEAD worktrees cannot be pinned.",
        ));
    }
    Ok(GitIdentity {
        root: canonical_directory(Path::new(lines[0]))?,
        git_dir: canonical_path(Path::new(lines[1]))?,
        git_common_dir: canonical_path(Path::new(lines[2]))?,
        branch: branch.stdout.trim().to_string(),
        head: lines[3].to_string(),
    })
}

fn is_ancestor(
    root: &Path,
    ancestor: &str,
    descendant: &str,
    deadline: Instant,
) -> Result<bool, ContextError> {
    let output = run_git(
        root,
        &["merge-base", "--is-ancestor", ancestor, descendant],
        deadline,
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(ContextError::new(
            "WORKSPACE_CONTEXT_MISMATCH",
            format!(
                "Git could not compare HEAD ancestry: {}",
                output.stderr.trim()
            ),
        )),
    }
}

struct GitOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn run_git(path: &Path, args: &[&str], deadline: Instant) -> Result<GitOutput, ContextError> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            ContextError::new(
                "WORKSPACE_CONTEXT_MISMATCH",
                format!("Failed to start Git: {error}"),
            )
        })?;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| {
            ContextError::new(
                "WORKSPACE_CONTEXT_MISMATCH",
                format!("Failed to inspect Git: {error}"),
            )
        })? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ContextError::new(
                "WORKSPACE_CONTEXT_MISMATCH",
                "Git worktree validation exceeded 5 seconds.",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut stream) = child.stdout.take() {
        let _ = stream.read_to_string(&mut stdout);
    }
    if let Some(mut stream) = child.stderr.take() {
        let _ = stream.read_to_string(&mut stderr);
    }
    Ok(GitOutput {
        status,
        stdout,
        stderr,
    })
}

fn require_success(output: &GitOutput, message: &str) -> Result<(), ContextError> {
    if output.status.success() {
        Ok(())
    } else {
        Err(ContextError::new(
            "WORKTREE_ROOT_REQUIRED",
            format!("{message} {}", output.stderr.trim()),
        ))
    }
}

fn canonical_directory(path: &Path) -> Result<PathBuf, ContextError> {
    let canonical = canonical_path(path)?;
    if !canonical.is_dir() {
        return Err(ContextError::new(
            "WORKSPACE_CONTEXT_PATH_INVALID",
            "Workspace context path must be a directory.",
        ));
    }
    Ok(canonical)
}

fn canonical_path(path: &Path) -> Result<PathBuf, ContextError> {
    path.canonicalize().map_err(|_| {
        ContextError::new(
            "WORKSPACE_CONTEXT_PATH_INVALID",
            format!("Path does not exist: {}", path.display()),
        )
    })
}

fn path_key(path: &Path) -> String {
    let key = path
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string();
    if cfg!(windows) {
        key.to_ascii_lowercase()
    } else {
        key
    }
}

fn path_is_within(root: &Path, path: &Path) -> bool {
    let root = path_key(root);
    let path = path_key(path);
    path == root || path.starts_with(&format!("{root}/"))
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

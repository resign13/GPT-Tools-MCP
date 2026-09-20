use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::tools::workspace::{WorkspaceError, WorkspaceResult};

pub(crate) const GIT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathIntent {
    Read,
    Write,
    CommandCwd,
    GitTarget,
    History,
    Harness,
}

impl PathIntent {
    fn allows_external(self) -> bool {
        matches!(self, Self::Read)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitIdentity {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub git_common_dir: PathBuf,
    pub branch: String,
    pub head: String,
}

#[derive(Clone, Debug)]
pub struct ExecutionContext {
    host_access: bool,
    host_directory_ids: Option<Vec<(PathBuf, (u64, u64))>>,
    repository_root: PathBuf,
    execution_root: PathBuf,
    git_dir: Option<PathBuf>,
    git_common_dir: Option<PathBuf>,
    branch: Option<String>,
    observed_head: Option<String>,
    default_cwd: PathBuf,
    git_probe_error: Option<GitIdentityError>,
}

impl ExecutionContext {
    /// Create a context for a regular workspace. Git identity is captured when the
    /// active root is a usable worktree, while ordinary non-Git directories remain
    /// compatible with the existing tool behavior.
    pub fn for_workspace(
        repository_root: PathBuf,
        execution_root: PathBuf,
    ) -> WorkspaceResult<Self> {
        let (repository_root, execution_root) = canonical_roots(&repository_root, &execution_root)?;
        let (identity, git_probe_error) =
            match probe_git_identity(&execution_root, Instant::now() + GIT_TIMEOUT) {
                Ok(identity) => (Some(identity), None),
                // A directory containing only an arbitrary `.git` folder (or an
                // empty repository without HEAD) remains compatible with the
                // non-Git workspace mode; policy checks still protect `.git` paths.
                Err(error) if error.code == "WORKTREE_ROOT_REQUIRED" => (None, None),
                Err(_error) if !git_metadata_present(&execution_root) => (None, None),
                Err(error) => (None, Some(error)),
            };
        Ok(Self::from_parts(
            repository_root,
            execution_root,
            identity,
            git_probe_error,
        ))
    }

    /// Create a context with an identity already validated by a caller such as the
    /// gateway worktree pin. This avoids a second, unsynchronised root observation.
    pub fn with_git_identity(
        repository_root: PathBuf,
        execution_root: PathBuf,
        identity: GitIdentity,
    ) -> WorkspaceResult<Self> {
        let (repository_root, execution_root) = canonical_roots(&repository_root, &execution_root)?;
        if path_key(&identity.root) != path_key(&execution_root) {
            return Err(context_mismatch(
                "Git worktree identity does not match the execution root.",
                json!({
                    "expected_root": execution_root,
                    "actual_root": identity.root,
                }),
            ));
        }
        if !is_related_worktree(&repository_root, &execution_root) {
            return Err(context_mismatch(
                "The execution root is outside the configured repository.",
                json!({
                    "repository_root": repository_root,
                    "execution_root": execution_root,
                }),
            ));
        }
        Ok(Self::from_parts(
            repository_root,
            execution_root,
            Some(identity),
            None,
        ))
    }

    fn from_parts(
        repository_root: PathBuf,
        execution_root: PathBuf,
        identity: Option<GitIdentity>,
        git_probe_error: Option<GitIdentityError>,
    ) -> Self {
        let (git_dir, git_common_dir, branch, observed_head) = identity
            .map(|identity| {
                (
                    Some(identity.git_dir),
                    Some(identity.git_common_dir),
                    Some(identity.branch),
                    Some(identity.head),
                )
            })
            .unwrap_or((None, None, None, None));
        Self {
            host_access: false,
            host_directory_ids: None,
            repository_root,
            execution_root: execution_root.clone(),
            git_dir,
            git_common_dir,
            branch,
            observed_head,
            default_cwd: execution_root,
            git_probe_error,
        }
    }

    pub fn enable_host_access(&mut self) {
        self.host_access = true;
        if self.git_probe_error.is_some() {
            if let Ok(identity) = probe_git_identity_with_detached(&self.execution_root, Instant::now() + GIT_TIMEOUT, true) {
                self.git_dir = Some(identity.git_dir);
                self.git_common_dir = Some(identity.git_common_dir);
                self.branch = Some(identity.branch);
                self.observed_head = Some(identity.head);
                self.git_probe_error = None;
            }
        }
        let mut roots = vec![self.execution_root.clone()];
        roots.extend(self.git_dir.iter().cloned());
        roots.extend(self.git_common_dir.iter().cloned());
        self.host_directory_ids = roots.into_iter().map(|path| directory_id(&path).map(|id| (path, id))).collect::<std::io::Result<Vec<_>>>().ok();
    }

    pub fn repository_root(&self) -> &Path {
        &self.repository_root
    }

    pub fn execution_root(&self) -> &Path {
        &self.execution_root
    }

    pub fn default_cwd(&self) -> &Path {
        &self.default_cwd
    }

    pub fn default_cwd_display(&self) -> String {
        crate::tools::workspace::relative_display(&self.execution_root, &self.default_cwd)
    }

    pub fn git_identity(&self) -> Option<GitIdentity> {
        Some(GitIdentity {
            root: self.execution_root.clone(),
            git_dir: self.git_dir.clone()?,
            git_common_dir: self.git_common_dir.clone()?,
            branch: self.branch.clone()?,
            head: self.observed_head.clone()?,
        })
    }

    /// Stable identity for command sessions. HEAD is deliberately excluded so a
    /// fast-forward made by the same worktree does not orphan an active process.
    pub fn fingerprint(&self) -> String {
        let material = format!(
            "{}\n{}\n{}\n{}\n{}",
            path_key(&self.repository_root),
            path_key(&self.execution_root),
            self.git_dir.as_deref().map(path_key).unwrap_or_default(),
            self.git_common_dir
                .as_deref()
                .map(path_key)
                .unwrap_or_default(),
            if self.host_access { "host" } else { self.branch.as_deref().unwrap_or_default() },
        );
        format!("{:x}", Sha256::digest(material.as_bytes()))
    }

    /// Validate a path after it has been resolved by a tool. Existing paths are
    /// canonicalized; new write targets retain their final component while their
    /// nearest existing ancestor is checked for containment and worktree identity.
    pub fn validate_resolved_path(
        &mut self,
        path: &Path,
        intent: PathIntent,
    ) -> WorkspaceResult<PathBuf> {
        self.validate()?;
        let (resolved, anchor, exists) = canonicalize_with_anchor(path)?;
        if self.host_access && matches!(intent, PathIntent::Read | PathIntent::Write | PathIntent::CommandCwd) {
            return Ok(resolved);
        }
        let lexical_inside_execution = path_is_within(&self.execution_root, path);
        let inside_execution = path_is_within(&self.execution_root, &resolved);
        let inside_repository = path_is_within(&self.repository_root, &resolved);

        if !inside_execution {
            if lexical_inside_execution {
                if path_contains_symlink(path, &self.execution_root) {
                    return Err(WorkspaceError::symlink_escape());
                }
                return Err(context_mismatch(
                    "The path resolves outside the execution root.",
                    json!({
                        "execution_root": self.execution_root,
                        "resolved_path": resolved,
                    }),
                ));
            }
            // Explicit read-only paths remain supported, but a sibling
            // worktree belonging to this repository is never an external
            // read target.  Probe the resolved path before applying the
            // external-read escape hatch so linked worktrees cannot be
            // reached by absolute or relative aliases.
            if let Ok(identity) = probe_git_identity_with_detached(&anchor, Instant::now() + GIT_TIMEOUT, self.host_access) {
                let same_repository = self
                    .git_common_dir
                    .as_ref()
                    .is_some_and(|common| path_key(common) == path_key(&identity.git_common_dir));
                if same_repository {
                    return Err(context_mismatch(
                        "The path belongs to another Git worktree.",
                        json!({
                            "execution_root": self.execution_root,
                            "path_worktree": identity.root,
                            "path": resolved,
                        }),
                    ));
                }
            }
            if intent.allows_external() && !inside_repository {
                return Ok(resolved);
            }
            return Err(context_mismatch(
                "The path belongs to another repository or Git worktree.",
                json!({
                    "repository_root": self.repository_root,
                    "execution_root": self.execution_root,
                    "resolved_path": resolved,
                    "intent": format!("{intent:?}"),
                }),
            ));
        }

        let identity = match probe_git_identity_with_detached(&anchor, Instant::now() + GIT_TIMEOUT, self.host_access) {
            Ok(identity) => Some(identity),
            Err(error) if git_metadata_present(&anchor) => {
                return Err(context_mismatch(
                    error.message,
                    json!({"reason": error.code, "path": resolved}),
                ));
            }
            Err(_) => None,
        };
        if let Some(identity) = identity {
            if path_key(&identity.root) != path_key(&self.execution_root) {
                return Err(context_mismatch(
                    "The path belongs to another Git worktree.",
                    json!({
                        "execution_root": self.execution_root,
                        "path_worktree": identity.root,
                        "path": resolved,
                    }),
                ));
            }
            if let Some(expected) = self.git_identity() {
                if path_key(&identity.git_dir) != path_key(&expected.git_dir)
                    || path_key(&identity.git_common_dir) != path_key(&expected.git_common_dir)
                    || identity.branch != expected.branch
                {
                    return Err(context_mismatch(
                        "The path Git identity does not match the execution context.",
                        json!({
                            "expected": identity_summary(&expected),
                            "actual": identity_summary(&identity),
                        }),
                    ));
                }
            }
        }

        let _ = exists;
        Ok(resolved)
    }

    /// Validate the active directory and, when captured, its Git worktree identity.
    /// A descendant HEAD is accepted and becomes the new observation; all other
    /// identity changes fail closed before a tool reaches its implementation.
    pub fn validate(&mut self) -> WorkspaceResult<()> {
        let active_root = self
            .execution_root
            .canonicalize()
            .map_err(|_| context_mismatch("The execution root is unavailable.", json!({})))?;
        if !active_root.is_dir() || path_key(&active_root) != path_key(&self.execution_root) {
            return Err(context_mismatch(
                "The execution root changed or is no longer a directory.",
                json!({
                    "expected_execution_root": self.execution_root,
                    "actual_execution_root": active_root,
                }),
            ));
        }
        if self.host_access {
            let ids = self.host_directory_ids.as_ref().ok_or_else(|| context_mismatch("Directory identity could not be captured.", json!({})))?;
            for (path, expected) in ids {
                if directory_id(path).ok().as_ref() != Some(expected) {
                    return Err(context_mismatch("The task directory or Git metadata was replaced.", json!({"path": path})));
                }
            }
        }
        if !is_related_worktree(&self.repository_root, &active_root) {
            return Err(context_mismatch(
                "The execution root moved outside the configured repository.",
                json!({
                    "repository_root": self.repository_root,
                    "execution_root": active_root,
                }),
            ));
        }

        if let Some(error) = self.git_probe_error.clone() {
            return Err(context_mismatch(
                error.message,
                json!({"reason": error.code}),
            ));
        }

        let current_cwd = self.default_cwd.canonicalize().map_err(|_| {
            context_mismatch(
                "The default cwd is unavailable.",
                json!({
                    "execution_root": self.execution_root,
                    "default_cwd": self.default_cwd,
                }),
            )
        })?;
        if !current_cwd.is_dir() || !path_is_within(&self.execution_root, &current_cwd) {
            return Err(context_mismatch(
                "The default cwd moved outside the execution root.",
                json!({
                    "execution_root": self.execution_root,
                    "default_cwd": current_cwd,
                }),
            ));
        }
        if path_key(&current_cwd) != path_key(&self.execution_root) {
            match probe_git_identity_with_detached(&current_cwd, Instant::now() + GIT_TIMEOUT, self.host_access) {
                Ok(identity) if path_key(&identity.root) != path_key(&self.execution_root) => {
                    return Err(context_mismatch(
                        "The default cwd belongs to another Git worktree.",
                        json!({
                            "execution_root": self.execution_root,
                            "default_cwd_worktree": identity.root,
                        }),
                    ));
                }
                Ok(_) => {}
                Err(error) if git_metadata_present(&current_cwd) => {
                    return Err(context_mismatch(
                        error.message,
                        json!({"reason": error.code, "default_cwd": current_cwd}),
                    ));
                }
                Err(_) => {}
            }
        }

        let Some(expected_git_dir) = self.git_dir.clone() else {
            return Ok(());
        };
        let expected = GitIdentity {
            root: self.execution_root.clone(),
            git_dir: expected_git_dir,
            git_common_dir: self
                .git_common_dir
                .clone()
                .expect("git common dir accompanies git dir"),
            branch: self.branch.clone().expect("branch accompanies git dir"),
            head: self
                .observed_head
                .clone()
                .expect("HEAD accompanies git dir"),
        };
        let deadline = Instant::now() + GIT_TIMEOUT;
        let actual = probe_git_identity_with_detached(&active_root, deadline, self.host_access).map_err(|error| {
            context_mismatch(
                error.message,
                json!({
                    "expected": identity_summary(&expected),
                    "actual": null,
                    "reason": error.code,
                }),
            )
        })?;
        if path_key(&actual.root) != path_key(&expected.root)
            || path_key(&actual.git_dir) != path_key(&expected.git_dir)
            || path_key(&actual.git_common_dir) != path_key(&expected.git_common_dir)
            || (!self.host_access && actual.branch != expected.branch)
        {
            return Err(context_mismatch(
                "The Git worktree identity or branch changed.",
                json!({
                    "expected": identity_summary(&expected),
                    "actual": identity_summary(&actual),
                }),
            ));
        }
        if self.host_access {
            self.branch = Some(actual.branch);
            self.observed_head = Some(actual.head);
            return Ok(());
        }
        if actual.head != expected.head {
            if !is_ancestor(&active_root, &expected.head, &actual.head, deadline).map_err(
                |error| {
                    context_mismatch(
                        error.message,
                        json!({
                            "expected_head": expected.head,
                            "actual_head": actual.head,
                            "reason": error.code,
                        }),
                    )
                },
            )? {
                return Err(context_mismatch(
                    "HEAD moved non-fast-forward from the last verified commit.",
                    json!({
                        "expected_head": expected.head,
                        "actual_head": actual.head,
                    }),
                ));
            }
            self.observed_head = Some(actual.head);
        }
        Ok(())
    }

    pub fn set_default_cwd(&mut self, path: PathBuf) -> WorkspaceResult<()> {
        let resolved = path
            .canonicalize()
            .map_err(|_| WorkspaceError::not_found("Default cwd does not exist"))?;
        if !resolved.is_dir() {
            return Err(WorkspaceError::not_a_directory(
                "Default cwd must be a directory",
            ));
        }
        let validated = self.validate_resolved_path(&resolved, PathIntent::CommandCwd)?;
        if !path_is_within(&self.execution_root, &validated) {
            return Err(context_mismatch(
                "Default cwd must remain inside the execution root.",
                json!({
                    "execution_root": self.execution_root,
                    "requested_cwd": validated,
                }),
            ));
        }
        self.default_cwd = validated;
        Ok(())
    }
}

fn canonicalize_with_anchor(path: &Path) -> WorkspaceResult<(PathBuf, PathBuf, bool)> {
    if path.exists() || path.is_symlink() {
        let resolved = path
            .canonicalize()
            .map_err(|_| WorkspaceError::not_found("Path could not be canonicalized"))?;
        let anchor = if resolved.is_dir() {
            resolved.clone()
        } else {
            resolved
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| WorkspaceError::not_found("Path has no parent"))?
        };
        return Ok((resolved, anchor, true));
    }

    let mut missing = Vec::<OsString>::new();
    let mut cursor = path;
    while !cursor.exists() && !cursor.is_symlink() {
        let name = cursor
            .file_name()
            .ok_or_else(|| WorkspaceError::not_found("Path has no existing ancestor"))?;
        missing.push(name.to_os_string());
        cursor = cursor
            .parent()
            .ok_or_else(|| WorkspaceError::not_found("Path has no existing ancestor"))?;
    }
    let anchor = cursor
        .canonicalize()
        .map_err(|_| WorkspaceError::not_found("Path ancestor could not be canonicalized"))?;
    let mut resolved = anchor.clone();
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    Ok((resolved, anchor, false))
}

pub(crate) fn probe_git_identity(
    path: &Path,
    deadline: Instant,
) -> Result<GitIdentity, GitIdentityError> {
    probe_git_identity_with_detached(path, deadline, false)
}

pub(crate) fn probe_git_identity_with_detached(path: &Path, deadline: Instant, allow_detached: bool) -> Result<GitIdentity, GitIdentityError> {
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
        return Err(GitIdentityError::new(
            "WORKTREE_ROOT_REQUIRED",
            "Git returned an incomplete worktree identity.",
        ));
    }
    let branch = run_git(
        path,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        deadline,
    )?;
    if !branch.status.success() && !(allow_detached && branch.status.code() == Some(1)) {
        return Err(GitIdentityError::new(
            "WORKSPACE_CONTEXT_MISMATCH",
            "Detached HEAD worktrees cannot be locked.",
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

pub(crate) fn is_ancestor(
    root: &Path,
    ancestor: &str,
    descendant: &str,
    deadline: Instant,
) -> Result<bool, GitIdentityError> {
    let output = run_git(
        root,
        &["merge-base", "--is-ancestor", ancestor, descendant],
        deadline,
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(GitIdentityError::new(
            "WORKSPACE_CONTEXT_MISMATCH",
            format!(
                "Git could not compare HEAD ancestry: {}",
                output.stderr.trim()
            ),
        )),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct GitIdentityError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl GitIdentityError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

struct GitOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn run_git(path: &Path, args: &[&str], deadline: Instant) -> Result<GitOutput, GitIdentityError> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|error| {
        GitIdentityError::new(
            "WORKSPACE_CONTEXT_MISMATCH",
            format!("Failed to start Git: {error}"),
        )
    })?;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| {
            GitIdentityError::new(
                "WORKSPACE_CONTEXT_MISMATCH",
                format!("Failed to inspect Git: {error}"),
            )
        })? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(GitIdentityError::new(
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

fn require_success(output: &GitOutput, message: &str) -> Result<(), GitIdentityError> {
    if output.status.success() {
        Ok(())
    } else {
        Err(GitIdentityError::new(
            "WORKTREE_ROOT_REQUIRED",
            message.to_string(),
        ))
    }
}

fn canonical_roots(
    repository_root: &Path,
    execution_root: &Path,
) -> WorkspaceResult<(PathBuf, PathBuf)> {
    let repository_root = repository_root
        .canonicalize()
        .map_err(|_| WorkspaceError::invalid_argument("Workspace root must exist"))?;
    if !repository_root.is_dir() {
        return Err(WorkspaceError::invalid_argument(
            "Workspace root must be a directory",
        ));
    }
    let execution_root = execution_root
        .canonicalize()
        .map_err(|_| WorkspaceError::invalid_argument("Active workspace root must exist"))?;
    if !execution_root.is_dir() {
        return Err(WorkspaceError::invalid_argument(
            "Active workspace root must be a directory",
        ));
    }
    if !is_related_worktree(&repository_root, &execution_root) {
        return Err(WorkspaceError::path_outside_workspace());
    }
    Ok((repository_root, execution_root))
}

fn identity_summary(identity: &GitIdentity) -> serde_json::Value {
    json!({
        "root": identity.root,
        "git_dir": identity.git_dir,
        "git_common_dir": identity.git_common_dir,
        "branch": identity.branch,
        "head": identity.head,
    })
}

fn context_mismatch(message: impl Into<String>, details: serde_json::Value) -> WorkspaceError {
    WorkspaceError::ToolDetails {
        code: "WORKSPACE_CONTEXT_MISMATCH",
        message: message.into(),
        category: "workspace_context",
        retryable: true,
        details,
    }
}

pub(crate) fn canonical_directory(path: &Path) -> Result<PathBuf, GitIdentityError> {
    let canonical = canonical_path(path)?;
    if !canonical.is_dir() {
        return Err(GitIdentityError::new(
            "WORKSPACE_CONTEXT_PATH_INVALID",
            "Workspace context path must be a directory.",
        ));
    }
    Ok(canonical)
}

pub(crate) fn canonical_path(path: &Path) -> Result<PathBuf, GitIdentityError> {
    path.canonicalize().map_err(|_| {
        GitIdentityError::new(
            "WORKSPACE_CONTEXT_PATH_INVALID",
            format!("Path does not exist: {}", path.display()),
        )
    })
}

pub(crate) fn path_key(path: &Path) -> String {
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

pub(crate) fn path_is_within(root: &Path, path: &Path) -> bool {
    let root = path_key(root);
    let path = path_key(path);
    path == root || path.starts_with(&format!("{root}/"))
}

/// Return whether `execution_root` is the configured repository itself, a
/// descendant of it, or a Git linked worktree whose common metadata belongs to
/// the repository.  A separate clone or unrelated directory is rejected.
pub(crate) fn is_related_worktree(repository_root: &Path, execution_root: &Path) -> bool {
    if path_is_within(repository_root, execution_root) {
        return true;
    }

    let git_file = execution_root.join(".git");
    let Ok(metadata) = std::fs::symlink_metadata(&git_file) else {
        return false;
    };
    if !metadata.file_type().is_file() {
        return false;
    }
    let Ok(contents) = std::fs::read_to_string(&git_file) else {
        return false;
    };
    let Some(raw_git_dir) = contents
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("gitdir:"))
        .map(str::trim)
    else {
        return false;
    };
    let git_dir = Path::new(raw_git_dir);
    let resolved_git_dir = if git_dir.is_absolute() {
        git_dir.to_path_buf()
    } else {
        execution_root.join(git_dir)
    };
    let Ok(resolved_git_dir) = resolved_git_dir.canonicalize() else {
        return false;
    };
    let repository_git_dir = repository_root.join(".git");
    let Ok(repository_git_dir) = repository_git_dir.canonicalize() else {
        return false;
    };
    path_is_within(&repository_git_dir, &resolved_git_dir)
}

fn path_contains_symlink(path: &Path, lexical_root: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(lexical_root) else {
        return false;
    };
    let mut cursor = lexical_root.to_path_buf();
    for component in relative.components() {
        cursor.push(component.as_os_str());
        if std::fs::symlink_metadata(&cursor)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

fn git_metadata_present(path: &Path) -> bool {
    let mut current = Some(path);
    while let Some(directory) = current {
        match std::fs::symlink_metadata(directory.join(".git")) {
            Ok(_) => return true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return true,
        }
        current = directory.parent();
    }
    false
}

// Identify directory objects, not just their names: a same-path replacement is
// a new task/repository and must not inherit a running host session.
#[cfg(windows)]
fn directory_id(path: &Path) -> std::io::Result<(u64, u64)> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS}};
    let file = std::fs::OpenOptions::new().access_mode(0).custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0).open(path)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.map_err(std::io::Error::other)?;
    Ok((info.dwVolumeSerialNumber as u64, ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64))
}
#[cfg(not(windows))]
fn directory_id(path: &Path) -> std::io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = path.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}

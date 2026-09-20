use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;

use super::context::ToolContext;
use super::dispatch::call_tool;
use super::execution_context::{ExecutionContext, GitIdentity};
use super::workspace::Workspace;
use crate::tools::policy::PolicySettings;
use crate::workspace::AuthConfig;

fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .status()
        .expect("git command");
    assert!(status.success(), "git command failed: {args:?}");
}

fn git_repository() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().expect("root");
    let repo = root.path().join("repo");
    std::fs::create_dir(&repo).expect("repo dir");
    git(&repo, &["init"]);
    git(&repo, &["config", "user.name", "Execution Context Test"]);
    git(
        &repo,
        &["config", "user.email", "execution-context@example.test"],
    );
    std::fs::write(repo.join("tracked.txt"), "base\n").expect("tracked file");
    git(&repo, &["add", "tracked.txt"]);
    git(&repo, &["commit", "-m", "base"]);
    git(&repo, &["branch", "-M", "main"]);
    (root, repo)
}

fn git_worktree_repository() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().expect("root");
    let main = root.path().join("main");
    std::fs::create_dir(&main).expect("main dir");
    git(&main, &["init"]);
    git(&main, &["config", "user.name", "Execution Context Test"]);
    git(
        &main,
        &["config", "user.email", "execution-context@example.test"],
    );
    std::fs::write(main.join("tracked.txt"), "base\n").expect("tracked file");
    git(&main, &["add", "tracked.txt"]);
    git(&main, &["commit", "-m", "base"]);
    git(&main, &["branch", "-M", "main"]);
    let worktree = root.path().join("feature");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-b",
            "feature/context-lock",
            worktree.to_str().expect("worktree path"),
        ],
    );
    (root, main, worktree)
}

fn worktree_context(
    repository_root: &Path,
    execution_root: &Path,
    harness_root: &Path,
) -> ToolContext {
    let workspace =
        Workspace::new_with_roots(repository_root.to_path_buf(), execution_root.to_path_buf())
            .expect("workspace");
    ToolContext::from_workspace_with_harness_root(
        workspace,
        AuthConfig {
            auth_type: "noauth".into(),
            ..AuthConfig::default()
        },
        PolicySettings::default(),
        "full".into(),
        "trusted".into(),
        harness_root.to_path_buf(),
    )
}

#[test]
fn workspace_exposes_repository_and_active_roots() {
    let root = tempfile::tempdir().expect("root");
    let active = root.path().join("active");
    std::fs::create_dir(&active).expect("active");
    let workspace =
        Workspace::new_with_roots(root.path().to_path_buf(), active.clone()).expect("workspace");
    assert_eq!(
        workspace.repository_root(),
        root.path().canonicalize().unwrap()
    );
    assert_eq!(workspace.active_root(), active.canonicalize().unwrap());
    assert_eq!(workspace.root(), workspace.active_root());
}

#[test]
fn fallible_context_constructor_reports_identity_mismatch_without_panicking() {
    let root = tempfile::tempdir().expect("root");
    let active = root.path().join("active");
    std::fs::create_dir(&active).expect("active");
    let workspace =
        Workspace::new_with_roots(root.path().to_path_buf(), active.clone()).expect("workspace");
    let harness = tempfile::tempdir().expect("harness");
    let result = ToolContext::try_from_workspace_with_harness_root_and_identity(
        workspace,
        AuthConfig {
            auth_type: "noauth".into(),
            ..AuthConfig::default()
        },
        PolicySettings::default(),
        "full".into(),
        "trusted".into(),
        harness.path().to_path_buf(),
        Some(GitIdentity {
            root: root.path().to_path_buf(),
            git_dir: root.path().join(".git"),
            git_common_dir: root.path().join(".git"),
            branch: "feature/context-lock".into(),
            head: "deadbeef".into(),
        }),
    );
    let error = result.err().expect("identity mismatch should be returned");
    assert_eq!(error.to_error_value()["code"], "WORKSPACE_CONTEXT_MISMATCH");
}

#[test]
fn default_cwd_cannot_escape_execution_root() {
    let root = tempfile::tempdir().expect("root");
    let outside = tempfile::tempdir().expect("outside");
    let context = ToolContext::for_test(root.path().to_path_buf(), root.path().join("harness"))
        .expect("context");
    let error = context
        .set_default_cwd(outside.path().to_path_buf())
        .expect_err("outside cwd should be rejected");
    assert_eq!(error.to_error_value()["code"], "WORKSPACE_CONTEXT_MISMATCH");
}

#[test]
fn symlink_escape_is_reported_as_security_error() {
    let root = tempfile::tempdir().expect("root");
    let outside = tempfile::tempdir().expect("outside");
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, "secret\n").expect("secret");
    let link = root.path().join("outside-link.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&secret, &link).expect("symlink");
    #[cfg(windows)]
    if std::os::windows::fs::symlink_file(&secret, &link).is_err() {
        return;
    }

    let context = ToolContext::for_test(root.path().to_path_buf(), root.path().to_path_buf())
        .expect("context");
    let result = call_tool(&context, "read_file", &json!({"path": "outside-link.txt"}));
    assert_eq!(result["ok"], false);
    assert_eq!(result["error"]["code"], "SYMLINK_ESCAPE");
    assert_eq!(result["error"]["category"], "security");
}

#[test]
fn default_cwd_cannot_enter_a_nested_git_repository() {
    let root = tempfile::tempdir().expect("root");
    let nested = root.path().join("nested");
    std::fs::create_dir(&nested).expect("nested");
    git(&nested, &["init"]);
    git(&nested, &["config", "user.name", "Nested Context Test"]);
    git(
        &nested,
        &["config", "user.email", "nested-context@example.test"],
    );
    std::fs::write(nested.join("nested.txt"), "nested\n").expect("nested file");
    git(&nested, &["add", "nested.txt"]);
    git(&nested, &["commit", "-m", "nested"]);
    let context = ToolContext::for_test(root.path().to_path_buf(), root.path().join("harness"))
        .expect("context");
    let error = context
        .set_default_cwd(nested)
        .expect_err("nested repository should not become the cwd");
    assert_eq!(error.to_error_value()["code"], "WORKSPACE_CONTEXT_MISMATCH");
}

#[test]
fn default_cwd_rejects_a_detached_nested_git_repository() {
    let root = tempfile::tempdir().expect("root");
    let nested = root.path().join("nested");
    std::fs::create_dir(&nested).expect("nested");
    git(&nested, &["init"]);
    git(&nested, &["config", "user.name", "Nested Context Test"]);
    git(
        &nested,
        &["config", "user.email", "nested-context@example.test"],
    );
    std::fs::write(nested.join("nested.txt"), "nested\n").expect("nested file");
    git(&nested, &["add", "nested.txt"]);
    git(&nested, &["commit", "-m", "nested"]);
    git(&nested, &["checkout", "--detach"]);

    let context = ToolContext::for_test(root.path().to_path_buf(), root.path().join("harness"))
        .expect("context");
    let error = context
        .set_default_cwd(nested)
        .expect_err("detached nested repository should not become the cwd");
    assert_eq!(error.to_error_value()["code"], "WORKSPACE_CONTEXT_MISMATCH");
}

#[test]
fn detached_git_workspace_fails_closed_during_preflight() {
    let (_root, repo) = git_repository();
    git(&repo, &["checkout", "--detach"]);
    let context = ToolContext::for_test(repo.clone(), repo.join("harness")).expect("context");
    let result = call_tool(&context, "get_default_cwd", &json!({}));
    assert_eq!(result["ok"], false);
    assert_eq!(result["error"]["code"], "WORKSPACE_CONTEXT_MISMATCH");
}

#[test]
fn git_context_rejects_branch_drift() {
    let (_root, repo) = git_repository();
    let mut execution =
        ExecutionContext::for_workspace(repo.clone(), repo.clone()).expect("context");
    git(&repo, &["checkout", "-b", "feature/drift"]);
    let error = execution
        .validate()
        .expect_err("branch drift should fail closed");
    assert_eq!(error.to_error_value()["code"], "WORKSPACE_CONTEXT_MISMATCH");
}

#[test]
fn call_tool_rejects_git_drift_before_writing() {
    let (_root, repo) = git_repository();
    let context = ToolContext::for_test(repo.clone(), repo.join("harness")).expect("context");
    git(&repo, &["checkout", "-b", "feature/drift"]);
    let result = call_tool(
        &context,
        "apply_patch",
        &json!({
            "patch": "*** Begin Patch\n*** Update File: tracked.txt\n@@\n-base\n+changed\n*** End Patch\n"
        }),
    );
    assert_eq!(result["ok"], false);
    assert_eq!(result["error"]["code"], "WORKSPACE_CONTEXT_MISMATCH");
    assert_eq!(
        std::fs::read_to_string(repo.join("tracked.txt")).unwrap(),
        "base\n"
    );
}

#[test]
fn git_context_accepts_fast_forward_head() {
    let (_root, repo) = git_repository();
    let mut execution =
        ExecutionContext::for_workspace(repo.clone(), repo.clone()).expect("context");
    std::fs::write(repo.join("tracked.txt"), "next\n").expect("update");
    git(&repo, &["add", "tracked.txt"]);
    git(&repo, &["commit", "-m", "next"]);
    execution
        .validate()
        .expect("fast-forward should be accepted");
    assert_eq!(execution.git_identity().unwrap().branch, "main");
}

#[test]
fn worktree_context_routes_default_cwd_patch_and_git_to_active_root() {
    let (root, main, worktree) = git_worktree_repository();
    let nested = worktree.join("nested");
    std::fs::create_dir(&nested).expect("nested directory");
    let harness = tempfile::tempdir().expect("harness");
    let context = worktree_context(root.path(), &worktree, harness.path());

    let cwd = call_tool(&context, "set_default_cwd", &json!({"path": "nested"}));
    assert_eq!(cwd["ok"], true, "{cwd}");
    let patched = call_tool(
        &context,
        "apply_patch",
        &json!({
            "patch": "*** Begin Patch\n*** Add File: note.txt\n+active worktree\n*** End Patch\n"
        }),
    );
    assert_eq!(patched["ok"], true, "{patched}");
    assert_eq!(
        std::fs::read_to_string(nested.join("note.txt")).unwrap(),
        "active worktree\n"
    );
    assert!(!main.join("nested/note.txt").exists());

    let status = call_tool(&context, "git_status", &json!({}));
    assert_eq!(status["ok"], true, "{status}");
    assert_eq!(status["branch"], "feature/context-lock");
    assert!(status["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["path"].as_str().unwrap_or("").starts_with("nested")));
    let main_status = Command::new("git")
        .arg("-C")
        .arg(&main)
        .args(["status", "--porcelain"])
        .output()
        .expect("main status");
    assert!(main_status.status.success());
    assert!(main_status.stdout.is_empty());
}

#[test]
fn session_fingerprint_rejects_cross_worktree_context() {
    let (root, main, worktree) = git_worktree_repository();
    let first_harness = tempfile::tempdir().expect("first harness");
    let second_harness = tempfile::tempdir().expect("second harness");
    let first = worktree_context(root.path(), &worktree, first_harness.path());
    let mut second = worktree_context(root.path(), &main, second_harness.path());
    second.sessions = first.sessions.clone();

    // Strict Git contexts reject generic child processes before spawn. Build a
    // session directly here so this test remains focused on fingerprint
    // isolation rather than the exec sandbox capability.
    let child = tokio::process::Command::new("git")
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let child = child.expect("test child");
    let session = first.sessions.insert(
        crate::tools::session::ExecSession::new_with_mode_and_fingerprint(
            child,
            false,
            first.execution_fingerprint(),
        ),
    );
    tauri::async_runtime::block_on(session.spawn_readers());
    let output_ref = format!("session:{}:stdout", session.session_id);
    let rejected = call_tool(&second, "read_output", &json!({"output_ref": output_ref}));
    assert_eq!(rejected["ok"], false, "{rejected}");
    assert_eq!(rejected["error"]["code"], "WORKSPACE_CONTEXT_MISMATCH");

    let session_id = output_ref
        .split(':')
        .nth(1)
        .expect("session id")
        .to_string();
    let killed = call_tool(&first, "kill_session", &json!({"session_id": session_id}));
    assert_eq!(killed["ok"], true, "{killed}");
}

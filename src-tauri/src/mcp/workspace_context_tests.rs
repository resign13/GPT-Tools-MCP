use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use super::workspace_context::{PinOptions, WorkspaceContextPin};

pub(crate) fn git(cwd: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .status()
        .expect("git command");
    assert!(status.success(), "git command failed: {args:?}");
}

pub(crate) fn repository() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().expect("root");
    let main = root.path().join("main");
    std::fs::create_dir(&main).expect("main dir");
    git(&main, &["init"]);
    git(&main, &["config", "user.name", "Workspace Context Test"]);
    git(
        &main,
        &["config", "user.email", "workspace-context@example.test"],
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

fn options(path: PathBuf) -> PinOptions {
    PinOptions {
        path,
        expected_branch: Some("feature/context-lock".into()),
        ttl: Duration::from_secs(300),
        allow_protected_branch: false,
        confirm: false,
    }
}

#[test]
fn pin_validates_worktree_and_allows_fast_forward_head() {
    let (root, _main, worktree) = repository();
    let mut pin = WorkspaceContextPin::create(
        root.path(),
        &options(worktree.clone()),
        "openai_conversation",
    )
    .expect("pin");
    std::fs::write(worktree.join("tracked.txt"), "next\n").expect("update");
    git(&worktree, &["add", "tracked.txt"]);
    git(&worktree, &["commit", "-m", "next"]);
    let identity = pin.validate().expect("fast-forward validation");
    assert_eq!(pin.last_head, identity.head);
    assert_ne!(pin.initial_head, pin.last_head);
    git(&worktree, &["reset", "--hard", "HEAD^"]);
    let error = pin.validate().expect_err("non-fast-forward HEAD");
    assert_eq!(error.code, "WORKSPACE_CONTEXT_MISMATCH");
}

#[test]
fn protected_branch_and_expired_context_fail_closed() {
    let (root, main, worktree) = repository();
    let mut protected_options = options(main);
    protected_options.expected_branch = Some("main".into());
    let protected = WorkspaceContextPin::create(root.path(), &protected_options, "transport")
        .expect_err("main requires confirmation");
    assert_eq!(protected.code, "PROTECTED_BRANCH_REQUIRES_CONFIRMATION");

    let mut pin =
        WorkspaceContextPin::create(root.path(), &options(worktree), "transport").expect("pin");
    pin.expire_for_test();
    let expired = pin.validate().expect_err("expired context");
    assert_eq!(expired.code, "WORKSPACE_CONTEXT_EXPIRED");
}

#[test]
fn branch_change_is_reported_as_context_mismatch() {
    let (root, _main, worktree) = repository();
    let mut pin = WorkspaceContextPin::create(root.path(), &options(worktree.clone()), "transport")
        .expect("pin");
    git(&worktree, &["checkout", "-b", "feature/other"]);
    let error = pin.validate().expect_err("branch drift");
    assert_eq!(error.code, "WORKSPACE_CONTEXT_MISMATCH");
}

#[test]
fn path_must_be_inside_workspace_and_equal_worktree_root() {
    let (root, main, worktree) = repository();
    let outside = WorkspaceContextPin::create(&main, &options(worktree.clone()), "transport")
        .expect_err("outside configured workspace");
    assert_eq!(outside.code, "WORKSPACE_CONTEXT_PATH_INVALID");

    let child = worktree.join("nested");
    std::fs::create_dir(&child).expect("nested directory");
    let not_root = WorkspaceContextPin::create(root.path(), &options(child), "transport")
        .expect_err("worktree root required");
    assert_eq!(not_root.code, "WORKTREE_ROOT_REQUIRED");
}

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use coding_tools_mcp_desktop_lib::tools::exec::exec_command;
use coding_tools_mcp_desktop_lib::tools::policy::PolicySettings;
use coding_tools_mcp_desktop_lib::tools::workspace::Workspace;
use common::{assert_ok, ctx_for, invoke};
use serde_json::json;

struct GitFixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    sibling: PathBuf,
}

fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("start git fixture command");
    assert!(
        output.status.success(),
        "git fixture command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_fixture() -> GitFixture {
    let temp = tempfile::Builder::new()
        .prefix("coding-tools-git-fixture-")
        .tempdir_in(r"D:\it")
        .expect("fixture tempdir");
    let main = temp.path().join("main-worktree");
    let sibling = temp.path().join("sibling-worktree");
    fs::create_dir_all(&main).expect("main root");
    fs::create_dir_all(&sibling).expect("sibling root");
    fs::write(main.join("README.md"), "fixture\n").expect("fixture file");
    run_git(&main, &["init", "-q"]);
    run_git(&main, &["config", "user.email", "test@example.com"]);
    run_git(&main, &["config", "user.name", "Coding Tools Test"]);
    run_git(&main, &["add", "README.md"]);
    run_git(&main, &["commit", "-q", "-m", "fixture"]);
    let sibling_arg = sibling.to_string_lossy().into_owned();
    run_git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "sandbox-sibling",
            &sibling_arg,
            "HEAD",
        ],
    );
    GitFixture {
        _temp: temp,
        root: main,
        sibling,
    }
}

fn assert_sandbox_unavailable(
    result: Result<
        serde_json::Value,
        coding_tools_mcp_desktop_lib::tools::workspace::WorkspaceError,
    >,
) {
    let error = result.expect_err("strict Git execution must fail closed");
    let value = error.to_error_value();
    println!("sandbox error: {value}");
    assert_eq!(value["code"], "EXEC_SANDBOX_UNAVAILABLE");
    assert_eq!(value["category"], "security");
    assert_eq!(value["retryable"], false);
    assert_eq!(value["details"]["sandbox_required"], true);
    assert_eq!(value["details"]["sandbox_available"], false);
    assert_eq!(value["details"]["sandbox_enforced"], false);
    assert_eq!(value["details"]["fallback_allowed"], false);
}

fn sandbox_is_available(ctx: &coding_tools_mcp_desktop_lib::tools::context::ToolContext) -> bool {
    invoke(ctx, "check_exec_environment", json!({}))
        .get("filesystem_sandbox")
        .and_then(|value| value.get("available"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

#[test]
fn strict_git_context_blocks_generic_child_processes_before_spawn() {
    let fixture = git_fixture();
    let ctx = ctx_for(&fixture.root);
    let escape = fixture.sibling.join("escape.txt");
    let escape_path = escape.to_string_lossy();

    let sandbox_available = sandbox_is_available(&ctx);
    for command in [
        format!(
            "git -C '{}' branch --show-current",
            fixture.sibling.display()
        ),
        format!(
            "python -c \"from pathlib import Path; Path(r'{}').write_text('escape')\"",
            escape_path
        ),
        format!(
            "node -e \"require('fs').writeFileSync('{}', 'escape')\"",
            escape_path.replace('\\', "\\\\")
        ),
        format!(
            "powershell -NoProfile -Command \"Set-Content -LiteralPath '{}' -Value escape\"",
            escape_path
        ),
    ] {
        let result = exec_command(&ctx, &json!({"cmd": command}));
        if sandbox_available {
            let output = result.expect("sandboxed command should return a command result");
            assert_eq!(output["sandbox_enforced"], true, "{output}");
            assert_eq!(
                output["execution_boundary"], "windows_appcontainer",
                "{output}"
            );
        } else {
            assert_sandbox_unavailable(result);
        }
    }

    assert!(
        !escape.exists(),
        "a rejected child process must not write files"
    );
}

#[test]
fn strict_git_context_keeps_native_diagnostics_available() {
    let fixture = git_fixture();
    let ctx = ctx_for(&fixture.root);

    let pwd_result = invoke(&ctx, "exec_command", json!({"cmd": "pwd"}));
    let pwd = assert_ok(&pwd_result);
    assert_eq!(pwd["child_process"], false);
    assert_eq!(pwd["execution_mode"], "native_builtin");
    assert_eq!(pwd["sandbox_enforced"], false);
    assert!(pwd["stdout"].as_str().unwrap_or_default().contains("main"));

    let environment_result = invoke(&ctx, "check_exec_environment", json!({}));
    let environment = assert_ok(&environment_result);
    assert_eq!(environment["execution_isolation_mode"], "strict");
    let available = environment["filesystem_sandbox"]["available"]
        .as_bool()
        .unwrap_or(false);
    assert_eq!(environment["filesystem_sandbox"]["enforced"], available);
    assert_eq!(
        environment["workspace_exec_boundary"],
        if available {
            "windows_appcontainer"
        } else {
            "unavailable"
        }
    );
    assert_eq!(environment["workspace_exec_sandbox_enforced"], available);
}

#[test]
fn strict_health_check_does_not_start_a_probe_process() {
    let fixture = git_fixture();
    let ctx = ctx_for(&fixture.root);

    let result = invoke(&ctx, "exec_health_check", json!({}));
    assert_eq!(result["ok"], true);
    if sandbox_is_available(&ctx) {
        assert_eq!(result["status"], "success", "{result}");
        assert_eq!(result["session_create"], true, "{result}");
        assert_eq!(result["command_run"], true, "{result}");
        assert_eq!(result["stdout_capture"], true, "{result}");
        assert_eq!(result["stderr_capture"], true, "{result}");
    } else {
        assert_eq!(result["status"], "error");
        assert_eq!(result["error"]["code"], "EXEC_SANDBOX_UNAVAILABLE");
        assert_eq!(result["session_create"], false);
        assert_eq!(result["command_run"], false);
    }
}

#[test]
fn strict_sandbox_allows_writes_inside_execution_root() {
    let fixture = git_fixture();
    let ctx = ctx_for(&fixture.root);
    let target = fixture.root.join("inside.txt");
    let result = exec_command(
        &ctx,
        &json!({
            "cmd": "python -c \"from pathlib import Path; Path('inside.txt').write_text('ok')\"",
            "timeout_ms": 10_000,
            "yield_time_ms": 10_000
        }),
    );

    if sandbox_is_available(&ctx) {
        let output = result.expect("sandboxed write should return a command result");
        assert_eq!(output["sandbox_enforced"], true, "{output}");
        assert_eq!(output["command_ok"], true, "{output}");
        assert_eq!(fs::read_to_string(&target).expect("inside file"), "ok");
    } else {
        assert_sandbox_unavailable(result);
        assert!(!target.exists());
    }
}

#[test]
fn strict_sandbox_keeps_git_in_the_current_worktree() {
    let fixture = git_fixture();
    let ctx = ctx_for(&fixture.root);
    for command in ["git rev-parse --show-toplevel", "git status --short"] {
        let result = exec_command(
            &ctx,
            &json!({
                "cmd": command,
                "timeout_ms": 10_000,
                "yield_time_ms": 10_000
            }),
        );

        if sandbox_is_available(&ctx) {
            let output = result.expect("sandboxed Git command should return a result");
            assert_eq!(output["sandbox_enforced"], true, "{command}: {output}");
            assert_eq!(output["command_ok"], true, "{command}: {output}");
            let stdout = output["stdout"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if command.contains("show-toplevel") {
                let root = fixture.root.to_string_lossy().to_ascii_lowercase();
                assert!(stdout.contains(&root), "{command}: {output}");
            }
        } else {
            assert_sandbox_unavailable(result);
        }
    }
}

#[test]
fn strict_sandbox_keeps_git_in_a_linked_worktree() {
    let fixture = git_fixture();
    let harness = tempfile::tempdir().expect("harness tempdir");
    let workspace = Workspace::new_with_roots(fixture.root.clone(), fixture.sibling.clone())
        .expect("linked worktree workspace");
    let ctx = coding_tools_mcp_desktop_lib::tools::ToolContext::from_workspace_with_harness_root(
        workspace,
        Default::default(),
        PolicySettings::default(),
        "full".into(),
        "trusted".into(),
        harness.path().to_path_buf(),
    );
    let result = exec_command(
        &ctx,
        &json!({
            "cmd": "git rev-parse --show-toplevel",
            "timeout_ms": 10_000,
            "yield_time_ms": 10_000
        }),
    );

    if sandbox_is_available(&ctx) {
        let output = result.expect("sandboxed linked-worktree Git command should return a result");
        assert_eq!(output["sandbox_enforced"], true, "{output}");
        assert_eq!(output["command_ok"], true, "{output}");
        let stdout = output["stdout"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let root = fixture.sibling.to_string_lossy().to_ascii_lowercase();
        assert!(stdout.contains(&root), "{output}");
    } else {
        assert_sandbox_unavailable(result);
    }
}

#[test]
fn strict_sandbox_contains_descendant_processes() {
    let fixture = git_fixture();
    let ctx = ctx_for(&fixture.root);
    let escape = fixture.sibling.join("descendant-escape.txt");
    let child_code = format!(
        "from pathlib import Path; Path(r'{}').write_text('escape')",
        escape.to_string_lossy().replace('\\', "\\\\")
    );
    let parent_script = fixture.root.join("spawn-child.py");
    let script = format!(
        "import subprocess\nsubprocess.run(['python', '-c', {}], check=False)\n",
        serde_json::to_string(&child_code).expect("serialize child code")
    );
    fs::write(&parent_script, script).expect("write parent script");
    let command = format!("python \"{}\"", parent_script.display());
    let result = exec_command(
        &ctx,
        &json!({"cmd": command, "timeout_ms": 10_000, "yield_time_ms": 10_000}),
    );

    if sandbox_is_available(&ctx) {
        let output = result.expect("sandboxed descendant command should return a result");
        assert_eq!(output["sandbox_enforced"], true, "{output}");
        assert!(
            !escape.exists(),
            "descendant escaped the AppContainer: {output}"
        );
    } else {
        assert_sandbox_unavailable(result);
        assert!(!escape.exists());
    }
}

#[cfg(windows)]
#[test]
fn strict_sandbox_blocks_symlinked_sibling_paths() {
    let fixture = git_fixture();
    let link = fixture.root.join("link-to-sibling");
    let created = std::os::windows::fs::symlink_dir(&fixture.sibling, &link).is_ok();
    if !created {
        return;
    }
    let ctx = ctx_for(&fixture.root);
    let escape = link.join("symlink-escape.txt");
    let command = format!(
        "python -c \"from pathlib import Path; Path(r'{}').write_text('escape')\"",
        escape.to_string_lossy().replace('\\', "\\\\")
    );
    let result = exec_command(&ctx, &json!({"cmd": command}));

    if sandbox_is_available(&ctx) {
        let output = result.expect("sandboxed symlink command should return a result");
        assert_eq!(output["sandbox_enforced"], true, "{output}");
        assert!(
            !escape.exists(),
            "symlink escaped the AppContainer: {output}"
        );
    } else {
        assert_sandbox_unavailable(result);
        assert!(!escape.exists());
    }
}

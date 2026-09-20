#![cfg(windows)]
use coding_tools_mcp_desktop_lib::tools::{
    call_tool,
    policy::{ExecutionPolicy, IsolationPolicy, PolicySettings},
    workspace::Workspace,
    ToolContext,
};
use serde_json::{json, Value};
use std::{fs, path::Path, process::Command};

fn context(root: &Path, harness: &Path, host: bool) -> ToolContext {
    let policy = PolicySettings {
        permissions: ExecutionPolicy::from_config(
            "full_access",
            1,
            if host {
                IsolationPolicy::Host
            } else {
                IsolationPolicy::Strict
            },
        )
        .unwrap(),
        ..PolicySettings::default()
    };
    ToolContext::from_workspace_with_harness_root(
        Workspace::new(root.into()).unwrap(),
        Default::default(),
        policy,
        "full".into(),
        "full_access".into(),
        harness.into(),
    )
}
fn invoke(ctx: &ToolContext, name: &str, args: Value) -> Value {
    call_tool(ctx, name, &args)
}
fn ok(v: Value) -> Value {
    assert_eq!(v["ok"], true, "{v}");
    v
}
fn exec(ctx: &ToolContext, cmd: &str) -> Value {
    let v = ok(invoke(
        ctx,
        "exec_command",
        json!({"cmd":cmd,"yield_time_ms":10000,"timeout_ms":15000}),
    ));
    assert_eq!(v["command_ok"], true, "{v}");
    v
}
fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}
fn init_git(root: &Path) {
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Host Test"]);
    git(root, &["config", "user.email", "host@test.invalid"]);
    fs::write(root.join("seed.txt"), "seed").unwrap();
    git(root, &["add", "seed.txt"]);
    git(root, &["commit", "-qm", "seed"]);
}
#[test]
fn host_access_permission_configuration_and_intersection() {
    let host = ExecutionPolicy::from_config("full_access", 1, IsolationPolicy::Host).unwrap();
    for (v, n) in [("default", 1), ("full_access", 0), ("dangerous", 0)] {
        assert!(ExecutionPolicy::from_config(v, n, IsolationPolicy::Host).is_err());
    }
    for isolation in [
        IsolationPolicy::Strict,
        IsolationPolicy::Compatibility,
        IsolationPolicy::Host,
    ] {
        let other = ExecutionPolicy::from_config("full_access", 1, isolation).unwrap();
        assert_eq!(host.intersect(&other).isolation, isolation);
        assert_eq!(host.intersect(&other), other.intersect(&host));
        assert_eq!(
            host.intersect(&other).host_access(),
            isolation == IsolationPolicy::Host
        );
    }
}
#[test]
fn host_access_diagnostics_and_scope_match_actual_backend() {
    let t = tempfile::tempdir().unwrap();
    let h = tempfile::tempdir().unwrap();
    let ctx = context(t.path(), h.path(), true);
    for tool in ["server_info", "permission_status", "check_exec_environment"] {
        let v = ok(invoke(&ctx, tool, json!({})));
        assert_eq!(v["isolation_backend"], "host_process", "{v}");
        assert_eq!(v["sandbox_enforced"], false);
        assert_eq!(v["sandbox_bypass"], true);
        assert_eq!(v["fallback_allowed"], false);
        assert_eq!(v["isolation_capabilities"]["write"]["mode"], "host");
        assert!(v["process_elevated"].is_boolean());
    }
    let status = ok(invoke(&ctx, "check_exec_environment", json!({})));
    assert_eq!(status["filesystem_sandbox"]["host_scope_available"], true);
    assert_eq!(status["filesystem_sandbox"]["default_scope"], "host");
    let v = invoke(
        &ctx,
        "exec_command",
        json!({"cmd":"echo blocked","filesystem_scope":"workspace"}),
    );
    assert_eq!(
        v["error"]["code"], "ISOLATION_CAPABILITY_UNSATISFIED",
        "{v}"
    );
    let restricted = context(t.path(), h.path(), false);
    assert_eq!(
        invoke(
            &restricted,
            "exec_command",
            json!({"cmd":"echo blocked","filesystem_scope":"host"})
        )["error"]["code"],
        "EXTERNAL_EXECUTION_NOT_ALLOWED"
    );
}
#[test]
fn host_access_external_patch_environment_and_toolchain() {
    let t = tempfile::tempdir().unwrap();
    let h = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let ctx = context(t.path(), h.path(), true);
    let target = outside
        .path()
        .join("external.txt")
        .to_string_lossy()
        .replace('\\', "/");
    ok(invoke(
        &ctx,
        "apply_patch",
        json!({"patch":format!("*** Begin Patch\n*** Add File: {target}\n+outside\n*** End Patch\n")}),
    ));
    assert_eq!(fs::read_to_string(&target).unwrap(), "outside\n");
    ok(invoke(&ctx, "read_file", json!({"path":target})));
    let python = which::which("python")
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    let v = ok(invoke(
        &ctx,
        "exec_command",
        json!({"cmd":format!("\"{python}\" -c \"import os; print(os.environ['HOST_TEST']); print(os.getcwd())\""),"workdir":outside.path(),"env":{"HOST_TEST":"custom","HOME":outside.path()},"yield_time_ms":10000}),
    ));
    assert_eq!(v["command_ok"], true, "{v}");
    assert!(v["stdout"].as_str().unwrap().contains("custom"));
    assert_eq!(v["filesystem_scope"], "host");
    assert_eq!(v["execution_boundary"], "host");
    fs::write(
        outside.path().join("hostprobe.cmd"),
        "@echo path-override-ok\r\n",
    )
    .unwrap();
    let v = ok(invoke(
        &ctx,
        "exec_command",
        json!({"cmd":"hostprobe","env":{"PATH":outside.path(),"PATHEXT":".CMD"},"yield_time_ms":10000}),
    ));
    assert_eq!(v["command_ok"], true, "{v}");
    assert!(v["stdout"].as_str().unwrap().contains("path-override-ok"));
    for cmd in ["git --version", "python -c \"import subprocess; print(subprocess.check_output(['python','-c','print(42)']).decode())\"", "node -e \"console.log(require('child_process').execFileSync(process.execPath,['-e','console.log(42)']).toString())\""] {exec(&ctx,cmd);}
    exec(&ctx, "echo It's host & echo pipeline");
    fs::write(
        t.path().join("package.json"),
        r#"{"scripts":{"build":"node -e \"console.log('build-ok')\""}}"#,
    )
    .unwrap();
    exec(&ctx, "npm run build");
    assert_eq!(ctx.default_cwd_path(), t.path().canonicalize().unwrap());
}
#[test]
fn host_access_git_assets_and_strict_protection() {
    let t = tempfile::tempdir().unwrap();
    init_git(t.path());
    let h = tempfile::tempdir().unwrap();
    let ctx = context(t.path(), h.path(), true);
    let strict = context(t.path(), h.path(), false);
    for path in [".github/host-probe.txt", ".git/host-probe.txt"] {
        let args = json!({"patch":format!("*** Begin Patch\n*** Add File: {path}\n+probe\n*** End Patch\n")});
        assert_eq!(
            invoke(&strict, "apply_patch", args.clone())["error"]["code"],
            "PROTECTED_REPOSITORY_ASSET"
        );
        ok(invoke(&ctx, "apply_patch", args));
        assert!(t.path().join(path).exists());
    }
}
#[test]
fn host_access_branch_changes_preserve_session_and_binding() {
    let t = tempfile::tempdir().unwrap();
    init_git(t.path());
    let h = tempfile::tempdir().unwrap();
    let ctx = context(t.path(), h.path(), true);
    assert!(invoke(
        &ctx,
        "start_task",
        json!({"objective":"Verify branch changes"})
    )["task"]["id"]
        .is_string());
    let pending = ok(invoke(
        &ctx,
        "exec_command",
        json!({"cmd":"python -c \"import time; print('alive',flush=True); time.sleep(20)\"","yield_time_ms":1}),
    ));
    let id = pending["session_id"].as_str().unwrap();
    let identity = ctx.execution_fingerprint();
    exec(&ctx, "git checkout -b host-feature");
    exec(&ctx, "git commit --allow-empty -m next");
    exec(&ctx, "git reset --hard HEAD~1");
    exec(&ctx, "git checkout --detach");
    ok(invoke(&ctx, "git_status", json!({})));
    ok(invoke(&ctx, "get_default_cwd", json!({})));
    assert_eq!(identity, ctx.execution_fingerprint());
    ok(invoke(
        &ctx,
        "read_output",
        json!({"output_ref":format!("session:{id}:stdout")}),
    ));
    ok(invoke(
        &ctx,
        "kill_session",
        json!({"session_id":id,"wait_ms":2000}),
    ));
}
#[test]
fn host_access_sessions_are_context_local() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let h = tempfile::tempdir().unwrap();
    let ca = context(a.path(), h.path(), true);
    let cb = context(b.path(), h.path(), true);
    let v = ok(invoke(
        &ca,
        "exec_command",
        json!({"cmd":"python -c \"import time; time.sleep(20)\"","yield_time_ms":1}),
    ));
    assert_eq!(
        invoke(
            &cb,
            "read_output",
            json!({"output_ref":format!("session:{}:stdout",v["session_id"].as_str().unwrap())})
        )["ok"],
        false
    );
    assert_ne!(ca.default_cwd_path(), cb.default_cwd_path());
    ok(invoke(
        &ca,
        "kill_session",
        json!({"session_id":v["session_id"],"wait_ms":2000}),
    ));
}
#[test]
fn host_access_timeout_terminates_descendant_tree() {
    let t = tempfile::tempdir().unwrap();
    let h = tempfile::tempdir().unwrap();
    let ctx = context(t.path(), h.path(), true);
    fs::write(t.path().join("spawn.py"),"import subprocess,sys,time\np=subprocess.Popen([sys.executable,'-c',\"import time; time.sleep(2); open('escaped.txt','w').write('bad')\"])\ntime.sleep(20)\n").unwrap();
    let v = invoke(
        &ctx,
        "exec_command",
        json!({"cmd":"python spawn.py","timeout_ms":300,"yield_time_ms":1000}),
    );
    assert_eq!(v["command_ok"], false, "{v}");
    std::thread::sleep(std::time::Duration::from_millis(2300));
    assert!(!t.path().join("escaped.txt").exists());
}

#[test]
fn host_access_replaced_task_root_is_rejected() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("task");
    fs::create_dir(&root).unwrap();
    let h = tempfile::tempdir().unwrap();
    let ctx = context(&root, h.path(), true);
    fs::rename(&root, t.path().join("old")).unwrap();
    fs::create_dir(&root).unwrap();
    assert_eq!(
        invoke(&ctx, "get_default_cwd", json!({}))["error"]["code"],
        "WORKSPACE_CONTEXT_MISMATCH"
    );
}
#[test]
fn host_access_service_shutdown_terminates_descendants() {
    let t = tempfile::tempdir().unwrap();
    let h = tempfile::tempdir().unwrap();
    let ctx = context(t.path(), h.path(), true);
    fs::write(t.path().join("spawn.py"),"import subprocess,sys,time\np=subprocess.Popen([sys.executable,'-c',\"import time; time.sleep(2); open('escaped.txt','w').write('bad')\"])\ntime.sleep(20)\n").unwrap();
    ok(invoke(
        &ctx,
        "exec_command",
        json!({"cmd":"python spawn.py","yield_time_ms":100}),
    ));
    tauri::async_runtime::block_on(ctx.sessions.shutdown_all("server_stop")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2300));
    assert!(!t.path().join("escaped.txt").exists());
}

#[test]
fn host_access_parent_exit_does_not_orphan_descendants() {
    let t = tempfile::tempdir().unwrap();
    let h = tempfile::tempdir().unwrap();
    let ctx = context(t.path(), h.path(), true);
    fs::write(t.path().join("spawn.py"), "import subprocess,sys\nsubprocess.Popen([sys.executable,'-c',\"import time; time.sleep(2); open('orphan.txt','w').write('bad')\"])\n").unwrap();
    exec(&ctx, "python spawn.py");
    std::thread::sleep(std::time::Duration::from_millis(2300));
    assert!(!t.path().join("orphan.txt").exists());
}

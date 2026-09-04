use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};
use uuid::Uuid;

use crate::tools::context::ToolContext;
use crate::tools::workspace::{tool_ok, WorkspaceError};
use crate::tools::PathIntent;

pub fn apply_patch(ctx: &ToolContext, args: &Value) -> Result<Value, WorkspaceError> {
    ctx.validate_execution_context()?;
    let patch = args
        .get("patch")
        .and_then(Value::as_str)
        .ok_or_else(|| WorkspaceError::invalid_argument("patch is required"))?;
    let dry_run = args
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let confirm = args
        .get("confirm")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let file_patches = parse_unified_diff(patch)?;
    if file_patches.is_empty() {
        return Err(patch_failed("No files were modified."));
    }
    if let Some(path) = file_patches
        .iter()
        .find(|file| is_protected_repository_asset(&file.path))
        .map(|file| file.path.as_str())
    {
        return Err(protected_repository_asset(format!(
            "禁止删除仓库保护资产: {path}"
        )));
    }
    if !confirm {
        if let Some(path) = file_patches
            .iter()
            .find(|file| file.is_deleted && is_critical_file(&file.path))
            .map(|file| file.path.as_str())
        {
            return Err(dangerous_operation(format!(
                "删除关键项目文件需要 confirm=true: {path}"
            )));
        }
    }

    let mut affected = Vec::new();
    let mut summaries = Vec::new();
    // Keep absolute validated paths in the transaction. Re-resolving a display
    // string from the workspace root is what previously allowed nested worktrees
    // to drift back to the parent repository.
    let mut staged: HashMap<PathBuf, Option<String>> = HashMap::new();

    for fp in &file_patches {
        let resolved = ctx.resolve_for_write_from_default_cwd(&fp.path)?;
        if !fp.is_new_file && !resolved.existed {
            return Err(WorkspaceError::not_found(format!(
                "File not found: {}",
                fp.path
            )));
        }

        let original = if fp.is_new_file {
            // An Add File envelope is replacement content even when an earlier
            // Delete File for the same path exists in this transaction.
            String::new()
        } else if resolved.existed {
            fs::read_to_string(&resolved.path)
                .map_err(|_| WorkspaceError::not_found(format!("File not found: {}", fp.path)))?
        } else if fp.is_deleted {
            String::new()
        } else {
            return Err(patch_failed(format!("File not found: {}", fp.path)));
        };

        if fp.is_deleted {
            staged.insert(resolved.path.clone(), None);
            affected.push(json!({ "path": resolved.display, "operation": "delete" }));
            summaries.push(format!("D {}", resolved.display));
            continue;
        }

        let updated = apply_hunks(&original, &fp.hunks)?;
        let op = if resolved.existed { "update" } else { "add" };
        staged.insert(resolved.path.clone(), Some(updated));
        affected.push(json!({ "path": resolved.display, "operation": op }));
        summaries.push(format!(
            "{} {}",
            if op == "add" { "A" } else { "M" },
            resolved.display
        ));
    }

    let files_created = affected_paths(&affected, "add");
    let files_modified = affected_paths(&affected, "update");
    let files_deleted = affected_paths(&affected, "delete");

    if !dry_run {
        let transaction_backups = commit_staged(ctx, &staged)?;
        if let Err(error) = verify_staged_with_context(ctx, &staged) {
            restore_backups(&transaction_backups);
            return Err(error);
        }
        ctx.validate_execution_context()?;
        let change_id = Uuid::new_v4().simple().to_string();
        return Ok(tool_ok(json!({
            "dry_run": false,
            "clean": true,
            "verified": true,
            "change_id": change_id,
            "summary": summaries.join("\n"),
            "affected_files": affected,
            "files_created": files_created,
            "files_modified": files_modified,
            "files_deleted": files_deleted,
            "recovery": "git",
            "warnings": []
        })));
    }

    Ok(tool_ok(json!({
        "dry_run": true,
        "preflight": true,
        "clean": true,
        "summary": summaries.join("\n"),
        "affected_files": affected,
        "would_create": files_created,
        "would_modify": files_modified,
        "would_delete": files_deleted,
        "warnings": []
    })))
}

pub fn patch_check(ctx: &ToolContext, args: &Value) -> Result<Value, WorkspaceError> {
    let mut check_args = args.clone();
    check_args["dry_run"] = Value::Bool(true);
    let mut result = apply_patch(ctx, &check_args)?;
    if let Some(object) = result.as_object_mut() {
        object.insert("preflight".into(), Value::Bool(true));
    }
    Ok(result)
}

#[derive(Debug)]
struct FilePatch {
    path: String,
    hunks: Vec<Hunk>,
    is_new_file: bool,
    is_deleted: bool,
}

#[derive(Debug)]
struct Hunk {
    lines: Vec<HunkLine>,
}

#[derive(Debug)]
enum HunkLine {
    Context(String),
    Add(String),
    Remove(String),
}

fn parse_unified_diff(patch: &str) -> Result<Vec<FilePatch>, WorkspaceError> {
    if patch
        .lines()
        .any(|line| line.trim_end_matches('\r') == "*** Begin Patch")
    {
        return parse_codex_patch(patch);
    }

    let mut files = Vec::new();
    let mut current: Option<FilePatch> = None;
    let mut current_hunk: Option<Hunk> = None;

    for line in patch.lines() {
        if line.starts_with("--- ") {
            if let Some(h) = current_hunk.take() {
                if let Some(ref mut f) = current {
                    f.hunks.push(h);
                }
            }
            if let Some(f) = current.take() {
                files.push(f);
            }
            let path = parse_diff_path(line.strip_prefix("--- ").unwrap_or(""));
            current = Some(FilePatch {
                path,
                hunks: Vec::new(),
                is_new_file: line.contains("/dev/null"),
                is_deleted: false,
            });
        } else if line.starts_with("+++ ") {
            if let Some(ref mut f) = current {
                let new_path = parse_diff_path(line.strip_prefix("+++ ").unwrap_or(""));
                if !new_path.is_empty() && new_path != "/dev/null" {
                    f.path = new_path;
                }
                if line.contains("/dev/null") {
                    f.is_deleted = true;
                }
            }
        } else if line.starts_with("@@") {
            if let Some(h) = current_hunk.take() {
                if let Some(ref mut f) = current {
                    f.hunks.push(h);
                }
            }
            current_hunk = Some(Hunk { lines: Vec::new() });
        } else if let Some(ref mut hunk) = current_hunk {
            if let Some(rest) = line.strip_prefix('+') {
                hunk.lines.push(HunkLine::Add(rest.to_string()));
            } else if let Some(rest) = line.strip_prefix('-') {
                hunk.lines.push(HunkLine::Remove(rest.to_string()));
            } else if let Some(rest) = line.strip_prefix(' ') {
                hunk.lines.push(HunkLine::Context(rest.to_string()));
            } else if line.is_empty() {
                hunk.lines.push(HunkLine::Context(String::new()));
            }
        }
    }
    if let Some(h) = current_hunk.take() {
        if let Some(ref mut f) = current {
            f.hunks.push(h);
        }
    }
    if let Some(f) = current.take() {
        files.push(f);
    }
    Ok(files)
}

fn parse_codex_patch(patch: &str) -> Result<Vec<FilePatch>, WorkspaceError> {
    let mut files = Vec::new();
    let mut current: Option<FilePatch> = None;
    let mut current_hunk: Option<Hunk> = None;

    for raw_line in patch.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line == "*** Begin Patch" {
            continue;
        }
        if line == "*** End Patch" {
            finish_codex_file(&mut files, &mut current, &mut current_hunk);
            continue;
        }

        let header = line
            .strip_prefix("*** Add File: ")
            .map(|path| (path, true, false))
            .or_else(|| {
                line.strip_prefix("*** Update File: ")
                    .map(|path| (path, false, false))
            })
            .or_else(|| {
                line.strip_prefix("*** Delete File: ")
                    .map(|path| (path, false, true))
            });
        if let Some((path, is_new_file, is_deleted)) = header {
            finish_codex_file(&mut files, &mut current, &mut current_hunk);
            current = Some(FilePatch {
                path: parse_diff_path(path),
                hunks: Vec::new(),
                is_new_file,
                is_deleted,
            });
            if is_new_file {
                current_hunk = Some(Hunk { lines: Vec::new() });
            }
            continue;
        }

        if line.starts_with("@@") {
            if let Some(hunk) = current_hunk.take() {
                if let Some(ref mut file) = current {
                    file.hunks.push(hunk);
                }
            }
            current_hunk = Some(Hunk { lines: Vec::new() });
            continue;
        }

        let Some(file) = current.as_ref() else {
            continue;
        };
        if file.is_deleted {
            continue;
        }
        let hunk = current_hunk.get_or_insert_with(|| Hunk { lines: Vec::new() });
        if let Some(rest) = line.strip_prefix('+') {
            hunk.lines.push(HunkLine::Add(rest.to_string()));
        } else if let Some(rest) = line.strip_prefix('-') {
            hunk.lines.push(HunkLine::Remove(rest.to_string()));
        } else if let Some(rest) = line.strip_prefix(' ') {
            hunk.lines.push(HunkLine::Context(rest.to_string()));
        } else if line.is_empty() {
            hunk.lines.push(HunkLine::Context(String::new()));
        }
    }

    finish_codex_file(&mut files, &mut current, &mut current_hunk);
    Ok(files)
}

fn finish_codex_file(
    files: &mut Vec<FilePatch>,
    current: &mut Option<FilePatch>,
    current_hunk: &mut Option<Hunk>,
) {
    if let Some(hunk) = current_hunk.take() {
        if let Some(file) = current.as_mut() {
            file.hunks.push(hunk);
        }
    }
    if let Some(file) = current.take() {
        files.push(file);
    }
}

fn affected_paths(affected: &[Value], operation: &str) -> Vec<String> {
    affected
        .iter()
        .filter(|file| file["operation"] == operation)
        .filter_map(|file| file["path"].as_str().map(str::to_string))
        .collect()
}

fn parse_diff_path(raw: &str) -> String {
    let trimmed = raw.trim();
    let path = trimmed
        .strip_prefix("a/")
        .or_else(|| trimmed.strip_prefix("b/"))
        .unwrap_or(trimmed);
    if path == "/dev/null" {
        return String::new();
    }
    path.replace('\\', "/")
}

fn apply_hunks(original: &str, hunks: &[Hunk]) -> Result<String, WorkspaceError> {
    let line_ending = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let had_trailing_newline = original.ends_with('\n');
    let mut lines: Vec<String> = if original.is_empty() {
        Vec::new()
    } else {
        original
            .split_terminator('\n')
            .map(|line| line.trim_end_matches('\r').to_string())
            .collect()
    };
    let mut offset: i64 = 0;

    for hunk in hunks {
        let search_at = 0usize;
        let hunk_old: Vec<String> = hunk
            .lines
            .iter()
            .filter_map(|l| match l {
                HunkLine::Context(s) | HunkLine::Remove(s) => Some(s.clone()),
                HunkLine::Add(_) => None,
            })
            .collect();

        let pos = find_hunk_position(&lines, &hunk_old, search_at)
            .ok_or_else(|| patch_failed("Hunk context did not match file content."))?;

        let mut idx = pos;
        for hl in &hunk.lines {
            match hl {
                HunkLine::Context(_) => idx += 1,
                HunkLine::Remove(_) => {
                    if idx < lines.len() {
                        lines.remove(idx);
                    }
                }
                HunkLine::Add(s) => {
                    lines.insert(idx, s.clone());
                    idx += 1;
                }
            }
        }
        offset += 0; // reserved for future fuzzy offset
        let _ = offset;
    }
    let mut output = lines.join(line_ending);
    if !output.is_empty() && (had_trailing_newline || original.is_empty()) {
        output.push_str(line_ending);
    }
    Ok(output)
}

fn find_hunk_position(lines: &[String], pattern: &[String], start: usize) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start);
    }
    if start > lines.len() || pattern.len() > lines.len().saturating_sub(start) {
        return None;
    }
    for i in start..=lines.len().saturating_sub(pattern.len()) {
        if lines[i..i + pattern.len()]
            .iter()
            .zip(pattern.iter())
            .all(|(a, b)| a == b)
        {
            return Some(i);
        }
    }
    None
}

fn commit_staged(
    ctx: &ToolContext,
    staged: &HashMap<PathBuf, Option<String>>,
) -> Result<HashMap<PathBuf, Option<Vec<u8>>>, WorkspaceError> {
    let staged_bytes = staged
        .iter()
        .map(|(path, content)| {
            (
                path.clone(),
                content.as_ref().map(|value| value.as_bytes().to_vec()),
            )
        })
        .collect::<HashMap<_, _>>();
    for (path, content) in staged {
        ctx.validate_path(path, PathIntent::Write)?;
        if content.is_none() && !path.exists() {
            return Err(patch_failed(format!(
                "Patch target does not exist: {}",
                ctx.display_path(path)
            )));
        }
    }
    commit_staged_paths(ctx, &staged_bytes)
}

fn commit_staged_paths(
    ctx: &ToolContext,
    staged: &HashMap<PathBuf, Option<Vec<u8>>>,
) -> Result<HashMap<PathBuf, Option<Vec<u8>>>, WorkspaceError> {
    ctx.validate_execution_context()?;
    commit_staged_paths_unchecked(staged)
}

fn commit_staged_paths_unchecked(
    staged: &HashMap<PathBuf, Option<Vec<u8>>>,
) -> Result<HashMap<PathBuf, Option<Vec<u8>>>, WorkspaceError> {
    let mut backups: HashMap<PathBuf, Option<Vec<u8>>> = HashMap::new();
    let mut temporary_files = HashMap::new();
    for (path, content) in staged {
        let path = path.clone();
        backups.insert(
            path.clone(),
            if path.exists() && path.is_file() {
                Some(fs::read(&path).unwrap_or_default())
            } else {
                None
            },
        );
        if let Some(bytes) = content {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|err| patch_failed(err.to_string()))?;
            }
            let temp = path.with_file_name(format!(
                ".{}.harness-stage-{}",
                path.file_name().and_then(|v| v.to_str()).unwrap_or("file"),
                Uuid::new_v4().simple()
            ));
            if let Err(err) = fs::write(&temp, bytes) {
                cleanup_temporary_files(temporary_files.values());
                restore_backups(&backups);
                return Err(patch_failed(format!("Failed to stage file: {err}")));
            }
            temporary_files.insert(path.clone(), temp);
        }
    }

    for (path, content) in staged {
        let path = path.clone();
        let result = if content.is_some() {
            let temp = temporary_files
                .get(&path)
                .cloned()
                .ok_or_else(|| patch_failed("Staged file is missing"));
            match temp {
                Ok(temp) => replace_file(&temp, &path),
                Err(error) => Err(std::io::Error::other(error.to_string())),
            }
        } else if path.exists() && path.is_file() {
            fs::remove_file(&path)
        } else {
            Ok(())
        };
        if let Err(err) = result {
            cleanup_temporary_files(temporary_files.values());
            restore_backups(&backups);
            return Err(patch_failed(format!("Failed to write file: {err}")));
        }
    }
    cleanup_temporary_files(temporary_files.values());
    Ok(backups)
}

fn restore_backups(backups: &HashMap<PathBuf, Option<Vec<u8>>>) {
    for (path, data) in backups {
        match data {
            None => {
                let _ = fs::remove_file(path);
            }
            Some(bytes) => {
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(path, bytes);
            }
        }
    }
}

#[cfg(test)]
fn verify_staged(
    ws: &crate::tools::workspace::Workspace,
    staged: &HashMap<String, Option<String>>,
) -> Result<(), WorkspaceError> {
    for (relative, expected) in staged {
        let path = ws.resolve_for_write(relative)?.path;
        match expected {
            Some(expected) => {
                let actual = fs::read_to_string(&path).map_err(|error| {
                    patch_failed(format!(
                        "Patch verification could not read {relative}: {error}"
                    ))
                })?;
                if &actual != expected {
                    return Err(patch_failed(format!(
                        "Patch verification failed for {relative}."
                    )));
                }
            }
            None if path.exists() => {
                return Err(patch_failed(format!(
                    "Patch verification expected {relative} to be deleted."
                )));
            }
            None => {}
        }
    }
    Ok(())
}

fn verify_staged_with_context(
    ctx: &ToolContext,
    staged: &HashMap<PathBuf, Option<String>>,
) -> Result<(), WorkspaceError> {
    ctx.validate_execution_context()?;
    for (path, expected) in staged {
        let path = ctx.validate_path(path, PathIntent::Write)?;
        let display = ctx.display_path(&path);
        match expected {
            Some(expected) => {
                let actual = fs::read_to_string(&path).map_err(|error| {
                    patch_failed(format!(
                        "Patch verification could not read {display}: {error}"
                    ))
                })?;
                if &actual != expected {
                    return Err(patch_failed(format!(
                        "Patch verification failed for {display}."
                    )));
                }
            }
            None if path.exists() => {
                return Err(patch_failed(format!(
                    "Patch verification expected {display} to be deleted."
                )));
            }
            None => {}
        }
    }
    ctx.validate_execution_context()
}

fn replace_file(temp: &PathBuf, path: &PathBuf) -> Result<(), std::io::Error> {
    #[cfg(windows)]
    {
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    fs::rename(temp, path)
}

fn cleanup_temporary_files<'a>(paths: impl Iterator<Item = &'a PathBuf>) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn is_critical_file(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let first = normalized.split('/').next().unwrap_or("");
    if matches!(first, ".git" | ".github") {
        return true;
    }
    let name = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    name == ".gitignore"
        || name == "Cargo.toml"
        || name == "Cargo.lock"
        || name == "package.json"
        || name == "package-lock.json"
        || name == "pnpm-lock.yaml"
        || name == "tauri.conf.json"
        || name.starts_with("README")
        || name.starts_with("LICENSE")
        || name.starts_with("vite.config.")
        || name == "pyproject.toml"
}

fn is_protected_repository_asset(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let first = normalized.split('/').next().unwrap_or("");
    matches!(first, ".git" | ".github")
}

fn dangerous_operation(message: impl Into<String>) -> WorkspaceError {
    WorkspaceError::Tool {
        code: "DANGEROUS_OPERATION_REQUIRES_CONFIRMATION",
        message: message.into(),
        category: "permission",
        retryable: false,
    }
}

fn protected_repository_asset(message: impl Into<String>) -> WorkspaceError {
    WorkspaceError::Tool {
        code: "PROTECTED_REPOSITORY_ASSET",
        message: message.into(),
        category: "security",
        retryable: false,
    }
}

fn patch_failed(message: impl Into<String>) -> WorkspaceError {
    WorkspaceError::Tool {
        code: "PATCH_FAILED",
        message: message.into(),
        category: "validation",
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::context::ToolContext;
    use serde_json::json;
    use tempfile::tempdir;

    fn context_with_file() -> (tempfile::TempDir, tempfile::TempDir, ToolContext) {
        let workspace = tempdir().expect("workspace");
        let harness = tempdir().expect("harness");
        std::fs::write(workspace.path().join("main.rs"), "old\n").expect("file");
        let context =
            ToolContext::for_test(workspace.path().to_path_buf(), harness.path().to_path_buf())
                .expect("context");
        (workspace, harness, context)
    }

    fn patch() -> Value {
        json!({
            "patch": "--- a/main.rs\n+++ b/main.rs\n@@\n-old\n+new\n"
        })
    }

    #[test]
    fn patch_check_does_not_modify_workspace() {
        let (_workspace, _harness, context) = context_with_file();
        let result = patch_check(&context, &patch()).expect("patch check");
        assert_eq!(result["preflight"], true);
        assert_eq!(
            std::fs::read_to_string(context.workspace.root().join("main.rs")).unwrap(),
            "old\n"
        );
    }

    #[test]
    fn preserves_crlf_when_inserting_multiple_lines() {
        let input = "one\r\ntwo\r\n";
        let hunk = Hunk {
            lines: vec![
                HunkLine::Context("one".into()),
                HunkLine::Add("insert-a".into()),
                HunkLine::Add("insert-b".into()),
                HunkLine::Context("two".into()),
            ],
        };
        assert_eq!(
            apply_hunks(input, &[hunk]).expect("patch"),
            "one\r\ninsert-a\r\ninsert-b\r\ntwo\r\n"
        );
    }

    #[test]
    fn delete_then_add_same_path_replaces_instead_of_concatenating_old_content() {
        let (_workspace, _harness, context) = context_with_file();
        let result = apply_patch(
            &context,
            &json!({
                "patch": "*** Begin Patch\n*** Delete File: main.rs\n*** Add File: main.rs\n+fresh\n*** End Patch\n"
            }),
        )
        .expect("replace file");
        assert_eq!(result["verified"], true);
        assert_eq!(result["files_modified"], json!(["main.rs"]));
        assert_eq!(
            std::fs::read_to_string(context.workspace.root().join("main.rs")).unwrap(),
            "fresh\n"
        );
    }

    #[test]
    fn validation_failure_in_later_file_keeps_all_files_unchanged() {
        let (_workspace, _harness, context) = context_with_file();
        let error = apply_patch(
            &context,
            &json!({
                "patch": "--- a/main.rs\n+++ b/main.rs\n@@\n-old\n+new\n--- a/missing.rs\n+++ b/missing.rs\n@@\n-old\n+new\n"
            }),
        )
        .expect_err("later file fails preflight");
        assert_eq!(error.to_error_value()["code"], "NOT_FOUND");
        assert_eq!(
            std::fs::read_to_string(context.workspace.root().join("main.rs")).unwrap(),
            "old\n"
        );
    }

    #[test]
    fn verification_rejects_content_that_does_not_match_disk() {
        let (_workspace, _harness, context) = context_with_file();
        let staged = HashMap::from([("main.rs".to_string(), Some("unexpected\n".to_string()))]);
        let error = verify_staged(&context.workspace, &staged).expect_err("verification mismatch");
        assert_eq!(error.to_error_value()["code"], "PATCH_FAILED");
    }
}

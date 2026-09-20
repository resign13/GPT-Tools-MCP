# Subspec: compatibility-tests-release

## 范围

Compatibility/security tests, version synchronization, build gates, and connector smoke acceptance.

## 需求回链

- FR-6

## 验收标准

WHEN gateway is disabled THEN existing MCP, OAuth, Actions, and schemas SHALL remain compatible.
WHEN release gates run THEN check, build, tests, and clippy SHALL pass before packaging.
WHEN versions are inspected THEN all five sources SHALL report `0.3.0`.

## 涉及文件

- `src-tauri/tests/`
- `src/**/*.test.ts`
- `package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`

## 不做项

- Automatic Cloudflare DNS changes.
- Persistent session storage.

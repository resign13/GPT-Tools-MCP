# Subspec: gateway-tools-security

## 范围

Gateway tool schemas, authorization intersection, invalidation, and redacted logging.

## 需求回链

- FR-3
- FR-4

## 验收标准

WHEN `list_workspaces` is called THEN it SHALL return only sanitized allowlist fields.
WHEN a target is outside the allowlist THEN the system SHALL reject it even through path arguments.
WHEN a tool is called THEN effective permission SHALL be the host/target intersection.

## 涉及文件

- `src-tauri/src/mcp/gateway.rs`
- `src-tauri/src/tools/registry.rs`
- `src-tauri/src/auth/`

## 不做项

- Secret storage redesign.
- GPT Actions routing.

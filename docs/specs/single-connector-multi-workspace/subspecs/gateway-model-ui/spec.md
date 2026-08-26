# Subspec: gateway-model-ui

## 范围

Persisted gateway configuration, migration, UI controls, and authorization projection.

## 需求回链

- FR-1
- FR-5

## 验收标准

WHEN old profile JSON is loaded THEN the system SHALL default gateway to disabled.
WHEN an allowlist is saved THEN the system SHALL include the host and reject missing IDs.
WHEN the form is rendered THEN it SHALL reuse existing stores and SHALL not edit Cloudflare DNS.

## 涉及文件

- `src-tauri/src/workspace/model.rs`
- `src-tauri/src/workspace/store.rs`
- `src/lib/components/GatewayConfigForm.svelte`

## 不做项

- GPT Actions multi-workspace routing.
- Automatic tunnel provisioning.

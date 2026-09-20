# Subspec: Conversation identity and automatic workspace binding

## 范围

Resolve logical conversation identity independently from the MCP transport
session and retain bounded aliases for connector reconnects.

## 需求回链

- FR-1

## 验收标准

1. WHEN OpenAI metadata is present THEN the gateway SHALL use source
   `openai_conversation`.
2. WHEN only the MCP header is present THEN the gateway SHALL use source
   `mcp_transport`.
3. WHEN initialize has no identifiers THEN the gateway SHALL return a
   generated `Mcp-Session-Id`.
4. WHEN known aliases conflict THEN the gateway SHALL return
   `session_identity_conflict`.
5. WHEN a request is logged THEN the gateway SHALL log a session hash instead
   of a raw ID.

## 涉及文件

Keep transport and logical IDs separate. Store transport-to-logical aliases
next to the existing bounded binding cache and reuse the existing TTL policy.

- `src-tauri/src/mcp/listener.rs`
- `src-tauri/src/mcp/gateway.rs`

## 不做项

- Persistent identity storage.
- Semantic session merging across different OpenAI IDs.

## 设计要点

Keep transport and logical IDs separate. Store transport-to-logical aliases
next to the existing bounded binding cache and reuse the existing TTL policy.

- Add identity extraction, alias resolution, conflict detection, and focused
  unit tests.

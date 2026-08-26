# Subspec: Concurrent routing and per-workspace serialization

## 范围

Run independent conversations concurrently while protecting same-directory
filesystem, command, history, and Harness writes.

## 需求回链

- FR-3

## 验收标准

1. WHEN canonical workspace paths differ THEN the gateway SHALL execute calls
   concurrently.
2. EACH binding SHALL own an independent cwd, command store, Harness, and
   history context.
3. WHEN same-workspace calls are read-only THEN the gateway SHALL allow overlap.
4. WHEN same-workspace calls mutate state THEN the gateway SHALL acquire an
   exclusive queued permit.
5. WHEN more than eight requests are active THEN the gateway SHALL wait within
   its bound or return retryable `gateway_busy`.

## 涉及文件

Add a canonical-path lock table and a bounded global request gate to
`GatewayRouter`. Route all target calls through the permit before invoking the
unchanged dispatch kernel.

- `src-tauri/src/mcp/gateway.rs`
- `src-tauri/src/mcp/listener.rs`

## 不做项

- Cross-process coordination with separately running target MCP servers.
- Persistent queue state after application restart.

## 设计要点

Add a canonical-path lock table and a bounded global request gate to
`GatewayRouter`. Route all target calls through the permit before invoking the
unchanged dispatch kernel.

- Add lock classification, permits, bounded waiting, and concurrency tests.

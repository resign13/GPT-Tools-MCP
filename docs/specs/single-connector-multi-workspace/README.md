# single-connector-multi-workspace v0.3.0

## 原则

The gateway is an opt-in MCP layer hosted by one existing workspace. It keeps
the existing per-workspace MCP and GPT Actions modes compatible.

## 子规格索引

| ID | 标题 | FR | 依赖 |
|---|---|---|---|
| gateway-model-ui | Gateway model and UI | FR-1, FR-5 | |
| mcp-session-routing | MCP session routing | FR-2 | gateway-model-ui |
| gateway-tools-security | Gateway tools and security | FR-3, FR-4 | mcp-session-routing |
| compatibility-tests-release | Compatibility tests and release | FR-6 | gateway-model-ui, mcp-session-routing, gateway-tools-security |

## 依赖关系

Dependencies are maintained by `dependsOn` in `spec-manifest.json`.

## 里程碑

1. Validate the parent and child specifications.
2. Implement model, routing, tools, UI, tests, and release gates in dependency order.

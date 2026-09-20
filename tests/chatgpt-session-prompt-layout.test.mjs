import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const workspacePagePath = new URL("../src/routes/workspace/[id]/+page.svelte", import.meta.url);
const gatewayPagePath = new URL("../src/routes/gateway/+page.svelte", import.meta.url);
const gatewayNewPagePath = new URL("../src/routes/gateway/new/+page.svelte", import.meta.url);
const quickCopyPath = new URL("../src/lib/components/GptQuickCopy.svelte", import.meta.url);
const sessionPromptPath = new URL(
  "../src/lib/components/ChatGptSessionPrompt.svelte",
  import.meta.url,
);

test("workspace configuration keeps only the MCP launcher and lower configuration", async () => {
  const source = await readFile(workspacePagePath, "utf8");

  assert.doesNotMatch(source, /WorkspaceMetaForm/);
  assert.match(source, /title="MCP"/);
  assert.doesNotMatch(source, /GatewayConfigForm/);
  assert.doesNotMatch(source, /<header/);
  assert.doesNotMatch(source, /activeService/);
  assert.doesNotMatch(source, /ActionsAuthForm|ActionsPolicyForm|title="Actions"/);
});

test("工作区配置页不再展示会话恢复快捷入口或 GPT 配置卡", async () => {
  const source = await readFile(workspacePagePath, "utf8");

  assert.doesNotMatch(source, /ChatGptSessionPrompt/);
  assert.doesNotMatch(source, /GptQuickCopy/);
});

test("网关任务页只保留任务列表和任务 ID，不渲染配置卡或会话提示卡", async () => {
  const source = await readFile(gatewayPagePath, "utf8");

  assert.doesNotMatch(source, /ChatGptSessionPrompt/);
  assert.doesNotMatch(source, /GptQuickCopy/);
  assert.match(source, /任务 ID/);
  assert.match(source, /bind_workspace/);
  assert.ok(
    source.indexOf('<p class="tx-section-label">当前任务') <
      source.indexOf('<p class="tx-section-label">工作区任务'),
    "当前任务详情应显示在任务列表上方",
  );
});

test("新建工作区任务会自动加入网关任务绑定", async () => {
  const source = await readFile(gatewayNewPagePath, "utf8");

  assert.match(source, /Array\.from\(new Set\(\[host\.id, \.\.\.items\.map\(\(item\) => item\.id\)\]\)\)/);
  assert.match(source, /workspace_ids: items\.map\(\(item\) => item\.id\)/);
  assert.match(source, /默认绑定到任务列表/);
});

test("GPT 配置卡片不再重复展示会话恢复入口", async () => {
  const source = await readFile(quickCopyPath, "utf8");

  assert.doesNotMatch(source, /ChatGptSessionPrompt/);
});

test("会话恢复快捷入口默认紧凑，并可展开完整提示词", async () => {
  const source = await readFile(sessionPromptPath, "utf8");

  assert.match(source, /let expanded = \$state\(false\)/);
  assert.match(source, /aria-expanded=\{expanded\}/);
  assert.match(source, /查看完整提示词/);
  assert.match(source, /\{#if expanded\}[\s\S]*<pre/);
});

test("复制和展开操作保留可触达尺寸与状态反馈", async () => {
  const source = await readFile(sessionPromptPath, "utf8");

  assert.ok((source.match(/min-h-11/g) ?? []).length >= 2, "两个操作按钮都应至少为 44px 高");
  assert.match(source, /aria-live="polite"/);
  assert.match(source, /复制完整提示词/);
  assert.match(source, /已复制/);
});

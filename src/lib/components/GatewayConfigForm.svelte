<script lang="ts">
  import type { GatewayConfig, WorkspaceProfile } from "$lib/types";

  interface Props {
    config?: GatewayConfig;
    workspaceOptions: WorkspaceProfile[];
    connectionUrl: string;
    hostWorkspaceId: string;
    onSave: (draft: GatewayConfig) => void | Promise<void>;
  }

  let { config, workspaceOptions, connectionUrl, hostWorkspaceId, onSave }: Props = $props();
  let enabled = $state(false);
  let selectedIds = $state<string[]>([]);
  let prompt = $state("");
  let saving = $state(false);

  $effect(() => {
    enabled = config?.enabled ?? false;
    selectedIds = [...(config?.workspace_ids ?? [])];
    prompt = config?.prompt ?? "";
  });

  const dirty = $derived(
    enabled !== (config?.enabled ?? false) ||
      prompt !== (config?.prompt ?? "") ||
      selectedIds.join(",") !== (config?.workspace_ids ?? []).join(","),
  );

  function toggleWorkspace(id: string) {
    if (id === hostWorkspaceId) return;
    selectedIds = selectedIds.includes(id)
      ? selectedIds.filter((value) => value !== id)
      : [...selectedIds, id];
  }

  async function save() {
    if (saving || !dirty) return;
    saving = true;
    try {
      const ids = selectedIds.includes(hostWorkspaceId)
        ? selectedIds
        : [hostWorkspaceId, ...selectedIds];
      await onSave({ enabled, workspace_ids: ids, prompt: prompt.trim() });
    } finally {
      saving = false;
    }
  }
</script>

<form
  class="grid gap-3"
  onsubmit={(event) => {
    event.preventDefault();
    void save();
  }}
>
  <label class="flex items-center gap-2 text-sm">
    <input type="checkbox" bind:checked={enabled} />
    <span>启用单连接器多工作区网关</span>
  </label>

  <div class="grid gap-2">
    <span class="text-xs text-[var(--color-text-muted)]">允许访问的工作区</span>
    <div class="grid gap-2 sm:grid-cols-2">
      {#each workspaceOptions as workspace}
        <label class="flex min-w-0 items-start gap-2 rounded-md border border-[var(--color-border)] px-3 py-2 text-sm">
          <input
            type="checkbox"
            checked={selectedIds.includes(workspace.id)}
            disabled={workspace.id === hostWorkspaceId}
            onchange={() => toggleWorkspace(workspace.id)}
          />
          <span class="min-w-0">
            <span class="block truncate">{workspace.name}</span>
            <span class="block truncate text-xs text-[var(--color-text-muted)]">{workspace.path}</span>
          </span>
        </label>
      {/each}
    </div>
  </div>

  <label class="grid gap-1">
    <span class="text-xs text-[var(--color-text-muted)]">当前连接地址</span>
    <input class="tx-input font-mono text-xs" value={connectionUrl} readonly />
  </label>

  <label class="grid gap-1">
    <span class="text-xs text-[var(--color-text-muted)]">网关提示词（可选）</span>
    <textarea
      class="tx-input min-h-20 resize-y text-sm"
      bind:value={prompt}
      placeholder="先列出并选择工作区，再初始化该项目历史会话"
    ></textarea>
  </label>

  <p class="text-xs text-[var(--color-text-muted)]">
    网关配置不会修改 Cloudflare DNS。会话绑定仅保存在宿主内存中，重启后需要重新选择工作区。
  </p>

  <div class="flex justify-end pt-1">
    <button type="submit" class="tx-btn-primary" disabled={saving || !dirty}>
      {saving ? "保存中..." : "保存网关配置"}
    </button>
  </div>
</form>

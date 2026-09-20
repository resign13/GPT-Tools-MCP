<script lang="ts">
  import ExecutionPolicyStatus from "./ExecutionPolicyStatus.svelte";
  export interface ActionsPolicyDraft {
    allowedCommands: string;
    maxPatchBytes: number;
    permissionMode: string;
    isolationPolicy: "strict" | "compatibility" | "host";
  }

  interface Props {
    workspaceId?: string;
    allowedCommands: string;
    maxPatchBytes: number;
    permissionMode: string;
    isolationPolicy: "strict" | "compatibility" | "host";
    onSave: (draft: ActionsPolicyDraft) => void | Promise<void>;
  }

  const PERMISSION_MODE_OPTIONS = [
    { value: "default", label: "Default" },
    { value: "full_access", label: "Full Access" },
  ] as const;

  function canonicalPermissionMode(value: string): "default" | "full_access" {
    switch (value.trim().toLowerCase().replaceAll("-", "_")) {
      case "full_access":
      case "fullaccess":
      case "dangerous":
      case "admin":
        return "full_access";
      default:
        return "default";
    }
  }

  let { workspaceId, allowedCommands, maxPatchBytes, permissionMode, isolationPolicy, onSave }: Props = $props();

  let draftCommands = $state("");
  let draftMaxPatch = $state(200_000);
  let draftMode = $state<"default" | "full_access">("default");
  let draftIsolation = $state<"strict" | "compatibility" | "host">("strict");
  let saving = $state(false);

  const dirty = $derived(
      draftCommands !== allowedCommands ||
      draftMaxPatch !== maxPatchBytes ||
      draftMode !== canonicalPermissionMode(permissionMode) ||
      draftIsolation !== isolationPolicy,
  );

  $effect(() => {
    draftCommands = allowedCommands;
    draftMaxPatch = maxPatchBytes;
    draftMode = canonicalPermissionMode(permissionMode);
    draftIsolation = isolationPolicy;
  });

  async function save() {
    if (saving || !dirty || (draftIsolation === "host" && draftMode !== "full_access")) return;
    saving = true;
    try {
      await onSave({
        allowedCommands: draftCommands.trim(),
        maxPatchBytes: draftMaxPatch,
        permissionMode: draftMode,
        isolationPolicy: draftIsolation,
      });
    } finally {
      saving = false;
    }
  }
</script>

{#if workspaceId}<ExecutionPolicyStatus {workspaceId} service="actions" revision={JSON.stringify([permissionMode, isolationPolicy, allowedCommands])} />{/if}
<form
  class="grid gap-3"
  onsubmit={(event) => {
    event.preventDefault();
    void save();
  }}
>
  <label class="grid gap-1">
    <span class="text-xs text-[var(--color-text-muted)]">允许命令（逗号分隔）</span>
    <input
      type="text"
      class="rounded-md border border-[var(--color-border)] bg-[var(--color-bg)] px-2.5 py-1.5 font-mono text-sm"
      placeholder="pytest,python,cargo,npm,..."
      bind:value={draftCommands}
    />
  </label>
  <label class="grid gap-1">
    <span class="text-xs text-[var(--color-text-muted)]">最大 Patch 字节数</span>
    <input
      type="number"
      min="1024"
      max="5000000"
      class="rounded-md border border-[var(--color-border)] bg-[var(--color-bg)] px-2.5 py-1.5 text-sm"
      bind:value={draftMaxPatch}
    />
  </label>
  <label class="grid gap-1">
    <span class="text-xs text-[var(--color-text-muted)]">权限模式</span>
    <select
      class="rounded-md border border-[var(--color-border)] bg-[var(--color-bg)] px-2.5 py-1.5 text-sm"
      bind:value={draftMode}
    >
      {#each PERMISSION_MODE_OPTIONS as option}
        <option value={option.value}>{option.label}</option>
      {/each}
    </select>
  </label>
  <label class="grid gap-1">
    <span class="text-xs text-[var(--color-text-muted)]">执行隔离</span>
    <select
      class="rounded-md border border-[var(--color-border)] bg-[var(--color-bg)] px-2.5 py-1.5 text-sm"
      bind:value={draftIsolation}
    >
      <option value="strict">严格隔离</option>
      <option value="host" disabled={draftMode !== "full_access"}>宿主完全访问</option>
      <option value="compatibility" disabled={draftIsolation !== "compatibility"}>兼容写限制（实验）</option>
    </select>
  </label>
  <p class="text-xs text-[var(--color-text-muted)]">
    审批方式：自动通过（Default / Full Access）。执行隔离保持当前配置。保存后需要重启 Actions。
  </p>
  {#if draftIsolation === "host"}
    <p class="text-xs text-[var(--color-text-muted)]">宿主完全访问：使用当前 Windows 用户执行，允许访问工作区外文件，不启用 OS 沙箱。宿主和目标任务均需开启；保存后重启服务并重新绑定。仅 Full Access 可保存。</p>
  {/if}
  <div class="flex justify-end pt-1">
    <button
      type="submit"
      class="rounded-md bg-[var(--color-accent)] px-3 py-1.5 text-sm font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
      disabled={saving || !dirty || (draftIsolation === "host" && draftMode !== "full_access")}
    >
      {saving ? "保存中…" : "保存策略"}
    </button>
  </div>
</form>

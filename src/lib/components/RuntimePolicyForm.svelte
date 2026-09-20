<script lang="ts">
  import ExecutionPolicyStatus from "./ExecutionPolicyStatus.svelte";
  export interface RuntimePolicyDraft {
    toolProfile: string;
    permissionMode: string;
    isolationPolicy: "strict" | "compatibility" | "host";
    allowedCommands: string;
    workspaceLocalEntries: boolean;
    workspaceScriptExtensions: string;
  }

  interface Props {
    workspaceId?: string;
    toolProfile: string;
    permissionMode: string;
    isolationPolicy: "strict" | "compatibility" | "host";
    allowedCommands: string;
    workspaceLocalEntries: boolean;
    workspaceScriptExtensions: string;
    onSave: (draft: RuntimePolicyDraft) => void | Promise<void>;
  }

  const TOOL_PROFILE_OPTIONS = [
    { value: "full", label: "完整工具" },
    { value: "read-only", label: "只读工具" },
    { value: "compat-readonly-all", label: "兼容只读" },
  ] as const;

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

  let { workspaceId, toolProfile, permissionMode, isolationPolicy, allowedCommands, workspaceLocalEntries, workspaceScriptExtensions, onSave }: Props = $props();

  let draftProfile = $state("full");
  let draftMode = $state<"default" | "full_access">("default");
  let draftIsolation = $state<"strict" | "compatibility" | "host">("strict");
  let draftCommands = $state("");
  let draftLocalEntries = $state(true);
  let draftExtensions = $state(".exe,.bat,.cmd,.ps1");
  let saving = $state(false);

  const dirty = $derived(
    draftProfile !== toolProfile || draftMode !== canonicalPermissionMode(permissionMode) || draftIsolation !== isolationPolicy || draftCommands !== allowedCommands || draftLocalEntries !== workspaceLocalEntries || draftExtensions !== workspaceScriptExtensions,
  );

  $effect(() => {
    draftProfile = toolProfile;
    draftMode = canonicalPermissionMode(permissionMode);
    draftIsolation = isolationPolicy;
    draftCommands = allowedCommands;
    draftLocalEntries = workspaceLocalEntries;
    draftExtensions = workspaceScriptExtensions;
  });

  async function save() {
    if (saving || !dirty || (draftIsolation === "host" && draftMode !== "full_access")) return;
    saving = true;
    try {
      await onSave({ toolProfile: draftProfile, permissionMode: draftMode, isolationPolicy: draftIsolation, allowedCommands: draftCommands.trim(), workspaceLocalEntries: draftLocalEntries, workspaceScriptExtensions: draftExtensions.trim() });
    } finally {
      saving = false;
    }
  }
</script>

{#if workspaceId}<ExecutionPolicyStatus {workspaceId} service="mcp" revision={JSON.stringify([permissionMode, isolationPolicy, allowedCommands])} />{/if}
<form
  class="grid gap-3"
  onsubmit={(event) => {
    event.preventDefault();
    void save();
  }}
>
  <label class="grid gap-1">
    <span class="text-xs text-[var(--color-text-muted)]">工具档位</span>
    <select
      class="rounded-md border border-[var(--color-border)] bg-[var(--color-bg)] px-2.5 py-1.5 text-sm"
      bind:value={draftProfile}
    >
      {#each TOOL_PROFILE_OPTIONS as option}
        <option value={option.value}>{option.label}</option>
      {/each}
    </select>
  </label>
  <label class="grid gap-1">
    <span class="text-xs text-[var(--color-text-muted)]">系统命令（逗号分隔）</span>
    <input type="text" class="rounded-md border border-[var(--color-border)] bg-[var(--color-bg)] px-2.5 py-1.5 font-mono text-sm" placeholder="python,git,curl,powershell,..." bind:value={draftCommands} />
  </label>
  <label class="flex items-center gap-2 text-sm">
    <input type="checkbox" bind:checked={draftLocalEntries} />
    <span>允许执行 Workspace 内本地入口</span>
  </label>
  <label class="grid gap-1">
    <span class="text-xs text-[var(--color-text-muted)]">本地脚本扩展名（逗号分隔）</span>
    <input type="text" class="rounded-md border border-[var(--color-border)] bg-[var(--color-bg)] px-2.5 py-1.5 font-mono text-sm" placeholder=".exe,.bat,.cmd,.ps1" bind:value={draftExtensions} disabled={!draftLocalEntries} />
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
    审批方式：自动通过（Default / Full Access）。执行隔离保持当前配置。保存策略后需要重启 MCP。
  </p>
  {#if draftIsolation === "host"}
    <p class="text-xs text-[var(--color-text-muted)]">宿主完全访问：使用当前 Windows 用户执行，允许访问工作区外文件，不启用 OS 沙箱。宿主和目标任务均需开启；保存后重启服务并重新绑定。仅 Full Access 可保存。</p>
  {/if}
  <div class="flex justify-end pt-1">
    <button
      type="submit"
      class="rounded-md bg-[var(--color-accent)] px-3 py-1.5 text-sm font-medium text-white disabled:opacity-50"
      disabled={saving || !dirty || (draftIsolation === "host" && draftMode !== "full_access")}
    >
      {saving ? "保存中…" : "保存策略"}
    </button>
  </div>
</form>

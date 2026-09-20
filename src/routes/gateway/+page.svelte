<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { ArrowRight, FolderKanban, Play, Square, Settings2, Radio, Plus, Trash2 } from "@lucide/svelte";
  import RuntimePolicyForm, { type RuntimePolicyDraft } from "$lib/components/RuntimePolicyForm.svelte";
  import CopyButton from "$lib/components/CopyButton.svelte";
  import PendingApprovals from "$lib/components/PendingApprovals.svelte";
  import StatusOrb from "$lib/components/StatusOrb.svelte";
  import { updateWorkspace, deleteWorkspace, getRuntimeStatus, listWorkspaces, startRuntime, stopRuntime } from "$lib/api/workspaces";
  import { workspaces } from "$lib/stores/app";
  import { showToast } from "$lib/stores/toast";
  import { mcpLocalEndpoint, type RuntimeState, type RuntimeStatus, type WorkspaceProfile } from "$lib/types";

  let profiles = $state<WorkspaceProfile[]>([]);
  let runtime = $state<RuntimeStatus | null>(null);
  let busy = $state(false);
  let deletingId = $state("");

  const selectedId = $derived($page.url.searchParams.get("workspace") ?? profiles[0]?.id ?? "");
  const selected = $derived(profiles.find((profile) => profile.id === selectedId) ?? profiles[0] ?? null);
  const host = $derived(profiles.find((profile) => profile.gateway?.enabled) ?? null);
  const gatewayRunning = $derived(runtime?.state === "running");

  function stateLabel(state: RuntimeState | undefined): string {
    switch (state) {
      case "running": return "运行中";
      case "starting": return "启动中";
      case "stopping": return "停止中";
      case "error": return "错误";
      default: return "已停止";
    }
  }

  async function load() {
    try {
      profiles = await listWorkspaces();
      workspaces.set(profiles);
      const gatewayHost = profiles.find((profile) => profile.gateway?.enabled);
      runtime = gatewayHost ? await getRuntimeStatus(gatewayHost.id) : null;
    } catch (error) {
      showToast(String(error), { title: "加载网关任务失败", kind: "error", duration: 7000 });
    }
  }

  async function toggleGateway() {
    if (!host || busy) return;
    busy = true;
    try {
      runtime = gatewayRunning ? await stopRuntime(host.id) : await startRuntime(host.id);
    } catch (error) {
      showToast(String(error), { title: "MCP 网关操作失败", kind: "error", duration: 8000 });
    } finally {
      busy = false;
    }
  }

  function selectTask(id: string) {
    goto(`/gateway?workspace=${encodeURIComponent(id)}`);
  }

  function openGatewayConfig() {
    if (host) goto(`/workspace/${encodeURIComponent(host.id)}`);
  }

  async function saveTaskPolicy(draft: RuntimePolicyDraft) {
    if (!selected) return;
    await updateWorkspace({ ...selected, runtime: { ...selected.runtime,
      tool_profile: draft.toolProfile, permission_mode: draft.permissionMode, permission_policy_version: 1,
      isolation_policy: draft.isolationPolicy, allowed_commands: draft.allowedCommands,
      workspace_local_entries: draft.workspaceLocalEntries, workspace_script_extensions: draft.workspaceScriptExtensions,
    }});
    await load();
    showToast("策略已保存；请重启宿主 MCP 并重新绑定任务。", { kind: "success" });
  }

  async function deleteTask(task: WorkspaceProfile) {
    if (task.id === host?.id || deletingId) return;
    const wasSelected = selectedId === task.id;
    deletingId = task.id;
    try {
      const accepted = await confirm(
        `确定删除工作区任务“${task.name}”吗？\n\n只会移除 Coding Tools MCP 中保存的任务配置，不会删除本地目录：\n${task.path}`,
        { title: "删除工作区任务", kind: "warning" },
      );
      if (!accepted) return;

      await deleteWorkspace(task.id);
      await load();

      if (wasSelected) {
        const fallback = profiles.find((profile) => profile.gateway?.enabled) ?? profiles[0] ?? null;
        await goto(fallback ? `/gateway?workspace=${encodeURIComponent(fallback.id)}` : "/gateway");
      }
      showToast(`已删除工作区任务“${task.name}”。本地目录未删除。`, {
        title: "删除成功",
        kind: "success",
        duration: 4000,
      });
    } catch (error) {
      showToast(String(error), { title: "删除工作区任务失败", kind: "error", duration: 8000 });
    } finally {
      deletingId = "";
    }
  }

  $effect(() => {
    void load();
  });
</script>

<svelte:head>
  <title>MCP 网关 · 工作区任务</title>
</svelte:head>

<section class="page-scroll">
  <header class="page-header">
    <div class="flex flex-wrap items-start justify-between gap-4">
      <div>
        <p class="page-kicker">单连接器</p>
        <h2 class="page-title">MCP 网关</h2>
        <p class="mt-2 max-w-2xl text-sm leading-6 text-[var(--color-text-secondary)]">
          一个公网 MCP 管理多个工作区任务。新建任务会自动加入可绑定列表，不会启动新的 MCP 服务。
        </p>
      </div>
      <div class="flex items-center gap-2 rounded-[10px] border border-[var(--color-border)] px-3 py-2 text-sm">
        <Radio size={15} class={gatewayRunning ? "text-[var(--success)]" : "text-[var(--color-text-muted)]"} aria-hidden="true" />
        <span>{stateLabel(runtime?.state)}</span>
      </div>
    </div>

    <div class="mt-5 flex flex-wrap items-center gap-3">
      <button
        type="button"
        class="tx-btn-primary gap-2"
        disabled={!host || busy}
        title={host ? "启动或停止宿主 MCP 网关" : "先在工作区详情中启用网关宿主"}
        onclick={() => void toggleGateway()}
      >
        {#if gatewayRunning}<Square size={14} aria-hidden="true" />{:else}<Play size={14} aria-hidden="true" />{/if}
        <span>{gatewayRunning ? "停止 MCP 网关" : "启动 MCP 网关"}</span>
      </button>
      <button type="button" class="tx-btn-ghost gap-2" onclick={() => goto("/gateway/new")}>
        <Plus size={14} aria-hidden="true" />
        <span>新建工作区任务</span>
      </button>
      {#if host}
        <span class="text-xs text-[var(--color-text-muted)]">宿主：{host.name}</span>
      {:else}
        <span class="text-xs text-[var(--danger)]">尚未配置网关宿主</span>
      {/if}
    </div>
  </header>

  <div class="page-body pb-0">
    <PendingApprovals />
  </div>

  <div class="page-body">
    <div class="min-w-0">
      {#if selected}
        <section class="mb-5 border-b border-[var(--color-border)] pb-5">
          <div class="flex flex-wrap items-start justify-between gap-3">
            <div class="min-w-0">
              <p class="tx-section-label">当前任务</p>
              <h3 class="mt-1 truncate text-lg font-semibold">{selected.name}</h3>
              <p class="mt-1 break-all text-xs text-[var(--color-text-muted)]">{selected.path}</p>
              <div class="mt-2 flex flex-wrap items-center gap-2">
                <span class="text-xs text-[var(--color-text-muted)]">任务 ID</span>
                <code class="max-w-full break-all rounded bg-[var(--color-surface-hover)] px-2 py-1 text-[11px] text-[var(--color-text-secondary)]">{selected.id}</code>
                <CopyButton value={selected.id} label="复制 ID" />
              </div>
            </div>
            {#if selected.id === host?.id}
              <button type="button" class="tx-btn-ghost gap-2" onclick={openGatewayConfig}>
                <Settings2 size={14} aria-hidden="true" />工作区配置
              </button>
            {:else}
              <span class="tx-badge">网关任务</span>
            {/if}
          </div>
          <div class="mt-4 grid gap-3 sm:grid-cols-2">
            <div class="tx-card p-4">
              <div class="flex items-center gap-2 text-sm font-medium"><StatusOrb state={selected.id === host?.id ? runtime?.state ?? "stopped" : "stopped"} />任务环境</div>
              <p class="mt-2 text-xs leading-5 text-[var(--color-text-muted)]">该目录复用宿主 MCP，不需要单独启动服务。</p>
            </div>
            <div class="tx-card p-4">
              <p class="text-sm font-medium">会话绑定</p>
              <p class="mt-2 text-xs leading-5 text-[var(--color-text-muted)]">新对话首句写出任务名称、ID 或路径，网关会调用 bind_workspace 并锁定本对话。</p>
            </div>
          </div>
        </section>
      {/if}

      {#if selected}
        <details class="tx-card mb-4 p-4">
          <summary class="cursor-pointer text-sm font-medium">当前任务权限与执行隔离</summary>
          <div class="mt-4">
            <RuntimePolicyForm workspaceId={selected.id} toolProfile={selected.runtime.tool_profile}
              permissionMode={selected.runtime.permission_mode} isolationPolicy={selected.runtime.isolation_policy ?? "strict"}
              allowedCommands={selected.runtime.allowed_commands ?? ""} workspaceLocalEntries={selected.runtime.workspace_local_entries ?? true}
              workspaceScriptExtensions={selected.runtime.workspace_script_extensions ?? ".exe,.bat,.cmd,.ps1"} onSave={saveTaskPolicy} />
          </div>
        </details>
      {/if}

      <div class="mb-3 flex items-center justify-between">
        <div>
          <p class="tx-section-label">工作区任务</p>
          <p class="mt-1 text-xs text-[var(--color-text-muted)]">每个任务只提供目录、策略和历史环境，加入后即可绑定网页端对话。</p>
        </div>
        <span class="text-xs text-[var(--color-text-muted)]">{profiles.length} 个</span>
      </div>

      {#if profiles.length === 0}
        <div class="tx-card flex min-h-40 flex-col items-center justify-center gap-3 p-6 text-center">
          <FolderKanban size={24} class="text-[var(--color-text-muted)]" aria-hidden="true" />
          <p class="text-sm text-[var(--color-text-secondary)]">还没有工作区任务</p>
          <button type="button" class="tx-btn-primary gap-2" onclick={() => goto("/gateway/new")}>
            <Plus size={14} aria-hidden="true" />新建任务
          </button>
        </div>
      {:else}
        <div class="grid gap-2">
          {#each profiles as task (task.id)}
            <div
              class="group relative tx-card flex items-center p-2 transition-colors hover:border-[var(--primary)] {task.id === selected?.id ? 'border-[var(--primary)] bg-[var(--primary-soft)]' : ''}"
            >
              <button
                type="button"
                class="flex min-w-0 flex-1 items-center gap-3 rounded-[8px] p-2 text-left"
                onclick={() => selectTask(task.id)}
              >
                <FolderKanban size={18} class="shrink-0 text-[var(--primary)]" aria-hidden="true" />
                <span class="min-w-0 flex-1">
                  <span class="block truncate text-sm font-semibold">{task.name}</span>
                  <span class="mt-1 block truncate text-xs text-[var(--color-text-muted)]">{task.path}</span>
                </span>
                {#if task.id === host?.id}<span class="tx-badge">宿主</span>{/if}
                <ArrowRight
                  size={15}
                  class="shrink-0 text-[var(--color-text-muted)] transition-opacity {task.id !== host?.id ? 'group-hover:opacity-0 group-focus-within:opacity-0' : ''}"
                  aria-hidden="true"
                />
              </button>
              {#if task.id !== host?.id}
                <button
                  type="button"
                  class="pointer-events-none absolute right-3 top-1/2 inline-flex h-8 w-8 -translate-y-1/2 items-center justify-center rounded-lg text-[var(--color-text-muted)] opacity-0 transition-[opacity,background-color,color] group-hover:pointer-events-auto group-hover:opacity-100 group-focus-within:pointer-events-auto group-focus-within:opacity-100 hover:bg-[rgba(239,68,68,0.08)] hover:text-[var(--danger)] focus-visible:bg-[rgba(239,68,68,0.08)] focus-visible:text-[var(--danger)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--primary)] disabled:pointer-events-none"
                  disabled={Boolean(deletingId)}
                  title={`删除工作区任务 ${task.name}`}
                  aria-label={`删除工作区任务 ${task.name}`}
                  onclick={() => void deleteTask(task)}
                >
                  <Trash2 size={15} aria-hidden="true" />
                </button>
              {/if}
            </div>
          {/each}
        </div>
      {/if}

    </div>

  </div>
</section>

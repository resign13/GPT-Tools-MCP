<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { ArrowRight, FolderKanban, Play, Square, Settings2, Radio, Plus } from "@lucide/svelte";
  import CopyButton from "$lib/components/CopyButton.svelte";
  import StatusOrb from "$lib/components/StatusOrb.svelte";
  import { getRuntimeStatus, listWorkspaces, startRuntime, stopRuntime } from "$lib/api/workspaces";
  import { workspaces } from "$lib/stores/app";
  import { showToast } from "$lib/stores/toast";
  import { mcpLocalEndpoint, type RuntimeState, type RuntimeStatus, type WorkspaceProfile } from "$lib/types";

  let profiles = $state<WorkspaceProfile[]>([]);
  let runtime = $state<RuntimeStatus | null>(null);
  let busy = $state(false);

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
            <button
              type="button"
              class="tx-card flex items-center gap-3 p-4 text-left transition-colors hover:border-[var(--primary)] {task.id === selected?.id ? 'border-[var(--primary)] bg-[var(--primary-soft)]' : ''}"
              onclick={() => selectTask(task.id)}
            >
              <FolderKanban size={18} class="shrink-0 text-[var(--primary)]" aria-hidden="true" />
              <span class="min-w-0 flex-1">
                <span class="block truncate text-sm font-semibold">{task.name}</span>
                <span class="mt-1 block truncate text-xs text-[var(--color-text-muted)]">{task.path}</span>
              </span>
              {#if task.id === host?.id}<span class="tx-badge">宿主</span>{/if}
              <ArrowRight size={15} class="shrink-0 text-[var(--color-text-muted)]" aria-hidden="true" />
            </button>
          {/each}
        </div>
      {/if}

    </div>

  </div>
</section>

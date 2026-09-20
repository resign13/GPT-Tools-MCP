<script lang="ts">
  import { goto } from "$app/navigation";
  import { open } from "@tauri-apps/plugin-dialog";
  import { FolderOpen, ArrowLeft, Plus } from "@lucide/svelte";
  import { createWorkspace, listWorkspaces, updateWorkspace } from "$lib/api/workspaces";
  import { showToast } from "$lib/stores/toast";

  let busy = $state(false);

  async function createTask() {
    if (busy) return;
    busy = true;
    try {
      const selected = await open({ directory: true, multiple: false });
      if (!selected || Array.isArray(selected)) return;
      const profile = await createWorkspace(selected);
      const items = await listWorkspaces();
      const host = items.find((item) => item.gateway?.enabled);
      if (host && host.id !== profile.id) {
        const gateway = host.gateway ?? { enabled: true, workspace_ids: [] };
        const workspaceIds = Array.from(new Set([host.id, ...items.map((item) => item.id)]));
        if (workspaceIds.join(",") !== gateway.workspace_ids.join(",")) {
          await updateWorkspace({
            ...host,
            gateway: { ...gateway, workspace_ids: workspaceIds },
          });
        }
      } else if (!host) {
        await updateWorkspace({
          ...profile,
          gateway: {
            enabled: true,
            workspace_ids: items.map((item) => item.id),
            prompt: "",
          },
        });
      }
      goto(`/gateway?workspace=${encodeURIComponent(profile.id)}`);
    } catch (error) {
      showToast(String(error), { title: "创建工作区任务失败", kind: "error", duration: 8000 });
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head><title>新建工作区任务 · MCP 网关</title></svelte:head>

<section class="page-scroll">
  <header class="page-header">
    <button type="button" class="tx-btn-ghost mb-4 gap-2" onclick={() => goto("/gateway")}>
      <ArrowLeft size={14} aria-hidden="true" />返回网关
    </button>
    <p class="page-kicker">MCP 网关</p>
    <h2 class="page-title">新建工作区任务</h2>
    <p class="mt-2 max-w-xl text-sm leading-6 text-[var(--color-text-secondary)]">
      选择一个本地目录。任务会自动加入网关并默认绑定到任务列表，不会启动第二个 MCP 服务。
    </p>
  </header>

  <div class="page-body">
    <div class="tx-card max-w-xl p-6">
      <div class="flex items-start gap-3">
        <span class="flex size-10 shrink-0 items-center justify-center rounded-[10px] bg-[var(--primary-soft)] text-[var(--primary)]">
          <FolderOpen size={18} aria-hidden="true" />
        </span>
        <div>
          <h3 class="text-base font-semibold">选择项目目录</h3>
          <p class="mt-1 text-sm leading-6 text-[var(--color-text-muted)]">目录将作为该任务的默认工作区和历史会话根目录。</p>
        </div>
      </div>
      <button type="button" class="tx-btn-primary mt-6 gap-2" disabled={busy} onclick={() => void createTask()}>
        <Plus size={14} aria-hidden="true" />{busy ? "创建中..." : "选择目录并创建任务"}
      </button>
    </div>
  </div>
</section>

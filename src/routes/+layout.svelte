<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { open } from "@tauri-apps/plugin-dialog";
  import AppShell from "$lib/components/AppShell.svelte";
  import ToastHost from "$lib/components/ToastHost.svelte";
  import { ListTodo, Settings2 } from "@lucide/svelte";
  import {
    createWorkspace,
    getRuntimeStatus,
    listWorkspaces,
    updateWorkspace,
  } from "$lib/api/workspaces";
  import { getLastWorkspaceId } from "$lib/api/settings";
  import { actionsRuntimeStates, mcpRuntimeStates, workspaces } from "$lib/stores/app";
  import { showToast } from "$lib/stores/toast";
  import { startUiMemoryGuard } from "$lib/ui-memory-guard";
  import { startCloseGuard } from "$lib/close-guard";
  import CloseConfirmDialog from "$lib/components/CloseConfirmDialog.svelte";
  import type { RuntimeState, WorkspaceProfile } from "$lib/types";

  let { children } = $props();
  let closeConfirmOpen = $state(false);

  async function refreshWorkspaces() {
    const items = await listWorkspaces();
    workspaces.set(items);

    const mcpStates: Record<string, RuntimeState> = Object.fromEntries(
      items.map((item) => [item.id, "stopped" as RuntimeState]),
    );
    const actionsStates: Record<string, RuntimeState> = Object.fromEntries(
      items.map((item) => [item.id, "stopped" as RuntimeState]),
    );
    const host = items.find((item) => item.gateway?.enabled);
    if (host) {
      try {
        mcpStates[host.id] = (await getRuntimeStatus(host.id)).state;
      } catch {
        mcpStates[host.id] = "stopped";
      }
    }
    mcpRuntimeStates.set(mcpStates);
    actionsRuntimeStates.set(actionsStates);
  }

  async function attachTaskToGateway(profile: WorkspaceProfile) {
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
      return;
    }
    if (!host) {
      await updateWorkspace({
        ...profile,
        gateway: { enabled: true, workspace_ids: items.map((item) => item.id), prompt: "" },
      });
    }
  }

  async function addWorkspace() {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (!selected || Array.isArray(selected)) return;
      const profile = await createWorkspace(selected);
      await attachTaskToGateway(profile);
      await refreshWorkspaces();
      goto(`/gateway?workspace=${encodeURIComponent(profile.id)}`);
    } catch (error) {
      showToast(String(error), {
        title: "添加工作区失败",
        kind: "error",
        duration: 8000,
      });
    }
  }

  function openGatewayConfig() {
    const host = $workspaces.find((item) => item.gateway?.enabled);
    if (host) {
      goto(`/workspace/${encodeURIComponent(host.id)}`);
    } else {
      goto("/gateway");
    }
  }

  function openFrpSettings() {
    goto("/settings/frp");
  }

  function openSoftwareSettings() {
    goto("/settings/software");
  }

  function openGeneralSettings() {
    goto("/settings/general");
  }

  function openKeysSettings() {
    goto("/settings/keys");
  }

  onMount(() => {
    const stopGuard = startUiMemoryGuard();
    const stopClose = startCloseGuard(() => {
      closeConfirmOpen = true;
    });
    void (async () => {
      await refreshWorkspaces();
      const path = $page.url.pathname;
      if (path === "/") {
        const lastId = await getLastWorkspaceId();
        if (lastId && $workspaces.some((item) => item.id === lastId)) {
          goto(`/gateway?workspace=${encodeURIComponent(lastId)}`);
        } else if ($workspaces.length > 0) {
          goto(`/gateway?workspace=${encodeURIComponent($workspaces[0].id)}`);
        } else {
          goto("/gateway");
        }
      }
    })();
    return () => {
      stopGuard();
      stopClose();
    };
  });
</script>

<AppShell onAddWorkspace={addWorkspace} gatewayMode={true}>
  {#snippet settingsNav()}
    <button
      type="button"
      class="tx-settings-link {$page.url.pathname === '/settings/general' ? 'active' : ''}"
      onclick={openGeneralSettings}
    >
      通用
    </button>
    <button
      type="button"
      class="tx-settings-link {$page.url.pathname === '/settings/keys' ? 'active' : ''}"
      onclick={openKeysSettings}
    >
      共享密钥
    </button>
    <button
      type="button"
      class="tx-settings-link {$page.url.pathname === '/settings/frp' ? 'active' : ''}"
      onclick={openFrpSettings}
    >
      FRP 配置
    </button>
    <button
      type="button"
      class="tx-settings-link {$page.url.pathname === '/settings/software' ? 'active' : ''}"
      onclick={openSoftwareSettings}
    >
      软件管理
    </button>
  {/snippet}
  {#snippet sidebar()}
    <div class="space-y-1">
      <div class="tx-nav-item" class:active={$page.url.pathname === "/gateway"}>
        <button
          type="button"
          class="tx-nav-button gap-2"
          class:active={$page.url.pathname === "/gateway"}
          aria-current={$page.url.pathname === "/gateway" ? "page" : undefined}
          title="打开工作区任务"
          onclick={() => goto("/gateway")}
        >
          <ListTodo size={15} strokeWidth={1.8} aria-hidden="true" />
          <span class="min-w-0 flex-1 truncate text-sm font-medium">工作区任务</span>
        </button>
      </div>
      {#if $workspaces.some((item) => item.gateway?.enabled)}
        {@const host = $workspaces.find((item) => item.gateway?.enabled)}
        <div class="tx-nav-item" class:active={$page.url.pathname.startsWith("/workspace/") && $page.params.id === host?.id}>
          <button
            type="button"
            class="tx-nav-button gap-2"
            class:active={$page.url.pathname.startsWith("/workspace/") && $page.params.id === host?.id}
            aria-current={$page.url.pathname.startsWith("/workspace/") && $page.params.id === host?.id ? "page" : undefined}
            title="打开唯一的工作区配置"
            onclick={openGatewayConfig}
          >
            <Settings2 size={15} strokeWidth={1.8} aria-hidden="true" />
            <span class="min-w-0 flex-1 truncate text-sm font-medium">工作区配置</span>
          </button>
        </div>
      {/if}
    </div>
  {/snippet}

  {#snippet children()}
    {@render children()}
  {/snippet}
</AppShell>

<ToastHost />
<CloseConfirmDialog
  open={closeConfirmOpen}
  onCancel={() => {
    closeConfirmOpen = false;
  }}
/>

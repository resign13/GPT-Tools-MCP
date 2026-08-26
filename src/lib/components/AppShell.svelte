<script lang="ts">
  import ThemeToggle from "$lib/components/ThemeToggle.svelte";
  import { APP_VERSION } from "$lib/app-version";
  import { REPO_URL } from "$lib/app-links";
  import { openUrl } from "$lib/api/app-info";
  import { message } from "@tauri-apps/plugin-dialog";
  import { Github } from "@lucide/svelte";
  import type { Snippet } from "svelte";

  interface Props {
    children: Snippet;
    sidebar: Snippet;
    onAddWorkspace?: () => void | Promise<void>;
    settingsNav?: Snippet;
    gatewayMode?: boolean;
  }

  let { children, sidebar, onAddWorkspace, settingsNav, gatewayMode = false }: Props = $props();

  async function openRepo() {
    try {
      await openUrl(REPO_URL);
    } catch (e) {
      await message(String(e), { title: "无法打开仓库", kind: "error" });
    }
  }
</script>

<div class="app-layout">
  <aside class="tx-sidebar">
    <div class="tx-sidebar-header">
      {#if gatewayMode}
        <div class="flex items-start justify-between gap-2">
          <div>
            <p class="tx-brand-kicker">Coding Tools</p>
            <h1 class="tx-brand-title">MCP 网关</h1>
          </div>
          <ThemeToggle />
        </div>
        {#if onAddWorkspace}
          <button type="button" class="tx-btn-primary tx-btn-sidebar" onclick={onAddWorkspace}>
            新建工作区任务
          </button>
        {/if}
      {/if}
      <div class="flex items-start justify-between gap-2" class:hidden={gatewayMode}>
        <div>
          <p class="tx-brand-kicker">Coding Tools</p>
          <h1 class="tx-brand-title">桌面控制台</h1>
        </div>
        <ThemeToggle />
      </div>
      {#if onAddWorkspace}
        <button type="button" class="tx-btn-primary tx-btn-sidebar" class:hidden={gatewayMode} style:display={gatewayMode ? "none" : undefined} onclick={onAddWorkspace}>
          添加工作区
        </button>
      {/if}
    </div>

    <div class="tx-sidebar-body" class:gatewayMode={gatewayMode}>
      {#if onAddWorkspace}
        <p class="tx-sidebar-section-label">工作区</p>
      {/if}
      {#if gatewayMode}
        <p class="tx-sidebar-section-label gateway-task-label">工作区任务</p>
      {/if}
      {@render sidebar()}
    </div>

    {#if settingsNav}
      <div class="tx-sidebar-footer">
        <p class="tx-sidebar-section-label">设置</p>
        {@render settingsNav()}
        <div class="tx-app-meta">
          <p class="tx-app-version">v{APP_VERSION}</p>
          <button type="button" class="tx-repo-link" onclick={() => void openRepo()}>
            <Github size={12} strokeWidth={2} />
            <span>仓库</span>
          </button>
        </div>
      </div>
    {:else}
      <div class="tx-sidebar-footer">
        <div class="tx-app-meta">
          <p class="tx-app-version">v{APP_VERSION}</p>
          <button type="button" class="tx-repo-link" onclick={() => void openRepo()}>
            <Github size={12} strokeWidth={2} />
            <span>仓库</span>
          </button>
        </div>
      </div>
    {/if}
  </aside>

  <main class="tx-main">
    {@render children()}
  </main>
</div>

<svelte:head>
  <title>Coding Tools MCP</title>
</svelte:head>

<style>
  .tx-sidebar-body.gatewayMode > .tx-sidebar-section-label:not(.gateway-task-label) {
    display: none;
  }
</style>

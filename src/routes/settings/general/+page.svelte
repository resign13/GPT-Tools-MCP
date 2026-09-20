<script lang="ts">
  import { onMount } from "svelte";
  import { ask, message } from "@tauri-apps/plugin-dialog";
  import { ExternalLink, RefreshCw } from "@lucide/svelte";
  import { getProxy, setProxy, type ProxyConfigDto } from "$lib/api/settings";
  import { checkAppUpdate, openUrl } from "$lib/api/app-info";
  import { APP_VERSION } from "$lib/app-version";
  import { RELEASES_LATEST_URL, REPO_URL } from "$lib/app-links";
  import { getWebviewMemorySample } from "$lib/api/ui-memory";
  import { showToast } from "$lib/stores/toast";

  let proxy = $state<ProxyConfigDto>({ mode: "none", url: "" });
  let changed = $state(false);
  let saving = $state(false);
  let checkingUpdate = $state(false);
  let memoryHint = $state<string | null>(null);

  async function refresh() {
    try {
      proxy = await getProxy();
      changed = false;
    } catch (e) {
      await message(String(e), { title: "加载失败", kind: "error" });
    }
  }

  async function save() {
    saving = true;
    try {
      await setProxy(proxy);
      changed = false;
      await message("代理设置已保存。", { title: "已保存", kind: "info" });
    } catch (e) {
      await message(String(e), { title: "保存失败", kind: "error" });
    } finally {
      saving = false;
    }
  }

  function handleChange() {
    changed = true;
  }

  async function openLink(url: string, title: string) {
    try {
      await openUrl(url);
    } catch (e) {
      await message(String(e), { title, kind: "error" });
    }
  }

  async function handleCheckUpdate() {
    if (checkingUpdate) return;
    checkingUpdate = true;
    try {
      const result = await checkAppUpdate();
      if (result.updateAvailable) {
        const openRelease = await ask(
          `发现新版本 ${result.latestTag}（当前 v${result.currentVersion}）。是否打开 Releases 页面下载？`,
          { title: "有可用更新", kind: "info", okLabel: "打开下载页", cancelLabel: "稍后" },
        );
        if (openRelease) {
          await openUrl(result.releaseUrl || RELEASES_LATEST_URL);
        }
      } else {
        await message(`当前已是最新版本（v${result.currentVersion}）。`, {
          title: "检查更新",
          kind: "info",
        });
      }
    } catch (e) {
      await message(String(e), { title: "检查更新失败", kind: "error" });
    } finally {
      checkingUpdate = false;
    }
  }

  async function refreshMemoryHint() {
    try {
      const sample = await getWebviewMemorySample();
      if (!sample.supported) {
        memoryHint = "当前平台暂不支持界面内存采样。";
        return;
      }
      memoryHint = `界面约 ${Math.round(sample.webviewMb)} MB（${sample.webviewProcessCount} 个 WebView 进程），主进程约 ${Math.round(sample.mainMb)} MB。`;
    } catch {
      memoryHint = null;
    }
  }

  onMount(() => {
    void refresh();
    void refreshMemoryHint();
  });
</script>

<section class="page-scroll">
  <header class="page-header">
    <p class="page-kicker">全局设置</p>
    <h2 class="page-title">通用</h2>
    <p class="mt-2 max-w-2xl text-sm text-[var(--color-text-muted)]">
      配置全局网络代理，并查看应用版本与官方仓库入口。
    </p>
  </header>

  <div class="page-body flex flex-col gap-6">
    <div class="tx-card p-4">
      <h3 class="text-sm font-semibold">关于</h3>
      <p class="mt-1 text-xs text-[var(--color-text-muted)]">
        当前版本 v{APP_VERSION}。仓库与新版本安装包都在 GitHub Releases。
      </p>
      <div class="mt-4 flex flex-wrap gap-2">
        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-md border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-1.5 text-sm"
          onclick={() => void openLink(REPO_URL, "无法打开仓库")}
        >
          <ExternalLink size={14} strokeWidth={2} />
          打开仓库
        </button>
        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-md border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-1.5 text-sm"
          onclick={() => void openLink(RELEASES_LATEST_URL, "无法打开 Releases")}
        >
          <ExternalLink size={14} strokeWidth={2} />
          打开 Releases
        </button>
        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-md bg-[var(--color-accent)] px-3 py-1.5 text-sm font-medium text-white disabled:opacity-50"
          disabled={checkingUpdate}
          onclick={() => void handleCheckUpdate()}
        >
          <RefreshCw size={14} strokeWidth={2} class={checkingUpdate ? "animate-spin" : ""} />
          {checkingUpdate ? "检查中…" : "检查更新"}
        </button>
      </div>
    </div>

    <div class="tx-card p-4">
      <h3 class="text-sm font-semibold">界面内存</h3>
      <p class="mt-1 text-xs text-[var(--color-text-muted)]">
        长时间运行后可查看 WebView 内存占用。自动和手动界面进程重建目前已暂停，以避免窗口丢失。
      </p>
      {#if memoryHint}
        <p class="mt-2 text-xs text-[var(--color-text-muted)]">{memoryHint}</p>
      {/if}
      <div class="mt-4 flex flex-wrap gap-2">
        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-md border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-1.5 text-sm"
          onclick={() => void refreshMemoryHint()}
        >
          刷新占用
        </button>
      </div>
    </div>

    <div class="tx-card p-4">
      <h3 class="text-sm font-semibold">网络代理</h3>
      <form
        class="mt-4 grid gap-3"
        onsubmit={(e) => { e.preventDefault(); void save(); }}
      >
        <label class="grid gap-1">
          <span class="text-xs text-[var(--color-text-muted)]">代理模式</span>
          <select
            class="rounded-md border border-[var(--color-border)] bg-[var(--color-bg)] px-2.5 py-1.5 text-sm"
            bind:value={proxy.mode}
            onchange={handleChange}
          >
            <option value="none">无代理</option>
            <option value="system">系统代理</option>
            <option value="manual">手动代理地址</option>
          </select>
        </label>

        {#if proxy.mode === "manual"}
          <label class="grid gap-1">
            <span class="text-xs text-[var(--color-text-muted)]">代理地址</span>
            <input
              type="text"
              class="tx-input tx-mono"
              placeholder="http://127.0.0.1:7890"
              bind:value={proxy.url}
              oninput={handleChange}
            />
            <span class="text-xs text-[var(--color-text-muted)]">
              支持 HTTP/HTTPS/SOCKS 代理，如 http://127.0.0.1:7890
            </span>
          </label>
        {/if}

        <div class="flex justify-end pt-1">
          <button
            type="submit"
            class="rounded-md bg-[var(--color-accent)] px-3 py-1.5 text-sm font-medium text-white disabled:opacity-50"
            disabled={!changed || saving}
          >
            {saving ? "保存中…" : "保存设置"}
          </button>
        </div>
      </form>
    </div>
  </div>
</section>

<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  let { workspaceId, service = "mcp", revision = "" }: { workspaceId: string; service?: string; revision?: string } = $props();
  let status = $state<{ running_context: boolean; restart_required?: boolean; effective?: { permission_mode: string; isolation_backend: string; sandbox_enforced: boolean } } | null>(null);
  let error = $state("");
  $effect(() => {
    const id = workspaceId, kind = service, config = revision;
    let active = true;
    async function refresh() {
      try {
        const result = await invoke<typeof status>("get_execution_policy_status", { id, service: kind });
        if (active) { status = result; error = ""; }
      } catch (e) { if (active) error = String(e); }
    }
    void config; void refresh();
    const timer = setInterval(() => void refresh(), 5000);
    return () => { active = false; clearInterval(timer); };
  });
</script>
<div class="text-xs leading-5 text-[var(--color-text-muted)]" aria-live="polite">
  {#if error}<p>运行状态读取失败：{error}</p>
  {:else if status?.running_context && status.effective}
    <p>最近活动上下文：{status.effective.permission_mode} · {status.effective.isolation_backend} · OS 沙箱{status.effective.sandbox_enforced ? "已实施" : "未实施"}</p>
    {#if status.restart_required}<p>配置已保存，待重启宿主服务并重新绑定任务。</p>{/if}
  {:else}<p>暂无活动上下文；启动服务并绑定任务后显示有效后端。</p>{/if}
</div>

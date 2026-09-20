<script lang="ts">
  import { onMount } from "svelte";
  import { Check, RefreshCw, ShieldAlert, X } from "@lucide/svelte";
  import { decideApproval, listPendingApprovals } from "$lib/api/approvals";
  import { showToast } from "$lib/stores/toast";
  import type { ApprovalStatus } from "$lib/types";

  let approvals = $state<ApprovalStatus[]>([]);
  let loading = $state(true);
  let refreshing = $state(false);
  let decidingId = $state("");

  async function refresh() {
    if (refreshing) return;
    refreshing = true;
    try {
      approvals = await listPendingApprovals();
    } catch (error) {
      showToast(String(error), {
        title: "加载待审批操作失败",
        kind: "error",
        duration: 6000,
      });
    } finally {
      loading = false;
      refreshing = false;
    }
  }

  async function decide(approvalId: string, approve: boolean) {
    if (decidingId) return;
    decidingId = approvalId;
    try {
      const status = await decideApproval(approvalId, approve);
      approvals = approvals.filter((item) => item.approval_id !== approvalId);
      if (status.decision !== (approve ? "approved" : "denied")) {
        showToast(`审批状态已变化：${status.decision}`, {
          title: "审批未更新",
          kind: "warning",
          duration: 5000,
        });
      }
    } catch (error) {
      showToast(String(error), {
        title: "审批操作失败",
        kind: "error",
        duration: 7000,
      });
      await refresh();
    } finally {
      decidingId = "";
    }
  }

  function formatTtl(seconds: number): string {
    if (seconds < 60) return `${Math.max(0, seconds)} 秒`;
    return `${Math.ceil(seconds / 60)} 分钟`;
  }

  onMount(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 5000);
    return () => window.clearInterval(timer);
  });
</script>

{#if loading || approvals.length > 0}
  <section class="tx-card mb-5 p-4" aria-labelledby="pending-approvals-title">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div class="flex items-center gap-2">
        <ShieldAlert size={17} class="text-[var(--warning)]" aria-hidden="true" />
        <h3 id="pending-approvals-title" class="text-sm font-semibold">待审批操作</h3>
        {#if approvals.length > 0}
          <span class="tx-badge">{approvals.length}</span>
        {/if}
      </div>
      <button
        type="button"
        class="tx-btn-ghost inline-flex items-center gap-1.5"
        disabled={refreshing}
        title="刷新待审批列表"
        onclick={() => void refresh()}
      >
        <RefreshCw size={14} class={refreshing ? "animate-spin" : ""} aria-hidden="true" />
        <span>刷新</span>
      </button>
    </div>

    {#if loading}
      <p class="mt-3 text-xs text-[var(--color-text-muted)]">正在加载…</p>
    {:else}
      <div class="mt-3 grid gap-2">
        {#each approvals as approval (approval.approval_id)}
          <article class="rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] p-3">
            <div class="flex flex-wrap items-start justify-between gap-3">
              <div class="min-w-0">
                <p class="font-mono text-xs font-semibold text-[var(--color-text)]">
                  {approval.operation_summary || approval.tool}
                </p>
                <p class="mt-1 break-all text-xs text-[var(--color-text-secondary)]">
                  执行根：{approval.execution_root}
                </p>
                {#if approval.task}
                  <p class="mt-1 text-xs text-[var(--color-text-muted)]">任务：{approval.task}</p>
                {/if}
              </div>
              <div class="flex shrink-0 items-center gap-2">
                <button
                  type="button"
                  class="tx-btn-ghost inline-flex items-center gap-1.5 text-[var(--danger)]"
                  disabled={Boolean(decidingId)}
                  onclick={() => void decide(approval.approval_id, false)}
                >
                  <X size={14} aria-hidden="true" />
                  <span>拒绝</span>
                </button>
                <button
                  type="button"
                  class="tx-btn-primary inline-flex items-center gap-1.5"
                  disabled={Boolean(decidingId)}
                  onclick={() => void decide(approval.approval_id, true)}
                >
                  <Check size={14} aria-hidden="true" />
                  <span>批准</span>
                </button>
              </div>
            </div>
            <div class="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-[11px] text-[var(--color-text-muted)]">
              {#if approval.capabilities.length > 0}
                <span>额外能力：{approval.capabilities.join("、")}</span>
              {/if}
              {#if approval.isolation}
                <span>隔离：{approval.isolation}</span>
              {/if}
              <span>有效期：{formatTtl(approval.remaining_seconds)}</span>
              <span>会话：{approval.context_hash.slice(0, 12)}…</span>
            </div>
          </article>
        {/each}
      </div>
    {/if}
  </section>
{/if}

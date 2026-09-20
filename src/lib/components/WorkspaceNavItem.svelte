<script lang="ts">
  import { FolderKanban, ChevronRight } from "@lucide/svelte";
  import type { WorkspaceProfile } from "$lib/types";

  interface Props {
    workspace: WorkspaceProfile;
    active: boolean;
    onClick: () => void;
  }

  let { workspace, active, onClick }: Props = $props();
</script>

<div class="tx-nav-item" class:active>
  <button
    type="button"
    class="tx-nav-button gap-2"
    class:active
    aria-current={active ? "page" : undefined}
    title={`打开工作区任务：${workspace.name}`}
    onclick={onClick}
  >
    <FolderKanban size={15} strokeWidth={1.8} aria-hidden="true" />
    <span class="min-w-0 flex-1 truncate text-sm font-medium">{workspace.name}</span>
    <ChevronRight size={14} class="shrink-0 opacity-50" aria-hidden="true" />
  </button>
</div>

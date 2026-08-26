<script lang="ts">
  import { goto } from "$app/navigation";
  import { onMount } from "svelte";

  onMount(() => {
    void goto("/gateway");
  });
</script>

<svelte:head><title>MCP 网关</title></svelte:head>

<div class="page-scroll flex items-center justify-center p-8">
  <p class="text-sm text-[var(--color-text-muted)]">正在打开 MCP 网关...</p>
</div>

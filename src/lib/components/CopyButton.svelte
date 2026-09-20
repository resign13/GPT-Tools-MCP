<script lang="ts">
  import { onDestroy } from "svelte";

  interface Props {
    value: string;
    label?: string;
    onCopy?: () => void;
  }

  let { value, label = "复制", onCopy }: Props = $props();
  let copied = $state(false);
  let resetTimer: ReturnType<typeof setTimeout> | undefined;

  onDestroy(() => {
    if (resetTimer !== undefined) clearTimeout(resetTimer);
  });

  async function copy() {
    await navigator.clipboard.writeText(value);
    copied = true;
    onCopy?.();
    if (resetTimer !== undefined) clearTimeout(resetTimer);
    resetTimer = setTimeout(() => {
      copied = false;
      resetTimer = undefined;
    }, 1500);
  }
</script>

<button
  type="button"
  class="tx-btn-ghost shrink-0 px-2.5 py-1 text-xs"
  onclick={copy}
>
  {copied ? "已复制" : label}
</button>

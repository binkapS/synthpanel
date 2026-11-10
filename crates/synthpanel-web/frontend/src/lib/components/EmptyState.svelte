<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    title,
    description = '',
    icon,
    action,
  }: {
    title: string;
    description?: string;
    icon?: Snippet;
    action?: Snippet;
  } = $props();
</script>

<div
  class="flex flex-col items-center justify-center gap-3 rounded-lg border border-dashed border-slate-300 px-6 py-10 text-center dark:border-slate-700"
>
  {#if icon}
    <div class="text-slate-400" aria-hidden="true">{@render icon()}</div>
  {/if}
  <div>
    <p class="text-sm font-semibold">{title}</p>
    {#if description}
      <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">{description}</p>
    {/if}
  </div>
  {#if action}
    <div>{@render action()}</div>
  {/if}
</div>

<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    label,
    value,
    hint = '',
    icon,
  }: {
    label: string;
    value: string;
    hint?: string;
    icon?: Snippet;
  } = $props();
</script>

<div class="card flex items-start gap-3 p-4">
  {#if icon}
    <div
      class="rounded-lg bg-blue-50 p-2 text-blue-600 dark:bg-blue-950/60 dark:text-blue-300"
      aria-hidden="true"
    >
      {@render icon()}
    </div>
  {/if}
  <div class="min-w-0 flex-1">
    <p class="truncate text-xs font-medium tracking-wide text-slate-500 uppercase dark:text-slate-400">
      {label}
    </p>
    <p class="mt-1.5 text-xl font-semibold tabular-nums">{value}</p>
    {#if hint}
      <p class="mt-0.5 truncate text-xs text-slate-500 dark:text-slate-400">{hint}</p>
    {/if}
  </div>
</div>

<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    title = '',
    subtitle = '',
    actions,
    padded = true,
    class: className = '',
    children,
  }: {
    title?: string;
    subtitle?: string;
    actions?: Snippet;
    padded?: boolean;
    class?: string;
    children: Snippet;
  } = $props();
</script>

<section class="card overflow-hidden {className}">
  {#if title || actions}
    <header
      class="flex items-center justify-between gap-4 border-b border-slate-200 px-4 py-3 dark:border-slate-800"
    >
      <div class="min-w-0">
        <h2 class="truncate text-sm font-semibold">{title}</h2>
        {#if subtitle}
          <p class="truncate text-xs text-slate-500 dark:text-slate-400">{subtitle}</p>
        {/if}
      </div>
      {#if actions}
        <div class="flex shrink-0 items-center gap-2">{@render actions()}</div>
      {/if}
    </header>
  {/if}
  <div class={padded ? 'p-4' : ''}>
    {@render children()}
  </div>
</section>

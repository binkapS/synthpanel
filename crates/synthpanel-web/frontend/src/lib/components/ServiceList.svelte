<script lang="ts">
  import StatusBadge from './StatusBadge.svelte';

  interface Service {
    unit: string;
    status: string;
  }

  let { services = [] }: { services?: Service[] } = $props();
</script>

{#if services.length === 0}
  <p class="text-sm text-slate-500 dark:text-slate-400">No service information available.</p>
{:else}
  <ul class="divide-y divide-slate-100 dark:divide-slate-800">
    {#each services as service (service.unit)}
      <li class="flex items-center justify-between gap-3 py-2 first:pt-0 last:pb-0">
        <span class="truncate font-mono text-xs text-slate-600 dark:text-slate-300">{service.unit}</span>
        <StatusBadge status={service.status} />
      </li>
    {/each}
  </ul>
{/if}

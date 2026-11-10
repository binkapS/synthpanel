<script lang="ts">
  import { onMount } from 'svelte';
  import { api, ApiError } from '../lib/api';
  import Card from '../lib/components/Card.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import { Globe } from '@lucide/svelte';
</script>

<div class="space-y-6">
  <div>
    <h1 class="text-2xl font-semibold">Sites</h1>
    <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
      Every domain gets its own Nginx vhost and an isolated PHP-FPM pool.
    </p>
  </div>

  <Card>
    <EmptyState
      title="No sites yet"
      description="Add your first site to get an Nginx vhost, a PHP-FPM pool and automatic HTTPS."
    >
      {#snippet icon()}<Globe size={28} />{/snippet}
      {#snippet action()}
        <span class="btn-primary pointer-events-none opacity-60">Add site</span>
      {/snippet}
    </EmptyState>
  </Card>
</div>

<script lang="ts">
  import { onMount } from 'svelte';
  import { Server, CircleCheck, CircleX } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import Card from '../lib/components/Card.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import StatusBadge from '../lib/components/StatusBadge.svelte';

  interface PhpVersion {
    version: string;
    binary: string | null;
    pool_dir: string;
    pools: string[];
    active: boolean;
  }

  interface PhpSite {
    id: string;
    domain: string;
    version: string;
  }

  interface PhpStatus {
    versions: string[];
    installed: PhpVersion[];
    sites: PhpSite[];
  }

  let status = $state<PhpStatus | null>(null);
  let loading = $state(true);
  let error = $state('');

  async function load() {
    try {
      status = await api<PhpStatus>('/api/v1/php/status');
      error = '';
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to load the PHP status';
    } finally {
      loading = false;
    }
  }

  onMount(load);

  function sitesFor(version: string): PhpSite[] {
    return (status?.sites ?? []).filter((site) => site.version === version);
  }
</script>

<div class="space-y-6">
  <div>
    <h1 class="text-2xl font-semibold">PHP</h1>
    <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
      Every site runs in its own PHP-FPM pool. The panel keeps the supported
      versions, the generated pools and the sites that use them in sync.
    </p>
  </div>

  {#if error}
    <p class="rounded-lg bg-rose-50 px-4 py-3 text-sm text-rose-700 dark:bg-rose-950/60 dark:text-rose-300">{error}</p>
  {/if}

  {#if loading}
    <p class="text-sm text-slate-500 dark:text-slate-400">Loading…</p>
  {:else if status}
    <div class="grid gap-4 sm:grid-cols-2">
      {#each status.installed as version (version.version)}
        <Card
          title="PHP {version.version}"
          subtitle={version.binary ? version.binary : 'not installed on this server'}
        >
          <div class="space-y-3 text-sm">
            <div class="flex items-center gap-2">
              {#if version.active}
                <CircleCheck size={16} class="text-emerald-600 dark:text-emerald-400" />
                <span class="text-emerald-700 dark:text-emerald-300">the service is running</span>
              {:else if version.binary}
                <CircleX size={16} class="text-amber-600 dark:text-amber-400" />
                <span class="text-amber-700 dark:text-amber-300">installed, but not running</span>
              {:else}
                <CircleX size={16} class="text-slate-400" />
                <span class="text-slate-500 dark:text-slate-400">not installed</span>
              {/if}
            </div>

            <div>
              <p class="text-xs font-medium text-slate-500 uppercase dark:text-slate-400">
                Pools ({version.pools.length})
              </p>
              {#if version.pools.length === 0}
                <p class="mt-1 text-xs text-slate-500 dark:text-slate-400">
                  No pools yet — they are created with a site.
                </p>
              {:else}
                <ul class="mt-1 space-y-1 font-mono text-xs text-slate-600 dark:text-slate-300">
                  {#each version.pools as pool (pool)}
                    <li>{pool}</li>
                  {/each}
                </ul>
              {/if}
            </div>

            <div>
              <p class="text-xs font-medium text-slate-500 uppercase dark:text-slate-400">
                Sites
              </p>
              {#if sitesFor(version.version).length === 0}
                <p class="mt-1 text-xs text-slate-500 dark:text-slate-400">
                  No site uses this version.
                </p>
              {:else}
                <ul class="mt-1 space-y-1 text-xs">
                  {#each sitesFor(version.version) as site (site.id)}
                    <li>
                      <a
                        href="/sites/{site.id}"
                        class="text-blue-600 hover:underline dark:text-blue-400">{site.domain}</a
                      >
                    </li>
                  {/each}
                </ul>
              {/if}
            </div>

            <p class="font-mono text-[11px] break-all text-slate-400 dark:text-slate-500">
              {version.pool_dir}
            </p>
          </div>
        </Card>
      {/each}
    </div>

    <Card title="How it works" subtitle="per-site pools">
      <ul class="space-y-2 text-sm text-slate-600 dark:text-slate-300">
        <li>
          Each site gets a pool named after its id, so a slow site can never
          starve the others.
        </li>
        <li>
          The pool user is <span class="font-mono">sp_&lt;site-id&gt;</span> and the
          socket lives in <span class="font-mono">/run/php/</span>.
        </li>
        <li>
          Change the version of a site on its detail page; the pool is
          regenerated and the service is reloaded.
        </li>
      </ul>
    </Card>
  {:else}
    <EmptyState title="No PHP information" description="The core did not report any PHP version.">
      {#snippet icon()}<Server size={28} />{/snippet}
    </EmptyState>
  {/if}
</div>

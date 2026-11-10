<script lang="ts">
  import { onMount } from 'svelte';
  import { Globe, Plus, Trash2 } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import Card from '../lib/components/Card.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import StatusBadge from '../lib/components/StatusBadge.svelte';

  interface Site {
    id: string;
    domain: string;
    aliases: string;
    document_root: string;
    template: string;
    php_version: string | null;
    pool_user: string;
    status: string;
    ssl_mode: string;
    created_at: string;
  }

  let sites = $state<Site[]>([]);
  let error = $state('');
  let status = $state('');
  let loading = $state(true);
  let busy = $state(false);
  let showForm = $state(false);

  let domain = $state('');
  let aliases = $state('');
  let template = $state('php');
  let phpVersion = $state('8.3');
  let sslMode = $state('none');
  let documentRoot = $state('');

  const templates = [
    { value: 'static', label: 'Static HTML' },
    { value: 'php', label: 'PHP' },
    { value: 'laravel', label: 'Laravel' },
    { value: 'symfony', label: 'Symfony' },
    { value: 'wordpress', label: 'WordPress' },
    { value: 'node', label: 'Node / Next.js' },
  ];

  const aliasList = $derived(
    aliases
      .split(',')
      .map((value) => value.trim())
      .filter(Boolean),
  );

  async function load() {
    try {
      const result = await api<{ sites: Site[] }>('/api/v1/sites');
      sites = result.sites ?? [];
      error = '';
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to load sites';
    } finally {
      loading = false;
    }
  }

  onMount(load);

  async function createSite() {
    busy = true;
    error = '';
    status = '';
    try {
      await api('/api/v1/sites', {
        method: 'POST',
        body: JSON.stringify({
          domain,
          aliases: aliasList,
          template,
          php_version: template === 'static' || template === 'node' ? null : phpVersion,
          ssl_mode: sslMode,
          document_root: documentRoot.trim() || undefined,
        }),
      });
      status = `Created ${domain}`;
      domain = '';
      aliases = '';
      documentRoot = '';
      showForm = false;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to create the site';
    } finally {
      busy = false;
    }
  }

  async function removeSite(site: Site) {
    if (!confirm(`Delete ${site.domain}? The vhost and PHP-FPM pool will be removed.`)) {
      return;
    }
    error = '';
    try {
      await api(`/api/v1/sites/${site.id}`, { method: 'DELETE' });
      status = `Deleted ${site.domain}`;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to delete the site';
    }
  }
</script>

<div class="space-y-6">
  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-2xl font-semibold">Sites</h1>
      <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
        Every domain gets its own Nginx vhost and an isolated PHP-FPM pool.
      </p>
    </div>
    <button class="btn-primary" onclick={() => (showForm = !showForm)}>
      <Plus size={15} />
      {showForm ? 'Cancel' : 'Add site'}
    </button>
  </div>

  {#if showForm}
    <Card title="New site">
      <form
        class="space-y-4"
        onsubmit={(event) => {
          event.preventDefault();
          createSite();
        }}
      >
        <div class="grid gap-4 sm:grid-cols-2">
          <div>
            <label class="label" for="domain">Domain</label>
            <input
              id="domain"
              class="input"
              placeholder="example.com"
              bind:value={domain}
              required
            />
          </div>
          <div>
            <label class="label" for="aliases">Aliases</label>
            <input
              id="aliases"
              class="input"
              placeholder="www.example.com, mail.example.com"
              bind:value={aliases}
            />
            <p class="mt-1 text-xs text-slate-500 dark:text-slate-400">Comma separated.</p>
          </div>
        </div>
        <div class="grid gap-4 sm:grid-cols-3">
          <div>
            <label class="label" for="template">Template</label>
            <select id="template" class="input" bind:value={template}>
              {#each templates as option (option.value)}
                <option value={option.value}>{option.label}</option>
              {/each}
            </select>
          </div>
          <div>
            <label class="label" for="php-version">PHP version</label>
            <select id="php-version" class="input" bind:value={phpVersion} disabled={template === 'static' || template === 'node'}>
              <option value="8.1">8.1</option>
              <option value="8.2">8.2</option>
              <option value="8.3">8.3</option>
              <option value="8.4">8.4</option>
            </select>
          </div>
          <div>
            <label class="label" for="ssl-mode">SSL</label>
            <select id="ssl-mode" class="input" bind:value={sslMode}>
              <option value="none">None</option>
              <option value="le">Let's Encrypt (HTTP-01)</option>
              <option value="custom">Custom certificate</option>
            </select>
          </div>
        </div>
        <div>
          <label class="label" for="document-root">Document root (optional)</label>
          <input
            id="document-root"
            class="input"
            placeholder="/var/www/sp_&lt;site-id&gt;"
            bind:value={documentRoot}
          />
        </div>
        <button class="btn-primary" type="submit" disabled={busy || !domain.trim()}>
          {busy ? 'Creating…' : 'Create site'}
        </button>
      </form>
    </Card>
  {/if}

  {#if error}
    <p class="rounded-lg bg-rose-50 px-4 py-3 text-sm text-rose-700 dark:bg-rose-950/60 dark:text-rose-300">
      {error}
    </p>
  {/if}
  {#if status}
    <p class="rounded-lg bg-emerald-50 px-4 py-3 text-sm text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300">
      {status}
    </p>
  {/if}

  <Card title="Your sites" subtitle="{sites.length} site(s)">
    {#if loading}
      <p class="text-sm text-slate-500 dark:text-slate-400">Loading…</p>
    {:else if sites.length === 0}
      <EmptyState
        title="No sites yet"
        description="Add your first site to get an Nginx vhost, a PHP-FPM pool and automatic HTTPS."
      >
        {#snippet icon()}<Globe size={28} />{/snippet}
      </EmptyState>
    {:else}
      <div class="overflow-x-auto">
        <table class="w-full text-left text-sm">
          <thead class="text-xs text-slate-500 uppercase dark:text-slate-400">
            <tr>
              <th class="py-2 pr-4 font-medium">Domain</th>
              <th class="py-2 pr-4 font-medium">Template</th>
              <th class="py-2 pr-4 font-medium">PHP</th>
              <th class="py-2 pr-4 font-medium">SSL</th>
              <th class="py-2 pr-4 font-medium">Status</th>
              <th class="py-2 font-medium"></th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100 dark:divide-slate-800">
            {#each sites as site (site.id)}
              <tr>
                <td class="py-2 pr-4">
                  <div class="font-medium">{site.domain}</div>
                  <div class="font-mono text-xs text-slate-500 dark:text-slate-400">
                    {site.document_root}
                  </div>
                </td>
                <td class="py-2 pr-4 capitalize">{site.template}</td>
                <td class="py-2 pr-4">{site.php_version ?? '—'}</td>
                <td class="py-2 pr-4 uppercase">{site.ssl_mode}</td>
                <td class="py-2 pr-4"><StatusBadge status={site.status} /></td>
                <td class="py-2 text-right">
                  <button
                    class="btn-secondary px-2.5 text-rose-600 dark:text-rose-400"
                    aria-label="Delete {site.domain}"
                    title="Delete site"
                    onclick={() => removeSite(site)}
                  >
                    <Trash2 size={15} />
                  </button>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </Card>
</div>

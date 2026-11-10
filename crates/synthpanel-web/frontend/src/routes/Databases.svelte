<script lang="ts">
  import { onMount } from 'svelte';
  import { Database, Plus, Trash2 } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import Card from '../lib/components/Card.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import StatusBadge from '../lib/components/StatusBadge.svelte';

  interface DatabaseRow {
    id: string;
    engine: string;
    name: string;
    created_at: string;
  }

  interface EngineStatus {
    engine: string;
    client_available: boolean;
    server_reachable: boolean;
  }

  let databases = $state<DatabaseRow[]>([]);
  let engines = $state<EngineStatus[]>([]);
  let error = $state('');
  let status = $state('');
  let credentials = $state('');
  let loading = $state(true);
  let busy = $state(false);
  let showForm = $state(false);

  let engine = $state('mysql');
  let name = $state('');
  let username = $state('');
  let privileges = $state('ALL');

  async function load() {
    try {
      const result = await api<{ databases: DatabaseRow[]; engines: EngineStatus[] }>(
        '/api/v1/databases',
      );
      databases = result.databases ?? [];
      engines = result.engines ?? [];
      error = '';
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to load databases';
    } finally {
      loading = false;
    }
  }

  onMount(load);

  async function createDatabase() {
    busy = true;
    error = '';
    status = '';
    try {
      const result = await api<{
        password: string;
        connection_string: string;
        env: string;
        database: { name: string };
      }>('/api/v1/databases', {
        method: 'POST',
        body: JSON.stringify({
          engine,
          name,
          username: username.trim() || undefined,
          privileges,
        }),
      });
      credentials = result.env;
      status = `Created ${result.database.name}. Password: ${result.password}`;
      name = '';
      username = '';
      showForm = false;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to create the database';
    } finally {
      busy = false;
    }
  }

  async function removeDatabase(database: DatabaseRow) {
    if (!confirm(`Delete ${database.name}? The data on the server is dropped too.`)) {
      return;
    }
    error = '';
    try {
      await api(`/api/v1/databases/${database.id}`, { method: 'DELETE' });
      status = `Deleted ${database.name}`;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to delete the database';
    }
  }
</script>

<div class="space-y-6">
  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-2xl font-semibold">Databases</h1>
      <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
        MariaDB/MySQL, PostgreSQL and Redis, with per-database credentials.
      </p>
    </div>
    <button class="btn-primary" onclick={() => (showForm = !showForm)}>
      <Plus size={15} />
      {showForm ? 'Cancel' : 'Add database'}
    </button>
  </div>

  {#if showForm}
    <Card title="New database">
      <form
        class="space-y-4"
        onsubmit={(event) => {
          event.preventDefault();
          createDatabase();
        }}
      >
        <div class="grid gap-4 sm:grid-cols-3">
          <div>
            <label class="label" for="engine">Engine</label>
            <select id="engine" class="input" bind:value={engine}>
              <option value="mysql">MariaDB / MySQL</option>
              <option value="postgres">PostgreSQL</option>
              <option value="redis">Redis</option>
            </select>
          </div>
          <div>
            <label class="label" for="name">Database name</label>
            <input id="name" class="input" placeholder="shop" bind:value={name} required />
          </div>
          <div>
            <label class="label" for="username">Username</label>
            <input id="username" class="input" placeholder="defaults to the database name" bind:value={username} />
          </div>
        </div>
        <div class="max-w-xs">
          <label class="label" for="privileges">Privileges</label>
          <select id="privileges" class="input" bind:value={privileges}>
            <option value="ALL">All</option>
            <option value="READWRITE">Read &amp; write</option>
            <option value="READ">Read only</option>
          </select>
        </div>
        <button class="btn-primary" type="submit" disabled={busy || !name.trim()}>
          {busy ? 'Creating…' : 'Create database'}
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
    <div class="rounded-lg bg-emerald-50 px-4 py-3 text-sm text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300">
      <p>{status}</p>
      {#if credentials}
        <pre class="mt-2 overflow-x-auto rounded bg-white/60 p-2 font-mono text-xs dark:bg-slate-900/60">{credentials}</pre>
      {/if}
    </div>
  {/if}

  <Card title="Engines" subtitle="server health">
    <ul class="grid gap-3 sm:grid-cols-3">
      {#each engines as item (item.engine)}
        <li class="rounded-lg border border-slate-200 p-3 dark:border-slate-800">
          <div class="flex items-center justify-between">
            <span class="text-sm font-medium capitalize">{item.engine}</span>
            <StatusBadge status={item.server_reachable ? 'active' : 'inactive'} />
          </div>
          <p class="mt-1 text-xs text-slate-500 dark:text-slate-400">
            client {item.client_available ? 'installed' : 'missing'}
          </p>
        </li>
      {/each}
    </ul>
  </Card>

  <Card title="Your databases" subtitle="{databases.length} database(s)">
    {#if loading}
      <p class="text-sm text-slate-500 dark:text-slate-400">Loading…</p>
    {:else if databases.length === 0}
      <EmptyState
        title="No databases yet"
        description="Create a database and the panel provisions the user and credentials for you."
      >
        {#snippet icon()}<Database size={28} />{/snippet}
      </EmptyState>
    {:else}
      <div class="overflow-x-auto">
        <table class="w-full text-left text-sm">
          <thead class="text-xs text-slate-500 uppercase dark:text-slate-400">
            <tr>
              <th class="py-2 pr-4 font-medium">Name</th>
              <th class="py-2 pr-4 font-medium">Engine</th>
              <th class="py-2 font-medium"></th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100 dark:divide-slate-800">
            {#each databases as database (database.id)}
              <tr>
                <td class="py-2 pr-4 font-medium">{database.name}</td>
                <td class="py-2 pr-4 capitalize">{database.engine}</td>
                <td class="py-2 text-right">
                  <button
                    class="btn-secondary px-2.5 text-rose-600 dark:text-rose-400"
                    aria-label="Delete {database.name}"
                    title="Delete database"
                    onclick={() => removeDatabase(database)}
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

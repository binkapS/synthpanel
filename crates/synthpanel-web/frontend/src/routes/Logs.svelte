<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { Pause, Play, RefreshCw, ScrollText } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import Card from '../lib/components/Card.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';

  interface LogSource {
    id: string;
    label: string;
    requires_site: boolean;
    available: boolean;
  }

  interface LogLine {
    timestamp: string | null;
    level: string | null;
    message: string;
    status: number | null;
  }

  interface SiteRow {
    id: string;
    domain: string;
  }

  let sources = $state<LogSource[]>([]);
  let sites = $state<SiteRow[]>([]);
  let lines = $state<LogLine[]>([]);
  let source = $state('panel');
  let siteId = $state('');
  let search = $state('');
  let status = $state('');
  let level = $state('');
  let limit = $state(300);
  let loading = $state(false);
  let following = $state(false);
  let error = $state('');

  let socket: WebSocket | null = null;

  const selectedSource = $derived(sources.find((item) => item.id === source));
  const needsSite = $derived(selectedSource?.requires_site ?? false);

  async function loadSources() {
    try {
      const result = await api<{ sources: LogSource[] }>('/api/v1/logs/sources');
      sources = result.sources ?? [];
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to load log sources';
    }
  }

  async function loadSites() {
    try {
      sites = await api<SiteRow[]>('/api/v1/sites');
    } catch {
      sites = [];
    }
  }

  async function readLog() {
    loading = true;
    error = '';
    try {
      const params = new URLSearchParams({ type: source, limit: String(limit) });
      if (siteId) params.set('site_id', siteId);
      if (search) params.set('search', search);
      if (status) params.set('status', status);
      if (level) params.set('level', level);
      const result = await api<{ lines: LogLine[] }>(`/api/v1/logs?${params.toString()}`);
      lines = result.lines ?? [];
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to read the log';
      lines = [];
    } finally {
      loading = false;
    }
  }

  function closeSocket() {
    if (socket) {
      socket.close();
      socket = null;
    }
    following = false;
  }

  function startFollow() {
    closeSocket();
    const params = new URLSearchParams({ type: source, lines: '200' });
    if (siteId) params.set('site_id', siteId);
    const scheme = window.location.protocol === 'https:' ? 'wss' : 'ws';
    const url = `${scheme}://${window.location.host}/ws/logs?${params.toString()}`;
    const ws = new WebSocket(url);
    socket = ws;
    following = true;
    ws.onmessage = (event) => {
      try {
        const payload = JSON.parse(event.data);
        if (payload.type === 'log' && typeof payload.line === 'string') {
          lines = [...lines, { timestamp: null, level: null, message: payload.line, status: null }].slice(-2000);
        } else if (payload.type === 'error') {
          error = String(payload.message);
          closeSocket();
        }
      } catch {
        // Ignore malformed frames.
      }
    };
    ws.onerror = () => {
      error = 'The live log connection failed.';
      closeSocket();
    };
    ws.onclose = () => {
      following = false;
      socket = null;
    };
  }

  function toggleFollow() {
    if (following) {
      closeSocket();
    } else {
      startFollow();
    }
  }

  onMount(async () => {
    await Promise.all([loadSources(), loadSites()]);
    await readLog();
  });

  onDestroy(closeSocket);
</script>

<div class="space-y-6">
  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-2xl font-semibold">Logs</h1>
      <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
        Filter nginx, PHP-FPM, panel and system logs, with live tail-follow.
      </p>
    </div>
    <div class="flex items-center gap-2">
      <button class="btn-secondary" onclick={readLog} disabled={loading}>
        <RefreshCw size={15} class={loading ? 'animate-spin' : ''} />
        Refresh
      </button>
      <button class="btn-primary" onclick={toggleFollow}>
        {#if following}
          <Pause size={15} />
          Stop following
        {:else}
          <Play size={15} />
          Follow
        {/if}
      </button>
    </div>
  </div>

  {#if error}
    <p class="rounded-lg bg-rose-50 px-4 py-3 text-sm text-rose-700 dark:bg-rose-950/60 dark:text-rose-300">{error}</p>
  {/if}

  <Card title="Filters" subtitle="the source is read through the core">
    <form
      class="grid gap-3 sm:grid-cols-5"
      onsubmit={(event) => {
        event.preventDefault();
        readLog();
      }}
    >
      <div>
        <label class="label" for="log-source">Source</label>
        <select id="log-source" class="input" bind:value={source} onchange={readLog}>
          {#each sources as item (item.id)}
            <option value={item.id} disabled={!item.available}>{item.label}{item.available ? '' : ' (unavailable)'}</option>
          {/each}
        </select>
      </div>
      <div>
        <label class="label" for="log-site">Site</label>
        <select id="log-site" class="input" bind:value={siteId} disabled={!needsSite} onchange={readLog}>
          <option value="">All</option>
          {#each sites as site (site.id)}
            <option value={site.id}>{site.domain}</option>
          {/each}
        </select>
      </div>
      <div>
        <label class="label" for="log-search">Search</label>
        <input id="log-search" class="input" bind:value={search} placeholder="text" />
      </div>
      <div>
        <label class="label" for="log-status">Status</label>
        <input id="log-status" class="input" bind:value={status} placeholder="404" inputmode="numeric" />
      </div>
      <div>
        <label class="label" for="log-level">Level</label>
        <select id="log-level" class="input" bind:value={level}>
          <option value="">Any</option>
          <option value="error">Error</option>
          <option value="warn">Warning</option>
          <option value="info">Info</option>
          <option value="debug">Debug</option>
        </select>
      </div>
      <div class="sm:col-span-5">
        <button class="btn-secondary" type="submit">Apply filters</button>
      </div>
    </form>
  </Card>

  <Card title="Output" subtitle="{lines.length} line(s)" padded={false}>
    {#if lines.length === 0}
      <div class="p-4">
        <EmptyState title="No lines" description="The log is empty or no line matches the filters.">
          {#snippet icon()}<ScrollText size={28} />{/snippet}
        </EmptyState>
      </div>
    {:else}
      <pre class="max-h-[32rem] overflow-auto p-4 font-mono text-xs leading-relaxed">{#each lines as line, index (index)}<span class={line.level === 'error' || (line.status ?? 0) >= 500 ? 'text-rose-600 dark:text-rose-400' : line.level === 'warn' || (line.status ?? 0) >= 400 ? 'text-amber-600 dark:text-amber-400' : ''}>{line.message}
</span>{/each}</pre>
    {/if}
  </Card>
</div>

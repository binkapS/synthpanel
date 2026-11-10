<script lang="ts">
  import { onMount } from 'svelte';
  import { Clock, Play, Plus, Trash2 } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import { formatDateTime, relativeTime } from '../lib/format';
  import Card from '../lib/components/Card.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import StatusBadge from '../lib/components/StatusBadge.svelte';

  interface CronJob {
    id: string;
    site_id: string | null;
    user: string;
    expression: string;
    command: string;
    last_run_at: string | null;
    last_status: string | null;
    next_run: string | null;
  }

  interface RunRecord {
    id: string;
    started_at: string;
    duration_ms: number;
    exit_code: number;
    output: string;
  }

  interface SiteRow {
    id: string;
    domain: string;
  }

  let jobs = $state<CronJob[]>([]);
  let sites = $state<SiteRow[]>([]);
  let runs = $state<Record<string, RunRecord[]>>({});
  let expanded = $state<string | null>(null);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let status = $state('');
  let showForm = $state(false);

  let expression = $state('0 3 * * *');
  let command = $state('');
  let user = $state('www-data');
  let siteId = $state('');
  let summary = $state('');

  async function load() {
    try {
      const result = await api<{ jobs: CronJob[] }>('/api/v1/cron');
      jobs = result.jobs ?? [];
      error = '';
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to load cron jobs';
    } finally {
      loading = false;
    }
  }

  onMount(async () => {
    await load();
    try {
      sites = await api<SiteRow[]>('/api/v1/sites');
    } catch {
      sites = [];
    }
  });

  async function describe() {
    summary = '';
    try {
      const result = await api<{ summary: string; next_run: string | null }>(
        `/api/v1/cron/describe?expression=${encodeURIComponent(expression)}`,
      );
      summary = result.next_run
        ? `${result.summary} — next ${formatDateTime(result.next_run)}`
        : result.summary;
    } catch (err) {
      summary = err instanceof ApiError ? err.message : 'Invalid expression';
    }
  }

  async function createJob() {
    busy = true;
    error = '';
    status = '';
    try {
      const body: Record<string, unknown> = {
        expression,
        command,
        user,
      };
      if (siteId) body.site_id = siteId;
      await api('/api/v1/cron', { method: 'POST', body: JSON.stringify(body) });
      status = 'Cron job created.';
      command = '';
      showForm = false;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to create the cron job';
    } finally {
      busy = false;
    }
  }

  async function removeJob(job: CronJob) {
    if (!confirm(`Delete the cron job "${job.command}"?`)) return;
    try {
      await api(`/api/v1/cron/${job.id}`, { method: 'DELETE' });
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to delete the cron job';
    }
  }

  async function runJob(job: CronJob) {
    error = '';
    status = '';
    try {
      const record = await api<RunRecord>(`/api/v1/cron/${job.id}/run`, { method: 'POST' });
      status = `Ran in ${record.duration_ms} ms with exit code ${record.exit_code}.`;
      await load();
      if (expanded === job.id) await loadRuns(job.id);
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to run the cron job';
    }
  }

  async function loadRuns(id: string) {
    try {
      const result = await api<{ runs: RunRecord[] }>(`/api/v1/cron/${id}/runs?limit=10`);
      runs = { ...runs, [id]: result.runs ?? [] };
    } catch {
      runs = { ...runs, [id]: [] };
    }
  }

  async function toggleRuns(job: CronJob) {
    if (expanded === job.id) {
      expanded = null;
      return;
    }
    expanded = job.id;
    await loadRuns(job.id);
  }
</script>

<div class="space-y-6">
  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-2xl font-semibold">Cron</h1>
      <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
        Scheduled shell commands with a run log and a run-now button.
      </p>
    </div>
    <button class="btn-primary" onclick={() => (showForm = !showForm)}>
      <Plus size={15} />
      {showForm ? 'Cancel' : 'Add job'}
    </button>
  </div>

  {#if error}
    <p class="rounded-lg bg-rose-50 px-4 py-3 text-sm text-rose-700 dark:bg-rose-950/60 dark:text-rose-300">{error}</p>
  {/if}
  {#if status}
    <p class="rounded-lg bg-emerald-50 px-4 py-3 text-sm text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300">{status}</p>
  {/if}

  {#if showForm}
    <Card title="New cron job">
      <form
        class="space-y-4"
        onsubmit={(event) => {
          event.preventDefault();
          createJob();
        }}
      >
        <div class="grid gap-4 sm:grid-cols-3">
          <div>
            <label class="label" for="cron-expression">Expression</label>
            <input
              id="cron-expression"
              class="input font-mono"
              bind:value={expression}
              onblur={describe}
              required
            />
            {#if summary}<p class="mt-1 text-xs text-slate-500 dark:text-slate-400">{summary}</p>{/if}
          </div>
          <div>
            <label class="label" for="cron-user">User</label>
            <input id="cron-user" class="input" bind:value={user} />
          </div>
          <div>
            <label class="label" for="cron-site">Site (optional)</label>
            <select id="cron-site" class="input" bind:value={siteId}>
              <option value="">None</option>
              {#each sites as site (site.id)}
                <option value={site.id}>{site.domain}</option>
              {/each}
            </select>
          </div>
        </div>
        <div>
          <label class="label" for="cron-command">Command</label>
          <input id="cron-command" class="input font-mono" bind:value={command} placeholder="php artisan schedule:run" required />
        </div>
        <button class="btn-primary" type="submit" disabled={busy || !command.trim()}>
          {busy ? 'Creating…' : 'Create job'}
        </button>
      </form>
    </Card>
  {/if}

  <Card title="Jobs" subtitle="{jobs.length} job(s)">
    {#if loading}
      <p class="text-sm text-slate-500 dark:text-slate-400">Loading…</p>
    {:else if jobs.length === 0}
      <EmptyState title="No cron jobs yet" description="Add a job to run commands on a schedule.">
        {#snippet icon()}<Clock size={28} />{/snippet}
      </EmptyState>
    {:else}
      <ul class="divide-y divide-slate-100 dark:divide-slate-800">
        {#each jobs as job (job.id)}
          <li class="py-3">
            <div class="flex flex-wrap items-center justify-between gap-3">
              <div class="min-w-0">
                <p class="truncate font-mono text-sm">{job.command}</p>
                <p class="mt-1 text-xs text-slate-500 dark:text-slate-400">
                  <span class="font-mono">{job.expression}</span> as {job.user}
                  {#if job.next_run}· next {formatDateTime(job.next_run)}{/if}
                  {#if job.last_run_at}· last {relativeTime(job.last_run_at)}{/if}
                </p>
              </div>
              <div class="flex items-center gap-2">
                {#if job.last_status}<StatusBadge status={job.last_status} />{/if}
                <button class="btn-secondary px-2.5" title="Run now" aria-label="Run {job.command}" onclick={() => runJob(job)}>
                  <Play size={15} />
                </button>
                <button class="btn-secondary px-2.5" onclick={() => toggleRuns(job)}>
                  {expanded === job.id ? 'Hide runs' : 'Runs'}
                </button>
                <button class="btn-secondary px-2.5 text-rose-600 dark:text-rose-400" title="Delete" aria-label="Delete {job.command}" onclick={() => removeJob(job)}>
                  <Trash2 size={15} />
                </button>
              </div>
            </div>
            {#if expanded === job.id}
              <div class="mt-3 rounded-lg bg-slate-50 p-3 dark:bg-slate-950">
                {#if (runs[job.id] ?? []).length === 0}
                  <p class="text-xs text-slate-500 dark:text-slate-400">No runs recorded yet.</p>
                {:else}
                  <ul class="space-y-2">
                    {#each runs[job.id] as run (run.id)}
                      <li class="text-xs">
                        <div class="flex items-center gap-2">
                          <StatusBadge status={run.exit_code === 0 ? 'ok' : 'failed'} />
                          <span class="text-slate-500 dark:text-slate-400">
                            {formatDateTime(run.started_at)} · {run.duration_ms} ms · exit {run.exit_code}
                          </span>
                        </div>
                        {#if run.output}
                          <pre class="mt-1 max-h-40 overflow-auto rounded bg-white p-2 font-mono dark:bg-slate-900">{run.output}</pre>
                        {/if}
                      </li>
                    {/each}
                  </ul>
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </Card>
</div>

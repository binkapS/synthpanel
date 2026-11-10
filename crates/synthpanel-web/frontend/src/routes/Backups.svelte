<script lang="ts">
  import { onMount } from 'svelte';
  import { Archive, CheckCircle2, Download, Play, RotateCcw, Trash2 } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import { formatBytes, formatDateTime } from '../lib/format';
  import Card from '../lib/components/Card.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import StatusBadge from '../lib/components/StatusBadge.svelte';

  interface BackupRow {
    id: string;
    name: string;
    kind: string;
    status: string;
    size: number;
    location: string;
    encrypted: boolean;
    created_at: string;
    error: string | null;
  }

  interface ScheduleRow {
    id: string;
    name: string;
    cron: string;
    destination: { type: string; path?: string };
    enabled: boolean;
    created_at: string;
  }

  interface SiteRow {
    id: string;
    domain: string;
  }

  let backups = $state<BackupRow[]>([]);
  let schedules = $state<ScheduleRow[]>([]);
  let sites = $state<SiteRow[]>([]);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let status = $state('');

  let siteId = $state('');
  let passphrase = $state('');
  let destination = $state('');

  let scheduleName = $state('');
  let scheduleCron = $state('0 3 * * *');
  let schedulePath = $state('');
  let scheduleKeep = $state(7);

  async function load() {
    try {
      const result = await api<{ backups: BackupRow[]; schedules: ScheduleRow[] }>(
        '/api/v1/backups',
      );
      backups = result.backups ?? [];
      schedules = result.schedules ?? [];
      error = '';
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to load backups';
    } finally {
      loading = false;
    }
  }

  async function loadSites() {
    try {
      sites = await api<SiteRow[]>('/api/v1/sites');
    } catch {
      sites = [];
    }
  }

  onMount(async () => {
    await Promise.all([load(), loadSites()]);
  });

  function destinationValue(): unknown {
    if (!destination.trim()) return undefined;
    return { type: 'local', path: destination.trim() };
  }

  async function runBackup() {
    busy = true;
    error = '';
    status = '';
    try {
      const body: Record<string, unknown> = {
        kind: siteId ? 'site' : 'full',
      };
      if (siteId) body.site_id = siteId;
      if (passphrase) body.passphrase = passphrase;
      const dest = destinationValue();
      if (dest) body.destination = dest;
      const result = await api<{ name: string; size: number }>('/api/v1/backups/run', {
        method: 'POST',
        body: JSON.stringify(body),
      });
      status = `Created ${result.name} (${formatBytes(result.size)}).`;
      passphrase = '';
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to run the backup';
    } finally {
      busy = false;
    }
  }

  async function verifyBackup(backup: BackupRow) {
    error = '';
    status = '';
    try {
      const result = await api<{ verified: boolean }>(`/api/v1/backups/${backup.id}/verify`, {
        method: 'POST',
        body: JSON.stringify({ passphrase: passphrase || undefined }),
      });
      status = result.verified ? `${backup.name} verified successfully.` : 'Verification failed.';
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to verify the backup';
    }
  }

  async function restoreBackup(backup: BackupRow) {
    if (!confirm(`Restore ${backup.name}? Files in the managed directories are overwritten.`)) {
      return;
    }
    error = '';
    status = '';
    try {
      const result = await api<{ restored: string[] }>(`/api/v1/backups/${backup.id}/restore`, {
        method: 'POST',
        body: JSON.stringify({ passphrase: passphrase || undefined }),
      });
      status = `Restored ${result.restored.length} file(s).`;
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to restore the backup';
    }
  }

  async function deleteBackup(backup: BackupRow) {
    if (!confirm(`Delete ${backup.name}?`)) return;
    error = '';
    try {
      await api(`/api/v1/backups/${backup.id}`, { method: 'DELETE' });
      status = `Deleted ${backup.name}.`;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to delete the backup';
    }
  }

  async function createSchedule() {
    busy = true;
    error = '';
    status = '';
    try {
      const body: Record<string, unknown> = {
        name: scheduleName,
        cron: scheduleCron,
        destination: schedulePath.trim()
          ? { type: 'local', path: schedulePath.trim() }
          : { type: 'local', path: '/var/backups/synthpanel/archives' },
        retention: { count: scheduleKeep },
      };
      await api('/api/v1/backups/schedules', {
        method: 'POST',
        body: JSON.stringify(body),
      });
      status = `Schedule ${scheduleName} created.`;
      scheduleName = '';
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to create the schedule';
    } finally {
      busy = false;
    }
  }

  async function deleteSchedule(schedule: ScheduleRow) {
    if (!confirm(`Delete schedule ${schedule.name}?`)) return;
    try {
      await api(`/api/v1/backups/schedules/${schedule.id}`, { method: 'DELETE' });
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to delete the schedule';
    }
  }

  function downloadUrl(backup: BackupRow): string {
    const query = passphrase ? `?passphrase=${encodeURIComponent(passphrase)}` : '';
    return `/api/v1/backups/${backup.id}/download${query}`;
  }
</script>

<div class="space-y-6">
  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-2xl font-semibold">Backups</h1>
      <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
        Archive nginx, PHP-FPM, document roots and databases, then restore in one click.
      </p>
    </div>
    <button class="btn-primary" onclick={runBackup} disabled={busy}>
      <Play size={15} />
      {busy ? 'Running…' : 'Run backup now'}
    </button>
  </div>

  {#if error}
    <p class="rounded-lg bg-rose-50 px-4 py-3 text-sm text-rose-700 dark:bg-rose-950/60 dark:text-rose-300">{error}</p>
  {/if}
  {#if status}
    <p class="rounded-lg bg-emerald-50 px-4 py-3 text-sm text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300">{status}</p>
  {/if}

  <Card title="Run a backup" subtitle="full or per-site">
    <div class="grid gap-4 sm:grid-cols-3">
      <div>
        <label class="label" for="backup-site">Scope</label>
        <select id="backup-site" class="input" bind:value={siteId}>
          <option value="">Everything</option>
          {#each sites as site (site.id)}
            <option value={site.id}>{site.domain}</option>
          {/each}
        </select>
      </div>
      <div>
        <label class="label" for="backup-passphrase">Passphrase (optional)</label>
        <input id="backup-passphrase" class="input" type="password" bind:value={passphrase} placeholder="encrypt the archive" />
      </div>
      <div>
        <label class="label" for="backup-destination">Local destination (optional)</label>
        <input id="backup-destination" class="input" bind:value={destination} placeholder="/var/backups/synthpanel/archives" />
      </div>
    </div>
  </Card>

  <Card title="Archives" subtitle="{backups.length} backup(s)">
    {#if loading}
      <p class="text-sm text-slate-500 dark:text-slate-400">Loading…</p>
    {:else if backups.length === 0}
      <EmptyState
        title="No backups yet"
        description="Run a backup to create the first archive."
      >
        {#snippet icon()}<Archive size={28} />{/snippet}
      </EmptyState>
    {:else}
      <div class="overflow-x-auto">
        <table class="w-full text-left text-sm">
          <thead class="text-xs text-slate-500 uppercase dark:text-slate-400">
            <tr>
              <th class="py-2 pr-4 font-medium">Name</th>
              <th class="py-2 pr-4 font-medium">Kind</th>
              <th class="py-2 pr-4 font-medium">Size</th>
              <th class="py-2 pr-4 font-medium">Created</th>
              <th class="py-2 pr-4 font-medium">Status</th>
              <th class="py-2 font-medium"></th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100 dark:divide-slate-800">
            {#each backups as backup (backup.id)}
              <tr>
                <td class="py-2 pr-4 font-medium">
                  {backup.name}
                  {#if backup.encrypted}<span class="ml-2 text-xs text-amber-600 dark:text-amber-400">encrypted</span>{/if}
                  {#if backup.error}<p class="text-xs text-rose-600 dark:text-rose-400">{backup.error}</p>{/if}
                </td>
                <td class="py-2 pr-4 capitalize">{backup.kind}</td>
                <td class="py-2 pr-4">{formatBytes(backup.size)}</td>
                <td class="py-2 pr-4">{formatDateTime(backup.created_at)}</td>
                <td class="py-2 pr-4"><StatusBadge status={backup.status} /></td>
                <td class="py-2">
                  <div class="flex justify-end gap-1">
                    <button class="btn-secondary px-2.5" title="Verify" aria-label="Verify {backup.name}" onclick={() => verifyBackup(backup)}>
                      <CheckCircle2 size={15} />
                    </button>
                    <a class="btn-secondary px-2.5" title="Download" aria-label="Download {backup.name}" href={downloadUrl(backup)}>
                      <Download size={15} />
                    </a>
                    <button class="btn-secondary px-2.5" title="Restore" aria-label="Restore {backup.name}" onclick={() => restoreBackup(backup)}>
                      <RotateCcw size={15} />
                    </button>
                    <button class="btn-secondary px-2.5 text-rose-600 dark:text-rose-400" title="Delete" aria-label="Delete {backup.name}" onclick={() => deleteBackup(backup)}>
                      <Trash2 size={15} />
                    </button>
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </Card>

  <Card title="Schedules" subtitle="automatic backups with retention">
    <form
      class="mb-4 grid gap-3 sm:grid-cols-4"
      onsubmit={(event) => {
        event.preventDefault();
        createSchedule();
      }}
    >
      <div>
        <label class="label" for="schedule-name">Name</label>
        <input id="schedule-name" class="input" bind:value={scheduleName} placeholder="Nightly" required />
      </div>
      <div>
        <label class="label" for="schedule-cron">Cron</label>
        <input id="schedule-cron" class="input font-mono" bind:value={scheduleCron} required />
      </div>
      <div>
        <label class="label" for="schedule-path">Path</label>
        <input id="schedule-path" class="input" bind:value={schedulePath} placeholder="default archive dir" />
      </div>
      <div>
        <label class="label" for="schedule-keep">Keep</label>
        <input id="schedule-keep" class="input" type="number" min="1" max="365" bind:value={scheduleKeep} />
      </div>
      <div class="sm:col-span-4">
        <button class="btn-primary" type="submit" disabled={busy || !scheduleName.trim()}>
          Add schedule
        </button>
      </div>
    </form>

    {#if schedules.length === 0}
      <p class="text-sm text-slate-500 dark:text-slate-400">No schedules configured.</p>
    {:else}
      <ul class="divide-y divide-slate-100 dark:divide-slate-800">
        {#each schedules as schedule (schedule.id)}
          <li class="flex items-center justify-between gap-3 py-2 text-sm">
            <div>
              <p class="font-medium">{schedule.name}</p>
              <p class="font-mono text-xs text-slate-500 dark:text-slate-400">{schedule.cron}</p>
            </div>
            <div class="flex items-center gap-2">
              <StatusBadge status={schedule.enabled ? 'enabled' : 'disabled'} />
              <button class="btn-secondary px-2.5 text-rose-600 dark:text-rose-400" aria-label="Delete schedule {schedule.name}" onclick={() => deleteSchedule(schedule)}>
                <Trash2 size={15} />
              </button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </Card>
</div>

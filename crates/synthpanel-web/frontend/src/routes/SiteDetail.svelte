<script lang="ts">
  import { onMount } from 'svelte';
  import { ArrowLeft, Lock, PauseCircle, PlayCircle, ShieldCheck } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import { formatDateTime } from '../lib/format';
  import { navigate } from '../lib/router';
  import Card from '../lib/components/Card.svelte';
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
    updated_at: string;
  }

  interface SslStatus {
    domain: string;
    ssl_mode: string;
    expires_at: string | null;
    expiring_soon: boolean;
  }

  let { id }: { id: string } = $props();

  let site = $state<Site | null>(null);
  let ssl = $state<SslStatus | null>(null);
  let error = $state('');
  let status = $state('');
  let busy = $state(false);

  async function load() {
    try {
      const result = await api<{ site: Site }>(`/api/v1/sites/${id}`);
      site = result.site;
      error = '';
      try {
        ssl = await api<SslStatus>(`/api/v1/sites/${id}/ssl/status`);
      } catch {
        ssl = null;
      }
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to load the site';
    }
  }

  onMount(load);

  async function issueSsl() {
    busy = true;
    error = '';
    status = '';
    try {
      const result = await api<{ result: { issued: boolean; message: string } }>(
        `/api/v1/sites/${id}/ssl/issue`,
        { method: 'POST' },
      );
      status = result.result.issued
        ? 'Certificate issued.'
        : `Issuance failed: ${result.result.message}`;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to issue the certificate';
    } finally {
      busy = false;
    }
  }

  async function toggleSuspend() {
    busy = true;
    error = '';
    status = '';
    const action = site?.status === 'suspended' ? 'unsuspend' : 'suspend';
    try {
      await api(`/api/v1/sites/${id}/${action}`, { method: 'POST' });
      status = action === 'suspend' ? 'Site suspended.' : 'Site back online.';
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : `Unable to ${action} the site`;
    } finally {
      busy = false;
    }
  }
</script>

<div class="space-y-6">
  <button class="btn-secondary" onclick={() => navigate('/sites')}>
    <ArrowLeft size={15} />
    Back to sites
  </button>

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

  {#if site}
    <div class="flex flex-wrap items-end justify-between gap-3">
      <div>
        <h1 class="text-2xl font-semibold">{site.domain}</h1>
        <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
          {site.template} · {site.php_version ?? 'no PHP'} · created
          {formatDateTime(site.created_at)}
        </p>
      </div>
      <div class="flex items-center gap-2">
        <StatusBadge status={site.status} />
        <button class="btn-secondary" onclick={toggleSuspend} disabled={busy}>
          {#if site.status === 'suspended'}
            <PlayCircle size={15} />
            Unsuspend
          {:else}
            <PauseCircle size={15} />
            Suspend
          {/if}
        </button>
      </div>
    </div>

    <div class="grid gap-4 lg:grid-cols-2">
      <Card title="Overview">
        <dl class="grid grid-cols-2 gap-y-3 text-sm">
          <dt class="text-slate-500 dark:text-slate-400">Domain</dt>
          <dd class="font-medium">{site.domain}</dd>
          <dt class="text-slate-500 dark:text-slate-400">Document root</dt>
          <dd class="font-mono text-xs break-all">{site.document_root}</dd>
          <dt class="text-slate-500 dark:text-slate-400">Pool user</dt>
          <dd class="font-mono text-xs">{site.pool_user}</dd>
          <dt class="text-slate-500 dark:text-slate-400">PHP version</dt>
          <dd>{site.php_version ?? '—'}</dd>
        </dl>
      </Card>

      <Card title="SSL" subtitle="Let's Encrypt over HTTP-01">
        <div class="space-y-3">
          <div class="flex items-center gap-2 text-sm">
            {#if site.ssl_mode === 'le'}
              <ShieldCheck size={16} class="text-emerald-600" />
              <span>Managed by Let's Encrypt</span>
            {:else if site.ssl_mode === 'custom'}
              <ShieldCheck size={16} class="text-blue-600" />
              <span>Custom certificate</span>
            {:else}
              <Lock size={16} class="text-slate-400" />
              <span>No certificate</span>
            {/if}
          </div>
          {#if ssl?.expires_at}
            <p class="text-sm text-slate-500 dark:text-slate-400">
              Expires {formatDateTime(ssl.expires_at)}
              {#if ssl.expiring_soon}
                <span class="text-amber-600 dark:text-amber-400">· renew soon</span>
              {/if}
            </p>
          {/if}
          <button class="btn-primary" onclick={issueSsl} disabled={busy}>
            <ShieldCheck size={15} />
            {busy ? 'Working…' : 'Issue certificate'}
          </button>
          <p class="text-xs text-slate-500 dark:text-slate-400">
            The domain must resolve to this server and port 80 must be reachable for the
            HTTP-01 challenge to succeed.
          </p>
        </div>
      </Card>
    </div>
  {/if}
</div>

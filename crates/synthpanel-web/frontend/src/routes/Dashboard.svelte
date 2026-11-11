<script lang="ts">
  import { onMount } from 'svelte';
  import {
    Archive,
    Cpu,
    Globe,
    HardDrive,
    MemoryStick,
    Network,
    RefreshCw,
    TerminalSquare,
  } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import { navigate } from '../lib/router';
  import { connectSocket } from '../lib/socket';
  import { formatBytes, formatPercent, relativeTime } from '../lib/format';
  import AuditFeed from '../lib/components/AuditFeed.svelte';
  import Card from '../lib/components/Card.svelte';
  import ServiceList from '../lib/components/ServiceList.svelte';
  import Sparkline from '../lib/components/Sparkline.svelte';
  import StatCard from '../lib/components/StatCard.svelte';

  interface Snapshot {
    ts: number;
    host: string;
    cpu: number;
    load1: number;
    mem_used: number;
    mem_total: number;
    swap_used: number;
    swap_total: number;
    disk_used: number;
    disk_total: number;
    net_rx: number;
    net_tx: number;
  }

  interface Service {
    unit: string;
    status: string;
  }

  interface AuditEntry {
    id: number;
    actor_id: string | null;
    actor_name: string | null;
    action: string;
    target: string | null;
    metadata_json: string;
    ip: string | null;
    created_at: string;
  }

  let snapshot = $state<Snapshot | null>(null);
  let points = $state<Snapshot[]>([]);
  let services = $state<Service[]>([]);
  let entries = $state<AuditEntry[]>([]);
  let error = $state('');
  let updatedAt = $state<Date | null>(null);
  let loading = $state(true);
  let live = $state(false);

  const cpuSeries = $derived(points.map((point) => point.cpu));
  const memorySeries = $derived(
    points.map((point) => (point.mem_total > 0 ? (point.mem_used / point.mem_total) * 100 : 0)),
  );

  const networkRate = $derived.by(() => {
    if (points.length < 2) return { rx: 0, tx: 0 };
    const last = points[points.length - 1];
    const previous = points[points.length - 2];
    const seconds = Math.max(1, last.ts - previous.ts);
    return {
      rx: Math.max(0, (last.net_rx - previous.net_rx) / seconds),
      tx: Math.max(0, (last.net_tx - previous.net_tx) / seconds),
    };
  });

  async function refresh() {
    try {
      const [snapshotResponse, rangeResponse, servicesResponse, auditResponse] = await Promise.all([
        api<Snapshot | null>('/api/v1/metrics/snapshot'),
        api<{ range: string; points: Snapshot[] }>('/api/v1/metrics/range?range=24h'),
        api<{ services: Service[] }>('/api/v1/services'),
        api<{ entries: AuditEntry[] }>('/api/v1/audit?limit=12'),
      ]);
      snapshot = snapshotResponse;
      points = rangeResponse.points ?? [];
      services = servicesResponse.services ?? [];
      entries = auditResponse.entries ?? [];
      updatedAt = new Date();
      error = '';
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to load dashboard data';
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    refresh();

    // Prefer the WebSocket channel; fall back to polling when it is closed.
    const disconnect = connectSocket('/ws/dashboard', {
      onMessage: (payload) => {
        const message = payload as {
          type?: string;
          metrics?: Snapshot | null;
          services?: Service[];
        };
        if (message.type === 'snapshot') {
          if (message.metrics) snapshot = message.metrics;
          if (message.services) services = message.services;
          updatedAt = new Date();
          live = true;
        }
      },
      onClose: () => {
        live = false;
      },
    });

    const timer = setInterval(() => {
      if (!live) refresh();
    }, 5000);

    return () => {
      disconnect();
      clearInterval(timer);
    };
  });
</script>

<div class="space-y-6">
  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-2xl font-semibold">Dashboard</h1>
      <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
        {snapshot?.host ? `Host ${snapshot.host}` : 'System overview'} · updated
        {updatedAt ? relativeTime(updatedAt.toISOString()) : 'never'}
        {live ? ' · live' : ''}
      </p>
    </div>
    <button class="btn-secondary" onclick={refresh} disabled={loading}>
      <RefreshCw size={15} />
      Refresh
    </button>
  </div>

  {#if error}
    <p class="rounded-lg bg-rose-50 px-4 py-3 text-sm text-rose-700 dark:bg-rose-950/60 dark:text-rose-300">
      {error}
    </p>
  {/if}

  <div class="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
    <StatCard
      label="CPU"
      value={snapshot ? formatPercent(snapshot.cpu) : '–'}
      hint={snapshot ? `load ${snapshot.load1.toFixed(2)}` : ''}
    >
      {#snippet icon()}<Cpu size={18} />{/snippet}
    </StatCard>
    <StatCard
      label="Memory"
      value={snapshot ? formatBytes(snapshot.mem_used) : '–'}
      hint={snapshot && snapshot.mem_total > 0
        ? `of ${formatBytes(snapshot.mem_total)}`
        : ''}
    >
      {#snippet icon()}<MemoryStick size={18} />{/snippet}
    </StatCard>
    <StatCard
      label="Disk"
      value={snapshot ? formatPercent(snapshot.disk_used) : '–'}
      hint={snapshot && snapshot.disk_total > 0
        ? `of ${formatBytes(snapshot.disk_total)}`
        : ''}
    >
      {#snippet icon()}<HardDrive size={18} />{/snippet}
    </StatCard>
    <StatCard
      label="Network"
      value={`↓ ${formatBytes(networkRate.rx)}/s`}
      hint={`↑ ${formatBytes(networkRate.tx)}/s`}
    >
      {#snippet icon()}<Network size={18} />{/snippet}
    </StatCard>
  </div>

  <div class="grid gap-4 lg:grid-cols-3">
    <Card title="CPU usage" subtitle="last 24 hours" class="lg:col-span-2">
      <Sparkline values={cpuSeries} height={120} />
    </Card>
    <Card title="Services" subtitle="systemd units">
      <ServiceList {services} />
    </Card>
  </div>

  <div class="grid gap-4 lg:grid-cols-3">
    <Card title="Memory usage" subtitle="last 24 hours" class="lg:col-span-2">
      <Sparkline values={memorySeries} height={120} />
    </Card>
    <Card title="Recent activity" subtitle="audit log">
      <AuditFeed {entries} />
    </Card>
  </div>

  <Card title="Quick actions" subtitle="common tasks">
    <div class="flex flex-wrap gap-2">
      <button class="btn-secondary" onclick={() => navigate('/sites')}>
        <Globe size={15} /> Add site
      </button>
      <button class="btn-secondary" onclick={() => navigate('/backups')}>
        <Archive size={15} /> Run backup
      </button>
      <button class="btn-secondary" onclick={() => navigate('/terminal')}>
        <TerminalSquare size={15} /> Open terminal
      </button>
    </div>
  </Card>
</div>

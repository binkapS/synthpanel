<script lang="ts">
  import { onMount } from 'svelte';
  import { BellRing, Plus, Send, Trash2 } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import { formatDateTime } from '../lib/format';
  import Card from '../lib/components/Card.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import StatusBadge from '../lib/components/StatusBadge.svelte';

  interface Channel {
    id: string;
    name: string;
    kind: string;
    config: Record<string, unknown>;
    enabled: boolean;
    created_at: string;
  }

  interface Rule {
    id: string;
    name: string;
    event: string;
    severity: string;
    channel_id: string;
    enabled: boolean;
    cooldown_secs: number;
  }

  interface Alert {
    id: string;
    event: string;
    severity: string;
    message: string;
    status: string;
    error: string | null;
    created_at: string;
  }

  const EVENTS = [
    'backup.failed',
    'backup.succeeded',
    'cron.failed',
    'deploy.failed',
    'deploy.succeeded',
    'ssl.expiring',
    'disk.usage',
    'cpu.usage',
    'update.failed',
    'service.down',
  ];

  let channels = $state<Channel[]>([]);
  let rules = $state<Rule[]>([]);
  let alerts = $state<Alert[]>([]);
  let loading = $state(true);
  let busy = $state(false);
  let error = $state('');
  let status = $state('');

  let showChannelForm = $state(false);
  let channelName = $state('');
  let channelKind = $state('webhook');
  let webhookUrl = $state('');
  let webhookSecret = $state('');
  let emailTo = $state('');
  let emailHost = $state('localhost');
  let emailPort = $state('25');
  let emailFrom = $state('synthpanel@localhost');

  let showRuleForm = $state(false);
  let ruleName = $state('');
  let ruleEvent = $state(EVENTS[0]);
  let ruleSeverity = $state('warning');
  let ruleChannel = $state('');
  let ruleCooldown = $state(900);

  async function load() {
    try {
      const [channelResult, ruleResult, alertResult] = await Promise.all([
        api<{ channels: Channel[] }>('/api/v1/alerts/channels'),
        api<{ rules: Rule[] }>('/api/v1/alerts/rules'),
        api<{ alerts: Alert[] }>('/api/v1/alerts'),
      ]);
      channels = channelResult.channels ?? [];
      rules = ruleResult.rules ?? [];
      alerts = alertResult.alerts ?? [];
      if (!ruleChannel && channels.length > 0) ruleChannel = channels[0].id;
      error = '';
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to load the alerts';
    } finally {
      loading = false;
    }
  }

  onMount(load);

  async function createChannel() {
    busy = true;
    error = '';
    status = '';
    try {
      const config =
        channelKind === 'webhook'
          ? {
              url: webhookUrl,
              ...(webhookSecret ? { secret: webhookSecret } : {}),
            }
          : {
              to: emailTo
                .split(',')
                .map((entry) => entry.trim())
                .filter(Boolean),
              from: emailFrom,
              smtp: { host: emailHost, port: Number(emailPort) },
            };
      await api('/api/v1/alerts/channels', {
        method: 'POST',
        body: JSON.stringify({ name: channelName, kind: channelKind, config }),
      });
      status = 'The alert channel was created.';
      channelName = '';
      webhookUrl = '';
      webhookSecret = '';
      emailTo = '';
      showChannelForm = false;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to create the channel';
    } finally {
      busy = false;
    }
  }

  async function createRule() {
    busy = true;
    error = '';
    status = '';
    try {
      await api('/api/v1/alerts/rules', {
        method: 'POST',
        body: JSON.stringify({
          name: ruleName,
          event: ruleEvent,
          severity: ruleSeverity,
          channel_id: ruleChannel,
          cooldown_secs: ruleCooldown,
        }),
      });
      status = 'The alert rule was created.';
      ruleName = '';
      showRuleForm = false;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to create the rule';
    } finally {
      busy = false;
    }
  }

  async function testChannel(channel: Channel) {
    error = '';
    status = '';
    try {
      await api('/api/v1/alerts/test', {
        method: 'POST',
        body: JSON.stringify({ channel_id: channel.id }),
      });
      status = `A test alert was sent to ${channel.name}.`;
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'The test alert failed';
    }
  }

  async function removeChannel(channel: Channel) {
    if (!confirm(`Delete the alert channel "${channel.name}"?`)) return;
    try {
      await api(`/api/v1/alerts/channels/${channel.id}`, { method: 'DELETE' });
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to delete the channel';
    }
  }

  async function removeRule(rule: Rule) {
    if (!confirm(`Delete the alert rule "${rule.name}"?`)) return;
    try {
      await api(`/api/v1/alerts/rules/${rule.id}`, { method: 'DELETE' });
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to delete the rule';
    }
  }

  function channelNameFor(id: string): string {
    return channels.find((channel) => channel.id === id)?.name ?? id;
  }
</script>

<div class="space-y-6">
  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-2xl font-semibold">Alerts</h1>
      <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
        Notify you by email or webhook when something needs attention.
      </p>
    </div>
    <div class="flex gap-2">
      <button class="btn-secondary" onclick={() => (showRuleForm = !showRuleForm)}>
        <Plus size={15} />
        {showRuleForm ? 'Cancel' : 'Add rule'}
      </button>
      <button class="btn-primary" onclick={() => (showChannelForm = !showChannelForm)}>
        <Plus size={15} />
        {showChannelForm ? 'Cancel' : 'Add channel'}
      </button>
    </div>
  </div>

  {#if error}
    <p class="rounded-lg bg-rose-50 px-4 py-3 text-sm text-rose-700 dark:bg-rose-950/60 dark:text-rose-300">{error}</p>
  {/if}
  {#if status}
    <p class="rounded-lg bg-emerald-50 px-4 py-3 text-sm text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300">{status}</p>
  {/if}

  {#if showChannelForm}
    <Card title="New alert channel">
      <form
        class="space-y-4"
        onsubmit={(event) => {
          event.preventDefault();
          createChannel();
        }}
      >
        <div class="grid gap-4 sm:grid-cols-2">
          <div>
            <label class="label" for="channel-name">Name</label>
            <input id="channel-name" class="input" bind:value={channelName} required />
          </div>
          <div>
            <label class="label" for="channel-kind">Type</label>
            <select id="channel-kind" class="input" bind:value={channelKind}>
              <option value="webhook">Webhook</option>
              <option value="email">Email</option>
            </select>
          </div>
        </div>

        {#if channelKind === 'webhook'}
          <div>
            <label class="label" for="channel-url">URL</label>
            <input id="channel-url" class="input" bind:value={webhookUrl} placeholder="https://hooks.example.com/xyz" required />
          </div>
          <div>
            <label class="label" for="channel-secret">Signing secret (optional)</label>
            <input id="channel-secret" class="input" bind:value={webhookSecret} type="password" />
            <p class="mt-1 text-xs text-slate-500 dark:text-slate-400">
              The payload is signed with HMAC-SHA256 and sent as
              <span class="font-mono">X-SynthPanel-Signature</span>.
            </p>
          </div>
        {:else}
          <div class="grid gap-4 sm:grid-cols-2">
            <div>
              <label class="label" for="channel-to">Recipients</label>
              <input id="channel-to" class="input" bind:value={emailTo} placeholder="ops@example.com, admin@example.com" required />
            </div>
            <div>
              <label class="label" for="channel-from">From</label>
              <input id="channel-from" class="input" bind:value={emailFrom} />
            </div>
            <div>
              <label class="label" for="channel-host">SMTP host</label>
              <input id="channel-host" class="input" bind:value={emailHost} />
            </div>
            <div>
              <label class="label" for="channel-port">SMTP port</label>
              <input id="channel-port" class="input" bind:value={emailPort} type="number" />
            </div>
          </div>
        {/if}

        <button class="btn-primary" type="submit" disabled={busy || !channelName.trim()}>
          {busy ? 'Creating…' : 'Create channel'}
        </button>
      </form>
    </Card>
  {/if}

  {#if showRuleForm}
    <Card title="New alert rule">
      <form
        class="space-y-4"
        onsubmit={(event) => {
          event.preventDefault();
          createRule();
        }}
      >
        <div class="grid gap-4 sm:grid-cols-2">
          <div>
            <label class="label" for="rule-name">Name</label>
            <input id="rule-name" class="input" bind:value={ruleName} required />
          </div>
          <div>
            <label class="label" for="rule-event">Event</label>
            <select id="rule-event" class="input" bind:value={ruleEvent}>
              {#each EVENTS as event (event)}
                <option value={event}>{event}</option>
              {/each}
            </select>
          </div>
          <div>
            <label class="label" for="rule-severity">Severity</label>
            <select id="rule-severity" class="input" bind:value={ruleSeverity}>
              <option value="info">info</option>
              <option value="warning">warning</option>
              <option value="critical">critical</option>
            </select>
          </div>
          <div>
            <label class="label" for="rule-channel">Channel</label>
            <select id="rule-channel" class="input" bind:value={ruleChannel} required>
              {#each channels as channel (channel.id)}
                <option value={channel.id}>{channel.name}</option>
              {/each}
            </select>
          </div>
          <div>
            <label class="label" for="rule-cooldown">Cooldown (seconds)</label>
            <input id="rule-cooldown" class="input" bind:value={ruleCooldown} type="number" />
          </div>
        </div>
        <button class="btn-primary" type="submit" disabled={busy || !ruleName.trim()}>
          {busy ? 'Creating…' : 'Create rule'}
        </button>
      </form>
    </Card>
  {/if}

  <Card title="Channels" subtitle="{channels.length} channel(s)">
    {#if loading}
      <p class="text-sm text-slate-500 dark:text-slate-400">Loading…</p>
    {:else if channels.length === 0}
      <EmptyState title="No alert channels" description="Add a webhook or an email address to receive alerts.">
        {#snippet icon()}<BellRing size={28} />{/snippet}
      </EmptyState>
    {:else}
      <ul class="divide-y divide-slate-100 dark:divide-slate-800">
        {#each channels as channel (channel.id)}
          <li class="flex flex-wrap items-center justify-between gap-3 py-3">
            <div class="min-w-0">
              <p class="truncate text-sm font-medium">{channel.name}</p>
              <p class="mt-1 text-xs text-slate-500 dark:text-slate-400">
                {channel.kind} · created {formatDateTime(channel.created_at)}
              </p>
            </div>
            <div class="flex items-center gap-2">
              <button class="btn-secondary px-2.5" title="Send a test alert" aria-label="Send a test alert to {channel.name}" onclick={() => testChannel(channel)}>
                <Send size={15} />
              </button>
              <button class="btn-secondary px-2.5 text-rose-600 dark:text-rose-400" title="Delete" aria-label="Delete {channel.name}" onclick={() => removeChannel(channel)}>
                <Trash2 size={15} />
              </button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </Card>

  <Card title="Rules" subtitle="{rules.length} rule(s)">
    {#if rules.length === 0}
      <p class="text-sm text-slate-500 dark:text-slate-400">No rules yet.</p>
    {:else}
      <ul class="divide-y divide-slate-100 dark:divide-slate-800">
        {#each rules as rule (rule.id)}
          <li class="flex flex-wrap items-center justify-between gap-3 py-3">
            <div class="min-w-0">
              <p class="truncate text-sm font-medium">{rule.name}</p>
              <p class="mt-1 text-xs text-slate-500 dark:text-slate-400">
                <span class="font-mono">{rule.event}</span> · {rule.severity} ·
                {channelNameFor(rule.channel_id)} · cooldown {rule.cooldown_secs}s
              </p>
            </div>
            <button class="btn-secondary px-2.5 text-rose-600 dark:text-rose-400" title="Delete" aria-label="Delete {rule.name}" onclick={() => removeRule(rule)}>
              <Trash2 size={15} />
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </Card>

  <Card title="History" subtitle="{alerts.length} alert(s)">
    {#if alerts.length === 0}
      <p class="text-sm text-slate-500 dark:text-slate-400">No alerts have been raised yet.</p>
    {:else}
      <ul class="divide-y divide-slate-100 dark:divide-slate-800">
        {#each alerts as alert (alert.id)}
          <li class="py-3">
            <div class="flex flex-wrap items-center gap-2">
              <StatusBadge status={alert.status === 'delivered' ? 'ok' : 'failed'} />
              <span class="font-mono text-xs">{alert.event}</span>
              <span class="text-xs text-slate-500 dark:text-slate-400">
                {formatDateTime(alert.created_at)}
              </span>
            </div>
            <p class="mt-1 text-sm">{alert.message}</p>
            {#if alert.error}
              <p class="mt-1 text-xs text-rose-600 dark:text-rose-400">{alert.error}</p>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </Card>
</div>

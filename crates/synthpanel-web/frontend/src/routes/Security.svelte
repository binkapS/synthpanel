<script lang="ts">
  import { onMount } from 'svelte';
  import { KeyRound, Plus, ShieldCheck, Trash2 } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import Card from '../lib/components/Card.svelte';
  import StatusBadge from '../lib/components/StatusBadge.svelte';

  interface Jail {
    name: string;
    status: string;
    currently_banned: number;
    total_banned: number;
  }

  interface SshKey {
    id: string;
    label: string;
    public_key: string;
    fingerprint: string;
    created_at: string;
  }

  let firewall = $state<{ available: boolean; status: string }>({
    available: false,
    status: '',
  });
  let jails = $state<Jail[]>([]);
  let fail2banAvailable = $state(false);
  let keys = $state<SshKey[]>([]);
  let error = $state('');
  let status = $state('');

  let port = $state('8443');
  let proto = $state('tcp');
  let source = $state('any');
  let action = $state('allow');
  let comment = $state('');
  let keyLabel = $state('');
  let keyValue = $state('');
  let busy = $state(false);

  async function load() {
    try {
      const [firewallResponse, fail2banResponse, keysResponse] = await Promise.all([
        api<{ available: boolean; status: string }>('/api/v1/security/firewall'),
        api<{ available: boolean; jails: Jail[] }>('/api/v1/security/fail2ban'),
        api<{ keys: SshKey[] }>('/api/v1/security/ssh-keys'),
      ]);
      firewall = firewallResponse;
      jails = fail2banResponse.jails ?? [];
      fail2banAvailable = fail2banResponse.available;
      keys = keysResponse.keys ?? [];
      error = '';
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to load the security status';
    }
  }

  onMount(load);

  async function applyRule() {
    busy = true;
    error = '';
    status = '';
    try {
      await api('/api/v1/security/firewall/rules', {
        method: 'POST',
        body: JSON.stringify({ port, proto, source, action, comment }),
      });
      status = `Rule applied: ${action} ${port}/${proto}`;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to apply the firewall rule';
    } finally {
      busy = false;
    }
  }

  async function unban(jail: Jail, ip: string) {
    error = '';
    try {
      await api('/api/v1/security/fail2ban/unban', {
        method: 'POST',
        body: JSON.stringify({ jail: jail.name, ip }),
      });
      status = `Unbanned ${ip} from ${jail.name}`;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to unban the address';
    }
  }

  async function addKey() {
    busy = true;
    error = '';
    status = '';
    try {
      await api('/api/v1/security/ssh-keys', {
        method: 'POST',
        body: JSON.stringify({ label: keyLabel, public_key: keyValue, user: 'root' }),
      });
      status = `Added ${keyLabel}`;
      keyLabel = '';
      keyValue = '';
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to add the SSH key';
    } finally {
      busy = false;
    }
  }

  async function removeKey(key: SshKey) {
    if (!confirm(`Remove ${key.label}?`)) return;
    error = '';
    try {
      await api(`/api/v1/security/ssh-keys/${key.id}`, { method: 'DELETE' });
      status = `Removed ${key.label}`;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to remove the SSH key';
    }
  }
</script>

<div class="space-y-6">
  <div>
    <h1 class="text-2xl font-semibold">Security</h1>
    <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
      Firewall rules, fail2ban jails and the SSH keys the panel installs.
    </p>
  </div>

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

  <div class="grid gap-4 lg:grid-cols-2">
    <Card title="Firewall" subtitle="UFW rules">
      {#if firewall.available}
        <form
          class="space-y-3"
          onsubmit={(event) => {
            event.preventDefault();
            applyRule();
          }}
        >
          <div class="grid grid-cols-3 gap-3">
            <div>
              <label class="label" for="port">Port</label>
              <input id="port" class="input" bind:value={port} required />
            </div>
            <div>
              <label class="label" for="proto">Protocol</label>
              <select id="proto" class="input" bind:value={proto}>
                <option value="tcp">tcp</option>
                <option value="udp">udp</option>
              </select>
            </div>
            <div>
              <label class="label" for="action">Action</label>
              <select id="action" class="input" bind:value={action}>
                <option value="allow">allow</option>
                <option value="deny">deny</option>
                <option value="reject">reject</option>
                <option value="limit">limit</option>
              </select>
            </div>
          </div>
          <div>
            <label class="label" for="source">Source</label>
            <input id="source" class="input" bind:value={source} placeholder="any" />
          </div>
          <div>
            <label class="label" for="comment">Comment</label>
            <input id="comment" class="input" bind:value={comment} placeholder="why is this rule needed?" />
          </div>
          <button class="btn-primary" type="submit" disabled={busy}>
            <Plus size={15} />
            Apply rule
          </button>
        </form>
        {#if firewall.status}
          <pre class="mt-4 max-h-64 overflow-auto rounded-lg bg-slate-50 p-3 font-mono text-xs text-slate-600 dark:bg-slate-950 dark:text-slate-300">{firewall.status}</pre>
        {/if}
      {:else}
        <p class="text-sm text-slate-500 dark:text-slate-400">
          UFW is not installed on this host, so the panel cannot manage the firewall.
        </p>
      {/if}
    </Card>

    <Card title="fail2ban" subtitle="jail status">
      {#if !fail2banAvailable}
        <p class="text-sm text-slate-500 dark:text-slate-400">
          fail2ban is not installed on this host.
        </p>
      {:else if jails.length === 0}
        <p class="text-sm text-slate-500 dark:text-slate-400">No jails are configured.</p>
      {:else}
        <ul class="divide-y divide-slate-100 dark:divide-slate-800">
          {#each jails as jail (jail.name)}
            <li class="flex items-center justify-between gap-3 py-2 first:pt-0 last:pb-0">
              <div>
                <div class="flex items-center gap-2">
                  <span class="font-mono text-xs">{jail.name}</span>
                  <StatusBadge status={jail.status} />
                </div>
                <p class="text-xs text-slate-500 dark:text-slate-400">
                  {jail.currently_banned} banned now · {jail.total_banned} total
                </p>
              </div>
              <button
                class="btn-secondary px-2.5"
                title="Unban an address"
                onclick={() => {
                  const ip = prompt('Address to unban');
                  if (ip) unban(jail, ip);
                }}
              >
                Unban
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </Card>
  </div>

  <Card title="SSH keys" subtitle="installed into /root/.ssh/authorized_keys">
    <form
      class="mb-4 space-y-3"
      onsubmit={(event) => {
        event.preventDefault();
        addKey();
      }}
    >
      <div class="grid gap-3 sm:grid-cols-[1fr_2fr_auto]">
        <div>
          <label class="label" for="key-label">Label</label>
          <input id="key-label" class="input" bind:value={keyLabel} placeholder="office laptop" required />
        </div>
        <div>
          <label class="label" for="key-value">Public key</label>
          <input id="key-value" class="input font-mono text-xs" bind:value={keyValue} placeholder="ssh-ed25519 AAAA…" required />
        </div>
        <div class="flex items-end">
          <button class="btn-primary" type="submit" disabled={busy}>
            <KeyRound size={15} />
            Add
          </button>
        </div>
      </div>
    </form>

    {#if keys.length === 0}
      <p class="flex items-center gap-2 text-sm text-slate-500 dark:text-slate-400">
        <ShieldCheck size={16} />
        No SSH keys managed by the panel yet.
      </p>
    {:else}
      <div class="overflow-x-auto">
        <table class="w-full text-left text-sm">
          <thead class="text-xs text-slate-500 uppercase dark:text-slate-400">
            <tr>
              <th class="py-2 pr-4 font-medium">Label</th>
              <th class="py-2 pr-4 font-medium">Fingerprint</th>
              <th class="py-2 font-medium"></th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100 dark:divide-slate-800">
            {#each keys as key (key.id)}
              <tr>
                <td class="py-2 pr-4 font-medium">{key.label}</td>
                <td class="py-2 pr-4 font-mono text-xs break-all">{key.fingerprint}</td>
                <td class="py-2 text-right">
                  <button
                    class="btn-secondary px-2.5 text-rose-600 dark:text-rose-400"
                    aria-label="Remove {key.label}"
                    onclick={() => removeKey(key)}
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

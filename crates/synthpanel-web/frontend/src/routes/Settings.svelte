<script lang="ts">
  import { onMount } from 'svelte';
  import { KeyRound, Plus, Save, ShieldCheck, Trash2 } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import { locales, setLocale, t } from '../lib/i18n';
  import { fetchMe, session } from '../lib/stores';
  import Card from '../lib/components/Card.svelte';

  type Settings = Record<string, unknown>;

  let settings = $state<Settings>({});
  let hostname = $state('');
  let timezone = $state('UTC');
  let locale = $state('en');
  let status = $state('');
  let error = $state('');
  let busy = $state(false);

  let twoFactorSecret = $state('');
  let twoFactorCode = $state('');
  let twoFactorUri = $state('');
  let twoFactorMessage = $state('');
  let twoFactorError = $state('');

  const totpEnabled = $derived($session?.user.totp_enabled ?? false);

  interface ApiKeyRow {
    id: string;
    name: string;
    prefix: string;
    scopes: string[];
    last_used_at: string | null;
    created_at: string;
  }

  interface UpdateStatus {
    current_version: string;
    channel: string;
    source: string;
    history: Array<{
      id: string;
      from_version: string;
      to_version: string;
      status: string;
      started_at: string;
      error: string | null;
    }>;
  }

  interface UpdateCheck {
    current_version: string;
    update_available: boolean;
    release: { version: string; notes: string } | null;
  }

  let updateStatus = $state<UpdateStatus | null>(null);
  let updateChannel = $state('stable');
  let updateCheck = $state<UpdateCheck | null>(null);
  let updateBusy = $state(false);
  let updateError = $state('');
  let updateMessage = $state('');

  let apiKeys = $state<ApiKeyRow[]>([]);
  let keyName = $state('');
  let keyScopes = $state('read');
  let createdToken = $state('');
  let keyError = $state('');

  async function loadApiKeys() {
    try {
      const result = await api<{ keys: ApiKeyRow[] }>('/api/v1/api-keys');
      apiKeys = result.keys ?? [];
    } catch {
      apiKeys = [];
    }
  }

  async function createApiKey() {
    keyError = '';
    createdToken = '';
    try {
      const scopes = keyScopes
        .split(',')
        .map((value) => value.trim())
        .filter(Boolean);
      const result = await api<{ token: string }>('/api/v1/api-keys', {
        method: 'POST',
        body: JSON.stringify({ name: keyName, scopes }),
      });
      createdToken = result.token;
      keyName = '';
      await Promise.all([loadApiKeys(), loadUpdateStatus()]);
    } catch (err) {
      keyError = err instanceof ApiError ? err.message : 'Unable to create the API key';
    }
  }

  async function deleteApiKey(key: ApiKeyRow) {
    if (!confirm(`Revoke API key ${key.name}?`)) return;
    try {
      await api(`/api/v1/api-keys/${key.id}`, { method: 'DELETE' });
      await Promise.all([loadApiKeys(), loadUpdateStatus()]);
    } catch (err) {
      keyError = err instanceof ApiError ? err.message : 'Unable to revoke the API key';
    }
  }

  onMount(async () => {
    try {
      settings = await api<Settings>('/api/v1/settings');
      hostname = String(settings.hostname ?? '');
      timezone = String(settings.timezone ?? 'UTC');
      locale = String(settings.locale ?? 'en');
      await Promise.all([loadApiKeys(), loadUpdateStatus()]);
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to load settings';
    }
  });

  async function loadUpdateStatus() {
    try {
      updateStatus = await api<UpdateStatus>('/api/v1/update');
      updateChannel = updateStatus.channel || 'stable';
    } catch {
      updateStatus = null;
    }
  }

  async function checkForUpdates() {
    updateBusy = true;
    updateError = '';
    updateMessage = '';
    updateCheck = null;
    try {
      updateCheck = await api<UpdateCheck>('/api/v1/update/check', {
        method: 'POST',
        body: JSON.stringify({ channel: updateChannel }),
      });
      updateMessage = updateCheck.update_available
        ? `Version ${updateCheck.release?.version} is available.`
        : 'The panel is up to date.';
    } catch (err) {
      updateError = err instanceof ApiError ? err.message : 'Unable to check for updates';
    } finally {
      updateBusy = false;
    }
  }

  async function applyUpdate() {
    if (!confirm('Install the update now? The services restart and the previous binaries are kept for rollback.')) {
      return;
    }
    updateBusy = true;
    updateError = '';
    updateMessage = '';
    try {
      const result = await api<{ updated: boolean; message?: string }>('/api/v1/update/apply', {
        method: 'POST',
        body: JSON.stringify({ channel: updateChannel }),
      });
      updateMessage = result.updated ? 'Update installed.' : (result.message ?? 'Already up to date.');
      updateCheck = null;
      await loadUpdateStatus();
    } catch (err) {
      updateError = err instanceof ApiError ? err.message : 'The update failed';
      await loadUpdateStatus();
    } finally {
      updateBusy = false;
    }
  }

  async function save() {
    busy = true;
    status = '';
    error = '';
    try {
      settings = await api<Settings>('/api/v1/settings', {
        method: 'PATCH',
        body: JSON.stringify({ hostname, timezone, locale }),
      });
      status = 'Settings saved.';
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to save settings';
    } finally {
      busy = false;
    }
  }

  async function startTwoFactor() {
    twoFactorMessage = '';
    twoFactorError = '';
    try {
      const result = await api<{ secret: string; otpauth_url: string }>('/api/v1/auth/2fa/setup', {
        method: 'POST',
      });
      twoFactorSecret = result.secret;
      twoFactorUri = result.otpauth_url;
      twoFactorMessage = 'Scan the secret in your authenticator app, then confirm the code.';
    } catch (err) {
      twoFactorError = err instanceof ApiError ? err.message : 'Unable to start 2FA setup';
    }
  }

  async function enableTwoFactor() {
    twoFactorMessage = '';
    twoFactorError = '';
    try {
      await api('/api/v1/auth/2fa/enable', {
        method: 'POST',
        body: JSON.stringify({ code: twoFactorCode }),
      });
      twoFactorSecret = '';
      twoFactorCode = '';
      twoFactorMessage = 'Two-factor authentication is now enabled.';
      const user = await fetchMe();
      session.set({ user });
    } catch (err) {
      twoFactorError = err instanceof ApiError ? err.message : 'Unable to enable 2FA';
    }
  }

  async function disableTwoFactor() {
    const password = prompt('Confirm your password to disable two-factor authentication');
    if (!password) return;
    twoFactorMessage = '';
    twoFactorError = '';
    try {
      await api('/api/v1/auth/2fa/disable', {
        method: 'POST',
        body: JSON.stringify({ password }),
      });
      twoFactorMessage = 'Two-factor authentication disabled.';
      const user = await fetchMe();
      session.set({ user });
    } catch (err) {
      twoFactorError = err instanceof ApiError ? err.message : 'Unable to disable 2FA';
    }
  }
</script>

<div class="space-y-6">
  <div>
    <h1 class="text-2xl font-semibold">Settings</h1>
    <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
      Panel-wide configuration. Changes are recorded in the audit log.
    </p>
  </div>

  <Card title="General" subtitle="hostname and localisation">
    <form
      class="space-y-4"
      onsubmit={(event) => {
        event.preventDefault();
        save();
      }}
    >
      <div>
        <label class="label" for="hostname">Hostname</label>
        <input id="hostname" class="input" bind:value={hostname} placeholder="panel.example.com" />
        <p class="mt-1 text-xs text-slate-500 dark:text-slate-400">
          Required before a Let's Encrypt certificate can be issued for the panel itself.
        </p>
      </div>
      <div class="grid gap-4 sm:grid-cols-2">
        <div>
          <label class="label" for="timezone">Timezone</label>
          <input id="timezone" class="input" bind:value={timezone} placeholder="UTC" />
        </div>
        <div>
          <label class="label" for="locale">Locale</label>
          <select
            id="locale"
            class="input"
            bind:value={locale}
            onchange={(event) => setLocale((event.currentTarget as HTMLSelectElement).value)}
          >
            {#each locales as value (value)}
              <option value={value}>{value}</option>
            {/each}
          </select>
        </div>
      </div>
      <div class="flex items-center gap-3">
        <button class="btn-primary" type="submit" disabled={busy}>
          <Save size={15} />
          {busy ? 'Saving…' : 'Save changes'}
        </button>
        {#if status}<span class="text-sm text-emerald-600 dark:text-emerald-400">{status}</span>{/if}
        {#if error}<span class="text-sm text-rose-600 dark:text-rose-400">{error}</span>{/if}
      </div>
    </form>
  </Card>

  <Card title="Two-factor authentication" subtitle="TOTP with an authenticator app">
    <div class="space-y-4">
      <div class="flex items-center gap-2 text-sm">
        <ShieldCheck size={16} class={totpEnabled ? 'text-emerald-600' : 'text-slate-400'} />
        <span>{totpEnabled ? 'Enabled for your account' : 'Not enabled for your account'}</span>
      </div>

      {#if !totpEnabled}
        {#if twoFactorSecret}
          <div class="rounded-lg bg-slate-50 p-4 dark:bg-slate-950">
            <p class="text-xs font-medium text-slate-500 uppercase dark:text-slate-400">Secret</p>
            <p class="mt-1 font-mono text-sm break-all">{twoFactorSecret}</p>
            <p class="mt-3 text-xs text-slate-500 dark:text-slate-400">
              otpauth URI: <span class="break-all font-mono">{twoFactorUri}</span>
            </p>
            <div class="mt-4 flex flex-wrap items-end gap-3">
              <div class="w-40">
                <label class="label" for="totp-code">Authenticator code</label>
                <input
                  id="totp-code"
                  class="input tracking-[0.4em]"
                  inputmode="numeric"
                  bind:value={twoFactorCode}
                  placeholder="000000"
                />
              </div>
              <button class="btn-primary" onclick={enableTwoFactor} disabled={twoFactorCode.length !== 6}>
                Enable
              </button>
            </div>
          </div>
        {:else}
          <button class="btn-secondary" onclick={startTwoFactor}>
            <KeyRound size={15} />
            Set up two-factor authentication
          </button>
        {/if}
      {:else}
        <button class="btn-secondary" onclick={disableTwoFactor}>Disable two-factor authentication</button>
      {/if}

      {#if twoFactorMessage}
        <p class="text-sm text-emerald-600 dark:text-emerald-400">{twoFactorMessage}</p>
      {/if}
      {#if twoFactorError}
        <p class="text-sm text-rose-600 dark:text-rose-400">{twoFactorError}</p>
      {/if}
    </div>
  </Card>

  <Card title={$t('settings.update')} subtitle={$t('settings.updateSubtitle')}>
    <div class="space-y-4">
      <div class="flex flex-wrap items-end gap-3">
        <div class="w-44">
          <label class="label" for="update-channel">{$t('settings.channel')}</label>
          <select id="update-channel" class="input" bind:value={updateChannel}>
            <option value="stable">stable</option>
            <option value="beta">beta</option>
          </select>
        </div>
        <button class="btn-secondary" onclick={checkForUpdates} disabled={updateBusy}>
          {$t('settings.check')}
        </button>
        {#if updateCheck?.update_available}
          <button class="btn-primary" onclick={applyUpdate} disabled={updateBusy}>
            {$t('settings.apply')}
          </button>
        {/if}
        {#if updateStatus}
          <span class="text-sm text-slate-500 dark:text-slate-400">
            v{updateStatus.current_version}
          </span>
        {/if}
      </div>

      {#if updateMessage}
        <p class="text-sm text-emerald-600 dark:text-emerald-400">{updateMessage}</p>
      {/if}
      {#if updateError}
        <p class="text-sm text-rose-600 dark:text-rose-400">{updateError}</p>
      {/if}

      {#if updateStatus && updateStatus.history.length > 0}
        <ul class="divide-y divide-slate-100 text-sm dark:divide-slate-800">
          {#each updateStatus.history as entry (entry.id)}
            <li class="flex items-center justify-between gap-3 py-2">
              <span>
                {entry.from_version} → {entry.to_version}
                {#if entry.error}<span class="block text-xs text-rose-600 dark:text-rose-400">{entry.error}</span>{/if}
              </span>
              <span class="text-xs text-slate-500 dark:text-slate-400">{entry.status}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  </Card>

  <Card title="API keys" subtitle="scoped bearer tokens for the REST API">
    <div class="space-y-4">
      <form
        class="grid gap-3 sm:grid-cols-3"
        onsubmit={(event) => {
          event.preventDefault();
          createApiKey();
        }}
      >
        <div>
          <label class="label" for="key-name">Name</label>
          <input id="key-name" class="input" bind:value={keyName} placeholder="CI deploy" required />
        </div>
        <div>
          <label class="label" for="key-scopes">Scopes</label>
          <input id="key-scopes" class="input" bind:value={keyScopes} placeholder="read,write" />
        </div>
        <div class="flex items-end">
          <button class="btn-primary" type="submit" disabled={!keyName.trim()}>
            <Plus size={15} />
            Create key
          </button>
        </div>
      </form>

      {#if createdToken}
        <div class="rounded-lg bg-emerald-50 p-3 text-sm text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300">
          <p class="font-medium">Copy this token now — it is shown once.</p>
          <pre class="mt-1 overflow-x-auto font-mono text-xs">{createdToken}</pre>
        </div>
      {/if}
      {#if keyError}
        <p class="text-sm text-rose-600 dark:text-rose-400">{keyError}</p>
      {/if}

      {#if apiKeys.length === 0}
        <p class="text-sm text-slate-500 dark:text-slate-400">No API keys yet.</p>
      {:else}
        <ul class="divide-y divide-slate-100 dark:divide-slate-800">
          {#each apiKeys as key (key.id)}
            <li class="flex items-center justify-between gap-3 py-2 text-sm">
              <div>
                <p class="font-medium">{key.name}</p>
                <p class="font-mono text-xs text-slate-500 dark:text-slate-400">
                  {key.prefix}… · {(key.scopes ?? []).join(', ') || 'read'}
                </p>
              </div>
              <button class="btn-secondary px-2.5 text-rose-600 dark:text-rose-400" aria-label="Revoke {key.name}" onclick={() => deleteApiKey(key)}>
                <Trash2 size={15} />
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  </Card>
</div>

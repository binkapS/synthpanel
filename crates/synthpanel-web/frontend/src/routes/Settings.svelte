<script lang="ts">
  import { onMount } from 'svelte';
  import { KeyRound, Save, ShieldCheck } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import { session, type SessionState } from '../lib/stores';
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

  onMount(async () => {
    try {
      settings = await api<Settings>('/api/v1/settings');
      hostname = String(settings.hostname ?? '');
      timezone = String(settings.timezone ?? 'UTC');
      locale = String(settings.locale ?? 'en');
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to load settings';
    }
  });

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
      const me = await api<SessionState>('/api/v1/auth/me');
      session.set(me);
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
      const me = await api<SessionState>('/api/v1/auth/me');
      session.set(me);
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
          <input id="locale" class="input" bind:value={locale} placeholder="en" />
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
</div>

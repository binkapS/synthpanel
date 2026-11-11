<script lang="ts">
  import { onMount } from 'svelte';
  import { api, ApiError } from '../lib/api';
  import { t } from '../lib/i18n';
  import { fetchMe, session } from '../lib/stores';

  interface LoginResponse {
    requires_2fa: boolean;
    challenge_id?: string;
  }

  let email = $state('');
  let password = $state('');
  let code = $state('');
  let challengeId = $state<string | null>(null);
  let error = $state('');
  let busy = $state(false);

  onMount(() => {
    const stored = localStorage.getItem('synthpanel-email');
    if (stored) email = stored;
  });

  async function submit() {
    error = '';
    busy = true;
    try {
      if (challengeId) {
        const result = await api<LoginResponse>('/api/v1/auth/2fa/verify', {
          method: 'POST',
          body: JSON.stringify({ challenge_id: challengeId, code }),
        });
        void result;
        challengeId = null;
      } else {
        const result = await api<LoginResponse>('/api/v1/auth/login', {
          method: 'POST',
          body: JSON.stringify({ email, password }),
        });
        if (result.requires_2fa) {
          challengeId = result.challenge_id ?? null;
          code = '';
          busy = false;
          return;
        }
      }
      localStorage.setItem('synthpanel-email', email);
      const user = await fetchMe();
      session.set({ user });
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to sign in';
    } finally {
      busy = false;
    }
  }
</script>

<div class="flex min-h-full items-center justify-center bg-slate-50 px-4 dark:bg-slate-950">
  <div class="w-full max-w-sm">
    <div class="mb-8 text-center">
      <div
        class="mx-auto mb-4 grid size-12 place-items-center rounded-xl bg-blue-600 text-base font-bold text-white"
      >
        SP
      </div>
      <h1 class="text-xl font-semibold">{$t('login.title')}</h1>
      <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
        {$t('login.subtitle')}
      </p>
    </div>

    <form class="card space-y-4 p-6" onsubmit={(event) => { event.preventDefault(); submit(); }}>
      {#if challengeId}
        <div>
          <label class="label" for="code">{$t('login.code')}</label>
          <input
            id="code"
            class="input tracking-[0.4em]"
            inputmode="numeric"
            autocomplete="one-time-code"
            placeholder="000000"
            bind:value={code}
            required
          />
          <p class="mt-1.5 text-xs text-slate-500 dark:text-slate-400">
            {$t('login.codeHint')}
          </p>
        </div>
      {:else}
        <div>
          <label class="label" for="email">{$t('login.email')}</label>
          <input
            id="email"
            class="input"
            type="email"
            autocomplete="username"
            bind:value={email}
            required
          />
        </div>
        <div>
          <label class="label" for="password">{$t('login.password')}</label>
          <input
            id="password"
            class="input"
            type="password"
            autocomplete="current-password"
            bind:value={password}
            required
          />
        </div>
      {/if}

      {#if error}
        <p class="rounded-lg bg-rose-50 px-3 py-2 text-sm text-rose-700 dark:bg-rose-950/60 dark:text-rose-300">
          {error}
        </p>
      {/if}

      <button class="btn-primary w-full" type="submit" disabled={busy}>
        {busy ? 'Please wait…' : challengeId ? 'Verify' : $t('login.submit')}
      </button>

      {#if challengeId}
        <button
          type="button"
          class="w-full text-center text-xs text-slate-500 hover:text-slate-700 dark:text-slate-400"
          onclick={() => { challengeId = null; code = ''; error = ''; }}
        >
          Use a different account
        </button>
      {/if}
    </form>

    <p class="mt-6 text-center text-xs text-slate-400">
      SynthPanel · self-hosted server control panel
    </p>
  </div>
</div>

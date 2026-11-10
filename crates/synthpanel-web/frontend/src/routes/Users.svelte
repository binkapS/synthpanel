<script lang="ts">
  import { onMount } from 'svelte';
  import { UserPlus, Users as UsersIcon } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import { formatDateTime } from '../lib/format';
  import Card from '../lib/components/Card.svelte';
  import StatusBadge from '../lib/components/StatusBadge.svelte';

  interface User {
    id: string;
    email: string;
    name: string;
    role: string;
    status: string;
    totp_enabled: boolean;
    quota_sites: number;
    quota_dbs: number;
    last_login_at: string | null;
    created_at: string;
  }

  let users = $state<User[]>([]);
  let error = $state('');
  let status = $state('');
  let showForm = $state(false);
  let email = $state('');
  let name = $state('');
  let password = $state('');
  let role = $state('operator');
  let busy = $state(false);

  async function load() {
    try {
      const result = await api<{ users: User[] }>('/api/v1/users');
      users = result.users ?? [];
      error = '';
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to load users';
    }
  }

  onMount(load);

  async function createUser() {
    busy = true;
    error = '';
    status = '';
    try {
      await api('/api/v1/users', {
        method: 'POST',
        body: JSON.stringify({ email, name, password, role }),
      });
      status = `Created ${email}`;
      email = '';
      name = '';
      password = '';
      showForm = false;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to create the user';
    } finally {
      busy = false;
    }
  }
</script>

<div class="space-y-6">
  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-2xl font-semibold">Users</h1>
      <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
        Panel accounts and their roles.
      </p>
    </div>
    <button class="btn-primary" onclick={() => (showForm = !showForm)}>
      <UserPlus size={15} />
      {showForm ? 'Cancel' : 'Add user'}
    </button>
  </div>

  {#if showForm}
    <Card title="New user">
      <form
        class="space-y-4"
        onsubmit={(event) => {
          event.preventDefault();
          createUser();
        }}
      >
        <div class="grid gap-4 sm:grid-cols-2">
          <div>
            <label class="label" for="new-email">Email</label>
            <input id="new-email" class="input" type="email" bind:value={email} required />
          </div>
          <div>
            <label class="label" for="new-name">Name</label>
            <input id="new-name" class="input" bind:value={name} />
          </div>
        </div>
        <div class="grid gap-4 sm:grid-cols-2">
          <div>
            <label class="label" for="new-password">Password</label>
            <input
              id="new-password"
              class="input"
              type="password"
              autocomplete="new-password"
              bind:value={password}
              required
              minlength="12"
            />
            <p class="mt-1 text-xs text-slate-500 dark:text-slate-400">At least 12 characters.</p>
          </div>
          <div>
            <label class="label" for="new-role">Role</label>
            <select id="new-role" class="input" bind:value={role}>
              <option value="admin">Admin</option>
              <option value="operator">Operator</option>
              <option value="user">User</option>
            </select>
          </div>
        </div>
        <button class="btn-primary" type="submit" disabled={busy}>
          {busy ? 'Creating…' : 'Create user'}
        </button>
      </form>
    </Card>
  {/if}

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

  <Card title="Accounts" subtitle="{users.length} user(s)">
    {#if users.length === 0}
      <div class="flex items-center gap-2 text-sm text-slate-500 dark:text-slate-400">
        <UsersIcon size={16} /> No users found.
      </div>
    {:else}
      <div class="overflow-x-auto">
        <table class="w-full text-left text-sm">
          <thead class="text-xs text-slate-500 uppercase dark:text-slate-400">
            <tr>
              <th class="py-2 pr-4 font-medium">User</th>
              <th class="py-2 pr-4 font-medium">Role</th>
              <th class="py-2 pr-4 font-medium">Status</th>
              <th class="py-2 pr-4 font-medium">2FA</th>
              <th class="py-2 font-medium">Last login</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100 dark:divide-slate-800">
            {#each users as user (user.id)}
              <tr>
                <td class="py-2 pr-4">
                  <div class="font-medium">{user.name}</div>
                  <div class="text-xs text-slate-500 dark:text-slate-400">{user.email}</div>
                </td>
                <td class="py-2 pr-4 capitalize">{user.role}</td>
                <td class="py-2 pr-4"><StatusBadge status={user.status} /></td>
                <td class="py-2 pr-4">{user.totp_enabled ? 'on' : 'off'}</td>
                <td class="py-2 text-xs text-slate-500 dark:text-slate-400">
                  {formatDateTime(user.last_login_at)}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </Card>
</div>

<script lang="ts">
  import { onMount } from 'svelte';
  import { currentPath } from './lib/router';
  import { loadSession, loading, session } from './lib/stores';
  import AppShell from './lib/components/AppShell.svelte';
  import Alerts from './routes/Alerts.svelte';
  import Backups from './routes/Backups.svelte';
  import Cron from './routes/Cron.svelte';
  import Deployments from './routes/Deployments.svelte';
  import Dashboard from './routes/Dashboard.svelte';
  import Databases from './routes/Databases.svelte';
  import Files from './routes/Files.svelte';
  import Login from './routes/Login.svelte';
  import Logs from './routes/Logs.svelte';
  import Placeholder from './routes/Placeholder.svelte';
  import Security from './routes/Security.svelte';
  import Settings from './routes/Settings.svelte';
  import SiteDetailRoute from './routes/SiteDetailRoute.svelte';
  import Sites from './routes/Sites.svelte';
  import Terminal from './routes/Terminal.svelte';
  import Users from './routes/Users.svelte';

  const route = $derived.by(() => {
    const path = $currentPath;
    if (path.startsWith('/sites/')) {
      return SiteDetailRoute;
    }
    switch (path) {
      case '/':
        return Dashboard;
      case '/sites':
        return Sites;
      case '/settings':
        return Settings;
      case '/users':
        return Users;
      case '/databases':
        return Databases;
      case '/security':
        return Security;
      case '/backups':
        return Backups;
      case '/cron':
        return Cron;
      case '/logs':
        return Logs;
      case '/files':
        return Files;
      case '/alerts':
        return Alerts;
      case '/deployments':
        return Deployments;
      case '/terminal':
        return Terminal;
      case '/php':
        return Placeholder;
      default:
        return Placeholder;
    }
  });

  onMount(async () => {
    await loadSession();
    loading.set(false);
  });
</script>

{#if $loading}
  <div class="grid h-full place-items-center bg-slate-50 dark:bg-slate-950">
    <div class="flex items-center gap-3 text-sm text-slate-500 dark:text-slate-400">
      <span class="size-4 animate-spin rounded-full border-2 border-slate-300 border-t-blue-600"></span>
      Loading SynthPanel…
    </div>
  </div>
{:else if !$session}
  <Login />
{:else}
  <AppShell>
    {#key $currentPath}
      {@const Component = route}
      <Component />
    {/key}
  </AppShell>
{/if}

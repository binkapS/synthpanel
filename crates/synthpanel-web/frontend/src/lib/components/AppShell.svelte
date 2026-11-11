<script lang="ts">
  import type { Snippet } from 'svelte';
  import {
    Activity,
    Archive,
    BellRing,
    Clock,
    Database,
    FolderTree,
    GitBranch,
    Globe,
    LayoutDashboard,
    LogOut,
    Moon,
    ScrollText,
    Server,
    Settings,
    ShieldCheck,
    Sun,
    TerminalSquare,
    Users,
  } from '@lucide/svelte';
  import { t } from '../i18n';
  import { currentPath, navigate } from '../router';
  import { logout, session, theme, toggleTheme } from '../stores';

  let { children }: { children: Snippet } = $props();

  const links = [
    { href: '/', labelKey: 'nav.dashboard', icon: LayoutDashboard },
    { href: '/sites', labelKey: 'nav.sites', icon: Globe },
    { href: '/databases', labelKey: 'nav.databases', icon: Database },
    { href: '/php', labelKey: 'nav.php', icon: Server },
    { href: '/security', labelKey: 'nav.security', icon: ShieldCheck },
    { href: '/backups', labelKey: 'nav.backups', icon: Archive },
    { href: '/cron', labelKey: 'nav.cron', icon: Clock },
    { href: '/logs', labelKey: 'nav.logs', icon: ScrollText },
    { href: '/files', labelKey: 'nav.files', icon: FolderTree },
    { href: '/deployments', labelKey: 'nav.deployments', icon: GitBranch },
    { href: '/alerts', labelKey: 'nav.alerts', icon: BellRing },
    { href: '/terminal', labelKey: 'nav.terminal', icon: TerminalSquare },
    { href: '/users', labelKey: 'nav.users', icon: Users },
    { href: '/settings', labelKey: 'nav.settings', icon: Settings },
  ];

</script>

<a
  href="#main"
  class="sr-only rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white focus:not-sr-only focus:absolute focus:top-3 focus:left-3 focus:z-50"
>{$t('nav.skip')}</a>

<div class="flex min-h-full">
  <aside
    class="hidden w-60 shrink-0 border-r border-slate-200 bg-white lg:block dark:border-slate-800 dark:bg-slate-900"
  >
    <div class="flex h-16 items-center gap-2 border-b border-slate-200 px-5 dark:border-slate-800">
      <div class="grid size-8 place-items-center rounded-lg bg-blue-600 text-sm font-bold text-white">
        SP
      </div>
      <span class="text-base font-semibold">SynthPanel</span>
    </div>
    <nav class="space-y-1 p-3" aria-label={$t('nav.main')}>
      {#each links as link (link.href)}
        <a
          href={link.href}
          class="nav-link {$currentPath === link.href ? 'nav-link-active' : ''}"
          aria-current={$currentPath === link.href ? 'page' : undefined}
          onclick={(event) => {
            event.preventDefault();
            navigate(link.href);
          }}
        >
          <link.icon size={16} />
          {$t(link.labelKey)}
        </a>
      {/each}
    </nav>
  </aside>

  <div class="flex min-w-0 flex-1 flex-col">
    <header
      class="sticky top-0 z-10 flex h-16 items-center justify-between gap-4 border-b border-slate-200 bg-white/90 px-4 backdrop-blur sm:px-6 dark:border-slate-800 dark:bg-slate-900/90"
    >
      <div class="flex items-center gap-2 lg:hidden">
        <div class="grid size-8 place-items-center rounded-lg bg-blue-600 text-xs font-bold text-white">
          SP
        </div>
        <span class="font-semibold">SynthPanel</span>
      </div>

      <div class="ml-auto flex items-center gap-2">
        <button
          class="btn-secondary px-2.5"
          onclick={toggleTheme}
          aria-label="Toggle colour scheme"
          title="Toggle colour scheme"
        >
          {#if $theme === 'dark'}
            <Sun size={16} />
          {:else}
            <Moon size={16} />
          {/if}
        </button>
        {#if $session}
          <div class="hidden items-center gap-2 sm:flex">
            <span class="text-sm text-slate-500 dark:text-slate-400">{$session.user.name}</span>
            <span class="rounded-full bg-slate-100 px-2 py-0.5 text-xs font-medium capitalize dark:bg-slate-800">
              {$session.user.role}
            </span>
          </div>
          <button class="btn-secondary px-2.5" onclick={logout} aria-label="Sign out" title="Sign out">
            <LogOut size={16} />
          </button>
        {/if}
      </div>
    </header>

    <main id="main" tabindex="-1" class="flex-1 px-4 py-6 sm:px-6">
      <div class="mx-auto w-full max-w-6xl">
        <div class="mb-6 flex items-center gap-2 text-sm text-slate-500 dark:text-slate-400">
          <Activity size={14} />
          <span>{$t('app.tagline')}</span>
        </div>
        {@render children()}
      </div>
    </main>
  </div>
</div>

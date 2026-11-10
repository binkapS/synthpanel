<script lang="ts">
  import type { Snippet } from 'svelte';
  import {
    Activity,
    Archive,
    Clock,
    Database,
    FolderTree,
    Globe,
    LayoutDashboard,
    LogOut,
    Moon,
    ScrollText,
    Server,
    Settings,
    ShieldCheck,
    Sun,
    Users,
  } from '@lucide/svelte';
  import { currentPath, navigate } from '../router';
  import { logout, session, theme, toggleTheme } from '../stores';

  let { children }: { children: Snippet } = $props();

  const links = [
    { href: '/', label: 'Dashboard', icon: LayoutDashboard },
    { href: '/sites', label: 'Sites', icon: Globe },
    { href: '/databases', label: 'Databases', icon: Database },
    { href: '/php', label: 'PHP', icon: Server },
    { href: '/security', label: 'Security', icon: ShieldCheck },
    { href: '/backups', label: 'Backups', icon: Archive },
    { href: '/cron', label: 'Cron', icon: Clock },
    { href: '/logs', label: 'Logs', icon: ScrollText },
    { href: '/files', label: 'Files', icon: FolderTree },
    { href: '/users', label: 'Users', icon: Users },
    { href: '/settings', label: 'Settings', icon: Settings },
  ];

</script>

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
    <nav class="space-y-1 p-3">
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
          {link.label}
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

    <main class="flex-1 px-4 py-6 sm:px-6">
      <div class="mx-auto w-full max-w-6xl">
        <div class="mb-6 flex items-center gap-2 text-sm text-slate-500 dark:text-slate-400">
          <Activity size={14} />
          <span>Live server overview</span>
        </div>
        {@render children()}
      </div>
    </main>
  </div>
</div>

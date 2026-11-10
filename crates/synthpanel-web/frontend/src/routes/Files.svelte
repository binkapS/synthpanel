<script lang="ts">
  import { onMount } from 'svelte';
  import { Download, File, Folder, FolderPlus, Save, Trash2, Upload, X } from '@lucide/svelte';
  import { api, ApiError } from '../lib/api';
  import { formatBytes, formatDateTime } from '../lib/format';
  import Card from '../lib/components/Card.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';

  interface Entry {
    name: string;
    path: string;
    is_directory: boolean;
    size: number;
    modified: string | null;
    mode: string;
    owner: string;
    group: string;
  }

  interface Listing {
    path: string;
    parent: string | null;
    entries: Entry[];
  }

  let scope = $state('www');
  let path = $state('');
  let listing = $state<Listing | null>(null);
  let loading = $state(false);
  let error = $state('');
  let status = $state('');

  let editing = $state<string | null>(null);
  let editorContent = $state('');
  let editorEncoding = $state('utf-8');

  let fileInput: HTMLInputElement | null = null;

  async function load(target = path) {
    loading = true;
    error = '';
    try {
      const params = new URLSearchParams({ scope, path: target });
      listing = await api<Listing>(`/api/v1/files?${params.toString()}`);
      path = listing.path;
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to list the directory';
      listing = null;
    } finally {
      loading = false;
    }
  }

  onMount(() => load(''));

  function open(entry: Entry) {
    if (entry.is_directory) {
      load(entry.path);
    } else {
      edit(entry);
    }
  }

  async function edit(entry: Entry) {
    error = '';
    try {
      const params = new URLSearchParams({ scope, path: entry.path });
      const result = await api<{ content: string; encoding: string }>(
        `/api/v1/files/content?${params.toString()}`,
      );
      editing = entry.path;
      editorContent = result.content;
      editorEncoding = result.encoding;
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to read the file';
    }
  }

  async function save() {
    if (!editing) return;
    error = '';
    status = '';
    try {
      const body: Record<string, unknown> = { scope, path: editing };
      if (editorEncoding === 'base64') body.content_base64 = editorContent;
      else body.content = editorContent;
      await api('/api/v1/files/content', { method: 'PUT', body: JSON.stringify(body) });
      status = `Saved ${editing}.`;
      editing = null;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to save the file';
    }
  }

  async function createFolder() {
    const name = prompt('New folder name');
    if (!name) return;
    const target = path ? `${path}/${name}` : name;
    try {
      await api('/api/v1/files/dir', {
        method: 'POST',
        body: JSON.stringify({ scope, path: target }),
      });
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to create the folder';
    }
  }

  async function remove(entry: Entry) {
    if (!confirm(`Delete ${entry.name}?`)) return;
    const params = new URLSearchParams({ scope, path: entry.path });
    try {
      await api(`/api/v1/files?${params.toString()}`, { method: 'DELETE' });
      status = `Deleted ${entry.name}.`;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to delete the entry';
    }
  }

  async function rename(entry: Entry) {
    const name = prompt('New name', entry.name);
    if (!name || name === entry.name) return;
    const parent = entry.path.includes('/') ? entry.path.slice(0, entry.path.lastIndexOf('/')) : '';
    const target = parent ? `${parent}/${name}` : name;
    try {
      await api('/api/v1/files/rename', {
        method: 'POST',
        body: JSON.stringify({ scope, from: entry.path, to: target }),
      });
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to rename the entry';
    }
  }

  async function chmod(entry: Entry) {
    const mode = prompt(`Permissions for ${entry.name} (octal)`, entry.mode);
    if (!mode) return;
    try {
      await api('/api/v1/files/permissions', {
        method: 'PATCH',
        body: JSON.stringify({ scope, path: entry.path, mode }),
      });
      status = `Updated permissions on ${entry.name}.`;
      await load();
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to change permissions';
    }
  }

  async function upload(files: FileList | null) {
    if (!files || files.length === 0) return;
    error = '';
    status = '';
    for (const file of Array.from(files)) {
      const buffer = await file.arrayBuffer();
      let binary = '';
      const bytes = new Uint8Array(buffer);
      for (const byte of bytes) binary += String.fromCharCode(byte);
      const encoded = btoa(binary);
      const target = path ? `${path}/${file.name}` : file.name;
      try {
        await api('/api/v1/files/content', {
          method: 'PUT',
          body: JSON.stringify({ scope, path: target, content_base64: encoded }),
        });
      } catch (err) {
        error = err instanceof ApiError ? err.message : `Unable to upload ${file.name}`;
        return;
      }
    }
    status = `Uploaded ${files.length} file(s).`;
    await load();
  }

  async function download(entry: Entry) {
    const params = new URLSearchParams({ scope, path: entry.path });
    try {
      const result = await api<{ content: string; encoding: string }>(
        `/api/v1/files/content?${params.toString()}`,
      );
      const blob =
        result.encoding === 'base64'
          ? new Blob([Uint8Array.from(atob(result.content), (c) => c.charCodeAt(0))])
          : new Blob([result.content], { type: 'text/plain' });
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement('a');
      anchor.href = url;
      anchor.download = entry.name;
      anchor.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      error = err instanceof ApiError ? err.message : 'Unable to download the file';
    }
  }

  function breadcrumbs(): { label: string; path: string }[] {
    const parts = path.split('/').filter(Boolean);
    const crumbs = [{ label: scope, path: '' }];
    let current = '';
    for (const part of parts) {
      current = current ? `${current}/${part}` : part;
      crumbs.push({ label: part, path: current });
    }
    return crumbs;
  }
</script>

<div class="space-y-6">
  <div class="flex flex-wrap items-end justify-between gap-3">
    <div>
      <h1 class="text-2xl font-semibold">Files</h1>
      <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
        Browse, edit and upload files inside the managed directories.
      </p>
    </div>
    <div class="flex items-center gap-2">
      <select class="input w-auto" bind:value={scope} onchange={() => load('')}>
        <option value="www">Web roots</option>
        <option value="etc">Panel config</option>
        <option value="backups">Backups</option>
        <option value="logs">Logs</option>
      </select>
      <button class="btn-secondary" onclick={createFolder}>
        <FolderPlus size={15} />
        Folder
      </button>
      <button class="btn-primary" onclick={() => fileInput?.click()}>
        <Upload size={15} />
        Upload
      </button>
      <input
        bind:this={fileInput}
        type="file"
        multiple
        class="hidden"
        onchange={(event) => upload((event.currentTarget as HTMLInputElement).files)}
      />
    </div>
  </div>

  {#if error}
    <p class="rounded-lg bg-rose-50 px-4 py-3 text-sm text-rose-700 dark:bg-rose-950/60 dark:text-rose-300">{error}</p>
  {/if}
  {#if status}
    <p class="rounded-lg bg-emerald-50 px-4 py-3 text-sm text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300">{status}</p>
  {/if}

  {#if editing}
    <Card title="Editing {editing}" subtitle={editorEncoding === 'base64' ? 'binary file' : 'text file'}>
      {#snippet actions()}
        <button class="btn-primary" onclick={save}><Save size={15} /> Save</button>
        <button class="btn-secondary" onclick={() => (editing = null)}><X size={15} /> Close</button>
      {/snippet}
      <textarea class="input min-h-80 font-mono text-xs" bind:value={editorContent} spellcheck="false"></textarea>
    </Card>
  {/if}

  <Card title="Directory" subtitle={path || '/'} padded={false}>
    <div class="flex flex-wrap items-center gap-1 border-b border-slate-200 px-4 py-2 text-sm dark:border-slate-800">
      {#each breadcrumbs() as crumb, index (crumb.path)}
        {#if index > 0}<span class="text-slate-400">/</span>{/if}
        <button class="rounded px-1.5 py-0.5 hover:bg-slate-100 dark:hover:bg-slate-800" onclick={() => load(crumb.path)}>
          {crumb.label}
        </button>
      {/each}
    </div>

    {#if loading}
      <p class="p-4 text-sm text-slate-500 dark:text-slate-400">Loading…</p>
    {:else if !listing || listing.entries.length === 0}
      <div class="p-4">
        <EmptyState title="Empty directory" description="Upload a file or create a folder.">
          {#snippet icon()}<Folder size={28} />{/snippet}
        </EmptyState>
      </div>
    {:else}
      <div class="overflow-x-auto">
        <table class="w-full text-left text-sm">
          <thead class="text-xs text-slate-500 uppercase dark:text-slate-400">
            <tr>
              <th class="px-4 py-2 font-medium">Name</th>
              <th class="py-2 pr-4 font-medium">Size</th>
              <th class="py-2 pr-4 font-medium">Mode</th>
              <th class="py-2 pr-4 font-medium">Owner</th>
              <th class="py-2 pr-4 font-medium">Modified</th>
              <th class="py-2 pr-4 font-medium"></th>
            </tr>
          </thead>
          <tbody class="divide-y divide-slate-100 dark:divide-slate-800">
            {#each listing.entries as entry (entry.path)}
              <tr class="hover:bg-slate-50 dark:hover:bg-slate-900/60">
                <td class="px-4 py-2">
                  <button class="flex items-center gap-2 font-medium" onclick={() => open(entry)}>
                    {#if entry.is_directory}
                      <Folder size={15} class="text-amber-500" />
                    {:else}
                      <File size={15} class="text-slate-400" />
                    {/if}
                    {entry.name}
                  </button>
                </td>
                <td class="py-2 pr-4">{entry.is_directory ? '–' : formatBytes(entry.size)}</td>
                <td class="py-2 pr-4 font-mono text-xs">{entry.mode}</td>
                <td class="py-2 pr-4 text-xs">{entry.owner}:{entry.group}</td>
                <td class="py-2 pr-4 text-xs">{formatDateTime(entry.modified)}</td>
                <td class="py-2 pr-4">
                  <div class="flex justify-end gap-1">
                    <button class="btn-secondary px-2" title="Rename" onclick={() => rename(entry)}>Rename</button>
                    <button class="btn-secondary px-2" title="Permissions" onclick={() => chmod(entry)}>Mode</button>
                    {#if !entry.is_directory}
                      <button class="btn-secondary px-2.5" title="Download" onclick={() => download(entry)}>
                        <Download size={14} />
                      </button>
                    {/if}
                    <button class="btn-secondary px-2.5 text-rose-600 dark:text-rose-400" title="Delete" onclick={() => remove(entry)}>
                      <Trash2 size={14} />
                    </button>
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  </Card>
</div>

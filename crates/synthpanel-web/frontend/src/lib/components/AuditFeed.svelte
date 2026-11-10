<script lang="ts">
  import { relativeTime } from '../format';

  interface AuditEntry {
    id: number;
    actor_id: string | null;
    actor_name: string | null;
    action: string;
    target: string | null;
    metadata_json: string;
    ip: string | null;
    created_at: string;
  }

  let { entries = [] }: { entries?: AuditEntry[] } = $props();
</script>

{#if entries.length === 0}
  <p class="text-sm text-slate-500 dark:text-slate-400">No activity recorded yet.</p>
{:else}
  <ol class="space-y-3">
    {#each entries as entry (entry.id)}
      <li class="flex items-start gap-3">
        <span class="mt-1.5 size-1.5 shrink-0 rounded-full bg-blue-500"></span>
        <div class="min-w-0 flex-1">
          <p class="truncate text-sm">
            <span class="font-medium">{entry.action}</span>
            {#if entry.target}
              <span class="text-slate-500 dark:text-slate-400"> · {entry.target}</span>
            {/if}
          </p>
          <p class="text-xs text-slate-500 dark:text-slate-400">
            {entry.actor_name || entry.actor_id || 'system'}
            {#if entry.ip} · {entry.ip}{/if} · {relativeTime(entry.created_at)}
          </p>
        </div>
      </li>
    {/each}
  </ol>
{/if}

<script lang="ts">
  let { status }: { status: string } = $props();

  const styles: Record<string, string> = {
    active: 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300',
    running: 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300',
    ok: 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300',
    enabled: 'bg-emerald-50 text-emerald-700 dark:bg-emerald-950/60 dark:text-emerald-300',
    inactive: 'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300',
    stopped: 'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300',
    disabled: 'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300',
    failed: 'bg-rose-50 text-rose-700 dark:bg-rose-950/60 dark:text-rose-300',
    error: 'bg-rose-50 text-rose-700 dark:bg-rose-950/60 dark:text-rose-300',
    unknown: 'bg-amber-50 text-amber-700 dark:bg-amber-950/60 dark:text-amber-300',
  };

  const dotStyles: Record<string, string> = {
    active: 'bg-emerald-500',
    running: 'bg-emerald-500',
    ok: 'bg-emerald-500',
    enabled: 'bg-emerald-500',
    inactive: 'bg-slate-400',
    stopped: 'bg-slate-400',
    disabled: 'bg-slate-400',
    failed: 'bg-rose-500',
    error: 'bg-rose-500',
    unknown: 'bg-amber-500',
  };
</script>

<span
  class="inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-xs font-medium {styles[status] ??
    'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300'}"
>
  <span class="size-1.5 rounded-full {dotStyles[status] ?? 'bg-slate-400'}"></span>
  {status}
</span>

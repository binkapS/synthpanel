<script lang="ts">
  let {
    title = 'Coming soon',
    description = 'This area is not implemented yet.',
    milestone = 'a later release',
  }: {
    title?: string;
    description?: string;
    milestone?: string;
  } = $props();
</script>

<div class="space-y-6">
  <div>
    <h1 class="text-2xl font-semibold">{title}</h1>
    <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">{description}</p>
  </div>

  <div class="card p-6">
    <p class="text-sm font-medium">Planned for {milestone}</p>
    <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">
      This area is wired up in a later phase of the SynthPanel roadmap. The REST API and the
      privileged core already expose the primitives it will build on.
    </p>
  </div>
</div>

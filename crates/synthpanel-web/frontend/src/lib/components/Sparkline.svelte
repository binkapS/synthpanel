<script lang="ts">
  let {
    values = [],
    height = 56,
    stroke = 'currentColor',
    fill = 'none',
  }: {
    values?: number[];
    height?: number;
    stroke?: string;
    fill?: string;
  } = $props();

  const width = 300;

  const points = $derived.by(() => {
    if (values.length < 2) return [];
    const max = Math.max(...values);
    const min = Math.min(...values);
    const span = max - min || 1;
    return values.map((value, index) => ({
      x: (index / (values.length - 1)) * width,
      y: height - ((value - min) / span) * (height - 6) - 3,
    }));
  });

  const path = $derived(
    points.map((point, index) => `${index === 0 ? 'M' : 'L'}${point.x.toFixed(2)},${point.y.toFixed(2)}`).join(' '),
  );

  const area = $derived(
    points.length > 0 ? `${path} L${width},${height} L0,${height} Z` : '',
  );
</script>

{#if points.length > 1}
  <svg
    viewBox="0 0 {width} {height}"
    preserveAspectRatio="none"
    class="w-full text-blue-600 dark:text-blue-400"
    style="height: {height}px"
    role="img"
    aria-label="trend chart"
  >
    {#if fill !== 'none'}
      <path d={area} fill="currentColor" opacity="0.08" />
    {/if}
    <path
      d={path}
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      vector-effect="non-scaling-stroke"
    />
  </svg>
{:else}
  <div
    class="flex items-center justify-center rounded-lg border border-dashed border-slate-300 text-xs text-slate-400 dark:border-slate-700"
    style="height: {height}px"
  >
    not enough data
  </div>
{/if}

<script lang="ts">
  import type { MetricCardProps } from '$lib/types';

  let { title, value, unit = '', trend, status = 'neutral' }: MetricCardProps = $props();

  const statusColors = {
    success: 'bg-green-500/10 border-green-500/20 text-green-400',
    warning: 'bg-yellow-500/10 border-yellow-500/20 text-yellow-400',
    error: 'bg-red-500/10 border-red-500/20 text-red-400',
    neutral: 'bg-gray-800/50 border-gray-700/50 text-gray-300'
  };

  const hasTrend = $derived(trend !== undefined);
  const trendIcon = $derived(hasTrend 
    ? trend > 0 ? '↑' : trend < 0 ? '↓' : '→'
    : ''
  );
  const trendColor = $derived(hasTrend
    ? trend > 0 ? 'text-green-400' : trend < 0 ? 'text-red-400' : 'text-gray-400'
    : ''
  );
</script>

<div class="metric-card {statusColors[status]} border rounded-lg p-4 transition-all hover:scale-105">
  <div class="flex justify-between items-start mb-2">
    <h3 class="text-sm font-medium opacity-80">{title}</h3>
    {#if hasTrend}
      <span class="text-xs {trendColor} flex items-center gap-1">
        {trendIcon} {hasTrend ? Math.abs(trend) : 0}%
      </span>
    {/if}
  </div>
  <div class="flex items-baseline gap-1">
    <span class="text-2xl font-bold">{value}</span>
    {#if unit}
      <span class="text-sm opacity-60">{unit}</span>
    {/if}
  </div>
</div>

<style>
  .metric-card {
    backdrop-filter: blur(10px);
  }
</style>
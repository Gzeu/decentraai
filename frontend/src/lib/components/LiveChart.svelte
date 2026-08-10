<script lang="ts">
  import type { ChartDataPoint } from '$lib/types';

  let { 
    data, 
    title, 
    color = 'blue',
    unit = '' 
  }: { 
    data: ChartDataPoint[]; 
    title: string;
    color?: 'blue' | 'green' | 'yellow' | 'red' | 'purple' | 'cyan';
    unit?: string;
  } = $props();

  const colors = {
    blue: '#3b82f6',
    green: '#22c55e',
    yellow: '#eab308',
    red: '#ef4444',
    purple: '#a855f7',
    cyan: '#06b6d4'
  };

  const chartColor = $derived(colors[color]);
  
  // Calculate SVG path
  const chartPath = $derived(data.length > 1 
    ? data.map((point, i) => {
        const x = (i / (data.length - 1)) * 100;
        const maxVal = Math.max(...data.map(d => d.value));
        const minVal = Math.min(...data.map(d => d.value));
        const range = maxVal - minVal || 1;
        const y = 100 - ((point.value - minVal) / range) * 100;
        return `${x},${y}`;
      }).join(' L ')
    : '');
  
  const areaPath = $derived(data.length > 1
    ? `M 0,100 L ${chartPath} L 100,100 Z`
    : '');
</script>

<div class="live-chart">
  <h3 class="text-sm font-medium text-gray-400 mb-3">{title}</h3>
  <div class="h-48 relative">
    {#if data.length > 0}
      <svg viewBox="0 0 100 100" preserveAspectRatio="none" class="w-full h-full">
        <!-- Grid lines -->
        <line x1="0" y1="25" x2="100" y2="25" stroke="rgba(255,255,255,0.1)" stroke-width="0.5" />
        <line x1="0" y1="50" x2="100" y2="50" stroke="rgba(255,255,255,0.1)" stroke-width="0.5" />
        <line x1="0" y1="75" x2="100" y2="75" stroke="rgba(255,255,255,0.1)" stroke-width="0.5" />
        
        <!-- Area fill -->
        <path d={areaPath} fill={chartColor} fill-opacity="0.2" />
        
        <!-- Line -->
        <path d={`M ${chartPath}`} fill="none" stroke={chartColor} stroke-width="1" />
        
        <!-- Data points -->
        {#each data as point, i}
          <circle 
            cx={(i / (data.length - 1)) * 100} 
            cy={100 - ((point.value - Math.min(...data.map(d => d.value))) / (Math.max(...data.map(d => d.value)) - Math.min(...data.map(d => d.value)) || 1)) * 100}
            r="1" 
            fill={chartColor}
          />
        {/each}
      </svg>
      
      <!-- Tooltip on hover -->
      <div class="absolute bottom-2 right-2 text-xs text-gray-400">
        Latest: {data[data.length - 1]?.value.toFixed(2)}{unit}
      </div>
    {:else}
      <div class="flex items-center justify-center h-full text-gray-500">
        No data available
      </div>
    {/if}
  </div>
</div>

<style>
  .live-chart {
    background: rgba(17, 24, 39, 0.8);
    border-radius: 0.5rem;
    padding: 1rem;
    backdrop-filter: blur(10px);
  }
</style>
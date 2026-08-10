<script lang="ts">
  let { 
    value, 
    size = 120, 
    strokeWidth = 8,
    color = 'blue' 
  }: { 
    value: number; 
    size?: number; 
    strokeWidth?: number;
    color?: 'blue' | 'green' | 'yellow' | 'red' | 'purple' | 'cyan';
  } = $props();

  const normalizedValue = $derived(Math.min(100, Math.max(0, value)));
  const radius = $derived((size - strokeWidth) / 2);
  const circumference = $derived(radius * 2 * Math.PI);
  const offset = $derived(circumference - (normalizedValue / 100) * circumference);

  const colors = {
    blue: '#3b82f6',
    green: '#22c55e',
    yellow: '#eab308',
    red: '#ef4444',
    purple: '#a855f7',
    cyan: '#06b6d4'
  };

  const strokeColor = $derived(colors[color]);
</script>

<div class="progress-ring" style="width: {size}px; height: {size}px;">
  <svg
    width={size}
    height={size}
    viewBox={`0 0 ${size} ${size}`}
    class="transform -rotate-90"
  >
    <!-- Background circle -->
    <circle
      cx={size / 2}
      cy={size / 2}
      r={radius}
      fill="none"
      stroke="rgba(255, 255, 255, 0.1)"
      stroke-width={strokeWidth}
    />
    <!-- Progress circle -->
    <circle
      cx={size / 2}
      cy={size / 2}
      r={radius}
      fill="none"
      stroke={strokeColor}
      stroke-width={strokeWidth}
      stroke-linecap="round"
      stroke-dasharray={circumference}
      stroke-dashoffset={offset}
      class="transition-all duration-500 ease-out"
    />
  </svg>
  <div class="absolute inset-0 flex items-center justify-center">
    <span class="text-lg font-bold text-white">{normalizedValue}%</span>
  </div>
</div>

<style>
  .progress-ring {
    position: relative;
  }
</style>
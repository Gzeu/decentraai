<script lang="ts">
  import type { WorkerTableRow } from '$lib/types';
  import StatusBadge from './StatusBadge.svelte';

  let { data }: { data: WorkerTableRow[] } = $props();
</script>

<div class="data-table">
  <table class="w-full">
    <thead>
      <tr class="border-b border-gray-700">
        <th class="text-left py-3 px-4 text-sm font-medium text-gray-400">Worker ID</th>
        <th class="text-left py-3 px-4 text-sm font-medium text-gray-400">Status</th>
        <th class="text-left py-3 px-4 text-sm font-medium text-gray-400">Load</th>
        <th class="text-left py-3 px-4 text-sm font-medium text-gray-400">Uptime</th>
        <th class="text-left py-3 px-4 text-sm font-medium text-gray-400">Tasks</th>
        <th class="text-left py-3 px-4 text-sm font-medium text-gray-400">Last Seen</th>
      </tr>
    </thead>
    <tbody>
      {#each data as row}
        <tr class="border-b border-gray-800 hover:bg-gray-800/50 transition-colors">
          <td class="py-3 px-4 text-sm text-gray-300 font-mono">{row.worker_id}</td>
          <td class="py-3 px-4">
            <StatusBadge status={row.status} />
          </td>
          <td class="py-3 px-4 text-sm text-gray-300">{row.load_percent.toFixed(1)}%</td>
          <td class="py-3 px-4 text-sm text-gray-300">{row.uptime_percent.toFixed(1)}%</td>
          <td class="py-3 px-4 text-sm text-gray-300">
            <span class="text-green-400">{row.tasks_completed}</span>
            {#if row.tasks_failed > 0}
              <span class="text-red-400 ml-2">({row.tasks_failed} failed)</span>
            {/if}
          </td>
          <td class="py-3 px-4 text-sm text-gray-400">{new Date(row.last_seen).toLocaleString()}</td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .data-table {
    background: rgba(17, 24, 39, 0.8);
    border-radius: 0.5rem;
    overflow: hidden;
    backdrop-filter: blur(10px);
  }
</style>
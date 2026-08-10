<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import Navigation from '$lib/components/Navigation.svelte';
  import {
    dashboardData,
    isLoading,
    error,
    isConnected,
    systemMetrics,
    networkMetrics,
    modelsMetrics,
    workersMetrics,
    performanceMetrics,
    cpuHistory,
    memoryHistory,
    latencyHistory,
    throughputHistory,
    updateHistory,
    resetDashboard
  } from '$lib/stores/dashboard';
  import MetricCard from '$lib/components/MetricCard.svelte';
  import StatusBadge from '$lib/components/StatusBadge.svelte';
  import ProgressRing from '$lib/components/ProgressRing.svelte';
  import DataTable from '$lib/components/DataTable.svelte';
  import LiveChart from '$lib/components/LiveChart.svelte';

  // Disable SSR for this page
  export const ssr = false;

  let refreshTimer: number;
  let eventSource: EventSource | null = null;

  // Auto-refresh function
  async function fetchMetrics() {
    try {
      isLoading.set(true);
      const response = await fetch('/api/metrics');
      
      if (!response.ok) {
        throw new Error(`HTTP error! status: ${response.status}`);
      }
      
      const data = await response.json();
      dashboardData.set(data);
      error.set(null);
      
      // Update chart history
      const timestamp = new Date().toISOString();
      if (data.system) {
        updateHistory(cpuHistory, data.system.cpu_usage_percent, timestamp);
        updateHistory(memoryHistory, data.system.memory_usage_percent, timestamp);
      }
      if (data.network) {
        updateHistory(latencyHistory, data.network.latency_avg_ms, timestamp);
      }
      if (data.performance) {
        updateHistory(throughputHistory, data.performance.throughput_rps, timestamp);
      }
    } catch (err) {
      error.set(err instanceof Error ? err.message : 'Failed to fetch metrics');
      console.error('Error fetching metrics:', err);
    } finally {
      isLoading.set(false);
    }
  }

  // Server-Sent Events connection for real-time updates
  function connectSSE() {
    eventSource = new EventSource('/api/metrics/ws');

    eventSource.onopen = () => {
      isConnected.set(true);
      console.log('SSE connected');
    };

    eventSource.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        dashboardData.set(data);
        
        // Update chart history
        const timestamp = new Date().toISOString();
        if (data.system) {
          updateHistory(cpuHistory, data.system.cpu_usage_percent, timestamp);
          updateHistory(memoryHistory, data.system.memory_usage_percent, timestamp);
        }
        if (data.network) {
          updateHistory(latencyHistory, data.network.latency_avg_ms, timestamp);
        }
        if (data.performance) {
          updateHistory(throughputHistory, data.performance.throughput_rps, timestamp);
        }
      } catch (err) {
        console.error('Error parsing SSE message:', err);
      }
    };

    eventSource.onerror = (err) => {
      console.error('SSE error:', err);
      isConnected.set(false);
      eventSource?.close();
      // Attempt to reconnect after 5 seconds
      setTimeout(connectSSE, 5000);
    };
  }

  onMount(() => {
    // Initial fetch
    fetchMetrics();
    
    // Set up auto-refresh (fallback if SSE fails)
    refreshTimer = window.setInterval(fetchMetrics, 5000);
    
    // Try SSE connection
    connectSSE();
  });

  onDestroy(() => {
    if (refreshTimer) {
      clearInterval(refreshTimer);
    }
    if (eventSource) {
      eventSource.close();
    }
    resetDashboard();
  });

  // Command palette keyboard shortcut (CMD+K)
  function handleKeyPress(event: KeyboardEvent) {
    if ((event.metaKey || event.ctrlKey) && event.key === 'k') {
      event.preventDefault();
      // Command palette implementation
      console.log('Command palette activated');
    }
  }
</script>

<svelte:window on:keydown={handleKeyPress} />

<Navigation currentPage="monitoring" />

<div class="min-h-screen bg-gray-950 text-gray-100 p-6">
  <!-- Header -->
  <header class="mb-8">
    <div class="flex justify-between items-center">
      <div>
        <h1 class="text-3xl font-bold bg-gradient-to-r from-blue-400 to-purple-500 bg-clip-text text-transparent">
          DecentraAI Monitoring
        </h1>
        <p class="text-gray-400 mt-1">Real-time system metrics and performance insights</p>
      </div>
      <div class="flex items-center gap-4">
        <div class="flex items-center gap-2">
          <div class="w-2 h-2 rounded-full {$isConnected ? 'bg-green-500' : 'bg-red-500'} animate-pulse"></div>
          <span class="text-sm text-gray-400">
            {$isConnected ? 'Live' : 'Reconnecting...'}
          </span>
        </div>
        <div class="text-sm text-gray-400">
          Last updated: {$dashboardData?.generated_at ? new Date($dashboardData.generated_at).toLocaleString() : 'Loading...'}
        </div>
      </div>
    </div>
  </header>

  {#if $error}
    <div class="bg-red-500/20 border border-red-500/30 rounded-lg p-4 mb-6">
      <p class="text-red-400">Error: {$error}</p>
    </div>
  {/if}

  {#if $isLoading && !$dashboardData}
    <div class="flex items-center justify-center h-64">
      <div class="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-500"></div>
    </div>
  {:else}
    <!-- System Health Section -->
    <section class="mb-8">
      <h2 class="text-xl font-semibold mb-4 text-gray-300">System Health</h2>
      <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        {#if $systemMetrics}
          <MetricCard 
            title="CPU Usage" 
            value={$systemMetrics.cpu_usage_percent.toFixed(1)} 
            unit="%" 
            status={$systemMetrics.cpu_usage_percent > 80 ? 'warning' : 'success'}
          />
          <MetricCard 
            title="Memory" 
            value={$systemMetrics.memory_usage_percent.toFixed(1)} 
            unit="%" 
            status={$systemMetrics.memory_usage_percent > 85 ? 'warning' : 'success'}
          />
          <MetricCard 
            title="GPU VRAM" 
            value={$systemMetrics.gpu_vram_percent.toFixed(1)} 
            unit="%" 
            status={$systemMetrics.gpu_vram_percent > 90 ? 'warning' : 'success'}
          />
          <MetricCard 
            title="Disk I/O" 
            value={$systemMetrics.disk_usage_percent.toFixed(1)} 
            unit="%" 
            status="neutral"
          />
        {/if}
      </div>
    </section>

    <!-- Network Metrics Section -->
    <section class="mb-8">
      <h2 class="text-xl font-semibold mb-4 text-gray-300">Network Metrics</h2>
      <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        {#if $networkMetrics}
          <MetricCard 
            title="P2P Connections" 
            value={$networkMetrics.p2p_connections_active} 
            unit="active"
            trend={12}
          />
          <MetricCard 
            title="Latency (avg)" 
            value={$networkMetrics.latency_avg_ms.toFixed(0)} 
            unit="ms"
            trend={-15}
          />
          <MetricCard 
            title="Bandwidth In" 
            value={$networkMetrics.bandwidth_in_mbps.toFixed(0)} 
            unit="Mbps"
            trend={20}
          />
          <MetricCard 
            title="Bandwidth Out" 
            value={$networkMetrics.bandwidth_out_mbps.toFixed(0)} 
            unit="Mbps"
            trend={-5}
          />
        {/if}
      </div>
    </section>

    <!-- Charts Section -->
    <section class="mb-8">
      <h2 class="text-xl font-semibold mb-4 text-gray-300">Performance Charts</h2>
      <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <LiveChart 
          data={$cpuHistory} 
          title="CPU Usage Over Time" 
          color="blue"
          unit="%" 
        />
        <LiveChart 
          data={$memoryHistory} 
          title="Memory Usage Over Time" 
          color="purple"
          unit="%" 
        />
        <LiveChart 
          data={$latencyHistory} 
          title="Network Latency" 
          color="yellow"
          unit="ms" 
        />
        <LiveChart 
          data={$throughputHistory} 
          title="Request Throughput" 
          color="green"
          unit="req/s" 
        />
      </div>
    </section>

    <!-- Model Performance Section -->
    <section class="mb-8">
      <h2 class="text-xl font-semibold mb-4 text-gray-300">Model Performance</h2>
      <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        {#if $modelsMetrics}
          <MetricCard 
            title="Active Models" 
            value={$modelsMetrics.models_active} 
            unit="total"
          />
          <MetricCard 
            title="Tokens/sec" 
            value={$modelsMetrics.avg_tokens_per_sec.toFixed(0)} 
            unit="t/s"
            trend={45}
          />
          <MetricCard 
            title="Avg Latency" 
            value={$performanceMetrics?.latency_p50_ms.toFixed(0) || 0} 
            unit="ms"
            trend={-20}
          />
          <MetricCard 
            title="Queue Depth" 
            value={$performanceMetrics?.queue_depth || 0} 
            unit="pending"
            trend={3}
          />
        {/if}
      </div>
    </section>

    <!-- Worker Status Section -->
    <section class="mb-8">
      <h2 class="text-xl font-semibold mb-4 text-gray-300">Worker Status</h2>
      <div class="grid grid-cols-1 lg:grid-cols-2 gap-6">
        <!-- Worker Metrics -->
        <div class="bg-gray-900/50 border border-gray-800 rounded-lg p-6">
          <div class="grid grid-cols-2 gap-4 mb-6">
            {#if $workersMetrics}
              <div class="text-center">
                <div class="text-3xl font-bold text-blue-400">{$workersMetrics.workers_active}</div>
                <div class="text-sm text-gray-400">Active Workers</div>
              </div>
              <div class="text-center">
                <div class="text-3xl font-bold text-green-400">{$workersMetrics.workers_busy}</div>
                <div class="text-sm text-gray-400">Busy Workers</div>
              </div>
              <div class="text-center">
                <div class="text-3xl font-bold text-gray-400">{$workersMetrics.workers_offline}</div>
                <div class="text-sm text-gray-400">Offline Workers</div>
              </div>
              <div class="text-center">
                <div class="text-3xl font-bold text-purple-400">{$workersMetrics.total_tasks_completed}</div>
                <div class="text-sm text-gray-400">Tasks Completed</div>
              </div>
            {/if}
          </div>
          
          <!-- Progress Rings -->
          <div class="flex justify-around">
            {#if $workersMetrics}
              <ProgressRing 
                value={$workersMetrics.avg_load_percent} 
                color="blue"
              />
              <ProgressRing 
                value={$workersMetrics.avg_uptime_percent} 
                color="green"
              />
            {/if}
          </div>
        </div>

        <!-- Worker Table -->
        <div>
          {#if $workersMetrics?.workers}
            <DataTable data={$workersMetrics.workers} />
          {:else}
            <div class="bg-gray-900/50 border border-gray-800 rounded-lg p-6 text-center text-gray-400">
              No workers available
            </div>
          {/if}
        </div>
      </div>
    </section>

    <!-- Request Metrics Section -->
    <section class="mb-8">
      <h2 class="text-xl font-semibold mb-4 text-gray-300">Request Metrics (24h)</h2>
      <div class="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        {#if $performanceMetrics}
          <MetricCard 
            title="Total Requests" 
            value={$performanceMetrics.requests_total.toLocaleString()} 
            trend={1234}
          />
          <MetricCard 
            title="Success Rate" 
            value={$performanceMetrics.success_rate_percent.toFixed(1)} 
            unit="%"
            trend={0.3}
            status={$performanceMetrics.success_rate_percent > 99 ? 'success' : 'warning'}
          />
          <MetricCard 
            title="P50 Latency" 
            value={$performanceMetrics.latency_p50_ms.toFixed(0)} 
            unit="ms"
            trend={-12}
          />
          <MetricCard 
            title="P99 Latency" 
            value={$performanceMetrics.latency_p99_ms.toFixed(0)} 
            unit="ms"
            trend={45}
            status={$performanceMetrics.latency_p99_ms > 300 ? 'warning' : 'success'}
          />
        {/if}
      </div>
    </section>
  {/if}
</div>
import type { RequestHandler } from './$types';
import type { DashboardData } from '$lib/types';

// Mock data generator for WebSocket updates
function generateMockData(): DashboardData {
  const now = new Date();
  const baseCPU = 45 + Math.random() * 10 - 5;
  const baseMemory = 62 + Math.random() * 5 - 2.5;
  const baseLatency = 85 + Math.random() * 20 - 10;
  const baseThroughput = 12 + Math.random() * 3 - 1.5;

  return {
    system: {
      cpu_usage_percent: baseCPU,
      memory_usage_percent: baseMemory,
      memory_total_gb: 32.0,
      gpu_vram_percent: 78 + Math.random() * 5 - 2.5,
      gpu_vram_total_gb: 24.0,
      disk_usage_percent: 23 + Math.random() * 2 - 1,
      disk_total_gb: 512.0,
      uptime_seconds: Math.floor((Date.now() - new Date().setHours(0, 0, 0, 0)) / 1000),
      load_average_1m: 2 + Math.random() * 0.5 - 0.25,
      load_average_5m: 1.8 + Math.random() * 0.4 - 0.2,
      load_average_15m: 1.5 + Math.random() * 0.3 - 0.15
    },
    network: {
      p2p_connections_active: 45 + Math.floor(Math.random() * 5 - 2),
      p2p_connections_total: 59,
      latency_avg_ms: baseLatency,
      latency_p95_ms: baseLatency * 1.4,
      latency_p99_ms: baseLatency * 4,
      bandwidth_in_mbps: 125 + Math.random() * 10 - 5,
      bandwidth_out_mbps: 98 + Math.random() * 8 - 4,
      packet_loss_percent: 0.1 + Math.random() * 0.05,
      errors_total: 23 + Math.floor(Math.random() * 2)
    },
    models: {
      models_active: 6,
      models_total: 12,
      models_by_type: [
        ['llm', 3],
        ['embedding', 2],
        ['vision', 1]
      ],
      models_loaded: [
        {
          name: 'tinyllama-1.1b',
          version: '1.0.0',
          model_type: 'llm',
          quantization: 'q4_k_m',
          vram_usage_gb: 2.5,
          context_length: 2048,
          tokens_per_sec: 45 + Math.random() * 5 - 2.5
        },
        {
          name: 'bge-small-en',
          version: '1.0.0',
          model_type: 'embedding',
          quantization: 'q8_0',
          vram_usage_gb: 0.5,
          context_length: 512,
          tokens_per_sec: 120 + Math.random() * 10 - 5
        }
      ],
      total_vram_usage_gb: 3.0,
      avg_tokens_per_sec: 342 + Math.random() * 20 - 10
    },
    workers: {
      workers_total: 4,
      workers_active: 2,
      workers_busy: 1,
      workers_offline: 1,
      workers: [
        {
          worker_id: 'gpu-worker-1',
          status: 'active',
          load_percent: 95 + Math.random() * 10 - 5,
          uptime_percent: 99.8,
          tasks_completed: 1250000 + Math.floor(Math.random() * 100),
          tasks_failed: 12,
          last_seen: now.toISOString()
        },
        {
          worker_id: 'gpu-worker-2',
          status: 'active',
          load_percent: 85 + Math.random() * 10 - 5,
          uptime_percent: 99.5,
          tasks_completed: 875000 + Math.floor(Math.random() * 80),
          tasks_failed: 8,
          last_seen: now.toISOString()
        },
        {
          worker_id: 'cpu-worker-1',
          status: 'busy',
          load_percent: 95 + Math.random() * 10 - 5,
          uptime_percent: 98.2,
          tasks_completed: 620000 + Math.floor(Math.random() * 60),
          tasks_failed: 15,
          last_seen: now.toISOString()
        },
        {
          worker_id: 'worker-4',
          status: 'offline',
          load_percent: 0.0,
          uptime_percent: 0.0,
          tasks_completed: 0,
          tasks_failed: 0,
          last_seen: new Date(Date.now() - 3600000).toISOString()
        }
      ],
      avg_load_percent: 70 + Math.random() * 5 - 2.5,
      avg_uptime_percent: 74.4,
      total_tasks_completed: 2745000 + Math.floor(Math.random() * 300)
    },
    performance: {
      requests_total: 12847 + Math.floor(Math.random() * 50),
      requests_success: 12742 + Math.floor(Math.random() * 45),
      requests_failed: 105 + Math.floor(Math.random() * 5),
      success_rate_percent: 99.2 + Math.random() * 0.2 - 0.1,
      throughput_rps: baseThroughput,
      latency_p50_ms: 95 + Math.random() * 10 - 5,
      latency_p95_ms: 180 + Math.random() * 20 - 10,
      latency_p99_ms: 340 + Math.random() * 30 - 15,
      queue_depth: 10 + Math.floor(Math.random() * 4 - 2),
      queue_avg_wait_ms: 45 + Math.random() * 10 - 5
    },
    generated_at: now.toISOString()
  };
}

export const GET: RequestHandler = async ({ url }) => {
  const headers = new Headers({
    'Content-Type': 'text/event-stream',
    'Cache-Control': 'no-cache',
    'Connection': 'keep-alive'
  });

  let interval: NodeJS.Timeout | null = null;

  const stream = new ReadableStream({
    start(controller) {
      // Send initial data
      try {
        controller.enqueue(new TextEncoder().encode(`data: ${JSON.stringify(generateMockData())}\n\n`));
      } catch (err) {
        console.error('Error sending initial data:', err);
      }

      // Send updates every 5 seconds
      interval = setInterval(() => {
        try {
          const data = generateMockData();
          controller.enqueue(new TextEncoder().encode(`data: ${JSON.stringify(data)}\n\n`));
        } catch (err) {
          console.error('Error sending SSE data:', err);
          if (interval) clearInterval(interval);
        }
      }, 5000);
      
      // Use url parameter to avoid unused variable warning
      url.searchParams.get('stop');
    },
    cancel() {
      if (interval) {
        clearInterval(interval);
      }
    }
  });

  return new Response(stream, { headers });
};
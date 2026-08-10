import { json } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import type { DashboardData } from '$lib/types';

// Mock data matching the Rust monitoring crate structure
const mockDashboardData: DashboardData = {
  system: {
    cpu_usage_percent: 45.2,
    memory_usage_percent: 62.8,
    memory_total_gb: 32.0,
    gpu_vram_percent: 78.5,
    gpu_vram_total_gb: 24.0,
    disk_usage_percent: 23.4,
    disk_total_gb: 512.0,
    uptime_seconds: 3600,
    load_average_1m: 2.1,
    load_average_5m: 1.8,
    load_average_15m: 1.5
  },
  network: {
    p2p_connections_active: 47,
    p2p_connections_total: 59,
    latency_avg_ms: 85.0,
    latency_p95_ms: 120.0,
    latency_p99_ms: 340.0,
    bandwidth_in_mbps: 125.0,
    bandwidth_out_mbps: 98.0,
    packet_loss_percent: 0.1,
    errors_total: 23
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
        tokens_per_sec: 45.2
      },
      {
        name: 'bge-small-en',
        version: '1.0.0',
        model_type: 'embedding',
        quantization: 'q8_0',
        vram_usage_gb: 0.5,
        context_length: 512,
        tokens_per_sec: 120.5
      }
    ],
    total_vram_usage_gb: 3.0,
    avg_tokens_per_sec: 342.0
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
        load_percent: 99.8,
        uptime_percent: 99.8,
        tasks_completed: 1250000,
        tasks_failed: 12,
        last_seen: new Date().toISOString()
      },
      {
        worker_id: 'gpu-worker-2',
        status: 'active',
        load_percent: 87.5,
        uptime_percent: 99.5,
        tasks_completed: 875000,
        tasks_failed: 8,
        last_seen: new Date().toISOString()
      },
      {
        worker_id: 'cpu-worker-1',
        status: 'busy',
        load_percent: 98.2,
        uptime_percent: 98.2,
        tasks_completed: 620000,
        tasks_failed: 15,
        last_seen: new Date().toISOString()
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
    avg_load_percent: 71.4,
    avg_uptime_percent: 74.4,
    total_tasks_completed: 2745000
  },
  performance: {
    requests_total: 12847,
    requests_success: 12742,
    requests_failed: 105,
    success_rate_percent: 99.2,
    throughput_rps: 12.5,
    latency_p50_ms: 95.0,
    latency_p95_ms: 180.0,
    latency_p99_ms: 340.0,
    queue_depth: 12,
    queue_avg_wait_ms: 45.0
  },
  generated_at: new Date().toISOString()
};

export const GET: RequestHandler = async () => {
  // Simulate network delay
  await new Promise(resolve => setTimeout(resolve, 100));
  
  return json(mockDashboardData);
};
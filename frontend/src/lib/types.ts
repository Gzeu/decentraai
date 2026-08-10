// TypeScript types matching the Rust monitoring crate structures
// Based on crates/monitoring/src/dashboards.rs

export interface DashboardData {
  system: SystemDashboard;
  network: NetworkDashboard;
  models: ModelsDashboard;
  workers: WorkersDashboard;
  performance: PerformanceDashboard;
  generated_at: string;
}

export interface SystemDashboard {
  cpu_usage_percent: number;
  memory_usage_percent: number;
  memory_total_gb: number;
  gpu_vram_percent: number;
  gpu_vram_total_gb: number;
  disk_usage_percent: number;
  disk_total_gb: number;
  uptime_seconds: number;
  load_average_1m: number;
  load_average_5m: number;
  load_average_15m: number;
}

export interface NetworkDashboard {
  p2p_connections_active: number;
  p2p_connections_total: number;
  latency_avg_ms: number;
  latency_p95_ms: number;
  latency_p99_ms: number;
  bandwidth_in_mbps: number;
  bandwidth_out_mbps: number;
  packet_loss_percent: number;
  errors_total: number;
}

export interface ModelsDashboard {
  models_active: number;
  models_total: number;
  models_by_type: [string, number][];
  models_loaded: LoadedModel[];
  total_vram_usage_gb: number;
  avg_tokens_per_sec: number;
}

export interface LoadedModel {
  name: string;
  version: string;
  model_type: string;
  quantization: string;
  vram_usage_gb: number;
  context_length: number;
  tokens_per_sec: number;
}

export interface WorkersDashboard {
  workers_total: number;
  workers_active: number;
  workers_busy: number;
  workers_offline: number;
  workers: WorkerStatus[];
  avg_load_percent: number;
  avg_uptime_percent: number;
  total_tasks_completed: number;
}

export interface WorkerStatus {
  worker_id: string;
  status: "active" | "busy" | "offline";
  load_percent: number;
  uptime_percent: number;
  tasks_completed: number;
  tasks_failed: number;
  last_seen: string;
}

export interface PerformanceDashboard {
  requests_total: number;
  requests_success: number;
  requests_failed: number;
  success_rate_percent: number;
  throughput_rps: number;
  latency_p50_ms: number;
  latency_p95_ms: number;
  latency_p99_ms: number;
  queue_depth: number;
  queue_avg_wait_ms: number;
}

// Additional types for UI components
export interface MetricCardProps {
  title: string;
  value: string | number;
  unit?: string;
  trend?: number;
  status?: "success" | "warning" | "error" | "neutral";
}

export interface ChartDataPoint {
  timestamp: string;
  value: number;
}

export interface WorkerTableRow {
  worker_id: string;
  status: WorkerStatus["status"];
  load_percent: number;
  uptime_percent: number;
  tasks_completed: number;
  tasks_failed: number;
  last_seen: string;
}
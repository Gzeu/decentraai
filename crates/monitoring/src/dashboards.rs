//! Dashboard data aggregation

use crate::metrics::MetricsCollector;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardData {
    pub system: SystemDashboard,
    pub network: NetworkDashboard,
    pub models: ModelsDashboard,
    pub workers: WorkersDashboard,
    pub performance: PerformanceDashboard,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemDashboard {
    pub cpu_usage_percent: f64,
    pub memory_usage_percent: f64,
    pub memory_total_gb: f64,
    pub gpu_vram_percent: f64,
    pub gpu_vram_total_gb: f64,
    pub disk_usage_percent: f64,
    pub disk_total_gb: f64,
    pub uptime_seconds: u64,
    pub load_average_1m: f64,
    pub load_average_5m: f64,
    pub load_average_15m: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkDashboard {
    pub p2p_connections_active: u32,
    pub p2p_connections_total: u32,
    pub latency_avg_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_p99_ms: f64,
    pub bandwidth_in_mbps: f64,
    pub bandwidth_out_mbps: f64,
    pub packet_loss_percent: f64,
    pub errors_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsDashboard {
    pub models_active: u32,
    pub models_total: u32,
    pub models_by_type: Vec<(String, u32)>,
    pub models_loaded: Vec<LoadedModel>,
    pub total_vram_usage_gb: f64,
    pub avg_tokens_per_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadedModel {
    pub name: String,
    pub version: String,
    pub model_type: String,
    pub quantization: String,
    pub vram_usage_gb: f64,
    pub context_length: u32,
    pub tokens_per_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkersDashboard {
    pub workers_total: u32,
    pub workers_active: u32,
    pub workers_busy: u32,
    pub workers_offline: u32,
    pub workers: Vec<WorkerStatus>,
    pub avg_load_percent: f64,
    pub avg_uptime_percent: f64,
    pub total_tasks_completed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStatus {
    pub worker_id: String,
    pub status: String, // active, busy, offline
    pub load_percent: f64,
    pub uptime_percent: f64,
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub last_seen: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceDashboard {
    pub requests_total: u64,
    pub requests_success: u64,
    pub requests_failed: u64,
    pub success_rate_percent: f64,
    pub throughput_rps: f64,
    pub latency_p50_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_p99_ms: f64,
    pub queue_depth: u32,
    pub queue_avg_wait_ms: f64,
}

impl DashboardData {
    pub fn from_metrics(metrics: &MetricsCollector, start_time: DateTime<Utc>) -> Self {
        Self {
            system: SystemDashboard {
                cpu_usage_percent: metrics.get_gauge("cpu_usage_percent").unwrap_or(0.0),
                memory_usage_percent: metrics.get_gauge("memory_usage_percent").unwrap_or(0.0),
                memory_total_gb: metrics.get_gauge("memory_total_gb").unwrap_or(0.0),
                gpu_vram_percent: metrics.get_gauge("gpu_vram_percent").unwrap_or(0.0),
                gpu_vram_total_gb: metrics.get_gauge("gpu_vram_total_gb").unwrap_or(0.0),
                disk_usage_percent: metrics.get_gauge("disk_usage_percent").unwrap_or(0.0),
                disk_total_gb: metrics.get_gauge("disk_total_gb").unwrap_or(0.0),
                uptime_seconds: (Utc::now() - start_time).num_seconds() as u64,
                load_average_1m: metrics.get_gauge("load_average_1m").unwrap_or(0.0),
                load_average_5m: metrics.get_gauge("load_average_5m").unwrap_or(0.0),
                load_average_15m: metrics.get_gauge("load_average_15m").unwrap_or(0.0),
            },
            network: NetworkDashboard {
                p2p_connections_active: metrics.get_gauge("p2p_connections_active").unwrap_or(0.0)
                    as u32,
                p2p_connections_total: metrics.get_gauge("p2p_connections_total").unwrap_or(0.0)
                    as u32,
                latency_avg_ms: metrics.get_gauge("latency_avg_ms").unwrap_or(0.0),
                latency_p95_ms: metrics.get_gauge("latency_p95_ms").unwrap_or(0.0),
                latency_p99_ms: metrics.get_gauge("latency_p99_ms").unwrap_or(0.0),
                bandwidth_in_mbps: metrics.get_gauge("bandwidth_in_mbps").unwrap_or(0.0),
                bandwidth_out_mbps: metrics.get_gauge("bandwidth_out_mbps").unwrap_or(0.0),
                packet_loss_percent: metrics.get_gauge("packet_loss_percent").unwrap_or(0.0),
                errors_total: metrics.get_counter("network_errors_total").unwrap_or(0.0) as u64,
            },
            models: ModelsDashboard {
                models_active: metrics.get_gauge("models_active").unwrap_or(0.0) as u32,
                models_total: metrics.get_gauge("models_total").unwrap_or(0.0) as u32,
                models_by_type: vec![
                    (
                        "llm".to_string(),
                        metrics.get_gauge("models_llm").unwrap_or(0.0) as u32,
                    ),
                    (
                        "embedding".to_string(),
                        metrics.get_gauge("models_embedding").unwrap_or(0.0) as u32,
                    ),
                    (
                        "vision".to_string(),
                        metrics.get_gauge("models_vision").unwrap_or(0.0) as u32,
                    ),
                ],
                models_loaded: vec![], // Populated from model registry
                total_vram_usage_gb: metrics.get_gauge("models_vram_total_gb").unwrap_or(0.0),
                avg_tokens_per_sec: metrics
                    .get_gauge("models_avg_tokens_per_sec")
                    .unwrap_or(0.0),
            },
            workers: WorkersDashboard {
                workers_total: metrics.get_gauge("workers_total").unwrap_or(0.0) as u32,
                workers_active: metrics.get_gauge("workers_active").unwrap_or(0.0) as u32,
                workers_busy: metrics.get_gauge("workers_busy").unwrap_or(0.0) as u32,
                workers_offline: metrics.get_gauge("workers_offline").unwrap_or(0.0) as u32,
                workers: vec![], // Populated from worker registry
                avg_load_percent: metrics.get_gauge("workers_avg_load").unwrap_or(0.0),
                avg_uptime_percent: metrics.get_gauge("workers_avg_uptime").unwrap_or(0.0),
                total_tasks_completed: metrics
                    .get_counter("workers_tasks_completed_total")
                    .unwrap_or(0.0) as u64,
            },
            performance: PerformanceDashboard {
                requests_total: metrics.get_counter("requests_total").unwrap_or(0.0) as u64,
                requests_success: metrics.get_counter("requests_success_total").unwrap_or(0.0)
                    as u64,
                requests_failed: metrics.get_counter("requests_failed_total").unwrap_or(0.0) as u64,
                success_rate_percent: metrics.get_gauge("requests_success_rate").unwrap_or(0.0),
                throughput_rps: metrics.get_gauge("requests_throughput_rps").unwrap_or(0.0),
                latency_p50_ms: metrics.get_gauge("requests_latency_p50_ms").unwrap_or(0.0),
                latency_p95_ms: metrics.get_gauge("requests_latency_p95_ms").unwrap_or(0.0),
                latency_p99_ms: metrics.get_gauge("requests_latency_p99_ms").unwrap_or(0.0),
                queue_depth: metrics.get_gauge("queue_depth").unwrap_or(0.0) as u32,
                queue_avg_wait_ms: metrics.get_gauge("queue_avg_wait_ms").unwrap_or(0.0),
            },
            generated_at: Utc::now(),
        }
    }
}

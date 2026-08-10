# Q4c: Monitoring Architecture

## Overview

Sistem complet de monitoring cu date reale și organizare clară pe 5 dashboard-uri.

## Arhitectură

```
┌─────────────────────────────────────────────────────────┐
│                    Monitoring Service                    │
├─────────────────────────────────────────────────────────┤
│                                                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │   Logs       │  │   Metrics    │  │  Dashboards  │  │
│  │  (JSON)      │  │ (Prometheus) │  │   (5 types)  │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
│                                                          │
└─────────────────────────────────────────────────────────┘
```

## 1. Structured Logging (JSON)

### LogEntry Fields

```rust
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,      // Debug, Info, Warn, Error
    pub component: String,    // e.g., "p2p", "worker", "scheduler"
    pub message: String,
    pub context: Value,       // Additional context (JSON)
}
```

### Exemplu

```json
{
  "timestamp": "2026-08-10T15:00:00Z",
  "level": "info",
  "component": "p2p",
  "message": "Connected to peer",
  "context": {
    "peer_id": "16Uiu8gK...",
    "latency_ms": 85
  }
}
```

## 2. Metrics Collection

### Metric Types

- **Counter**: Valori care cresc (requests_total, errors_total)
- **Gauge**: Valori curente (cpu_usage, memory_usage)
- **Histogram**: Distribuții (latency_buckets)

### Exemple de Metrics

```rust
// System metrics
monitoring.record_metric("cpu_usage_percent", 45.5, vec![]).await;
monitoring.record_metric("memory_usage_percent", 62.0, vec![]).await;
monitoring.record_metric("gpu_vram_percent", 78.0, vec![]).await;

// Network metrics
monitoring.record_metric("p2p_connections_active", 47.0, vec![]).await;
monitoring.record_metric("latency_avg_ms", 85.0, vec![]).await;
monitoring.record_metric("bandwidth_in_mbps", 125.0, vec![]).await;

// Model metrics
monitoring.record_metric("models_active", 6.0, vec![]).await;
monitoring.record_metric("models_avg_tokens_per_sec", 342.0, vec![]).await;

// Worker metrics
monitoring.record_metric("workers_active", 12.0, vec![]).await;
monitoring.record_metric("workers_avg_load", 65.0, vec![]).await;

// Performance metrics
monitoring.record_metric("requests_total", 12847.0, vec![]).await;
monitoring.record_metric("requests_latency_p95_ms", 240.0, vec![]).await;
```

## 3. Dashboard-uri (5 categorii)

### 3.1 System Dashboard

**Ce monitorizează:**
- CPU usage (%) - per core
- Memory usage (%) - total / available
- GPU VRAM usage (%) - per GPU
- Disk usage (%) - total / available
- Load average (1m, 5m, 15m)
- Uptime

**Metrics:**
- `cpu_usage_percent`
- `memory_usage_percent`
- `gpu_vram_percent`
- `disk_usage_percent`
- `load_average_1m`
- `uptime_seconds`

### 3.2 Network Dashboard

**Ce monitorizează:**
- P2P connections (active / total)
- Network latency (avg, p95, p99)
- Bandwidth (in/out Mbps)
- Packet loss (%)
- Network errors

**Metrics:**
- `p2p_connections_active`
- `latency_avg_ms`
- `latency_p95_ms`
- `bandwidth_in_mbps`
- `packet_loss_percent`
- `network_errors_total`

### 3.3 Models Dashboard

**Ce monitorizează:**
- Active models count
- Models by type (LLM, Embedding, Vision)
- Loaded model details (version, quantization)
- VRAM usage per model
- Tokens/sec throughput

**Metrics:**
- `models_active`
- `models_total`
- `models_llm`
- `models_embedding`
- `models_avg_tokens_per_sec`
- `models_vram_total_gb`

### 3.4 Workers Dashboard

**Ce monitorizează:**
- Workers total / active / busy / offline
- Per-worker status (load, uptime, tasks)
- Average load %
- Average uptime %
- Total tasks completed

**Metrics:**
- `workers_total`
- `workers_active`
- `workers_busy`
- `workers_avg_load`
- `workers_tasks_completed_total`

### 3.5 Performance Dashboard

**Ce monitorizează:**
- Total requests (24h)
- Success rate (%)
- Throughput (requests/sec)
- Latency (p50, p95, p99)
- Queue depth
- Average wait time

**Metrics:**
- `requests_total`
- `requests_success_rate`
- `requests_throughput_rps`
- `requests_latency_p50_ms`
- `requests_latency_p99_ms`
- `queue_depth`

## 4. Data Aggregation

### Time Resolution

- **High-frequency**: 1s (for real-time charts)
- **Medium-frequency**: 10s (for dashboards)
- **Low-frequency**: 1m (for historical analysis)

### Storage

- **In-memory**: Last 10,000 metrics
- **History**: Last 1 hour at 1s resolution
- **Aggregated**: Last 24h at 1m resolution

## 5. Export & Frontend

### JSON Export

```rust
let monitoring = MonitoringService::new();
let json_data = monitoring.export_json().await;

// Output:
// {
//   "start_time": "2026-08-10T10:00:00Z",
//   "current_time": "2026-08-10T15:00:00Z",
//   "logs": [...],
//   "metrics": [...],
//   "dashboard": {
//     "system": {...},
//     "network": {...},
//     "models": {...},
//     "workers": {...},
//     "performance": {...}
//   }
// }
```

### Frontend Integration

```javascript
// Fetch dashboard data
async function getDashboardData() {
  const response = await fetch('/api/metrics');
  const data = await response.json();
  
  // Update charts
  updateSystemChart(data.dashboard.system);
  updateNetworkChart(data.dashboard.network);
  updateModelsChart(data.dashboard.models);
  updateWorkersChart(data.dashboard.workers);
  updatePerformanceChart(data.dashboard.performance);
}

// Auto-refresh every 5 seconds
setInterval(getDashboardData, 5000);
```

## 6. Alerting

### Thresholds

```rust
// Alert configuration
struct AlertConfig {
    metric: String,
    threshold: f64,
    operator: String,  // "gt", "lt", "eq"
    severity: String,  // "warning", "critical"
}

// Example alerts
let alerts = vec![
    AlertConfig {
        metric: "cpu_usage_percent".to_string(),
        threshold: 80.0,
        operator: "gt".to_string(),
        severity: "warning".to_string(),
    },
    AlertConfig {
        metric: "requests_latency_p99_ms".to_string(),
        threshold: 1000.0,
        operator: "gt".to_string(),
        severity: "critical".to_string(),
    },
    AlertConfig {
        metric: "workers_offline".to_string(),
        threshold: 1.0,
        operator: "gt".to_string(),
        severity: "warning".to_string(),
    },
];
```

## 7. Usage

### Initialize

```rust
use monitoring::MonitoringService;

let monitoring = MonitoringService::new();
```

### Record metrics

```rust
// System metrics
monitoring.record_metric("cpu_usage_percent", 45.5, vec![]).await;
monitoring.record_metric("memory_usage_percent", 62.0, vec![]).await;

// Network metrics
monitoring.record_metric("p2p_connections_active", 47.0, vec![]).await;
monitoring.record_metric("latency_avg_ms", 85.0, vec![]).await;
```

### Get dashboard data

```rust
let dashboard = monitoring.get_metrics().await;
println!("CPU: {}%", dashboard.system.cpu_usage_percent);
println!("Workers: {}/{} active", 
    dashboard.workers.workers_active,
    dashboard.workers.workers_total
);
```

### Export JSON

```rust
let json = monitoring.export_json().await;
std::fs::write("metrics.json", json)?;
```

---

**Implemented**: August 2026  
**Files**: monitoring crate (logs.rs, metrics.rs, dashboards.rs)  
**Lines**: ~800  
**Dashboards**: 5 organized dashboards  
**Metrics**: 30+ real metrics

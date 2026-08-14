//! Structured logging and metrics collection

mod dashboards;
mod logs;
mod metrics;

pub use dashboards::DashboardData;
pub use logs::{LogCollector, LogEntry, LogLevel};
pub use metrics::{Metric, MetricType, MetricsCollector};

use chrono::{DateTime, Utc};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Main monitoring service
pub struct MonitoringService {
    logs: Arc<RwLock<LogCollector>>,
    metrics: Arc<RwLock<MetricsCollector>>,
    start_time: DateTime<Utc>,
}

impl MonitoringService {
    pub fn new() -> Self {
        Self {
            logs: Arc::new(RwLock::new(LogCollector::new())),
            metrics: Arc::new(RwLock::new(MetricsCollector::new())),
            start_time: Utc::now(),
        }
    }

    /// Get structured logs
    pub async fn get_logs(&self, level: Option<LogLevel>, last_n: usize) -> Vec<LogEntry> {
        self.logs.read().await.get_recent(level, last_n)
    }

    /// Get metrics for dashboard
    pub async fn get_metrics(&self) -> DashboardData {
        let metrics = self.metrics.read().await;
        DashboardData::from_metrics(&metrics, self.start_time)
    }

    /// Record a log entry
    pub async fn log(&self, entry: LogEntry) {
        self.logs.write().await.add(entry);
    }

    /// Record a metric
    pub async fn record_metric(&self, name: String, value: f64, labels: Vec<(String, String)>) {
        self.metrics.write().await.record(name, value, labels);
    }

    /// Export all data to JSON
    pub async fn export_json(&self) -> String {
        let logs = self.logs.read().await.get_recent(None, 1000);
        let metrics = self.metrics.read().await.get_all();

        serde_json::json!({
            "start_time": self.start_time.to_rfc3339(),
            "current_time": Utc::now().to_rfc3339(),
            "logs": logs,
            "metrics": metrics,
            "dashboard": self.get_metrics().await
        })
        .to_string()
    }
}

impl Default for MonitoringService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_monitoring_service() {
        let monitoring = MonitoringService::new();

        // Record log
        monitoring
            .log(LogEntry::info("test", "Test message".to_string()))
            .await;

        // Record metric
        monitoring
            .record_metric(
                "cpu_usage_percent".to_string(),
                45.5,
                vec![("host".to_string(), "worker-1".to_string())],
            )
            .await;

        // Get data
        let logs = monitoring.get_logs(None, 10).await;
        assert_eq!(logs.len(), 1);

        let metrics = monitoring.get_metrics().await;
        assert!(metrics.system.cpu_usage_percent > 0.0);
    }
}

//! Metrics collection (Prometheus-style)

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    pub value: f64,
    pub labels: Vec<(String, String)>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum MetricType {
    Counter(f64),
    Gauge(f64),
    Histogram { count: u64, sum: f64, buckets: Vec<(f64, u64)> },
}

pub struct MetricsCollector {
    counters: HashMap<String, f64>,
    gauges: HashMap<String, f64>,
    history: Vec<Metric>,
    max_history: usize,
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
            gauges: HashMap::new(),
            history: Vec::new(),
            max_history: 10000,
        }
    }

    pub fn record(&mut self, name: String, value: f64, labels: Vec<(String, String)>) {
        let key = format!("{}|{:?}", name, labels);
        
        // Update current value with labels
        self.gauges.insert(key.clone(), value);
        
        // Also store by name alone for dashboard queries
        self.gauges.insert(name.clone(), value);
        
        // Add to history
        self.history.push(Metric {
            name,
            value,
            labels,
            timestamp: Utc::now(),
        });
        
        // Trim history
        if self.history.len() > self.max_history {
            self.history.remove(0);
        }
    }

    pub fn increment_counter(&mut self, name: &str, by: f64) {
        let counter = self.counters.entry(name.to_string()).or_insert(0.0);
        *counter += by;
    }

    pub fn set_gauge(&mut self, name: &str, value: f64) {
        self.gauges.insert(name.to_string(), value);
    }

    pub fn get_gauge(&self, name: &str) -> Option<f64> {
        self.gauges.get(name).copied()
    }

    pub fn get_counter(&self, name: &str) -> Option<f64> {
        self.counters.get(name).copied()
    }

    pub fn get_all(&self) -> Vec<Metric> {
        self.history.clone()
    }

    pub fn get_by_name(&self, name: &str, last_n: usize) -> Vec<Metric> {
        let mut metrics: Vec<_> = self.history
            .iter()
            .filter(|m| m.name == name)
            .collect();
        
        metrics.reverse();
        metrics.truncate(last_n);
        metrics.into_iter().cloned().collect()
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

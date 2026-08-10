//! Structured logging

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub component: String,
    pub message: String,
    pub context: serde_json::Value,
}

impl LogEntry {
    pub fn debug(component: &str, message: String) -> Self {
        Self {
            timestamp: Utc::now(),
            level: LogLevel::Debug,
            component: component.to_string(),
            message,
            context: serde_json::Value::Null,
        }
    }

    pub fn info(component: &str, message: String) -> Self {
        Self {
            timestamp: Utc::now(),
            level: LogLevel::Info,
            component: component.to_string(),
            message,
            context: serde_json::Value::Null,
        }
    }

    pub fn warn(component: &str, message: String) -> Self {
        Self {
            timestamp: Utc::now(),
            level: LogLevel::Warn,
            component: component.to_string(),
            message,
            context: serde_json::Value::Null,
        }
    }

    pub fn error(component: &str, message: String) -> Self {
        Self {
            timestamp: Utc::now(),
            level: LogLevel::Error,
            component: component.to_string(),
            message,
            context: serde_json::Value::Null,
        }
    }

    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = context;
        self
    }
}

pub struct LogCollector {
    entries: Vec<LogEntry>,
    max_entries: usize,
}

impl LogCollector {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            max_entries: 10000,
        }
    }

    pub fn add(&mut self, entry: LogEntry) {
        self.entries.push(entry);
        if self.entries.len() > self.max_entries {
            self.entries.remove(0);
        }
    }

    pub fn get_recent(&self, level: Option<LogLevel>, n: usize) -> Vec<LogEntry> {
        let mut filtered: Vec<_> = match level {
            Some(lvl) => self.entries.iter().filter(|e| e.level == lvl).collect(),
            None => self.entries.iter().collect(),
        };
        
        filtered.reverse();
        filtered.truncate(n);
        filtered.into_iter().cloned().collect()
    }

    pub fn get_by_component(&self, component: &str, n: usize) -> Vec<LogEntry> {
        let mut filtered: Vec<_> = self.entries
            .iter()
            .filter(|e| e.component == component)
            .collect();
        
        filtered.reverse();
        filtered.truncate(n);
        filtered.into_iter().cloned().collect()
    }
}

impl Default for LogCollector {
    fn default() -> Self {
        Self::new()
    }
}

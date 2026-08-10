//! Append-only security audit log (M6).
//!
//! One JSON object per line at `<data_dir>/logs/audit.jsonl`. The log
//! records security-relevant events only: peer bans, chunk verification
//! failures, inference admission rejections, and inference starts.
//! Prompts and outputs are never audit material.

use anyhow::{Context, Result};
use serde::Serialize;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// The audit file name inside the logs directory.
pub const AUDIT_FILE_NAME: &str = "audit.jsonl";

#[derive(Debug, Serialize)]
struct AuditEvent<'a> {
    timestamp: u64,
    event: &'a str,
    details: serde_json::Value,
}

/// Appends one event to the audit log, creating the directory and file
/// on first use. Syncs before returning so events survive a crash.
pub fn record(logs_dir: &Path, event: &str, details: serde_json::Value) -> Result<()> {
    std::fs::create_dir_all(logs_dir)?;
    let path = logs_dir.join(AUDIT_FILE_NAME);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening audit log {}", path.display()))?;
    let entry = AuditEvent {
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        event,
        details,
    };
    serde_json::to_writer(&mut file, &entry)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

/// Audit must never break the main flow: failures are logged, not raised.
pub fn record_best_effort(logs_dir: &Path, event: &str, details: serde_json::Value) {
    if let Err(e) = record(logs_dir, event, details) {
        tracing::warn!(error = %e, "failed to write audit log");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_append_as_json_lines() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        record(&logs, "peer_banned", serde_json::json!({"peer": "abc"})).unwrap();
        record(&logs, "inference_started", serde_json::json!({"model": "m.gguf"})).unwrap();

        let content = std::fs::read_to_string(logs.join(AUDIT_FILE_NAME)).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["event"], "peer_banned");
        assert_eq!(first["details"]["peer"], "abc");
        assert!(first["timestamp"].as_u64().unwrap() > 0);
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["event"], "inference_started");
    }
}

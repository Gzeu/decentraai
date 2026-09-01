//! Audit bridge — every `EventBus` event with `priority >= Normal` is
//! also written to the audit log (jsonl). This is Sprint 0.1's
//! minimal dual-write: the event bus is the canonical surface; the
//! audit log is a verifiable index over it.
//!
//! Wiring: `attach_audit_bridge(bus, log_dir)` returns a guard
//! that subscribes to the bus's broadcast channel and writes each
//! event to `<log_dir>/audit.jsonl` (append-only, one JSON object
//! per line). The guard's Drop cancels the subscription.
//!
//! This is a *minimal* implementation: no batching, no fsync, no
//! rotation. The audit crate's `record_best_effort` is the v1
//! primitive; the bridge calls it when an event fires.

use crate::Event;
use crate::EventPriority;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// A single audit record. Mirrors the shape of `crates/audit`'s
/// `AuditEvent` but is duplicated here so the bridge has no
/// dependency on the audit crate (avoids a circular dep: event-bus
/// is the foundation; audit is a sink).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub timestamp: u64,
    pub event_id: String,
    pub event_type: String,
    pub topic: String,
    pub source: String,
    pub priority: String,
    pub details: serde_json::Value,
}

/// Handle returned by `attach_audit_bridge`. Drop cancels the writer.
pub struct AuditBridge {
    /// Path to the audit log file (for inspection).
    pub log_path: PathBuf,
    /// Sender side: dropped by Drop to signal the writer to stop.
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
    /// The writer task.
    handle: Option<JoinHandle<()>>,
}

impl Drop for AuditBridge {
    fn drop(&mut self) {
        if let Some(tx) = self.cancel.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.handle.take() {
            // Best-effort: don't block on shutdown.
            h.abort();
        }
    }
}

/// Subscribe to the bus and write every event with `priority >=
/// Normal` to `<log_dir>/audit.jsonl`. Returns a guard.
pub fn attach_audit_bridge(bus: &crate::EventBus, log_dir: &Path) -> std::io::Result<AuditBridge> {
    std::fs::create_dir_all(log_dir)?;
    let log_path = log_dir.join("audit.jsonl");
    let mut receiver = bus.subscribe_broadcast();

    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    let (tx, mut rx) = mpsc::channel::<AuditRecord>(1024);

    // Forwarder: event-bus broadcast channel -> mpsc.
    let forwarder = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut cancel_rx => break,
                ev = receiver.recv() => {
                    if let Ok(ev) = ev
                        && should_record(&ev) {
                            let rec = record_from_event(&ev);
                            if tx.send(rec).await.is_err() {
                                break;
                            }
                        }
                }
            }
        }
    });

    // Writer: mpsc -> file.
    let log_path_clone = log_path.clone();
    let writer = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        let mut file = match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path_clone)
            .await
        {
            Ok(f) => f,
            Err(_e) => return,
        };
        while let Some(rec) = rx.recv().await {
            if let Ok(line) = serde_json::to_string(&rec) {
                let _ = file.write_all(line.as_bytes()).await;
                let _ = file.write_all(b"\n").await;
                let _ = file.flush().await;
            }
        }
    });

    // Keep the forwarder running until cancellation.
    #[allow(clippy::let_underscore_future)]
    let _ = forwarder;
    Ok(AuditBridge {
        log_path,
        cancel: Some(cancel_tx),
        handle: Some(writer),
    })
}

fn should_record(event: &Event) -> bool {
    event.metadata.priority >= EventPriority::Normal
}

fn record_from_event(event: &Event) -> AuditRecord {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    AuditRecord {
        timestamp: now,
        event_id: event.id.0.clone(),
        event_type: event.event_type.clone(),
        topic: event.topic.0.clone(),
        source: event.source.to_string(),
        priority: format!("{:?}", event.metadata.priority),
        details: event.payload.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Event, EventId, EventMetadata, EventPriority, Topic};
    use decentraai_protocol::AgentId;
    use std::sync::Arc;

    #[tokio::test]
    async fn bridge_writes_events_to_file() {
        let store = Arc::new(crate::InMemoryEventStore::new(1024));
        let bus = crate::EventBus::new(store);
        let dir = tempdir();
        let bridge = attach_audit_bridge(&bus, &dir).unwrap();

        // Publish one event.
        let ev = Event {
            id: EventId::new(),
            topic: Topic::system(),
            source: AgentId::from("test"),
            timestamp: 0,
            event_type: "test.event".to_string(),
            payload: serde_json::json!({"hello": "world"}),
            metadata: EventMetadata {
                priority: EventPriority::Normal,
                ..Default::default()
            },
        };
        bus.publish(ev).await.unwrap();

        // Give the writer a chance to flush.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        drop(bridge);

        let contents = std::fs::read_to_string(dir.join("audit.jsonl")).unwrap();
        assert!(contents.contains("test.event"));
        assert!(contents.contains("hello"));
    }

    #[tokio::test]
    async fn bridge_filters_out_low_priority() {
        let store = Arc::new(crate::InMemoryEventStore::new(1024));
        let bus = crate::EventBus::new(store);
        let dir = tempdir();
        let bridge = attach_audit_bridge(&bus, &dir).unwrap();

        let ev = Event {
            id: EventId::new(),
            topic: Topic::system(),
            source: AgentId::from("test"),
            timestamp: 0,
            event_type: "low.priority".to_string(),
            payload: serde_json::json!({}),
            metadata: EventMetadata {
                priority: EventPriority::Low,
                ..Default::default()
            },
        };
        bus.publish(ev).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        drop(bridge);

        // The file should not exist (or be empty): low-priority events
        // are filtered out.
        let path = dir.join("audit.jsonl");
        if path.exists() {
            let contents = std::fs::read_to_string(&path).unwrap();
            assert!(!contents.contains("low.priority"));
        }
    }

    fn tempdir() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("audit-bridge-{}", rand_suffix()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn rand_suffix() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        format!("{}", nanos)
    }
}

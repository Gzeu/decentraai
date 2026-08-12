use decentraai_protocol::InferMessage;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

/// Tracks in-flight distributed inference conversations by request_id.
/// Coordinators register a receiver for a request_id and the P2P handler
/// delivers incoming InferMessage frames into the corresponding channel.
#[derive(Clone, Debug)]
pub struct RequestTracker {
    inner: Arc<Mutex<HashMap<Uuid, mpsc::UnboundedSender<InferMessage>>>>,
}

impl RequestTracker {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a new channel for request_id. Returns a receiver that the
    /// caller can await on to receive progress and final messages.
    pub async fn register(&self, request_id: Uuid) -> mpsc::UnboundedReceiver<InferMessage> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut guard = self.inner.lock().await;
        guard.insert(request_id, tx);
        rx
    }

    /// Deliver an incoming message to any registered receiver. Returns true
    /// if delivered, false if no receiver exists for the request.
    pub async fn deliver(&self, msg: InferMessage) -> bool {
        let mut guard = self.inner.lock().await;
        let id = msg.request_id();
        if let Some(tx) = guard.get(&id) {
            // ignore send errors (receiver dropped)
            let _ = tx.send(msg);
            true
        } else {
            false
        }
    }

    /// Remove the registration for a request id (cleanup)
    pub async fn remove(&self, request_id: &Uuid) {
        let mut guard = self.inner.lock().await;
        guard.remove(request_id);
    }
}

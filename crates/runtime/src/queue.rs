//! Fair FIFO queue for inference requests (Q2).
//!
//! One request at a time reaches the backend (llama-server runs a
//! single slot on this node); everyone else waits in arrival order,
//! with the serving request and the waiting list visible on the
//! dashboard. A request that finishes, errors, or disconnects releases
//! its slot through the ticket's Drop — the queue can never deadlock
//! behind a vanished client.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

/// Snapshot of the request currently being served.
#[derive(Debug, Clone)]
pub struct ServingSnapshot {
    pub who: String,
    pub endpoint: String,
    pub elapsed_secs: u64,
}

/// Snapshot of one waiting request (its position is its index + 1).
#[derive(Debug, Clone)]
pub struct WaitingSnapshot {
    pub who: String,
    pub endpoint: String,
    pub waited_secs: u64,
}

struct Entry {
    ticket: u64,
    who: String,
    endpoint: String,
    since: Instant,
}

struct State {
    next_ticket: u64,
    serving: Option<Entry>,
    waiting: VecDeque<Entry>,
}

/// The queue itself: shared via Arc, notified on every release.
pub struct InferenceQueue {
    state: Mutex<State>,
    notify: Notify,
    max_waiting: usize,
    wait_timeout: Duration,
}

/// The waiting room is full (config: inference.queue_max_requests).
#[derive(Debug)]
pub struct QueueFull;

/// The request waited longer than the configured timeout.
#[derive(Debug)]
pub struct WaitTimeout;

impl InferenceQueue {
    pub fn new(max_waiting: usize, wait_timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                next_ticket: 1,
                serving: None,
                waiting: VecDeque::new(),
            }),
            notify: Notify::new(),
            max_waiting,
            wait_timeout,
        })
    }

    /// Joins the queue. Fails fast when the waiting room is full.
    pub fn enqueue(self: &Arc<Self>, who: &str, endpoint: &str) -> Result<QueueTicket, QueueFull> {
        let mut state = self.state.lock().unwrap();
        if state.waiting.len() >= self.max_waiting {
            return Err(QueueFull);
        }
        let ticket = state.next_ticket;
        state.next_ticket += 1;
        state.waiting.push_back(Entry {
            ticket,
            who: who.to_string(),
            endpoint: endpoint.to_string(),
            since: Instant::now(),
        });
        Ok(QueueTicket {
            queue: Arc::clone(self),
            ticket,
        })
    }

    /// Resolves when this ticket reaches the head of the queue.
    /// Idempotent: a ticket already being served returns immediately.
    /// Times out after the configured wait limit.
    async fn wait_turn(&self, ticket: u64) -> Result<(), WaitTimeout> {
        let deadline = Instant::now() + self.wait_timeout;
        loop {
            {
                let mut state = self.state.lock().unwrap();
                if state.serving.as_ref().is_some_and(|e| e.ticket == ticket) {
                    return Ok(()); // already promoted
                }
                if state.serving.is_none()
                    && state.waiting.front().is_some_and(|e| e.ticket == ticket)
                {
                    let entry = state.waiting.pop_front().expect("front exists");
                    state.serving = Some(entry);
                    return Ok(());
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero()
                || tokio::time::timeout(remaining, self.notify.notified())
                    .await
                    .is_err()
            {
                return Err(WaitTimeout);
            }
        }
    }

    /// Releases the ticket: frees the serving slot or removes the
    /// waiting entry, then wakes everyone so the head promotes.
    fn release(&self, ticket: u64) {
        {
            let mut state = self.state.lock().unwrap();
            if state.serving.as_ref().is_some_and(|e| e.ticket == ticket) {
                state.serving = None;
            }
            state.waiting.retain(|e| e.ticket != ticket);
        }
        self.notify.notify_waiters();
    }

    /// Dashboard view: who is being served and who waits, in order.
    pub fn snapshot(&self) -> (Option<ServingSnapshot>, Vec<WaitingSnapshot>) {
        let state = self.state.lock().unwrap();
        let serving = state.serving.as_ref().map(|e| ServingSnapshot {
            who: e.who.clone(),
            endpoint: e.endpoint.clone(),
            elapsed_secs: e.since.elapsed().as_secs(),
        });
        let waiting = state
            .waiting
            .iter()
            .map(|e| WaitingSnapshot {
                who: e.who.clone(),
                endpoint: e.endpoint.clone(),
                waited_secs: e.since.elapsed().as_secs(),
            })
            .collect();
        (serving, waiting)
    }

    #[cfg(test)]
    fn waiting_len(&self) -> usize {
        self.state.lock().unwrap().waiting.len()
    }
}

/// RAII handle: dropping releases the slot, whatever happened —
/// success, error, or a client that disconnected mid-wait.
pub struct QueueTicket {
    queue: Arc<InferenceQueue>,
    ticket: u64,
}

impl QueueTicket {
    /// Waits until this ticket is the one being served.
    pub async fn wait_turn(&self) -> Result<(), WaitTimeout> {
        self.queue.wait_turn(self.ticket).await
    }
}

impl Drop for QueueTicket {
    fn drop(&mut self) {
        self.queue.release(self.ticket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn requests_are_served_in_arrival_order() {
        let queue = InferenceQueue::new(10, Duration::from_secs(5));
        let first = queue.enqueue("alice", "/v1/chat/completions").unwrap();
        let second = queue.enqueue("bob", "/v1/chat/completions").unwrap();

        first.wait_turn().await.unwrap();
        let (serving, waiting) = queue.snapshot();
        assert_eq!(serving.unwrap().who, "alice");
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].who, "bob");

        // Bob must not be promoted while Alice is being served.
        let bob = tokio::spawn(async move {
            second.wait_turn().await.unwrap();
            second
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!bob.is_finished(), "second request must wait for the first");

        drop(first); // Alice finishes; the slot releases via Drop.
        let second = bob.await.unwrap();
        second.wait_turn().await.unwrap(); // idempotent: already serving
        let (serving, _) = queue.snapshot();
        assert_eq!(serving.unwrap().who, "bob");
        drop(second);
        assert!(queue.snapshot().0.is_none());
    }

    #[tokio::test]
    async fn full_waiting_room_rejects_newcomers() {
        let queue = InferenceQueue::new(1, Duration::from_secs(5));
        let first = queue.enqueue("a", "/x").unwrap();
        first.wait_turn().await.unwrap(); // now serving
        let _second = queue.enqueue("b", "/x").unwrap(); // waiting room 1/1
        assert!(queue.enqueue("c", "/x").is_err(), "the room is capped");
    }

    #[tokio::test]
    async fn dropping_a_waiting_ticket_frees_the_slot() {
        let queue = InferenceQueue::new(1, Duration::from_secs(5));
        let first = queue.enqueue("a", "/x").unwrap();
        first.wait_turn().await.unwrap();
        let second = queue.enqueue("b", "/x").unwrap();
        assert_eq!(queue.waiting_len(), 1);
        drop(second); // client disconnected before its turn
        assert_eq!(queue.waiting_len(), 0);
        let third = queue.enqueue("c", "/x").unwrap();
        drop(first);
        third.wait_turn().await.unwrap();
        assert_eq!(queue.snapshot().0.unwrap().who, "c");
    }

    #[tokio::test]
    async fn waiting_too_long_times_out() {
        let queue = InferenceQueue::new(10, Duration::from_millis(100));
        let first = queue.enqueue("a", "/x").unwrap();
        first.wait_turn().await.unwrap();
        let second = queue.enqueue("b", "/x").unwrap();
        assert!(second.wait_turn().await.is_err(), "first never finishes");
    }
}

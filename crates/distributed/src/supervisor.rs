//! Heartbeat supervisor for periodic fabric tasks (M24 hardening).
//!
//! Observed failure (2026-09-09): the compute broadcaster died silently —
//! no panic surfaced, no watchdog revived it, and the mesh decayed over ~5h
//! (VPS showed 0 workers with a healthy engine). Pattern going forward:
//! periodic workers record a millisecond beat after every successful
//! iteration; the supervisor respawns the worker when the beat goes stale
//! or the task handle finishes. The staleness rule is pure and unit-tested;
//! the respawn loop is integration-tested with real tasks below.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::task::JoinHandle;

/// Wall-clock milliseconds for heartbeat bookkeeping.
pub fn heartbeat_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Stale after 3 missed beats (saturating arithmetic — clock jumps never
/// panic). `interval_ms == 0` is a caller bug: report not-stale so a
/// misconfigured supervisor cannot spin-respawn.
pub fn is_stale(last_beat_ms: u64, now_ms: u64, interval_ms: u64) -> bool {
    if interval_ms == 0 {
        return false;
    }
    now_ms.saturating_sub(last_beat_ms) > interval_ms.saturating_mul(3)
}

/// Respawn when the worker finished (panic or return — both are silent
/// death for a periodic task) or its beat went stale.
pub fn should_respawn(
    worker_finished: bool,
    last_beat_ms: u64,
    now_ms: u64,
    interval_ms: u64,
) -> bool {
    worker_finished || is_stale(last_beat_ms, now_ms, interval_ms)
}

/// Supervisor for one named periodic worker. The worker closure receives a
/// beat handle to stamp after every successful iteration.
pub struct Supervisor {
    name: &'static str,
    interval_ms: u64,
    beat: Arc<AtomicU64>,
}

impl Supervisor {
    /// Beat starts at construction: a slow first tick is not a failure.
    pub fn new(name: &'static str, interval_ms: u64) -> Self {
        Self {
            name,
            interval_ms,
            beat: Arc::new(AtomicU64::new(heartbeat_now_ms())),
        }
    }

    /// Record a successful iteration (called by the worker, not the loop).
    pub fn record_beat(&self) {
        self.beat.store(heartbeat_now_ms(), Ordering::Relaxed);
    }

    /// Supervise forever: respawn on stale beat or finished handle.
    /// Returns never — the caller `tokio::spawn`s it next to the old fire-
    /// and-forget tasks it replaces.
    pub async fn run(&self, mut spawn_worker: impl FnMut(Arc<AtomicU64>) -> JoinHandle<()>) {
        let mut handle = spawn_worker(Arc::clone(&self.beat));
        let mut interval = tokio::time::interval(Duration::from_millis(self.interval_ms.max(1)));
        loop {
            interval.tick().await;
            let now = heartbeat_now_ms();
            let last = self.beat.load(Ordering::Relaxed);
            if should_respawn(handle.is_finished(), last, now, self.interval_ms) {
                tracing::warn!(
                    task = self.name,
                    last_beat_ms = last,
                    finished = handle.is_finished(),
                    "supervised task stale/finished — respawning"
                );
                handle.abort();
                handle = spawn_worker(Arc::clone(&self.beat));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn stale_after_three_missed_beats() {
        assert!(!is_stale(1000, 1000, 100));
        assert!(!is_stale(1000, 1300, 100));
        assert!(is_stale(1000, 1301, 100));
        assert!(!is_stale(0, 0, 0), "zero interval never respawns");
        assert!(!is_stale(2000, 1000, 100), "clock jump is not stale");
    }

    #[test]
    fn finished_task_always_respawns() {
        assert!(should_respawn(true, 1000, 1000, 100));
        assert!(!should_respawn(false, 1000, 1000, 100));
        assert!(should_respawn(false, 0, 10_000, 100));
    }

    #[tokio::test]
    async fn dead_worker_is_revived() {
        // Worker returns immediately without beating: every supervisor check
        // must respawn it. Assert at least 3 generations appear.
        let generations = Arc::new(AtomicUsize::new(0));
        let sup = Supervisor::new("test-dead", 10);
        let gens = Arc::clone(&generations);
        let run = tokio::spawn(async move {
            sup.run(move |_beat| {
                gens.fetch_add(1, Ordering::Relaxed);
                tokio::spawn(async {})
            })
            .await;
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while generations.load(Ordering::Relaxed) < 3 {
            if tokio::time::Instant::now() > deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        run.abort();
        assert!(
            generations.load(Ordering::Relaxed) >= 3,
            "supervisor must revive a dead worker"
        );
    }

    #[tokio::test]
    async fn healthy_worker_is_left_alone() {
        // Worker beats faster than the staleness horizon: exactly 1 generation.
        let generations = Arc::new(AtomicUsize::new(0));
        let sup = Supervisor::new("test-healthy", 50);
        let gens = Arc::clone(&generations);
        let run = tokio::spawn(async move {
            sup.run(move |beat| {
                gens.fetch_add(1, Ordering::Relaxed);
                tokio::spawn(async move {
                    loop {
                        beat.store(heartbeat_now_ms(), Ordering::Relaxed);
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
            })
            .await;
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        run.abort();
        assert_eq!(
            generations.load(Ordering::Relaxed),
            1,
            "healthy worker must not be respawned"
        );
    }
}

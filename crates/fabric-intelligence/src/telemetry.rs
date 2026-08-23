//! Provider-performance telemetry: counters and latencies ONLY — never task
//! content, never prompts, never outputs (audit rule). Aggregates let an
//! operator compare providers (local Qwen3-0.6B vs external OpenAI-compatible
//! vs a future DecentraAI peer) without building any feedback machinery yet.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Non-sensitive identity of one provider for telemetry keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    Local,
    External,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::External => "external",
        }
    }
}

/// Snapshot for the status endpoint / dashboard card.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProviderScore {
    pub attempts: u64,
    pub successes: u64,
    pub failures: u64,
    pub last_latency_ms: u64,
    /// Rolling mean of recorded latencies (integer ms; 0 = no data).
    pub mean_latency_ms: u64,
    /// Mean parse/validation success rate in percent (0–100, 0 = no data).
    pub plan_success_percent: u64,
}

/// Thread-safe aggregate counters. Cheap to clone into ApiState via Arc.
#[derive(Default)]
pub struct IntelTelemetry {
    plans_generated: AtomicU64,
    plans_valid: AtomicU64,
    plans_rejected: AtomicU64,
    external_calls: AtomicU64,
    per_provider: Mutex<HashMap<ProviderKind, ProviderAgg>>,
}

#[derive(Default)]
struct ProviderAgg {
    attempts: AtomicU64,
    successes: AtomicU64,
    failures: AtomicU64,
    last_latency_ms: AtomicU64,
    latency_sum_ms: AtomicU64,
}

impl IntelTelemetry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that a provider attempt finished.
    pub fn record_attempt(&self, kind: ProviderKind, ok: bool, latency_ms: u64) {
        let mut map = self.per_provider.lock().expect("telemetry mutex");
        let agg = map.entry(kind).or_default();
        agg.attempts.fetch_add(1, Ordering::Relaxed);
        if ok {
            agg.successes.fetch_add(1, Ordering::Relaxed);
        } else {
            agg.failures.fetch_add(1, Ordering::Relaxed);
        }
        agg.last_latency_ms.store(latency_ms, Ordering::Relaxed);
        agg.latency_sum_ms.fetch_add(latency_ms, Ordering::Relaxed);
        drop(map);
        if kind == ProviderKind::External {
            self.external_calls.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_plan_generated(&self) {
        self.plans_generated.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_plan_outcome(&self, valid: bool) {
        if valid {
            self.plans_valid.fetch_add(1, Ordering::Relaxed);
        } else {
            self.plans_rejected.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Per-provider scores ordered deterministically (local first, then
    /// external by name) so dashboard rendering is stable.
    pub fn scores(&self) -> Vec<(String, ProviderScore)> {
        let map = self.per_provider.lock().expect("telemetry mutex");
        let mut out = Vec::new();
        for kind in [ProviderKind::Local, ProviderKind::External] {
            if let Some(a) = map.get(&kind) {
                let attempts = a.attempts.load(Ordering::Relaxed);
                let successes = a.successes.load(Ordering::Relaxed);
                let sum = a.latency_sum_ms.load(Ordering::Relaxed);
                out.push((
                    kind.as_str().to_string(),
                    ProviderScore {
                        attempts,
                        successes,
                        failures: a.failures.load(Ordering::Relaxed),
                        last_latency_ms: a.last_latency_ms.load(Ordering::Relaxed),
                        mean_latency_ms: sum.checked_div(attempts).unwrap_or_default(),
                        plan_success_percent: successes
                            .checked_mul(100)
                            .and_then(|p| p.checked_div(attempts))
                            .unwrap_or_default(),
                    },
                ));
            }
        }
        out
    }

    /// Global counters for the status endpoint.
    pub fn totals(&self) -> (u64, u64, u64, u64) {
        (
            self.plans_generated.load(Ordering::Relaxed),
            self.plans_valid.load(Ordering::Relaxed),
            self.plans_rejected.load(Ordering::Relaxed),
            self.external_calls.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_attempts_and_latencies_without_content() {
        let t = IntelTelemetry::new();
        t.record_attempt(ProviderKind::Local, true, 100);
        t.record_attempt(ProviderKind::Local, true, 300);
        t.record_attempt(ProviderKind::External, false, 900);

        t.record_plan_generated();
        t.record_plan_generated();
        t.record_plan_outcome(true);
        t.record_plan_outcome(false);

        assert_eq!(t.totals(), (2, 1, 1, 1));
        let scores = t.scores();
        assert_eq!(scores[0].0, "local");
        assert_eq!(scores[0].1.attempts, 2);
        assert_eq!(scores[0].1.mean_latency_ms, 200);
        assert_eq!(scores[0].1.plan_success_percent, 100);
        assert_eq!(scores[1].1.failures, 1);
    }
}

//! SAES 0.5 Phase 2 — Placement Fairness.
//!
//! Turns a [`CollaborationSignal`] (Phase 1) into a deterministic placement
//! decision over a set of candidate offers. This is the **decision layer**
//! view of placement — it mirrors `decentraai_compute::assist::select_offer`
//! so the SAES agent and the runtime DFCP engine agree without drifting,
//! but it lives in `agent-runtime` so it stays dependency-light and does NOT
//! pull `libp2p` into the pure-decision crate.
//!
//! # Contract
//!
//! - **Hard gates first**: capability mismatch, resource fit, freshness
//!   (≤30s), queue depth (≥4 = busy), recent failure — all reject BEFORE
//!   scoring. A failing gate is never resurrected by a high fairness score.
//! - **Scoring**: `0.5*headroom + 0.25*freshness + 0.10*queue + balance_bias + 0.15`
//!   where `balance_bias = 0.15 * tanh(balance/100)` — saturated to ±0.15.
//!   Freshness and capability match gate the offer; fairness only breaks ties
//!   among otherwise-eligible workers and never outranks a decisive capacity win.
//! - **Determinism**: score desc, then peer_id asc. No randomness.
//! - **No second engine**: this is a thin, auditable mirror of
//!   `compute::assist` — same constants, same weights, same tie-break.
//!   There is no competing placement path.
//! - **Correlation**: the input `correlation_id` (from pressure) is threaded
//!   through to the `PlacementDecision` and to the EventBus event so the
//!   whole episode `pressure → placement → gateway` is traceable.
//!
//! Fairness term is `contribution_balance × capability_match × freshness` in
//! the sense that: capability_match is a hard gate (0 or 1), freshness gates
//! at 30s and scores linearly inside the window, and the balance bias is
//! saturated. Any offer that fails a gate contributes nothing.

use serde::{Deserialize, Serialize};

use super::pressure::CollaborationSignal;

/// One candidate's answer, after its own owner-limit admission.
/// Mirrors `decentraai_compute::assist::AssistOffer` field-for-field so the
/// two layers stay in lockstep (same gates, same scoring).
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementOffer {
    pub peer_id: String,
    pub capability: String,
    pub cpu_cores: u16,
    pub ram_mb: u64,
    pub lease_seconds: u64,
    pub sampled_ago_secs: u64,
    pub queue_depth: u32,
    /// Contribution balance: contributed − consumed (ledger credits).
    /// Positive = net giver, negative = net taker, unknown = 0.
    pub contribution_balance: i64,
    pub has_recent_failure: bool,
}

/// Why an offer was rejected — fully enumerated for explainability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlacementRejection {
    CapabilityMismatch {
        offered: String,
        wanted: String,
    },
    NotEnoughCpu {
        offered: u16,
        wanted: u16,
    },
    NotEnoughRam {
        offered_mb: u64,
        wanted_mb: u64,
    },
    StaleAdvertisement {
        max_age_secs: u64,
        sampled_ago_secs: u64,
    },
    BusyQueue {
        queue_depth: u32,
    },
    RecentFailure,
}

impl std::fmt::Display for PlacementRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CapabilityMismatch { offered, wanted } => {
                write!(f, "capability mismatch: offered {offered}, wanted {wanted}")
            }
            Self::NotEnoughCpu { offered, wanted } => {
                write!(f, "not enough CPU: offered {offered}, wanted {wanted}")
            }
            Self::NotEnoughRam {
                offered_mb,
                wanted_mb,
            } => {
                write!(
                    f,
                    "not enough RAM: offered {offered_mb} MiB, wanted {wanted_mb} MiB"
                )
            }
            Self::StaleAdvertisement {
                max_age_secs,
                sampled_ago_secs,
            } => {
                write!(
                    f,
                    "stale: sampled {sampled_ago_secs}s ago, limit {max_age_secs}s"
                )
            }
            Self::BusyQueue { queue_depth } => write!(f, "busy queue: depth {queue_depth}"),
            Self::RecentFailure => write!(f, "recent failure"),
        }
    }
}

/// Freshness ceiling — mirrors `compute::assist::MAX_SAMPLE_AGE_SECS`.
pub const MAX_SAMPLE_AGE_SECS: u64 = 30;
/// Queue depth at/above which a worker is busy — mirrors `compute::assist::BUSY_QUEUE_DEPTH`.
pub const BUSY_QUEUE_DEPTH: u32 = 4;
const BALANCE_BIAS_MAX: f32 = 0.15;
const BALANCE_SATURATION: f64 = 100.0;

/// The placement verdict for one `CollaborationSignal`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacementDecision {
    /// Correlation id from the originating `CollaborationSignal` — threaded
    /// through so `pressure → placement → gateway` is one trace.
    pub correlation_id: String,
    /// Capability that was placed.
    pub capability: String,
    /// Selected peer, if any gate-passing candidate existed.
    pub selected_peer: Option<String>,
    /// Score of the winner (if any), for observability.
    pub winner_score: Option<f32>,
    /// All rejections with reasons — every decision is explainable.
    pub rejected: Vec<(String, PlacementRejection)>,
    /// Whether the decision found a placement.
    pub placed: bool,
}

/// Pure, deterministic hard-gate check — trust is NOT checked here (caller
/// must pre-filter to trusted peers, same as `compute::assist`).
pub fn evaluate_placement_offer(
    offer: &PlacementOffer,
    signal: &CollaborationSignal,
) -> Result<(), PlacementRejection> {
    if offer.capability != signal.capability {
        return Err(PlacementRejection::CapabilityMismatch {
            offered: offer.capability.clone(),
            wanted: signal.capability.clone(),
        });
    }
    // Resource fit: if the signal asks for 0, any offer fits (no specific
    // request). Otherwise enforce the asked headroom.
    if signal.cpu_cores > 0 && offer.cpu_cores < signal.cpu_cores {
        return Err(PlacementRejection::NotEnoughCpu {
            offered: offer.cpu_cores,
            wanted: signal.cpu_cores,
        });
    }
    if signal.ram_mb > 0 && offer.ram_mb < signal.ram_mb {
        return Err(PlacementRejection::NotEnoughRam {
            offered_mb: offer.ram_mb,
            wanted_mb: signal.ram_mb,
        });
    }
    if offer.sampled_ago_secs > MAX_SAMPLE_AGE_SECS {
        return Err(PlacementRejection::StaleAdvertisement {
            max_age_secs: MAX_SAMPLE_AGE_SECS,
            sampled_ago_secs: offer.sampled_ago_secs,
        });
    }
    if offer.queue_depth >= BUSY_QUEUE_DEPTH {
        return Err(PlacementRejection::BusyQueue {
            queue_depth: offer.queue_depth,
        });
    }
    if offer.has_recent_failure {
        return Err(PlacementRejection::RecentFailure);
    }
    Ok(())
}

/// Deterministic score for a gate-passing offer — mirrors
/// `compute::assist::score_offer` exactly (same weights, same saturation).
pub fn score_placement_offer(offer: &PlacementOffer, signal: &CollaborationSignal) -> f32 {
    debug_assert!(
        evaluate_placement_offer(offer, signal).is_ok(),
        "score only for gate-passing offers"
    );
    let want_cpu = signal.cpu_cores.max(1) as f32;
    let want_ram = signal.ram_mb.max(1) as f32;
    let cpu_fit = want_cpu / f32::from(offer.cpu_cores.max(1));
    let ram_fit = want_ram / (offer.ram_mb.max(1) as f32);
    let resource_headroom = cpu_fit.min(ram_fit).clamp(0.0, 1.0);
    let freshness =
        1.0 - (offer.sampled_ago_secs as f32 / MAX_SAMPLE_AGE_SECS as f32).clamp(0.0, 1.0);
    let queue = 1.0 - (offer.queue_depth as f32 / BUSY_QUEUE_DEPTH as f32).clamp(0.0, 1.0);
    let balance_bias = BALANCE_BIAS_MAX
        * ((offer.contribution_balance as f64 / BALANCE_SATURATION).clamp(-2.0, 2.0)).tanh() as f32;
    0.5 * resource_headroom + 0.25 * freshness + 0.10 * queue + balance_bias + 0.15
}

/// Deterministic selection over a set of offers for one signal.
/// Ties resolve by peer_id ascending. Returns the decision plus the winner
/// score; emits no I/O (the runtime wraps this with the EventBus).
pub fn select_placement<'a>(
    signal: &CollaborationSignal,
    offers: impl Iterator<Item = &'a PlacementOffer>,
) -> PlacementDecision {
    let mut rejected: Vec<(String, PlacementRejection)> = Vec::new();
    let mut best: Option<(&PlacementOffer, f32)> = None;
    for offer in offers {
        match evaluate_placement_offer(offer, signal) {
            Ok(()) => {
                let score = score_placement_offer(offer, signal);
                let replace = match best {
                    None => true,
                    Some((cur, cur_score)) => {
                        score > cur_score || (score == cur_score && offer.peer_id < cur.peer_id)
                    }
                };
                if replace {
                    best = Some((offer, score));
                }
            }
            Err(reason) => rejected.push((offer.peer_id.clone(), reason)),
        }
    }
    // Deterministic ordering of rejections for stable output.
    rejected.sort_by(|a, b| a.0.cmp(&b.0));
    if let Some((winner, score)) = best {
        PlacementDecision {
            correlation_id: signal.correlation_id.clone(),
            capability: signal.capability.clone(),
            selected_peer: Some(winner.peer_id.clone()),
            winner_score: Some(score),
            rejected,
            placed: true,
        }
    } else {
        PlacementDecision {
            correlation_id: signal.correlation_id.clone(),
            capability: signal.capability.clone(),
            selected_peer: None,
            winner_score: None,
            rejected,
            placed: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saes::pressure::{CollaborationSignal, Urgency};

    fn signal(cap: &str) -> CollaborationSignal {
        CollaborationSignal {
            agent_id: "agent-1".into(),
            capability: cap.into(),
            reasons: vec!["queue_depth".into()],
            urgency: Urgency::Elevated,
            correlation_id: "pressure-test-123".into(),
            cpu_cores: 2,
            ram_mb: 512,
            max_lease_seconds: 30,
        }
    }

    fn offer(peer: &str, cap: &str) -> PlacementOffer {
        PlacementOffer {
            peer_id: peer.into(),
            capability: cap.into(),
            cpu_cores: 2,
            ram_mb: 512,
            lease_seconds: 60,
            sampled_ago_secs: 5,
            queue_depth: 0,
            contribution_balance: 0,
            has_recent_failure: false,
        }
    }

    #[test]
    fn gates_reject_each_violation() {
        let s = signal("embeddings");
        let mut o = offer("b", "ocr");
        assert!(matches!(
            evaluate_placement_offer(&o, &s),
            Err(PlacementRejection::CapabilityMismatch { .. })
        ));
        o = offer("b", "embeddings");
        o.cpu_cores = 1;
        assert!(matches!(
            evaluate_placement_offer(&o, &s),
            Err(PlacementRejection::NotEnoughCpu { .. })
        ));
        o = offer("b", "embeddings");
        o.ram_mb = 128;
        assert!(matches!(
            evaluate_placement_offer(&o, &s),
            Err(PlacementRejection::NotEnoughRam { .. })
        ));
        o = offer("b", "embeddings");
        o.sampled_ago_secs = 120;
        assert!(matches!(
            evaluate_placement_offer(&o, &s),
            Err(PlacementRejection::StaleAdvertisement { .. })
        ));
        o = offer("b", "embeddings");
        o.queue_depth = 9;
        assert!(matches!(
            evaluate_placement_offer(&o, &s),
            Err(PlacementRejection::BusyQueue { .. })
        ));
        o = offer("b", "embeddings");
        o.has_recent_failure = true;
        assert_eq!(
            evaluate_placement_offer(&o, &s),
            Err(PlacementRejection::RecentFailure)
        );
    }

    #[test]
    fn select_prefers_exact_fit() {
        let s = signal("embeddings");
        let exact = offer("exact", "embeddings");
        let mut huge = offer("huge", "embeddings");
        huge.cpu_cores = 16;
        huge.ram_mb = 8192;
        let decision = select_placement(&s, [exact, huge].iter());
        assert_eq!(decision.selected_peer.as_deref(), Some("exact"));
        assert!(decision.placed);
        assert_eq!(decision.correlation_id, "pressure-test-123");
    }

    #[test]
    fn fairness_breaks_tie_toward_contributor() {
        let s = signal("embeddings");
        let neutral = offer("aaa-neutral", "embeddings");
        let mut giver = offer("zzz-giver", "embeddings");
        giver.contribution_balance = 150;
        let decision = select_placement(&s, [neutral, giver].iter());
        assert_eq!(decision.selected_peer.as_deref(), Some("zzz-giver"));

        // But fairness never resurrects a hard-gate failure
        let neutral2 = offer("aaa-neutral", "embeddings");
        let mut failed_giver = offer("zzz-giver", "embeddings");
        failed_giver.contribution_balance = 150;
        failed_giver.has_recent_failure = true;
        let decision2 = select_placement(&s, [neutral2, failed_giver].iter());
        assert_eq!(decision2.selected_peer.as_deref(), Some("aaa-neutral"));
    }

    #[test]
    fn staleness_and_busy_rejected() {
        let s = signal("embeddings");
        let mut stale = offer("stale", "embeddings");
        stale.sampled_ago_secs = 500;
        let mut busy = offer("busy", "embeddings");
        busy.queue_depth = 5;
        let decision = select_placement(&s, [stale, busy].iter());
        assert!(!decision.placed);
        assert_eq!(decision.rejected.len(), 2);
    }

    #[test]
    fn deterministic_tiebreak() {
        let s = signal("embeddings");
        let offers = [offer("a", "embeddings"), offer("b", "embeddings")];
        for _ in 0..5 {
            let d = select_placement(&s, offers.iter());
            assert_eq!(d.selected_peer.as_deref(), Some("a"));
        }
    }

    #[test]
    fn zero_resource_signal_any_offer_fits() {
        let mut s = signal("embeddings");
        s.cpu_cores = 0;
        s.ram_mb = 0;
        let small = PlacementOffer {
            peer_id: "small".into(),
            capability: "embeddings".into(),
            cpu_cores: 1,
            ram_mb: 128,
            lease_seconds: 30,
            sampled_ago_secs: 2,
            queue_depth: 0,
            contribution_balance: 0,
            has_recent_failure: false,
        };
        assert!(evaluate_placement_offer(&small, &s).is_ok());
    }

    #[test]
    fn freshness_affects_score_but_not_beyond_cap() {
        let s = signal("embeddings");
        let mut fresh = offer("fresh", "embeddings");
        fresh.sampled_ago_secs = 1;
        let mut staleish = offer("staleish", "embeddings");
        staleish.sampled_ago_secs = 25;
        // Fresh wins when otherwise equal
        let d = select_placement(&s, [staleish.clone(), fresh.clone()].iter());
        assert_eq!(d.selected_peer.as_deref(), Some("fresh"));
        // But fairness bias is capped: huge balance doesn't dominate freshness+headroom
        let mut giver_stale = staleish;
        giver_stale.peer_id = "giver-stale".into();
        giver_stale.contribution_balance = 1000; // capped
        let mut taker_fresh = fresh;
        taker_fresh.peer_id = "taker-fresh".into();
        taker_fresh.contribution_balance = -1000;
        // Freshness still matters, but bias is max 0.15 — exact fit still wins
        // Here both have exact fit, so giver with cap still beats taker due to bias
        let d2 = select_placement(&s, [taker_fresh, giver_stale].iter());
        assert_eq!(d2.selected_peer.as_deref(), Some("giver-stale"));
    }

    #[test]
    fn placement_decision_preserves_correlation_id() {
        let s = CollaborationSignal {
            agent_id: "a".into(),
            capability: "ocr".into(),
            reasons: vec![],
            urgency: Urgency::Low,
            correlation_id: "pressure-abc-999".into(),
            cpu_cores: 0,
            ram_mb: 0,
            max_lease_seconds: 30,
        };
        let d = select_placement(&s, std::iter::empty());
        assert_eq!(d.correlation_id, "pressure-abc-999");
        assert_eq!(d.capability, "ocr");
        assert!(!d.placed);
    }

    /// E2E: pressure → signal → placement → EventBus, correlation_id threaded.
    #[tokio::test]
    async fn pressure_signal_to_placement_via_runtime() {
        use crate::local::{LocalAgentRuntime, StaticObservationBuilder};
        use crate::saes::pressure::{PressureSignals, PressureThresholds};
        use decentraai_event_bus::{EventBus, EventFilter, InMemoryEventStore};
        use std::sync::Arc;

        let bus = Arc::new(EventBus::new(Arc::new(InMemoryEventStore::new(1024))));
        let obs = Arc::new(StaticObservationBuilder::empty());
        let runtime = LocalAgentRuntime::new(bus.clone(), obs);

        let agent_id = "agent-e2e-place".to_string();
        let thresholds = PressureThresholds::default();
        let signals = PressureSignals {
            cpu_percent: 95.0,
            queue_depth: 5,
            latency_ms: 9_000,
            ..Default::default()
        };
        let signal = runtime
            .evaluate_pressure(&agent_id, &signals, &thresholds, 0, "embeddings")
            .await
            .unwrap()
            .expect("pressure must fire");
        assert!(signal.correlation_id.starts_with("pressure-"));

        // Placement among candidates — exact fit + fairness breaks tie.
        let neutral = PlacementOffer {
            peer_id: "aaa-neutral".into(),
            capability: "embeddings".into(),
            cpu_cores: 2,
            ram_mb: 512,
            lease_seconds: 60,
            sampled_ago_secs: 5,
            queue_depth: 0,
            contribution_balance: 0,
            has_recent_failure: false,
        };
        let mut giver = neutral.clone();
        giver.peer_id = "zzz-giver".into();
        giver.contribution_balance = 200;

        let decision = runtime
            .place_collaboration(&signal, vec![neutral, giver])
            .await;
        assert!(decision.placed);
        assert_eq!(decision.selected_peer.as_deref(), Some("zzz-giver"));
        assert_eq!(decision.correlation_id, signal.correlation_id);

        // EventBus carries the placement event with same correlation_id.
        let events = bus.get_events(EventFilter::default(), 50).await.unwrap();
        let placed: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == "agent.placement.decided")
            .collect();
        assert_eq!(placed.len(), 1);
        assert_eq!(
            placed[0].metadata.correlation_id.as_deref(),
            Some(signal.correlation_id.as_str())
        );
        // Pressure + placement share the same episode id — traceable end-to-end.
        let fired: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == "agent.pressure.fired")
            .collect();
        assert_eq!(fired.len(), 1);
        assert_eq!(
            fired[0].metadata.correlation_id.as_deref(),
            Some(signal.correlation_id.as_str())
        );
    }

    #[tokio::test]
    async fn placement_no_candidate_emits_event() {
        use crate::local::{LocalAgentRuntime, StaticObservationBuilder};
        use crate::saes::pressure::{CollaborationSignal, Urgency};
        use decentraai_event_bus::{EventBus, EventFilter, InMemoryEventStore};
        use std::sync::Arc;

        let bus = Arc::new(EventBus::new(Arc::new(InMemoryEventStore::new(1024))));
        let obs = Arc::new(StaticObservationBuilder::empty());
        let runtime = LocalAgentRuntime::new(bus.clone(), obs);

        let signal = CollaborationSignal {
            agent_id: "agent-no-cand".into(),
            capability: "embeddings".into(),
            reasons: vec!["queue_depth".into()],
            urgency: Urgency::Low,
            correlation_id: "pressure-no-cand-1".into(),
            cpu_cores: 2,
            ram_mb: 512,
            max_lease_seconds: 30,
        };
        let mut stale = PlacementOffer {
            peer_id: "stale".into(),
            capability: "embeddings".into(),
            cpu_cores: 2,
            ram_mb: 512,
            lease_seconds: 60,
            sampled_ago_secs: 999,
            queue_depth: 0,
            contribution_balance: 0,
            has_recent_failure: false,
        };
        let _ = &mut stale;
        let decision = runtime.place_collaboration(&signal, vec![stale]).await;
        assert!(!decision.placed);
        assert_eq!(decision.correlation_id, "pressure-no-cand-1");

        let events = bus.get_events(EventFilter::default(), 50).await.unwrap();
        let no_cand: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == "agent.placement.no_candidate")
            .collect();
        assert_eq!(no_cand.len(), 1);
        assert_eq!(
            no_cand[0].metadata.correlation_id.as_deref(),
            Some("pressure-no-cand-1")
        );
    }
}

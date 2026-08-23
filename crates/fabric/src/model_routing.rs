//! Deterministic model routing (Model Colony) — the Governor may PROPOSE
//! the capability it needs; THIS module decides which model serves it.
//!
//! # Contract
//!
//! ```text
//! AI proposes (capability + context need + traffic class)
//!   → deterministic policy (hard gates, then a fixed scoring formula)
//!   → selected model + ordered fallbacks
//! ```
//!
//! The model itself never selects its own authority: scoring consumes only
//! registry facts (claims, governance, hardware) plus verified observations
//! from Collective Memory. Same inputs → same selection, always; ties break
//! by model_id ascending, matching every other fabric decision.
//!
//! Hard gates first (a gate failure is a REJECTION with reason, never a
//! silent score penalty), then the score:
//!
//! ```text
//! score = 2 × effective_capability_strength      (evidence-weighted)
//!       + quality points        (verified observations; cold-start floor)
//!       + latency points        (verified observations; cold-start floor)
//!       − degraded penalty      (runtime fact)
//!       + context headroom      (fits comfortably beats barely fits)
//! ```

use decentraai_hub::capability::CapabilityKind;
use decentraai_hub::model_intel::{AvailabilityState, ModelIntelRecord};

/// What class of traffic is being routed. Governance decides eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrafficClass {
    /// Real user/fabric work — approved models only.
    Production,
    /// Shadow copies of real work — shadow/candidate models.
    Shadow,
    /// Evidence-gathering benchmark tasks — anything not rejected.
    Benchmark,
}

/// Verified performance facts for one model, aggregated from Collective
/// Memory observations (`kind = model_evaluation`). `None` = cold start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedPerformance {
    /// Verified success percent 0..=100.
    pub success_percent: u8,
    /// Mean latency over verified observations, milliseconds.
    pub mean_latency_ms: u64,
}

/// One candidate as presented to the router.
#[derive(Debug, Clone, Copy)]
pub struct RoutedCandidate<'a> {
    pub record: &'a ModelIntelRecord,
    /// Runtime availability on this node right now.
    pub availability: AvailabilityState,
    /// Aggregated verified observations, when any exist.
    pub observed: Option<ObservedPerformance>,
    /// Current node RAM pressure percent 0..=100 (from system probe).
    pub ram_pressure_percent: u8,
}

/// The routing request: what the caller NEEDS (never which model it wants).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteNeed {
    /// Capability that MUST be claimed by the selected model (hard gate).
    pub required: CapabilityKind,
    /// Minimum context length in tokens (hard gate).
    pub min_context_tokens: u32,
    /// Traffic class being routed.
    pub traffic: TrafficClass,
}

/// Why one candidate was rejected at a hard gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub model_id: String,
    pub reason: String,
}

/// Deterministic routing outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutingDecision {
    /// Best eligible candidate, `None` when every candidate was rejected.
    pub selected: Option<String>,
    /// Remaining eligible candidates in score order (deterministic fallbacks).
    pub fallbacks: Vec<String>,
    /// Every hard-gate rejection with its reason (auditability).
    pub rejections: Vec<Rejection>,
}

/// Cold-start floors: an unobserved model competes but verified evidence
/// dominates once it exists. Chosen so one strong verified observation
/// outweighs any inferred claim alone.
const COLD_START_QUALITY_POINTS: u32 = 120;
const COLD_START_LATENCY_POINTS: u32 = 60;
const MAX_LATENCY_POINTS: u32 = 150;
const DEGRADED_PENALTY: u32 = 80;

fn latency_points(mean_latency_ms: u64) -> u32 {
    // 0 ms → full 150 pts; ≥3000 ms → 0. Linear, integer-only.
    MAX_LATENCY_POINTS.saturating_sub((mean_latency_ms / 20).min(MAX_LATENCY_POINTS.into()) as u32)
}

/// Scoring for ONE candidate against the need. Pure and deterministic.
/// Returns `Err(reason)` on hard-gate rejection.
pub fn score_candidate(
    candidate: &RoutedCandidate<'_>,
    need: &RouteNeed,
) -> Result<u32, String> {
    let record = candidate.record;

    // ---- hard gates (rejection reasons, never penalties) ----
    if !record.capabilities.iter().any(|c| c.kind == need.required) {
        return Err(format!(
            "capability '{}' not claimed",
            decentraai_hub::capability::CapabilityKind::label(&need.required)
        ));
    }
    match need.traffic {
        TrafficClass::Production if !record.governance.serves_production() => {
            return Err(format!(
                "governance stage '{:?}' cannot serve production",
                record.governance
            ));
        }
        TrafficClass::Shadow if !record.governance.receives_shadow() => {
            return Err(format!(
                "governance stage '{:?}' cannot receive shadow traffic",
                record.governance
            ));
        }
        TrafficClass::Benchmark if !record.governance.may_benchmark() => {
            return Err("rejected models cannot be benchmarked".to_string());
        }
        _ => {}
    }
    if candidate.availability == AvailabilityState::Unavailable {
        return Err("model is unavailable on this node".to_string());
    }
    if record.context_length < need.min_context_tokens {
        return Err(format!(
            "context {} < required {}",
            record.context_length, need.min_context_tokens
        ));
    }

    // ---- deterministic score ----
    let claim = record
        .claim_strength(need.required)
        .expect("claimed capability (gated above)");
    let mut score = claim.effective_strength() * 2;

    let (quality, latency) = match candidate.observed {
        Some(o) => (u32::from(o.success_percent) * 3, latency_points(o.mean_latency_ms)),
        None => (COLD_START_QUALITY_POINTS, COLD_START_LATENCY_POINTS),
    };
    score += quality + latency;

    if candidate.availability == AvailabilityState::Degraded {
        score = score.saturating_sub(DEGRADED_PENALTY);
    }

    // Context headroom: comfortable fit earns up to 50 pts (integer math:
    // ratio = context / max(required,1), capped ×2 → headroom 0..=50).
    let needed = need.min_context_tokens.max(1);
    let ratio = (record.context_length / needed).min(2);
    let headroom = (ratio - 1) * 50;
    score += headroom;

    // Resource pressure bites PROPORTIONALLY TO FOOTPRINT: a 3 GiB model
    // under 95 % RAM pressure loses far more than a 0.5 GiB one — pressure
    // is exactly when small models shine. Integer math only.
    let footprint_gb = u32::try_from(
        record.hardware.ram_needed_bytes / (1024 * 1024 * 1024),
    )
    .unwrap_or(u32::MAX);
    let pressure_penalty = u32::from(candidate.ram_pressure_percent)
        * footprint_gb.min(8)
        * 15
        / 100;
    score = score.saturating_sub(pressure_penalty);

    Ok(score)
}

/// Routes the need across all candidates: hard gates first, deterministic
/// score second, id-ascending tie-break. Produces a primary selection and
/// ordered fallbacks — the caller still goes through the planner's
/// reservation machinery; nothing here bypasses policy.
pub fn route<'a>(
    candidates: &[RoutedCandidate<'a>],
    need: &RouteNeed,
) -> RoutingDecision {
    let mut scored: Vec<(u32, &str)> = Vec::new();
    let mut rejections = Vec::new();

    for c in candidates {
        match score_candidate(c, need) {
            Ok(s) => scored.push((s, c.record.model_id.as_str())),
            Err(reason) => rejections.push(Rejection {
                model_id: c.record.model_id.clone(),
                reason,
            }),
        }
    }

    // Score desc, then model_id asc — the fabric's tie-break discipline.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));

    let mut ids = scored.into_iter().map(|(_, id)| id.to_string());
    let selected = ids.next();
    RoutingDecision {
        selected,
        fallbacks: ids.collect(),
        rejections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_hub::model_intel::{
        CapabilityClaim, GovernanceStage, HardwareRequirements, ModelIntelRecord,
        seed_model_colony,
    };

    fn record(id: &str, cap: CapabilityKind, ctx: u32, governance: GovernanceStage) -> ModelIntelRecord {
        ModelIntelRecord {
            model_id: id.into(),
            provider: "local".into(),
            runtime: "llama.cpp".into(),
            quantization: "q4_k_m".into(),
            context_length: ctx,
            capabilities: vec![CapabilityClaim::inferred(cap, 75)],
            romanian_strength: 50,
            version: "v1".into(),
            hardware: HardwareRequirements { ram_needed_bytes: 3_221_225_472, min_free_ram_bytes: 2_147_483_648 },
            governance,
        }
    }

    fn cand<'a>(r: &'a ModelIntelRecord, availability: AvailabilityState) -> RoutedCandidate<'a> {
        RoutedCandidate {
            record: r,
            availability,
            observed: None,
            ram_pressure_percent: 30,
        }
    }

    const NEED: RouteNeed = RouteNeed {
        required: CapabilityKind::Reasoning,
        min_context_tokens: 4096,
        traffic: TrafficClass::Production,
    };

    #[test]
    fn hard_gates_reject_with_reasons_not_penalties() {
        let reg = seed_model_colony();
        let gemma = reg.get("gemma-3-1b-q4").unwrap(); // no Reasoning claim
        let qwen = reg.get("qwen3-1.7b-q4").unwrap(); // has Reasoning, Experimental

        // Capability gate fires first: gemma lacks the required claim.
        let mut candidates = vec![cand(gemma, AvailabilityState::Available)];
        let d = route(&candidates, &NEED);
        assert!(d.selected.is_none());
        assert!(
            d.rejections.iter().any(|r| r.model_id == "gemma-3-1b-q4"
                && r.reason.contains("not claimed")),
            "missing capability is a rejection"
        );

        // Governance gate: capable but Experimental cannot serve production.
        candidates.push(cand(qwen, AvailabilityState::Available));
        let d = route(&candidates, &NEED);
        assert!(d.selected.is_none());
        assert!(d.rejections.iter().any(|r| r.reason.contains("cannot serve production")));

        // Approved + capable but UNAVAILABLE: availability gate rejects.
        let reasoning_approved =
            ModelIntelRecord { governance: GovernanceStage::Approved, ..qwen.clone() };
        candidates.push(cand(&reasoning_approved, AvailabilityState::Unavailable));
        let d = route(&candidates, &NEED);
        assert!(d.rejections.iter().any(|r| r.reason.contains("unavailable")));

        // Context gate: require more than any seed provides.
        let huge_ctx_need = RouteNeed { min_context_tokens: 100_000, ..NEED };
        let approved_qwen = ModelIntelRecord { governance: GovernanceStage::Approved, ..qwen.clone() };
        let d = route(&[cand(&approved_qwen, AvailabilityState::Available)], &huge_ctx_need);
        assert!(d.rejections.iter().any(|r| r.reason.contains("context")));
    }

    #[test]
    fn traffic_classes_match_governance_stages_exactly() {
        let reg = seed_model_colony();
        let qwen = reg.get("qwen3-1.7b-q4").unwrap();
        let shadow_rec = ModelIntelRecord { governance: GovernanceStage::Shadow, ..qwen.clone() };

        let shadow_need = RouteNeed { traffic: TrafficClass::Shadow, ..NEED };
        let d = route(&[cand(&shadow_rec, AvailabilityState::Available)], &shadow_need);
        assert_eq!(d.selected.as_deref(), Some("qwen3-1.7b-q4"));

        // A shadow-stage model canNOT take production even though capable.
        let d = route(&[cand(&shadow_rec, AvailabilityState::Available)], &NEED);
        assert!(d.selected.is_none());

        // Rejected models are benchmark-invisible too.
        let rejected = ModelIntelRecord { governance: GovernanceStage::Rejected, ..qwen.clone() };
        let bench_need = RouteNeed { traffic: TrafficClass::Benchmark, ..NEED };
        let d = route(&[cand(&rejected, AvailabilityState::Available)], &bench_need);
        assert!(d.selected.is_none());
    }

    #[test]
    fn verified_evidence_beats_cold_start_and_inference_alone() {
        let reg = seed_model_colony();
        let mut gemma = reg.get("gemma-3-1b-q4").unwrap().clone();
        gemma.governance = GovernanceStage::Approved;
        gemma.capabilities = vec![decentraai_hub::model_intel::CapabilityClaim::verified(
            CapabilityKind::Reasoning,
            60,
        )];
        let mut qwen = reg.get("qwen3-1.7b-q4").unwrap().clone();
        qwen.governance = GovernanceStage::Approved;

        // Gemma: verified 60% success at fast latency. Qwen: unobserved.
        let observed = RoutedCandidate {
            record: &gemma,
            availability: AvailabilityState::Available,
            observed: Some(ObservedPerformance { success_percent: 60, mean_latency_ms: 400 }),
            ram_pressure_percent: 30,
        };
        let cold = cand(&qwen, AvailabilityState::Available);
        let d = route(&[cold, observed], &NEED);

        // Verified evidence wins over an unobserved competitor even though
        // Qwen's inferred claim strength (75) exceeds Gemma's raw (60):
        // effective 120×2 + 180+130 vs 150×2 + cold floors.
        assert_eq!(d.selected.as_deref(), Some("gemma-3-1b-q4"));
        assert_eq!(d.fallbacks, vec!["qwen3-1.7b-q4"], "ordered fallback exists");
    }

    #[test]
    fn ties_break_by_model_id_ascending_and_routing_is_pure() {
        let a = record("alpha", CapabilityKind::Reasoning, 8192, GovernanceStage::Approved);
        let b = record("beta", CapabilityKind::Reasoning, 8192, GovernanceStage::Approved);
        let ca = cand(&a, AvailabilityState::Available);
        let cb = cand(&b, AvailabilityState::Available);

        let d1 = route(&[ca, cb], &NEED);
        let d2 = route(&[cb, ca], &NEED);
        assert_eq!(d1.selected.as_deref(), Some("alpha"), "id asc tie-break");
        assert_eq!(d1.selected, d2.selected, "input order must not matter");
        assert_eq!(d1.fallbacks, vec!["beta"]);
    }

    #[test]
    fn degraded_and_pressure_shift_the_order_deterministically() {
        let big = record("big-model", CapabilityKind::Reasoning, 8192, GovernanceStage::Approved);
        let small = ModelIntelRecord {
            model_id: "small-model".into(),
            hardware: HardwareRequirements { ram_needed_bytes: 512 * 1024 * 1024, min_free_ram_bytes: 2_147_483_648 },
            ..record("x", CapabilityKind::Reasoning, 8192, GovernanceStage::Approved)
        };
        let mut small = small;
        small.model_id = "small-model".into();

        // Under heavy pressure, the small model overtakes the big healthy one.
        fn high_pressure(r: &ModelIntelRecord) -> RoutedCandidate<'_> {
            RoutedCandidate {
                record: r,
                availability: AvailabilityState::Available,
                observed: None,
                ram_pressure_percent: 95,
            }
        }
        let d = route(&[high_pressure(&big), high_pressure(&small)], &NEED);
        assert_eq!(d.selected.as_deref(), Some("small-model"), "pressure favors small footprints");

        // Degraded health penalizes but does not reject.
        let degraded_big = RoutedCandidate {
            record: &big,
            availability: AvailabilityState::Degraded,
            observed: None,
            ram_pressure_percent: 10,
        };
        let ok_small = RoutedCandidate {
            record: &small,
            availability: AvailabilityState::Available,
            observed: None,
            ram_pressure_percent: 10,
        };
        let d = route(&[degraded_big, ok_small], &NEED);
        assert!(d.selected.is_some(), "both remained eligible");
        assert_eq!(d.fallbacks.len(), 1);
    }
}

//! P11 — agent economy: pure offer/booking primitives.
//!
//! The collective-intelligence fabric uses **synthetic, non-monetary**
//! economics: `QuotaLedger` (contribute → quota) and `CompensationLedger`
//! (reputation-scaled credits) in `decentraai-compute`. The architecture
//! (`docs/COLLECTIVE_INTELLIGENCE.md` §P11 / §4.6) is explicit that this
//! module is **modular and NOT the full economy** — it defines the pure
//! offer/negotiation primitives agents use to *offer a capability as a
//! service* and *book it*, priced in synthetic credits. Wiring those credits
//! into the existing quota/compensation ledgers is a later integration step.
//!
//! # Design decisions
//!
//! - **Non-monetary by construction.** `price_per_unit` is a count of
//!   synthetic credits, never currency. The module only *decides* whether an
//!   offer satisfies a request; it never moves or accounts for credits.
//! - **Pure and deterministic.** No I/O, no async, no clock dependence in
//!   decisions. Every decision is a pure function ([`negotiate`]) that tests
//!   can drive with synthetic inputs, and the ledger is a bounded, ordered
//!   registry so the same inserts always give the same shape.
//! - **Modular.** [`EconomyLedger`] is the local bookkeeping half. The
//!   negotiation threshold floor (quality/reliability ≥ 0.5) is an explicit,
//!   documented policy constant — a *starting* policy that later integration
//!   with reputation can tune per agent.
//! - **Capability as a free-form string.** Like reputation, `capability` is
//!   the snake_case form of a hub `CapabilityKind` today but kept as `String`
//!   for extensibility beyond the fixed taxonomy.
//!
//! The wire types (`CapabilityOffer`, `BookingRequest`, `BookingVerdict`)
//! derive serde so offers and bookings can travel between nodes, and all
//! enums serialize `snake_case` for stable wire names.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// Lifecycle of a capability offer.
///
/// `Active` offers are negotiable; `Suspended` offers exist but are held out
/// of negotiation (e.g. the agent is overloaded); `Retired` offers are no
/// longer offered. The lifecycle is one-way by policy: a retired offer is
/// normally removed, but keeping the state lets consumers distinguish "was
/// offered, now withdrawn" from "never existed".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfferStatus {
    /// Currently bookable.
    Active,
    /// Held out of negotiation; may return to `Active`.
    Suspended,
    /// Permanently withdrawn.
    Retired,
}

/// Minimum acceptable quality/reliability for an offer to be negotiable.
///
/// A floor is a *hard policy* (the task: "quality and reliability thresholds
/// (>= 0.5 each)"). It is deliberately a coarse starting floor — later
/// integration with agent reputation can raise it per capability, but never
/// below this. An offer below the floor is explicitly not worth booking.
const QUALITY_FLOOR: f32 = 0.5;
const RELIABILITY_FLOOR: f32 = 0.5;

/// An agent's standing offer to serve one capability at a price/SLA.
///
/// Price and SLA are inherent to the offer (the seller sets them); the
/// [`BookingRequest`] is the buyer's counter. The offer is created once and
/// consulted repeatedly, so negotiation stays a pure function of two values
/// rather than a round-trip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityOffer {
    /// Unique id of this offer.
    pub offer_id: String,
    /// The logical agent selling the capability.
    pub agent_id: String,
    /// Capability being offered (snake_case hub kind, or free-form string).
    pub capability: String,
    /// Measured output quality in `0..=1` (clamped by [`CapabilityOffer::new`]).
    pub quality: f32,
    /// Measured reliability in `0..=1` (clamped by [`CapabilityOffer::new`]).
    pub reliability: f32,
    /// Price per unit in synthetic credits.
    pub price_per_unit: u64,
    /// Maximum simultaneous bookings this offer accepts.
    pub max_concurrency: u32,
    /// SLA: target latency in ms. `None` = no latency commitment.
    pub sla_latency_ms: Option<u64>,
    /// Current lifecycle state.
    pub status: OfferStatus,
    /// When the offer was created (unix ms).
    pub created_at_ms: u64,
}

impl CapabilityOffer {
    /// Creates an offer, clamping `quality`/`reliability` to `0..=1` so raw
    /// measurements can never produce an out-of-range guarantee.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        offer_id: impl Into<String>,
        agent_id: impl Into<String>,
        capability: impl Into<String>,
        quality: f32,
        reliability: f32,
        price_per_unit: u64,
        max_concurrency: u32,
        sla_latency_ms: Option<u64>,
        created_at_ms: u64,
    ) -> Self {
        Self {
            offer_id: offer_id.into(),
            agent_id: agent_id.into(),
            capability: capability.into(),
            quality: quality.clamp(0.0, 1.0),
            reliability: reliability.clamp(0.0, 1.0),
            price_per_unit,
            max_concurrency,
            sla_latency_ms,
            status: OfferStatus::Active,
            created_at_ms,
        }
    }

    /// Whether this offer meets its own SLA for an observed latency.
    ///
    /// An offer with no latency commitment (`sla_latency_ms` is `None`) always
    /// meets the SLA; otherwise the observed latency must be present and
    /// within the commitment.
    pub fn meets_sla(&self, latency_ms: Option<u64>) -> bool {
        match self.sla_latency_ms {
            None => true,
            Some(sla) => latency_ms.is_some_and(|lat| lat <= sla),
        }
    }
}

/// A buyer's request to book `units` of a capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookingRequest {
    /// Unique id of this request.
    pub request_id: String,
    /// The logical agent requesting the booking.
    pub client_agent: String,
    /// Capability requested (must match the offer's capability).
    pub capability: String,
    /// Number of units to book; must be ≥ 1.
    pub units: u64,
    /// Maximum the buyer will pay per unit (synthetic credits).
    pub max_price_per_unit: u64,
    /// Buyer's latency requirement in ms; `None` = no requirement.
    pub latency_requirement_ms: Option<u64>,
}

/// Outcome of a negotiation between a request and an offer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookingVerdict {
    /// The booking succeeded.
    Booked {
        /// The id of the accepted offer.
        offer_id: String,
        /// Total price = `price_per_unit * units`.
        total_price: u64,
    },
    /// The booking was refused with an explainable reason.
    Rejected {
        /// Human-readable reason a user/operator can act on.
        reason: String,
    },
}

/// Whether an offer can satisfy a request's latency requirement.
///
/// The buyer's `latency_requirement_ms` is a **ceiling**: the offer must
/// promise latency at least as good as the buyer demands. An offer whose SLA
/// is at least as tight as the requirement satisfies it; an offer that makes
/// no latency commitment cannot prove it and is refused when the buyer demands
/// one. A buyer with no requirement is always satisfied.
fn meets_request_latency(request: &BookingRequest, offer: &CapabilityOffer) -> bool {
    match request.latency_requirement_ms {
        None => true,
        Some(required) => offer.sla_latency_ms.is_some_and(|sla| sla <= required),
    }
}

/// Negotiates one booking request against one offer.
///
/// An offer is bookable iff it is `Active`, its capability matches the
/// request, it meets the request's latency requirement (if any), it accepts
/// work (`max_concurrency > 0`), its quality and reliability are at or above
/// the documented floor ([`QUALITY_FLOOR`]/[`RELIABILITY_FLOOR`]), and its
/// per-unit price is within the request's budget. The total price is
/// `price_per_unit * units`. Each rejection carries an explainable reason.
///
/// # Why these rules
///
/// - **Capability must match exactly** — booking a different capability than
///   requested would be a broken contract.
/// - **Latency is a hard gate** only when the buyer demands it; otherwise
///   latency is advisory.
/// - **`max_concurrency == 0`** means the offer is not currently accepting
///   bookings regardless of status (a misconfigured offer is not negotiable).
/// - **Quality/reliability floors** keep the market from booking offers that
///   cannot be trusted to deliver; the floor is coarse and tunable later.
pub fn negotiate(request: &BookingRequest, offer: &CapabilityOffer) -> BookingVerdict {
    if offer.status != OfferStatus::Active {
        return BookingVerdict::Rejected {
            reason: format!(
                "offer {} is not active (status {:?})",
                offer.offer_id, offer.status
            ),
        };
    }
    if offer.capability != request.capability {
        return BookingVerdict::Rejected {
            reason: format!(
                "capability mismatch: offer '{}' vs requested '{}'",
                offer.capability, request.capability
            ),
        };
    }
    if !meets_request_latency(request, offer) {
        return BookingVerdict::Rejected {
            reason: format!(
                "offer {} cannot meet latency requirement {:?} (sla {:?})",
                offer.offer_id, request.latency_requirement_ms, offer.sla_latency_ms
            ),
        };
    }
    if offer.max_concurrency == 0 {
        return BookingVerdict::Rejected {
            reason: format!("offer {} has zero concurrency and accepts no bookings", offer.offer_id),
        };
    }
    if offer.quality < QUALITY_FLOOR {
        return BookingVerdict::Rejected {
            reason: format!(
                "offer {} quality {:.2} below floor {:.2}",
                offer.offer_id, offer.quality, QUALITY_FLOOR
            ),
        };
    }
    if offer.reliability < RELIABILITY_FLOOR {
        return BookingVerdict::Rejected {
            reason: format!(
                "offer {} reliability {:.2} below floor {:.2}",
                offer.offer_id, offer.reliability, RELIABILITY_FLOOR
            ),
        };
    }
    if offer.price_per_unit > request.max_price_per_unit {
        return BookingVerdict::Rejected {
            reason: format!(
                "offer {} price {} exceeds budget {}",
                offer.offer_id, offer.price_per_unit, request.max_price_per_unit
            ),
        };
    }
    BookingVerdict::Booked {
        offer_id: offer.offer_id.clone(),
        total_price: offer.price_per_unit * request.units,
    }
}

/// Errors from registering/retiring offers in the ledger.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EconomyError {
    /// An offer with this id is already registered.
    #[error("offer with id '{offer_id}' already exists")]
    DuplicateOffer { offer_id: String },
    /// No offer with this id is registered.
    #[error("unknown offer id '{offer_id}'")]
    UnknownOffer { offer_id: String },
}

/// Default cap on the number of offers tracked.
///
/// The ledger is *bounded* so a flood of offer registrations (or a memory
/// leak from forgetting to retire) cannot grow without limit. When at the cap,
/// the oldest offer is evicted — registration is best-effort and cheap.
pub const MAX_OFFERS: usize = 256;

/// The deterministic, bounded local registry of offers.
///
/// Ordering is stable for two reasons: the `BTreeMap` gives deterministic
/// iteration by `offer_id`, and a separate [`VecDeque`] of insertion order
/// drives eviction at the cap (oldest first). `active_offers` and
/// `best_offer` sort by price so negotiation picks the cheapest compatible
/// offer deterministically.
#[derive(Debug, Clone, Default)]
pub struct EconomyLedger {
    offers: BTreeMap<String, CapabilityOffer>,
    order: VecDeque<String>,
}

impl EconomyLedger {
    /// An empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an offer.
    ///
    /// Returns [`EconomyError::DuplicateOffer`] if the id already exists. The
    /// offer's `status` is left as-is (a caller may re-register a `Suspended`
    /// offer). When the ledger is at [`MAX_OFFERS`], the oldest offer is
    /// evicted to make room.
    pub fn register(&mut self, offer: CapabilityOffer) -> Result<(), EconomyError> {
        if self.offers.contains_key(&offer.offer_id) {
            return Err(EconomyError::DuplicateOffer {
                offer_id: offer.offer_id,
            });
        }
        self.order.push_back(offer.offer_id.clone());
        self.offers.insert(offer.offer_id.clone(), offer);
        if self.offers.len() > MAX_OFFERS {
            if let Some(oldest) = self.order.pop_front() {
                self.offers.remove(&oldest);
            }
        }
        Ok(())
    }

    /// Retires an offer, removing it from the ledger. Returns `false` (and
    /// records nothing) if the id is unknown.
    pub fn retire(&mut self, offer_id: &str) -> bool {
        if !self.offers.contains_key(offer_id) {
            return false;
        }
        self.offers.remove(offer_id);
        self.order.retain(|id| id != offer_id);
        true
    }

    /// Looks up one offer by id.
    pub fn get(&self, offer_id: &str) -> Option<&CapabilityOffer> {
        self.offers.get(offer_id)
    }

    /// All `Active` offers for a capability, sorted by `price_per_unit`
    /// ascending, ties broken by `offer_id` ascending (deterministic).
    pub fn active_offers(&self, capability: &str) -> Vec<&CapabilityOffer> {
        let mut offers: Vec<&CapabilityOffer> = self
            .offers
            .values()
            .filter(|o| o.capability == capability && o.status == OfferStatus::Active)
            .collect();
        offers.sort_by(|a, b| {
            a.price_per_unit
                .cmp(&b.price_per_unit)
                .then_with(|| a.offer_id.cmp(&b.offer_id))
        });
        offers
    }

    /// The cheapest `Active` offer that would satisfy the request, as decided
    /// by [`negotiate`]. Returns `None` if no compatible offer exists. Ties
    /// break on `offer_id` ascending (via [`active_offers`] ordering).
    pub fn best_offer(&self, request: &BookingRequest) -> Option<&CapabilityOffer> {
        self.active_offers(&request.capability)
            .into_iter()
            .find(|offer| matches!(negotiate(request, offer), BookingVerdict::Booked { .. }))
    }

    /// Number of offers tracked.
    pub fn count(&self) -> usize {
        self.offers.len()
    }

    /// All offers (any status), as owned values sorted by `offer_id`.
    pub fn offers(&self) -> Vec<CapabilityOffer> {
        self.offers.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_offer(id: &str, cap: &str, price: u64) -> CapabilityOffer {
        CapabilityOffer::new(
            id,
            "agent-a",
            cap,
            0.9,
            0.9,
            price,
            1,
            Some(500),
            1000,
        )
    }

    fn mk_request(id: &str, cap: &str, budget: u64, units: u64) -> BookingRequest {
        BookingRequest {
            request_id: id.to_string(),
            client_agent: "client-1".to_string(),
            capability: cap.to_string(),
            units,
            max_price_per_unit: budget,
            latency_requirement_ms: None,
        }
    }

    #[test]
    fn active_compatible_offer_within_budget_is_booked_with_total_price() {
        let offer = mk_offer("o1", "ocr", 5);
        let req = mk_request("r1", "ocr", 10, 3);
        match negotiate(&req, &offer) {
            BookingVerdict::Booked { offer_id, total_price } => {
                assert_eq!(offer_id, "o1");
                assert_eq!(total_price, 15);
            }
            BookingVerdict::Rejected { reason } => panic!("unexpected rejection: {reason}"),
        }
    }

    #[test]
    fn non_active_offer_is_rejected() {
        let mut offer = mk_offer("o1", "ocr", 5);
        offer.status = OfferStatus::Suspended;
        let req = mk_request("r1", "ocr", 10, 1);
        assert!(matches!(negotiate(&req, &offer), BookingVerdict::Rejected { .. }));
        offer.status = OfferStatus::Retired;
        assert!(matches!(negotiate(&req, &offer), BookingVerdict::Rejected { .. }));
    }

    #[test]
    fn capability_mismatch_is_rejected() {
        let offer = mk_offer("o1", "ocr", 5);
        let req = mk_request("r1", "coding", 10, 1);
        assert!(matches!(negotiate(&req, &offer), BookingVerdict::Rejected { .. }));
    }

    #[test]
    fn price_above_budget_is_rejected() {
        let offer = mk_offer("o1", "ocr", 11);
        let req = mk_request("r1", "ocr", 10, 1);
        match negotiate(&req, &offer) {
            BookingVerdict::Rejected { reason } => assert!(reason.contains("budget")),
            BookingVerdict::Booked { .. } => panic!("over-budget offer must be rejected"),
        }
    }

    #[test]
    fn quality_below_floor_is_rejected() {
        let mut offer = mk_offer("o1", "ocr", 5);
        offer.quality = 0.4;
        let req = mk_request("r1", "ocr", 10, 1);
        match negotiate(&req, &offer) {
            BookingVerdict::Rejected { reason } => assert!(reason.contains("quality")),
            BookingVerdict::Booked { .. } => panic!("low-quality offer must be rejected"),
        }
        // Reliability has the same floor.
        let mut offer = mk_offer("o1", "ocr", 5);
        offer.reliability = 0.49;
        let req = mk_request("r1", "ocr", 10, 1);
        match negotiate(&req, &offer) {
            BookingVerdict::Rejected { reason } => assert!(reason.contains("reliability")),
            BookingVerdict::Booked { .. } => panic!("low-reliability offer must be rejected"),
        }
    }

    #[test]
    fn latency_requirement_not_met_is_rejected() {
        let offer = mk_offer("o1", "ocr", 5); // sla 500ms
        let mut req = mk_request("r1", "ocr", 10, 1);
        req.latency_requirement_ms = Some(100);
        match negotiate(&req, &offer) {
            BookingVerdict::Rejected { reason } => assert!(reason.contains("latency")),
            BookingVerdict::Booked { .. } => panic!("unmet latency must be rejected"),
        }
        // An offer with no SLA commitment cannot prove a latency requirement.
        let mut offer = mk_offer("o1", "ocr", 5);
        offer.sla_latency_ms = None;
        let mut req = mk_request("r1", "ocr", 10, 1);
        req.latency_requirement_ms = Some(1);
        match negotiate(&req, &offer) {
            BookingVerdict::Rejected { reason } => assert!(reason.contains("latency")),
            BookingVerdict::Booked { .. } => panic!("unproven latency must be rejected"),
        }
        // An offer at least as tight as the requirement satisfies it.
        let offer = mk_offer("o1", "ocr", 5); // sla 500ms
        let mut req = mk_request("r1", "ocr", 10, 1);
        req.latency_requirement_ms = Some(1000);
        assert!(matches!(negotiate(&req, &offer), BookingVerdict::Booked { .. }));
        // No requirement on either side is satisfied.
        let offer = mk_offer("o1", "ocr", 5);
        let req = mk_request("r1", "ocr", 10, 1);
        assert!(matches!(negotiate(&req, &offer), BookingVerdict::Booked { .. }));
    }

    #[test]
    fn zero_concurrency_offer_is_rejected() {
        let mut offer = mk_offer("o1", "ocr", 5);
        offer.max_concurrency = 0;
        let req = mk_request("r1", "ocr", 10, 1);
        match negotiate(&req, &offer) {
            BookingVerdict::Rejected { reason } => assert!(reason.contains("concurrency")),
            BookingVerdict::Booked { .. } => panic!("zero-concurrency offer must be rejected"),
        }
    }

    #[test]
    fn meets_sla_is_true_without_a_commitment_and_checks_with_one() {
        let mut offer = mk_offer("o1", "ocr", 5);
        offer.sla_latency_ms = None;
        assert!(offer.meets_sla(None));
        assert!(offer.meets_sla(Some(10_000)));

        offer.sla_latency_ms = Some(500);
        assert!(offer.meets_sla(Some(500)));
        assert!(offer.meets_sla(Some(499)));
        assert!(!offer.meets_sla(Some(501)));
        // No observed latency -> cannot satisfy a stated SLA.
        assert!(!offer.meets_sla(None));
    }

    #[test]
    fn ledger_registers_gets_retires_and_rejects_duplicates() {
        let mut ledger = EconomyLedger::new();
        assert_eq!(ledger.count(), 0);
        ledger.register(mk_offer("o1", "ocr", 5)).unwrap();
        ledger.register(mk_offer("o2", "ocr", 3)).unwrap();
        assert_eq!(ledger.count(), 2);
        assert_eq!(ledger.get("o1").unwrap().offer_id, "o1");

        // Duplicate rejected.
        let dup = ledger.register(mk_offer("o1", "ocr", 9));
        assert_eq!(dup, Err(EconomyError::DuplicateOffer { offer_id: "o1".into() }));

        // Unknown retire is a no-op.
        assert!(!ledger.retire("nope"));
        assert!(ledger.retire("o1"));
        assert!(!ledger.retire("o1"));
        assert_eq!(ledger.count(), 1);
        assert!(ledger.get("o1").is_none());
        assert_eq!(ledger.offers().len(), 1);
    }

    #[test]
    fn active_offers_are_sorted_by_price_then_id() {
        let mut ledger = EconomyLedger::new();
        ledger.register(mk_offer("o-zz", "ocr", 7)).unwrap();
        ledger.register(mk_offer("o-aa", "ocr", 3)).unwrap();
        ledger.register(mk_offer("o-mm", "ocr", 3)).unwrap();
        // A suspended and a different-capability offer must not leak in.
        let mut suspended = mk_offer("o-susp", "ocr", 1);
        suspended.status = OfferStatus::Suspended;
        ledger.register(suspended).unwrap();
        ledger.register(mk_offer("o-code", "coding", 1)).unwrap();

        let active = ledger.active_offers("ocr");
        assert_eq!(active.len(), 3);
        assert_eq!(active[0].offer_id, "o-aa");
        assert_eq!(active[1].offer_id, "o-mm");
        assert_eq!(active[2].offer_id, "o-zz");
    }

    #[test]
    fn best_offer_picks_the_cheapest_compatible_active_offer() {
        let mut ledger = EconomyLedger::new();
        ledger.register(mk_offer("o-expensive", "ocr", 9)).unwrap();
        ledger.register(mk_offer("o-cheap", "ocr", 4)).unwrap();
        ledger.register(mk_offer("o-cheapest", "ocr", 2)).unwrap();
        // Out of budget / wrong capability / non-active must not be chosen.
        ledger.register(mk_offer("o-over-budget", "ocr", 20)).unwrap();
        ledger.register(mk_offer("o-code", "coding", 1)).unwrap();
        // The cheapest candidate is suspended, so it must not win either.
        let mut suspended = mk_offer("o-cheap-suspended", "ocr", 1);
        suspended.status = OfferStatus::Suspended;
        ledger.register(suspended).unwrap();

        let req = mk_request("r1", "ocr", 10, 1);
        let best = ledger.best_offer(&req).expect("a compatible offer exists");
        assert_eq!(best.offer_id, "o-cheapest");

        // No active offer within budget -> None.
        let tight = mk_request("r2", "ocr", 1, 1);
        assert!(ledger.best_offer(&tight).is_none());

        // No compatible offer (wrong capability) -> None.
        let none = mk_request("r3", "vision", 10, 1);
        assert!(ledger.best_offer(&none).is_none());
    }

    #[test]
    fn ledger_evicts_oldest_when_at_capacity() {
        let mut ledger = EconomyLedger::new();
        for i in 0..MAX_OFFERS {
            ledger
                .register(mk_offer(&format!("o{i}"), "ocr", i as u64))
                .unwrap();
        }
        assert_eq!(ledger.count(), MAX_OFFERS);
        // Adding one more evicts the oldest (o0).
        ledger.register(mk_offer("o-new", "ocr", 1)).unwrap();
        assert_eq!(ledger.count(), MAX_OFFERS);
        assert!(ledger.get("o0").is_none());
        assert!(ledger.get("o-new").is_some());
    }

    #[test]
    fn statuses_and_verdicts_serialize_snake_case() {
        let offer = mk_offer("o1", "ocr", 5);
        assert!(serde_json::to_string(&offer).unwrap().contains("\"status\":\"active\""));

        let suspended = {
            let mut o = mk_offer("o1", "ocr", 5);
            o.status = OfferStatus::Suspended;
            o
        };
        assert!(serde_json::to_string(&suspended).unwrap().contains("\"status\":\"suspended\""));

        let verdict = BookingVerdict::Booked {
            offer_id: "o1".into(),
            total_price: 15,
        };
        assert!(serde_json::to_string(&verdict).unwrap().contains("\"booked\":"));
        let rejected = BookingVerdict::Rejected { reason: "no".into() };
        assert!(serde_json::to_string(&rejected).unwrap().contains("\"rejected\":"));
    }

    #[test]
    fn offers_round_trip_over_wire() {
        let offer = mk_offer("o1", "ocr", 5);
        let json = serde_json::to_string(&offer).unwrap();
        let back: CapabilityOffer = serde_json::from_str(&json).unwrap();
        assert_eq!(offer, back);

        let req = mk_request("r1", "ocr", 10, 2);
        let json = serde_json::to_string(&req).unwrap();
        let back: BookingRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);

        let verdict = negotiate(&req, &offer);
        let json = serde_json::to_string(&verdict).unwrap();
        let back: BookingVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(verdict, back);
    }

    #[test]
    fn offer_clamps_quality_and_reliability_to_unit_range() {
        let clamped = CapabilityOffer::new("o1", "agent-a", "ocr", 1.5, -0.2, 5, 1, Some(500), 1000);
        assert_eq!(clamped.quality, 1.0);
        assert_eq!(clamped.reliability, 0.0);
        assert_eq!(clamped.offer_id, "o1");
    }
}

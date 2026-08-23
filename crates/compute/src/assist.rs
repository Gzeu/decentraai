//! Compute Assist negotiation core ("Sharing is Caring", M14/M15 M1).
//!
//! Pure, deterministic, I/O-free: given an incoming [`AssistRequest`] and a
//! set of candidate offers (each ALREADY owner-limit-admitted by the worker
//! that produced it), decide which offer wins — or none. The decision is
//! explainable: every factor and its weight is visible in
//! [`score_offer`].
//!
//! Fairness principle: `contribution_balance` biases selection toward nodes
//! that recently CONTRIBUTED verified work. It can never override the hard
//! gates (trust, capability match, resource fit) — it only breaks ties among
//! otherwise-eligible workers, and it saturates so one hero node cannot
//! monopolize routing either.

/// A request for assist capacity on the mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct AssistRequest {
    pub capability: String,
    pub cpu_cores: u16,
    pub ram_mb: u64,
}

/// One candidate's answer, after ITS OWN owner-limit admission.
#[derive(Debug, Clone, PartialEq)]
pub struct AssistOffer {
    /// Peer id string of the offering node.
    pub peer_id: String,
    pub capability: String,
    pub cpu_cores: u16,
    pub ram_mb: u64,
    pub lease_seconds: u64,
    /// Freshness of the underlying availability sample (seconds ago).
    pub sampled_ago_secs: u64,
    pub queue_depth: u32,
    /// Recent contribution balance for this peer from the CreditLedger:
    /// contributed − consumed (units are ledger credits). Positive = net
    /// giver; negative = net taker; unknown = 0 (never penalized).
    pub contribution_balance: i64,
    /// Whether this peer previously FAILED an assist task (any failure in
    /// its last attempts window). Hard penalty, not a soft score term.
    pub has_recent_failure: bool,
}

/// Why an offer was rejected. Fully enumerated so tests pin every rule and
/// operators can explain any decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferRejection {
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

impl std::fmt::Display for OfferRejection {
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
                    "stale advertisement: sampled {sampled_ago_secs}s ago, limit {max_age_secs}s"
                )
            }
            Self::BusyQueue { queue_depth } => write!(f, "worker busy: queue depth {queue_depth}"),
            Self::RecentFailure => write!(f, "peer has a recent unproven failure"),
        }
    }
}

/// Freshness ceiling for availability samples backing an offer. Beyond this
/// the sample cannot be treated as current capacity.
pub const MAX_SAMPLE_AGE_SECS: u64 = 30;
/// Queue depth at/above which a worker is considered busy regardless of
/// advertised headroom.
pub const BUSY_QUEUE_DEPTH: u32 = 4;

/// Deterministic hard-gate check + explanation. Trust is NOT re-checked
/// here: the caller must pre-filter to trusted peers (trust is transport +
/// admission policy, not a scoring concern).
pub fn evaluate_offer(offer: &AssistOffer, request: &AssistRequest) -> Result<(), OfferRejection> {
    if offer.capability != request.capability {
        return Err(OfferRejection::CapabilityMismatch {
            offered: offer.capability.clone(),
            wanted: request.capability.clone(),
        });
    }
    if offer.cpu_cores < request.cpu_cores {
        return Err(OfferRejection::NotEnoughCpu {
            offered: offer.cpu_cores,
            wanted: request.cpu_cores,
        });
    }
    if offer.ram_mb < request.ram_mb {
        return Err(OfferRejection::NotEnoughRam {
            offered_mb: offer.ram_mb,
            wanted_mb: request.ram_mb,
        });
    }
    if offer.sampled_ago_secs > MAX_SAMPLE_AGE_SECS {
        return Err(OfferRejection::StaleAdvertisement {
            max_age_secs: MAX_SAMPLE_AGE_SECS,
            sampled_ago_secs: offer.sampled_ago_secs,
        });
    }
    if offer.queue_depth >= BUSY_QUEUE_DEPTH {
        return Err(OfferRejection::BusyQueue {
            queue_depth: offer.queue_depth,
        });
    }
    if offer.has_recent_failure {
        // A recent failure is a HARD gate: the retry path needs proven-good
        // workers, and credit is only awarded on verified success anyway.
        return Err(OfferRejection::RecentFailure);
    }
    Ok(())
}

/// Saturation bounds for the fairness bias (see [`score_offer`]).
const BALANCE_BIAS_MAX: f32 = 0.15;
/// Balance magnitude at which the bias reaches its cap. A node that gave
/// ~100 more than it took is already maximally favored.
const BALANCE_SATURATION: f64 = 100.0;

/// Deterministic, explainable offer score. Higher wins.
///
/// Components (all 0..1 unless noted):
/// - `resource_headroom`: how far above the request the offer sits — exact
///   fit scores best (over-offering wastes capacity the fabric may need);
/// - `freshness`: newer samples win linearly within the freshness window;
/// - `queue`: shallower queue wins;
/// - fairness bias: `contribution_balance` scaled into ±[`BALANCE_BIAS_MAX`]
///   — enough to break ties and reward givers, never enough to outrank a
///   hard-fit competitor by a wide margin, never able to resurrect a failed
///   gate (those reject earlier).
pub fn score_offer(offer: &AssistOffer, request: &AssistRequest) -> f32 {
    debug_assert!(
        evaluate_offer(offer, request).is_ok(),
        "score_offer is only defined for gate-passing offers"
    );

    // Headroom: ratio-based, capped at 1 when the offer exactly covers the
    // ask. Over-provisioning decays the score gently (wasted capacity).
    let cpu_fit = f32::from(request.cpu_cores.max(1)) / f32::from(offer.cpu_cores.max(1));
    let ram_fit = (request.ram_mb.max(1) as f32) / (offer.ram_mb.max(1) as f32);
    let resource_headroom = (cpu_fit.min(ram_fit)).clamp(0.0, 1.0);

    let freshness =
        1.0 - (offer.sampled_ago_secs as f32 / MAX_SAMPLE_AGE_SECS as f32).clamp(0.0, 1.0);

    let queue = 1.0 - (offer.queue_depth as f32 / BUSY_QUEUE_DEPTH as f32).clamp(0.0, 1.0);

    // Fairness: signed, saturated. tanh keeps the mapping smooth and bounded
    // without inventing a currency — this is priority-of-access arithmetic,
    // not money.
    let balance_bias = BALANCE_BIAS_MAX
        * ((offer.contribution_balance as f64 / BALANCE_SATURATION).clamp(-2.0, 2.0)).tanh() as f32;

    // Weights: capacity fit dominates, freshness and queue break ties, the
    // fairness bias is deliberately the smallest voice.
    0.5 * resource_headroom + 0.25 * freshness + 0.10 * queue + balance_bias + 0.15
}

/// Picks the best gate-passing offer deterministically. Ties resolve by
/// peer_id ascending (stable ordering, no randomness anywhere).
pub fn select_offer<'a>(
    offers: impl Iterator<Item = &'a AssistOffer>,
    request: &AssistRequest,
) -> Result<&'a AssistOffer, Vec<(String, OfferRejection)>> {
    let mut rejected: Vec<(String, OfferRejection)> = Vec::new();
    let mut best: Option<(&AssistOffer, f32)> = None;
    for offer in offers {
        match evaluate_offer(offer, request) {
            Ok(()) => {
                let score = score_offer(offer, request);
                let replace = match best {
                    None => true,
                    Some((current, current_score)) => {
                        score > current_score
                            || (score == current_score && offer.peer_id < current.peer_id)
                    }
                };
                if replace {
                    best = Some((offer, score));
                }
            }
            Err(reason) => rejected.push((offer.peer_id.clone(), reason)),
        }
    }
    best.map(|(o, _)| o).ok_or(rejected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> AssistRequest {
        AssistRequest {
            capability: "embeddings".into(),
            cpu_cores: 2,
            ram_mb: 512,
        }
    }

    fn offer(peer: &str) -> AssistOffer {
        AssistOffer {
            peer_id: peer.into(),
            capability: "embeddings".into(),
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
    fn gates_reject_each_violation_class() {
        let r = req();
        let mut o = offer("b");
        o.capability = "ocr".into();
        assert_eq!(
            evaluate_offer(&o, &r),
            Err(OfferRejection::CapabilityMismatch {
                offered: "ocr".into(),
                wanted: "embeddings".into()
            })
        );
        o = offer("b");
        o.cpu_cores = 1;
        assert!(matches!(
            evaluate_offer(&o, &r),
            Err(OfferRejection::NotEnoughCpu { .. })
        ));
        o = offer("b");
        o.ram_mb = 128;
        assert!(matches!(
            evaluate_offer(&o, &r),
            Err(OfferRejection::NotEnoughRam { .. })
        ));
        o = offer("b");
        o.sampled_ago_secs = 120;
        assert!(matches!(
            evaluate_offer(&o, &r),
            Err(OfferRejection::StaleAdvertisement { .. })
        ));
        o = offer("b");
        o.queue_depth = 9;
        assert!(matches!(
            evaluate_offer(&o, &r),
            Err(OfferRejection::BusyQueue { .. })
        ));
        o = offer("b");
        o.has_recent_failure = true;
        assert_eq!(evaluate_offer(&o, &r), Err(OfferRejection::RecentFailure));
    }

    #[test]
    fn select_prefers_exact_fit_over_over_offering() {
        let r = req();
        let exact = offer("exact");
        let mut huge = offer("huge");
        huge.cpu_cores = 16;
        huge.ram_mb = 8192;
        let offers = [exact.clone(), huge.clone()];
        let picked = select_offer(offers.iter(), &r).unwrap();
        assert_eq!(
            picked.peer_id, "exact",
            "exact fit wins: over-offering wastes shared capacity"
        );
    }

    #[test]
    fn fairness_breaks_ties_toward_contributors_without_beating_hard_gates() {
        let r = req();
        // peer ids already encode the tie order ("aaa-neutral" sorts first)
        let neutral = offer("aaa-neutral");
        let mut giver = offer("zzz-giver");
        giver.contribution_balance = 150;

        let offers = [neutral.clone(), giver.clone()];
        let picked = select_offer(offers.iter(), &r).unwrap();
        assert_eq!(
            picked.peer_id, "zzz-giver",
            "recent contributor wins the otherwise-equal tie despite sorting last"
        );

        // But fairness NEVER resurrects a hard-gate failure:
        let mut failed_giver = offer("zzz-giver");
        failed_giver.contribution_balance = 150;
        failed_giver.has_recent_failure = true;
        let offers = [neutral.clone(), failed_giver];
        let picked = select_offer(offers.iter(), &r).unwrap();
        assert_eq!(picked.peer_id, "aaa-neutral");
    }

    #[test]
    fn all_rejected_returns_full_explanation() {
        let r = req();
        let mut stale = offer("stale");
        stale.sampled_ago_secs = 500;
        let mut wrong_cap = offer("wrong-cap");
        wrong_cap.capability = "tts".into();
        let offers = [stale, wrong_cap];

        let err = select_offer(offers.iter(), &r).unwrap_err();
        assert_eq!(err.len(), 2, "every rejection carries its own reason");
    }

    #[test]
    fn deterministic_tiebreak_by_peer_id() {
        let r = req();
        let offers = [offer("a"), offer("b")];
        for _ in 0..5 {
            let picked = select_offer(offers.iter(), &r).unwrap();
            assert_eq!(picked.peer_id, "a", "stable tie order, no randomness");
        }
    }
}

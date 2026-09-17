//! Metered billing for assisted compute (§7 `compute`).
//!
//! A versioned rate card converts MEASURED usage into integer micro-CU.
//! Rules, all deliberate:
//!
//! - Measured only: billing inputs are engine/router-reported token counts.
//!   Absent usage bills 0 (no charge for unmeasured work) — never estimated.
//! - Rounding is per-component CEIL: any real measured work bills at least
//!   1 µCU; exact zeros bill 0. Nothing is ever billed above what the
//!   measured counts imply.
//! - The card is versioned (`RATE_CARD_VERSION`) and echoed in every
//!   receipt, so a caller can predict the bill and detect card changes.
//! - Units are synthetic bookkeeping (micro-CU integers), consistent with
//!   the rest of the non-monetary economy. No money moves anywhere here.

use serde::{Deserialize, Serialize};

/// Active rate-card version. Bump if and only if any rate below changes;
/// receipts echo it so callers detect card drift.
pub const RATE_CARD_VERSION: u32 = 1;

/// Billable rates, micro-CU per quantum (v1, agreed §7 rates).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateCard {
    /// Card version (mirrors [`RATE_CARD_VERSION`]).
    pub version: u32,
    /// Embeddings: micro-CU per 1000 INPUT tokens.
    pub embeddings_per_1k_in: u64,
    /// Chat: micro-CU per 500 OUTPUT tokens.
    pub chat_per_500_out: u64,
    /// Chat: micro-CU per 2000 INPUT tokens.
    pub chat_per_2k_in: u64,
    /// OCR: micro-CU per page.
    pub ocr_per_page: u64,
}

impl RateCard {
    /// The v1 card (§7 agreed rates).
    pub const fn v1() -> Self {
        Self {
            version: RATE_CARD_VERSION,
            embeddings_per_1k_in: 1,
            chat_per_500_out: 1,
            chat_per_2k_in: 1,
            ocr_per_page: 1,
        }
    }
}

/// Ceiling division (rounds partial quanta up to a full billed unit).
fn ceil_div(n: u64, d: u64) -> u64 {
    n.saturating_add(d.saturating_sub(1)) / d.max(1)
}

/// Bills measured usage for a capability under the card. Unknown
/// capabilities bill 0 (no executor exists for them either — and failed
/// executions are never billed by the caller).
pub fn bill(card: &RateCard, capability: &str, tokens_in: u64, tokens_out: u64, pages: u64) -> u64 {
    match capability {
        "embeddings" => ceil_div(tokens_in, 1000).saturating_mul(card.embeddings_per_1k_in),
        "chat" | "text_generation" => ceil_div(tokens_out, 500)
            .saturating_mul(card.chat_per_500_out)
            .saturating_add(ceil_div(tokens_in, 2000).saturating_mul(card.chat_per_2k_in)),
        "ocr" => pages.saturating_mul(card.ocr_per_page),
        _ => 0,
    }
}

/// Upper-bound cost estimate from a REQUEST (before execution), for the
/// over-quota pre-check. Input tokens are upper-bounded by input chars
/// (no tokenizer emits more than one token per char for these backends);
/// output is bounded by the requested `max_tokens`. Unknown capabilities
/// estimate 0 (their executor fails before any billing anyway).
pub fn estimate_cost(card: &RateCard, capability: &str, input_chars: u64, max_tokens: u64) -> u64 {
    match capability {
        "embeddings" => bill(card, capability, input_chars, 0, 0),
        "chat" | "text_generation" => bill(card, capability, input_chars, max_tokens, 0),
        "ocr" => bill(card, capability, 0, 0, 1),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceil_rounds_partial_quanta_up() {
        let card = RateCard::v1();
        // 3 output tokens still bill 1 (never free real work).
        assert_eq!(bill(&card, "chat", 0, 3, 0), 1);
        // Exact zeros bill 0 (absent stays absent).
        assert_eq!(bill(&card, "chat", 0, 0, 0), 0);
        assert_eq!(bill(&card, "embeddings", 0, 0, 0), 0);
        // Component math: 2000 in + 500 out = 1 + 1.
        assert_eq!(bill(&card, "chat", 2000, 500, 0), 2);
        // 1000 embedding tokens = 1.
        assert_eq!(bill(&card, "embeddings", 1000, 0, 0), 1);
        // Unknown capability bills nothing.
        assert_eq!(bill(&card, "teleport", 99999, 99999, 99), 0);
        // OCR is per page.
        assert_eq!(bill(&card, "ocr", 0, 0, 3), 3);
    }

    #[test]
    fn estimate_bounds_realistic_calls() {
        let card = RateCard::v1();
        // A small chat request estimates > 0 (blocks only the truly broke).
        assert!(estimate_cost(&card, "chat", 40, 64) > 0);
        // Unknown capability estimates 0 (fails before billing anyway).
        assert_eq!(estimate_cost(&card, "teleport", 40, 64), 0);
    }

    #[test]
    fn card_version_is_pinned() {
        assert_eq!(RateCard::v1().version, RATE_CARD_VERSION);
        assert_eq!(RATE_CARD_VERSION, 1);
    }
}

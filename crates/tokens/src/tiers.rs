//! Contribution → token-tier application (P4).
//!
//! The subscription model's promise is "your tier reflects your
//! contribution". M17 measures contribution (compute served) and suggests a
//! tier per worker. This module turns those suggestions into a concrete,
//! admin-confirmable set of changes against the **token registry** — the
//! thing that actually gates models and request rates at the proxy.
//!
//! A token and a worker are paired by name: each contributor runs one node,
//! and its `token.name` is also its advertised `node_name` in the
//! contribution report. The planner is pure and I/O-free so the CLI can
//! render a dry-run from the same inputs it later applies, and so policy
//! ("does a Guest earn their way to Contributor?") is unit-testable.

use crate::{Tier, TokenRecord};

/// A single tier change proposed for an active token.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TierChange {
    /// Token name (== worker node name).
    pub name: String,
    /// The token's current tier as stored.
    pub from: u8,
    /// The tier suggested by measured contribution.
    pub to: u8,
}

/// One worker's suggested tier, paired to a token by name.
#[derive(Debug, Clone)]
pub struct SuggestedTier {
    pub name: String,
    /// 1 (guest), 2 (contributor), or 3 (core).
    pub suggested: u8,
}

/// Computes which active tokens change tier to match their measured
/// contribution. A suggestion pairs to the active token of the same name;
/// suggestions with no matching active token, and out-of-range tiers, are
/// skipped. Tokens already at the suggested tier are not emitted. The result
/// is deterministic (sorted by name) so a dry-run matches a later apply.
pub fn plan_tier_changes(
    suggestions: &[SuggestedTier],
    tokens: &[TokenRecord],
) -> Vec<TierChange> {
    let mut changes = Vec::new();
    for s in suggestions {
        if !Tier::parse(s.suggested).is_ok() {
            continue;
        }
        let Some(rec) = tokens
            .iter()
            .find(|r| !r.revoked && r.name == s.name)
        else {
            continue;
        };
        if rec.tier != s.suggested {
            changes.push(TierChange {
                name: s.name.clone(),
                from: rec.tier,
                to: s.suggested,
            });
        }
    }
    changes.sort_by(|a, b| a.name.cmp(&b.name));
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(name: &str, tier: u8) -> TokenRecord {
        TokenRecord {
            name: name.to_string(),
            tier,
            created_at: 1,
            revoked: false,
        }
    }

    #[test]
    fn matches_suggestions_to_active_tokens_by_name() {
        let suggestions = vec![
            SuggestedTier {
                name: "alice".into(),
                suggested: 2,
            },
            SuggestedTier {
                name: "bob".into(),
                suggested: 3,
            },
        ];
        let tokens = vec![token("alice", 1), token("bob", 1)];
        let changes = plan_tier_changes(&suggestions, &tokens);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].name, "alice");
        assert_eq!((changes[0].from, changes[0].to), (1, 2));
        assert_eq!(changes[1].name, "bob");
        assert_eq!((changes[1].from, changes[1].to), (1, 3));
    }

    #[test]
    fn no_change_when_already_at_suggested_tier() {
        let suggestions = vec![SuggestedTier {
            name: "carol".into(),
            suggested: 3,
        }];
        let tokens = vec![token("carol", 3)];
        assert!(plan_tier_changes(&suggestions, &tokens).is_empty());
    }

    #[test]
    fn skips_revoked_tokens_and_unknown_names() {
        let mut revoked = token("dave", 1);
        revoked.revoked = true;
        let suggestions = vec![
            SuggestedTier {
                name: "dave".into(),
                suggested: 2,
            },
            SuggestedTier {
                name: "nobody".into(),
                suggested: 2,
            },
        ];
        let tokens = vec![revoked];
        assert!(plan_tier_changes(&suggestions, &tokens).is_empty());
    }

    #[test]
    fn skips_out_of_range_suggested_tiers() {
        let suggestions = vec![SuggestedTier {
            name: "eve".into(),
            suggested: 0,
        }];
        let tokens = vec![token("eve", 1)];
        assert!(plan_tier_changes(&suggestions, &tokens).is_empty());
    }

    #[test]
    fn planning_is_deterministic_sorted_by_name() {
        let suggestions = vec![
            SuggestedTier {
                name: "zeta".into(),
                suggested: 2,
            },
            SuggestedTier {
                name: "alpha".into(),
                suggested: 2,
            },
        ];
        let tokens = vec![token("zeta", 1), token("alpha", 1)];
        let changes = plan_tier_changes(&suggestions, &tokens);
        assert_eq!(
            changes.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["alpha", "zeta"]
        );
    }

    #[test]
    fn pairing_is_first_match_across_duplicates() {
        // The registry dedups names on create, but a revoked name can be
        // reused; only the active one should be paired.
        let mut stale = token("nick", 1);
        stale.revoked = true;
        let fresh = token("nick", 2);
        let suggestions = vec![SuggestedTier {
            name: "nick".into(),
            suggested: 3,
        }];
        let tokens = vec![stale, fresh];
        let changes = plan_tier_changes(&suggestions, &tokens);
        assert_eq!(changes.len(), 1);
        assert_eq!((changes[0].from, changes[0].to), (2, 3));
    }
}
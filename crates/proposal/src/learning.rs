//! Learning — derived from evidence, never invented (Evidence → Learning).
//!
//! [`derive_learnings`] aggregates sealed evidence into per-proposal
//! learning entries: outcome counts and success rates computed from what
//! actually ran. Zero evidence in, zero learnings out — the same honesty
//! rule as the fabric evidence index. Statements are templated from
//! aggregates (counts, rates); no free-text conclusions are generated.

use crate::evidence::{ExperimentEvidence, ExperimentOutcome};

/// One derived learning, bound to the evidence it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LearningEntry {
    /// Stable id (`learn:<proposal_id>`).
    pub id: String,
    /// Proposal the learning summarizes.
    pub proposal_id: String,
    /// Evidence ids this learning was derived from.
    pub evidence_ids: Vec<String>,
    /// Aggregate-derived statement (counts/rates only).
    pub statement: String,
    /// Majority outcome across the evidence (ties → least favorable:
    /// Failed > Inconclusive > Partial > Success).
    pub outcome: ExperimentOutcome,
    /// Success share in basis points (0..=10000), integer math.
    pub success_bp: u32,
}

/// Aggregate sealed evidence into learnings, one per proposal.
///
/// Pure and deterministic: entries sorted by proposal id, outcomes counted,
/// statement templated from the counts. Empty input yields empty output.
#[must_use]
pub fn derive_learnings(evidence: &[ExperimentEvidence]) -> Vec<LearningEntry> {
    use std::collections::BTreeMap;

    let mut by_proposal: BTreeMap<&str, Vec<&ExperimentEvidence>> = BTreeMap::new();
    for e in evidence {
        by_proposal
            .entry(e.proposal_id.as_str())
            .or_default()
            .push(e);
    }
    by_proposal
        .into_iter()
        .map(|(proposal_id, evs)| {
            let mut success = 0u32;
            let mut partial = 0u32;
            let mut failed = 0u32;
            let mut inconclusive = 0u32;
            for e in &evs {
                match e.outcome {
                    ExperimentOutcome::Success => success += 1,
                    ExperimentOutcome::Partial => partial += 1,
                    ExperimentOutcome::Failed => failed += 1,
                    ExperimentOutcome::Inconclusive => inconclusive += 1,
                }
            }
            let total = evs.len() as u32;
            // Integer-only success share; zero evidence yields zero.
            let success_bp = success
                .checked_mul(10_000)
                .and_then(|scaled| scaled.checked_div(total))
                .unwrap_or(0);
            let outcome = if failed > 0 {
                ExperimentOutcome::Failed
            } else if inconclusive > 0 {
                ExperimentOutcome::Inconclusive
            } else if partial > 0 {
                ExperimentOutcome::Partial
            } else {
                ExperimentOutcome::Success
            };
            LearningEntry {
                id: format!("learn:{proposal_id}"),
                proposal_id: proposal_id.to_string(),
                evidence_ids: evs.iter().map(|e| e.id.clone()).collect(),
                statement: format!(
                    "{proposal_id}: {success}/{total} success, {partial} partial, \
                     {failed} failed, {inconclusive} inconclusive across {} sealed evidence",
                    evs.len()
                ),
                outcome,
                success_bp,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economic::DenyAllEconomicAuthorization;
    use crate::evidence::{EvidenceLog, ExperimentEvidence};
    use crate::policy::decide;
    use crate::protocol::parse_proposal;
    use crate::sandbox::execute;

    fn report_at(at: u64) -> crate::sandbox::ExecutionReport {
        let p = parse_proposal(&crate::protocol::sandbox_proposal_json()).unwrap();
        let d = decide(&p, &DenyAllEconomicAuthorization);
        execute(&p, &d, at).unwrap()
    }

    #[test]
    fn empty_evidence_yields_no_learnings() {
        assert!(derive_learnings(&[]).is_empty());
    }

    #[test]
    fn aggregates_counts_and_rates() {
        let e0 = ExperimentEvidence::seal(&report_at(1), ExperimentOutcome::Success, 1, None);
        let e1 =
            ExperimentEvidence::seal(&report_at(2), ExperimentOutcome::Failed, 2, Some(e0.hash));
        assert!(e0.verify_seal() && e1.verify_seal());
        let learnings = derive_learnings(&[e0, e1]);
        assert_eq!(learnings.len(), 1);
        let l = &learnings[0];
        assert_eq!(l.outcome, ExperimentOutcome::Failed);
        assert_eq!(l.success_bp, 5_000);
        assert!(l.statement.contains("1/2 success"));
        assert_eq!(l.evidence_ids.len(), 2);
    }

    #[test]
    fn log_chain_feeds_learning() {
        let r = {
            let p = parse_proposal(&crate::protocol::sandbox_proposal_json()).unwrap();
            let d = decide(&p, &DenyAllEconomicAuthorization);
            execute(&p, &d, 7).unwrap()
        };
        let mut log = EvidenceLog::new();
        log.append(ExperimentEvidence::seal(
            &r,
            ExperimentOutcome::Success,
            7,
            None,
        ))
        .unwrap();
        let learnings = derive_learnings(log.entries());
        assert_eq!(learnings.len(), 1);
        assert_eq!(learnings[0].success_bp, 10_000);
    }
}

//! Result verification + consensus (P4) — the pure decision layer an
//! orchestrator runs after an agent returns a result (used by P3 delegation
//! and P9 workflows).
//!
//! # Why pure and separate
//!
//! Verification must be *explainable and deterministic* before it can feed
//! reputation (P6) or memory (P5): every verdict carries the checks that led
//! to it, every consensus decision is a pure function of the opinions, and
//! every disagreement escalates through a fixed ordered ladder. All types are
//! serde-serializable so reports can travel over the P2P channel or be
//! persisted alongside the task.
//!
//! # Honesty boundary
//!
//! [`check_output_schema`] is deliberately shallow: it checks *that* an
//! output parses as JSON, not *whether* it satisfies the schema's
//! constraints. A full JSON-Schema validator is out of scope, and we never
//! claim more validation than we perform. `serde_json` is a dev-only
//! dependency (wire fields are `String`, never `serde_json::Value`), so the
//! parse check is a small structural validator with no external deps.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

/// The outcome of verifying an agent's result.
///
/// `Verified` carries no reason by design: it is the unremarkable happy path.
/// Only the failure-ish outcomes explain themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationVerdict {
    /// The result passed verification.
    Verified,
    /// The result failed verification; the reason is actionable.
    Rejected { reason: String },
    /// Verification could not decide; more evidence / a third opinion is
    /// needed before the result may be consumed.
    Uncertain { reason: String },
}

/// The kind of check that produced an entry in a [`VerificationReport`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    /// Output was checked against the task's `output_schema` hint.
    Schema,
    /// The executing agent self-checked its own output.
    SelfCheck,
    /// A dedicated critic agent reviewed the result.
    CriticReview,
    /// The result was compared against another agent's result.
    CrossCheck,
    /// An agreement threshold across several agents was evaluated.
    Consensus,
    /// Provenance / evidence trace for the result was inspected.
    Evidence,
}

/// One named check inside a [`VerificationReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationCheck {
    /// What was checked.
    pub check_kind: CheckKind,
    /// Whether the check passed.
    pub passed: bool,
    /// Human-readable detail; free text that explains the outcome.
    pub detail: String,
}

/// Clamps a confidence value into `0.0..=1.0`.
///
/// Wire data may carry garbage; every report constructor must pass through
/// here so confidence is always a defensible fraction.
pub fn clamped_confidence(confidence: f32) -> f32 {
    confidence.clamp(0.0, 1.0)
}

/// A full verification result for one task, produced by one agent (or by the
/// fabric when the verdict is `Consensus`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationReport {
    /// The task that was verified.
    pub task_id: String,
    /// The agent (or verifier) that produced this report.
    pub agent_id: String,
    /// The overall outcome.
    pub verdict: VerificationVerdict,
    /// The individual checks that led to the verdict.
    pub checks: Vec<VerificationCheck>,
    /// Aggregated confidence in the verdict, clamped to `0.0..=1.0`.
    pub confidence: f32,
    /// Wall-clock timestamp of verification (ms since epoch).
    pub verified_at_ms: u64,
}

impl VerificationReport {
    /// Builds a report, clamping `confidence` into `0.0..=1.0`.
    ///
    /// The clamp happens here (the construction boundary) so a report that
    /// reaches the ledger always carries a defensible fraction; deserialized
    /// wire values are trusted after they pass through this constructor.
    pub fn new(
        task_id: impl Into<String>,
        agent_id: impl Into<String>,
        verdict: VerificationVerdict,
        checks: Vec<VerificationCheck>,
        confidence: f32,
        verified_at_ms: u64,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            agent_id: agent_id.into(),
            verdict,
            checks,
            confidence: clamped_confidence(confidence),
            verified_at_ms,
        }
    }
}

/// A lightweight honesty check that an output is shaped like its schema hint.
///
/// The check is deliberately shallow — it only verifies *that* both strings
/// parse as JSON when the hint claims to be JSON. It never validates the
/// output *against* the schema's constraints; a real JSON-Schema validator is
/// out of scope, and we never claim more validation than we perform.
///
/// Returns a [`VerificationCheck`] of kind [`CheckKind::Schema`].
pub fn check_output_schema(output: &str, schema_hint: Option<&str>) -> VerificationCheck {
    let (passed, detail) = match schema_hint {
        None => (true, "no schema required".to_string()),
        Some(hint) if !looks_like_json(hint) => (
            true,
            "schema hint not JSON — structural check skipped (honest)".to_string(),
        ),
        Some(_) if looks_like_json(output) => (true, "output is valid JSON".to_string()),
        Some(_) => (
            false,
            "output is not valid JSON per schema hint".to_string(),
        ),
    };
    VerificationCheck {
        check_kind: CheckKind::Schema,
        passed,
        detail,
    }
}

/// Deterministic, explainable JSON-shape detector used by the schema check.
///
/// Full recursive-descent validation (objects, arrays, strings with escapes,
/// numbers, literals) with no dependencies, so `serde_json` stays a dev-only
/// dependency.
fn looks_like_json(input: &str) -> bool {
    let b = input.as_bytes();
    let mut pos = 0;
    skip_ws(b, &mut pos);
    if !parse_value(b, &mut pos) {
        return false;
    }
    skip_ws(b, &mut pos);
    pos == b.len()
}

fn skip_ws(b: &[u8], pos: &mut usize) {
    while *pos < b.len() && matches!(b[*pos], b' ' | b'\t' | b'\n' | b'\r') {
        *pos += 1;
    }
}

fn parse_value(b: &[u8], pos: &mut usize) -> bool {
    if *pos >= b.len() {
        return false;
    }
    match b[*pos] {
        b'{' => parse_object(b, pos),
        b'[' => parse_array(b, pos),
        b'"' => parse_string(b, pos),
        b't' => parse_literal(b, pos, b"true"),
        b'f' => parse_literal(b, pos, b"false"),
        b'n' => parse_literal(b, pos, b"null"),
        b'-' | b'0'..=b'9' => parse_number(b, pos),
        _ => false,
    }
}

fn parse_object(b: &[u8], pos: &mut usize) -> bool {
    *pos += 1;
    skip_ws(b, pos);
    if *pos < b.len() && b[*pos] == b'}' {
        *pos += 1;
        return true;
    }
    loop {
        skip_ws(b, pos);
        if !parse_string(b, pos) {
            return false;
        }
        skip_ws(b, pos);
        if *pos >= b.len() || b[*pos] != b':' {
            return false;
        }
        *pos += 1;
        skip_ws(b, pos);
        if !parse_value(b, pos) {
            return false;
        }
        skip_ws(b, pos);
        if *pos >= b.len() {
            return false;
        }
        match b[*pos] {
            b',' => {
                *pos += 1;
            }
            b'}' => {
                *pos += 1;
                return true;
            }
            _ => return false,
        }
    }
}

fn parse_array(b: &[u8], pos: &mut usize) -> bool {
    *pos += 1;
    skip_ws(b, pos);
    if *pos < b.len() && b[*pos] == b']' {
        *pos += 1;
        return true;
    }
    loop {
        skip_ws(b, pos);
        if !parse_value(b, pos) {
            return false;
        }
        skip_ws(b, pos);
        if *pos >= b.len() {
            return false;
        }
        match b[*pos] {
            b',' => {
                *pos += 1;
            }
            b']' => {
                *pos += 1;
                return true;
            }
            _ => return false,
        }
    }
}

fn parse_string(b: &[u8], pos: &mut usize) -> bool {
    if *pos >= b.len() || b[*pos] != b'"' {
        return false;
    }
    *pos += 1;
    while *pos < b.len() {
        match b[*pos] {
            b'"' => {
                *pos += 1;
                return true;
            }
            b'\\' => {
                *pos += 1;
                if *pos >= b.len() {
                    return false;
                }
                match b[*pos] {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {
                        *pos += 1;
                    }
                    b'u' => {
                        *pos += 1;
                        if *pos + 4 > b.len() {
                            return false;
                        }
                        for _ in 0..4 {
                            if !b[*pos].is_ascii_hexdigit() {
                                return false;
                            }
                            *pos += 1;
                        }
                    }
                    _ => return false,
                }
            }
            // Unescaped control characters are illegal in JSON.
            c if c < b' ' => return false,
            _ => *pos += 1,
        }
    }
    false
}

fn parse_literal(b: &[u8], pos: &mut usize, lit: &[u8]) -> bool {
    if *pos + lit.len() > b.len() || b[*pos..*pos + lit.len()] != *lit {
        return false;
    }
    *pos += lit.len();
    true
}

fn parse_number(b: &[u8], pos: &mut usize) -> bool {
    if *pos >= b.len() {
        return false;
    }
    if b[*pos] == b'-' {
        *pos += 1;
    }
    if *pos >= b.len() {
        return false;
    }
    match b[*pos] {
        b'0' => *pos += 1,
        b'1'..=b'9' => {
            while *pos < b.len() && b[*pos].is_ascii_digit() {
                *pos += 1;
            }
        }
        _ => return false,
    }
    if *pos < b.len() && b[*pos] == b'.' {
        *pos += 1;
        if *pos >= b.len() || !b[*pos].is_ascii_digit() {
            return false;
        }
        while *pos < b.len() && b[*pos].is_ascii_digit() {
            *pos += 1;
        }
    }
    if *pos < b.len() && (b[*pos] == b'e' || b[*pos] == b'E') {
        *pos += 1;
        if *pos < b.len() && (b[*pos] == b'+' || b[*pos] == b'-') {
            *pos += 1;
        }
        if *pos >= b.len() || !b[*pos].is_ascii_digit() {
            return false;
        }
        while *pos < b.len() && b[*pos].is_ascii_digit() {
            *pos += 1;
        }
    }
    true
}

/// One agent's vote in a consensus evaluation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsensusResult {
    /// The voting agent.
    pub agent_id: String,
    /// Whether this agent's result agrees with the candidate result.
    pub agrees: bool,
    /// The agent's confidence in its own result, `0.0..=1.0`.
    pub confidence: f32,
}

/// Policy governing a consensus evaluation.
///
/// `require_schema` is reserved for the orchestrator (P3/P9): the per-result
/// schema gate is a separate [`check_output_schema`] decision that runs before
/// the votes are collected, so this pure function only consumes
/// `required_agents` and `agreement_threshold`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsensusPolicy {
    /// Minimum number of opinions before a consensus verdict is allowed.
    pub required_agents: u32,
    /// Fraction of agreeing opinions required for `Verified`.
    pub agreement_threshold: f32,
    /// Whether results must also pass their schema check before the vote
    /// counts (enforced by the caller, not by this pure function).
    pub require_schema: bool,
}

impl Default for ConsensusPolicy {
    fn default() -> Self {
        Self {
            required_agents: 3,
            agreement_threshold: 0.6,
            require_schema: false,
        }
    }
}

/// Pure consensus decision over a set of agent opinions.
///
/// - Fewer than `required_agents` opinions → [`VerificationVerdict::Uncertain`]
///   (the fabric must not decide on thin evidence).
/// - Otherwise the fraction of agreeing opinions is compared against
///   `agreement_threshold`: at or above it → `Verified`, below it → `Rejected`.
/// - A zero-confidence agreement is still *counted* (a vote is a vote), but it
///   is flagged in the reason string whenever the verdict can carry one.
///   `Verified` has no reason slot by design, so for that case the note is
///   left to the caller's `Consensus` check detail.
pub fn evaluate_consensus(
    results: &[ConsensusResult],
    policy: &ConsensusPolicy,
) -> VerificationVerdict {
    let required = policy.required_agents as usize;
    if results.len() < required {
        return VerificationVerdict::Uncertain {
            reason: format!("consensus needs {required} opinions, got {}", results.len()),
        };
    }

    let agrees = results.iter().filter(|r| r.agrees).count();
    let zero_confidence_agrees: Vec<&str> = results
        .iter()
        .filter(|r| r.agrees && r.confidence == 0.0)
        .map(|r| r.agent_id.as_str())
        .collect();

    let fraction = agrees as f32 / results.len() as f32;
    if fraction >= policy.agreement_threshold {
        return VerificationVerdict::Verified;
    }

    let note = if zero_confidence_agrees.is_empty() {
        String::new()
    } else {
        format!(
            "; zero-confidence agreement from {} counted (noted, not trusted)",
            zero_confidence_agrees.join(", ")
        )
    };
    VerificationVerdict::Rejected {
        reason: format!(
            "agreement {fraction:.2} below threshold {:.2} ({agrees}/{}){note}",
            results.len(),
            policy.agreement_threshold
        ),
    }
}

/// How the fabric should respond to two disagreeing reports, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisagreementResolution {
    /// Ask a third agent for an independent opinion.
    ThirdAgent,
    /// Look for evidence (provenance, intermediate artifacts).
    SeekEvidence,
    /// Run a benchmark to decide objectively.
    RunBenchmark,
    /// Escalate to a human operator.
    HumanReview,
    /// Give up on deciding; mark the result uncertain.
    MarkUncertain,
}

/// Pure decision on how to resolve a disagreement between two reports.
///
/// - Either verdict is `Uncertain` → the disagreement cannot be reasoned
///   about; mark the result uncertain (`MarkUncertain`).
/// - Both reports agree (both `Verified` or both `Rejected`) → no resolution
///   needed.
/// - One `Verified` and one `Rejected` → escalate through the fixed,
///   deterministic ladder `ThirdAgent → SeekEvidence → RunBenchmark`.
///
/// `HumanReview` is deliberately never auto-selected here: it is the operator
/// escape hatch, chosen by the orchestrator when the ladder is exhausted.
pub fn resolve_disagreement(
    first: &VerificationReport,
    second: &VerificationReport,
) -> Vec<DisagreementResolution> {
    use VerificationVerdict::{Rejected, Uncertain, Verified};
    match (&first.verdict, &second.verdict) {
        (Uncertain { .. }, _) | (_, Uncertain { .. }) => {
            vec![DisagreementResolution::MarkUncertain]
        }
        (Verified, Verified) | (Rejected { .. }, Rejected { .. }) => Vec::new(),
        (Verified, Rejected { .. }) | (Rejected { .. }, Verified) => vec![
            DisagreementResolution::ThirdAgent,
            DisagreementResolution::SeekEvidence,
            DisagreementResolution::RunBenchmark,
        ],
    }
}

/// Maximum number of reports the [`VerificationLedger`] keeps.
///
/// Verification results are immutable per task (see [`VerificationError`]), so
/// eviction only ever drops the *oldest* tasks — never the current one.
pub const MAX_EVENTS: usize = 512;

/// Ledger errors — all recoverable and explainable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerificationError {
    /// A report for this task already exists; verification results are
    /// immutable per task, so re-recording is an error, not an overwrite.
    #[error("verification result for task '{task_id}' is already recorded and immutable")]
    DuplicateTaskId { task_id: String },
}

/// Bounded, deterministic history of verification reports keyed by task id.
///
/// Reports are appended in arrival order; beyond [`MAX_EVENTS`] the oldest
/// entries are evicted, so the ledger is a fixed-size window of the most
/// recent verification activity. `list`/`recent` return newest-first.
#[derive(Debug, Clone, Default)]
pub struct VerificationLedger {
    reports: BTreeMap<String, VerificationReport>,
    order: VecDeque<String>,
}

impl VerificationLedger {
    /// An empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a report. Fails with [`VerificationError::DuplicateTaskId`]
    /// when the task was already verified — a task's verification result is
    /// immutable once recorded.
    pub fn record(&mut self, report: VerificationReport) -> Result<(), VerificationError> {
        let task_id = report.task_id.clone();
        if self.reports.contains_key(&task_id) {
            return Err(VerificationError::DuplicateTaskId { task_id });
        }
        self.reports.insert(task_id.clone(), report);
        self.order.push_back(task_id);
        while self.order.len() > MAX_EVENTS {
            if let Some(oldest) = self.order.pop_front() {
                self.reports.remove(&oldest);
            }
        }
        Ok(())
    }

    /// Looks up a report by task id.
    pub fn get(&self, task_id: &str) -> Option<&VerificationReport> {
        self.reports.get(task_id)
    }

    /// All recorded reports, newest first.
    pub fn list(&self) -> Vec<VerificationReport> {
        self.order
            .iter()
            .rev()
            .filter_map(|id| self.reports.get(id).cloned())
            .collect()
    }

    /// Number of recorded reports.
    pub fn count(&self) -> usize {
        self.reports.len()
    }

    /// The `n` most recent reports, newest first.
    pub fn recent(&self, n: usize) -> Vec<VerificationReport> {
        self.order
            .iter()
            .rev()
            .take(n)
            .filter_map(|id| self.reports.get(id).cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(task_id: &str, verdict: VerificationVerdict) -> VerificationReport {
        VerificationReport::new(task_id, "a:1", verdict, Vec::new(), 1.0, 42)
    }

    fn result(agent_id: &str, agrees: bool, confidence: f32) -> ConsensusResult {
        ConsensusResult {
            agent_id: agent_id.into(),
            agrees,
            confidence,
        }
    }

    #[test]
    fn schema_check_passes_when_no_schema_required() {
        let check = check_output_schema("anything", None);
        assert!(check.passed);
        assert_eq!(check.check_kind, CheckKind::Schema);
        assert_eq!(check.detail, "no schema required");
    }

    #[test]
    fn schema_check_passes_json_output_under_json_hint() {
        let check = check_output_schema(r#"{"ok":true}"#, Some(r#"{"type":"object"}"#));
        assert!(check.passed);
        assert_eq!(check.detail, "output is valid JSON");
    }

    #[test]
    fn schema_check_rejects_non_json_output_under_json_hint() {
        let check = check_output_schema("not json at all", Some(r#"{"type":"object"}"#));
        assert!(!check.passed);
        assert_eq!(check.detail, "output is not valid JSON per schema hint");
    }

    #[test]
    fn schema_check_is_honest_about_non_json_hint() {
        let check = check_output_schema("anything", Some("just prose"));
        assert!(check.passed);
        assert!(check.detail.contains("structural check skipped"));
    }

    #[test]
    fn json_detector_accepts_common_shapes_and_rejects_garbage() {
        assert!(looks_like_json(r#"{"a":[1,2.5,true,null,"s"]}"#));
        assert!(looks_like_json("[ 1 , -2.5e3, false ]"));
        assert!(looks_like_json("\"just a string\""));
        assert!(looks_like_json("  12  "));
        assert!(!looks_like_json("{not json}"));
        assert!(!looks_like_json("[1,]"));
        assert!(!looks_like_json(""));
    }

    #[test]
    fn report_clamps_confidence_to_unit_range() {
        let high = VerificationReport::new("t", "a", VerificationVerdict::Verified, vec![], 7.5, 1);
        assert_eq!(high.confidence, 1.0);
        let low = VerificationReport::new("t", "a", VerificationVerdict::Verified, vec![], -2.0, 1);
        assert_eq!(low.confidence, 0.0);
        assert_eq!(clamped_confidence(0.5), 0.5);
    }

    #[test]
    fn consensus_needs_enough_opinions_before_deciding() {
        let policy = ConsensusPolicy::default();
        let results = [result("a", true, 1.0), result("b", true, 1.0)];
        match evaluate_consensus(&results, &policy) {
            VerificationVerdict::Uncertain { reason } => {
                assert!(reason.contains("needs 3 opinions, got 2"), "{reason}")
            }
            other => panic!("expected uncertain, got {other:?}"),
        }
    }

    #[test]
    fn consensus_verifies_when_agreement_clears_threshold() {
        let policy = ConsensusPolicy::default();
        let results = [
            result("a", true, 1.0),
            result("b", true, 1.0),
            result("c", false, 0.9),
        ];
        assert_eq!(
            evaluate_consensus(&results, &policy),
            VerificationVerdict::Verified
        );
    }

    #[test]
    fn consensus_rejects_when_agreement_is_insufficient() {
        let policy = ConsensusPolicy::default();
        let results = [
            result("a", true, 1.0),
            result("b", false, 1.0),
            result("c", false, 1.0),
        ];
        match evaluate_consensus(&results, &policy) {
            VerificationVerdict::Rejected { reason } => {
                assert!(reason.contains("agreement 0.33"), "{reason}")
            }
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[test]
    fn consensus_zero_confidence_agreement_counts_toward_threshold() {
        let policy = ConsensusPolicy {
            required_agents: 2,
            agreement_threshold: 0.5,
            require_schema: false,
        };
        let results = [result("a", true, 0.0), result("b", false, 1.0)];
        assert_eq!(
            evaluate_consensus(&results, &policy),
            VerificationVerdict::Verified
        );
    }

    #[test]
    fn consensus_notes_zero_confidence_agreement_in_reason() {
        let policy = ConsensusPolicy {
            required_agents: 2,
            agreement_threshold: 1.0,
            require_schema: false,
        };
        let results = [result("a", true, 0.0), result("b", false, 1.0)];
        match evaluate_consensus(&results, &policy) {
            VerificationVerdict::Rejected { reason } => {
                assert!(
                    reason.contains("zero-confidence agreement from a"),
                    "{reason}"
                )
            }
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[test]
    fn disagreement_one_verified_one_rejected_escalates_in_order() {
        let ok = report("t", VerificationVerdict::Verified);
        let bad = report(
            "t",
            VerificationVerdict::Rejected {
                reason: "nope".into(),
            },
        );
        let expected = vec![
            DisagreementResolution::ThirdAgent,
            DisagreementResolution::SeekEvidence,
            DisagreementResolution::RunBenchmark,
        ];
        assert_eq!(resolve_disagreement(&ok, &bad), expected);
        // The ladder order is deterministic regardless of argument order.
        assert_eq!(resolve_disagreement(&bad, &ok), expected);
    }

    #[test]
    fn disagreement_agreement_needs_no_resolution() {
        let ok1 = report("t", VerificationVerdict::Verified);
        let ok2 = report("t", VerificationVerdict::Verified);
        assert!(resolve_disagreement(&ok1, &ok2).is_empty());
        let bad1 = report("t", VerificationVerdict::Rejected { reason: "x".into() });
        let bad2 = report("t", VerificationVerdict::Rejected { reason: "y".into() });
        assert!(resolve_disagreement(&bad1, &bad2).is_empty());
    }

    #[test]
    fn disagreement_uncertain_sends_to_mark_uncertain() {
        let u = report(
            "t",
            VerificationVerdict::Uncertain {
                reason: "huh".into(),
            },
        );
        let ok = report("t", VerificationVerdict::Verified);
        let bad = report(
            "t",
            VerificationVerdict::Rejected {
                reason: "no".into(),
            },
        );
        assert_eq!(
            resolve_disagreement(&u, &ok),
            vec![DisagreementResolution::MarkUncertain]
        );
        assert_eq!(
            resolve_disagreement(&bad, &u),
            vec![DisagreementResolution::MarkUncertain]
        );
    }

    #[test]
    fn ledger_records_and_looks_up_reports() {
        let mut ledger = VerificationLedger::new();
        assert_eq!(ledger.count(), 0);
        ledger
            .record(report("t:1", VerificationVerdict::Verified))
            .unwrap();
        ledger
            .record(report(
                "t:2",
                VerificationVerdict::Rejected {
                    reason: "nope".into(),
                },
            ))
            .unwrap();
        assert_eq!(ledger.count(), 2);
        assert_eq!(
            ledger.get("t:1").unwrap().verdict,
            VerificationVerdict::Verified
        );
        assert!(ledger.get("t:3").is_none());
    }

    #[test]
    fn ledger_rejects_duplicate_task_ids_as_immutable() {
        let mut ledger = VerificationLedger::new();
        ledger
            .record(report("t:1", VerificationVerdict::Verified))
            .unwrap();
        let err = ledger
            .record(report(
                "t:1",
                VerificationVerdict::Rejected {
                    reason: "late".into(),
                },
            ))
            .unwrap_err();
        assert_eq!(
            err,
            VerificationError::DuplicateTaskId {
                task_id: "t:1".into()
            }
        );
        assert_eq!(
            ledger.get("t:1").unwrap().verdict,
            VerificationVerdict::Verified
        );
    }

    #[test]
    fn ledger_lists_newest_first_and_recent_slices() {
        let mut ledger = VerificationLedger::new();
        for i in 0..5 {
            ledger
                .record(report(&format!("t:{i}"), VerificationVerdict::Verified))
                .unwrap();
        }
        let list = ledger.list();
        assert_eq!(list.len(), 5);
        assert_eq!(list[0].task_id, "t:4");
        assert_eq!(list[4].task_id, "t:0");
        let recent = ledger.recent(2);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].task_id, "t:4");
        assert_eq!(recent[1].task_id, "t:3");
        assert!(ledger.recent(0).is_empty());
    }

    #[test]
    fn ledger_evicts_oldest_beyond_max_events() {
        let mut ledger = VerificationLedger::new();
        for i in 0..(MAX_EVENTS + 3) {
            ledger
                .record(report(&format!("t:{i}"), VerificationVerdict::Verified))
                .unwrap();
        }
        assert_eq!(ledger.count(), MAX_EVENTS);
        assert!(ledger.get("t:0").is_none());
        assert!(ledger.get("t:2").is_none());
        assert!(ledger.get("t:3").is_some());
        assert_eq!(
            ledger.list().first().unwrap().task_id,
            format!("t:{}", MAX_EVENTS + 2)
        );
    }

    #[test]
    fn reports_round_trip_over_json() {
        let report = VerificationReport::new(
            "t:roundtrip",
            "agent:7",
            VerificationVerdict::Rejected {
                reason: "schema mismatch".into(),
            },
            vec![
                VerificationCheck {
                    check_kind: CheckKind::Schema,
                    passed: false,
                    detail: "output is not valid JSON per schema hint".into(),
                },
                VerificationCheck {
                    check_kind: CheckKind::Consensus,
                    passed: true,
                    detail: "2/3 agreement".into(),
                },
            ],
            0.75,
            1_723_900_000_000,
        );
        let json = serde_json::to_string(&report).unwrap();
        let back: VerificationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(report, back);
    }

    #[test]
    fn verdict_and_check_kinds_serialize_in_snake_case() {
        assert_eq!(
            serde_json::to_string(&CheckKind::CriticReview).unwrap(),
            "\"critic_review\""
        );
        assert_eq!(
            serde_json::to_string(&VerificationVerdict::Uncertain { reason: "x".into() }).unwrap(),
            r#"{"uncertain":{"reason":"x"}}"#
        );
    }
}

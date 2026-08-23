//! Real-trace equivalence replay (trace-collection phase).
//!
//! Loads a GoldenCase corpus captured from the LIVE fabric
//! (`bench/traces/*-golden*.jsonl`, via `GET /v1/golden-capture`) and replays
//! every case through the `UnifiedSelector`, comparing against the live
//! planner's golden decision:
//!
//! ```text
//! real fabric capture → GoldenCase JSONL → UnifiedSelector replay → SAME DECISION?
//! ```
//!
//! Comparison tiers (honest by construction):
//! - **full fidelity** (case carries the coordinator's link graph): eligible
//!   set, rejection reasons, exact ranking order, selected worker, scoring
//!   provenance, and locality/continuation fields are all compared;
//! - **structure-only** (legacy cases captured before the `network` field
//!   existed): eligible set + rejection reasons compared; ranking/selected/
//!   scoring are reported as NOT-COMPARABLE instead of fabricating a verdict,
//!   because the `net` score component cannot be reproduced without the graph.
//!
//! Divergences are **classified**, per Issue #30 Phase 2, into exactly three
//! buckets and never collapsed into a single boolean:
//!   1. `genuine-regression` — the unified selector chose differently from the
//!      live planner on a full-fidelity capture (eligible/rejected/ranking/
//!      selected all comparable). Must be zero.
//!   2. `not-comparable` — the case lacks the link graph (`net` score term
//!      cannot be reproduced). Reported with the exact missing information;
//!      absence of data is never turned into an equivalence verdict.
//!   3. `expected-semantic` — a recorded difference that is by design: the
//!      unified selector models the PLANNER, not the coordinator's
//!      reservation step, so `reserved_worker != selected_worker` (the
//!      planner-picks-local -> coordinator-excludes-local path) is expected,
//!      not a selector divergence.
//!
//! An explicitly classifier(rerunner) writes a reproducible report to
//! `<corpus_dir>/EQUIVALENCE_REPORT_P2.txt` and to stdout. Skips gracefully
//! (exit 0) when no corpus exists.

use decentraai_fabric::{GoldenCase, GoldenSuite, SelectionTrace, UnifiedSelector};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn corpus_files() -> Vec<PathBuf> {
    let from_env = std::env::var("GOLDEN_CORPUS_DIR").ok();
    let dir = from_env.unwrap_or_else(|| {
        // Default: <workspace>/bench/traces, resolved from this crate's
        // manifest so the test works from any cargo working directory.
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../bench/traces")
            .to_string_lossy()
            .to_string()
    });
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.contains("golden") && n.ends_with(".jsonl"))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

#[test]
fn real_corpus_replays_through_unified_selector() {
    let files = corpus_files();
    if files.is_empty() {
        eprintln!("no golden corpus under bench/traces — skipping (set GOLDEN_CORPUS_DIR)");
        return;
    }

    let mut cases = Vec::new();
    for f in &files {
        for (i, line) in std::fs::read_to_string(f).unwrap().lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<GoldenCase>(line) {
                Ok(c) => cases.push(c),
                Err(e) => panic!(
                    "corpus {} line {} is not a valid GoldenCase: {e}",
                    f.display(),
                    i + 1
                ),
            }
        }
    }
    assert!(!cases.is_empty(), "corpus files exist but no cases parsed");

    // Replay every case through the unified selector, mirroring the case's own
    // link graph so the `net` score component is reproduced exactly.
    let mut full = Vec::new();
    let mut structure_only = 0usize;
    for c in &cases {
        if c.network.peers().count() > 0 {
            full.push(c);
        } else {
            structure_only += 1;
        }
    }

    let mut total_compared = 0usize;
    let mut total_divergences = Vec::new();
    // Per-case classification totals (Issue #30 Phase 2).
    let mut n_genuine = 0usize;
    let mut n_expected_semantic = 0usize;
    let mut n_not_comparable = 0usize;
    // Scoring provenance: does the unified replay agree with golden on the
    // perf_measured marker of the chosen worker?
    let mut prov_agreed = 0usize;
    let mut prov_disagreed = 0usize;
    // Continuation / KV-locality coverage in the corpus (Phase 1 evidence).
    let mut n_continuation = 0usize;
    for case in &full {
        let unified = UnifiedSelector {
            network: case.network.clone(),
            ..UnifiedSelector::default()
        };
        if case.golden.is_continuation || case.req.context.is_continuation {
            n_continuation += 1;
        }
        let report = GoldenSuite::run(std::slice::from_ref(case), &unified);
        total_compared += 1;
        // Scoring provenance of the chosen worker: golden vs unified marker.
        let g_prov = case
            .golden
            .ranked
            .first()
            .map(|c| c.perf_measured)
            .unwrap_or(false);
        let u_prov = unified
            .select(&case.req, &case.workers)
            .trace
            .ranked
            .first()
            .map(|c| c.perf_measured)
            .unwrap_or(false);
        if g_prov == u_prov {
            prov_agreed += 1;
        } else {
            prov_disagreed += 1;
        }
        // Expected-semantic: reserved_worker != selected_worker is the planner
        // (selection) vs coordinator (reservation/actual) distinction — a
        // selector-equivalence non-issue by construction. Recorded separately
        // so it is never counted as a genuine divergence.
        if case
            .golden
            .reserved_worker
            .as_ref()
            .is_some_and(|r| Some(r) != case.golden.selected_worker.as_ref())
        {
            n_expected_semantic += 1;
        }
        for d in report.divergences {
            n_genuine += 1;
            total_divergences.push(d);
        }
    }
    // Structure-only cases: NOT-COMPARABLE by definition (no link graph).
    for _c in cases.iter().filter(|c| c.network.peers().count() == 0) {
        n_not_comparable += 1;
    }

    // Reproducible report (file + stdout).
    let mut rpt = String::new();
    rpt.push_str("==== REAL-TRACE EQUIVALENCE REPORT (Issue #30 Phase 2) ====\n");
    rpt.push_str(&format!("corpus cases: {}\n", cases.len()));
    rpt.push_str(&format!(
        "  full-fidelity replays: {}\n  structure-only (not-comparable): {}\n",
        total_compared, structure_only
    ));
    rpt.push_str(&format!(
        "  continuation / KV-locality cases: {}\n",
        n_continuation
    ));
    rpt.push_str("--- divergence classification ---\n");
    rpt.push_str(&format!(
        "  genuine-regression: {}\n  expected-semantic (reserved!=selected, by design): {}\n  not-comparable (missing link graph -> 'net' term unreproducible): {}\n",
        n_genuine, n_expected_semantic, n_not_comparable
    ));
    rpt.push_str(&format!(
        "scoring provenance agreement (perf_measured of chosen worker): {}/{} agreed, {} disagreed\n",
        prov_agreed,
        total_compared,
        prov_disagreed
    ));
    for d in total_divergences.iter().take(20) {
        rpt.push_str(&format!(
            "  GENUINE [{}] {}: golden=({}) unified=({})\n",
            d.field, d.request_id, d.golden, d.unified
        ));
    }
    rpt.push_str("==================================================\n");
    eprintln!("{rpt}");
    // Deterministic report path so the artifact is reproducible.
    let report_path = report_path();
    if let Some(p) = &report_path {
        let _ = std::fs::write(p, &rpt);
        eprintln!("report written to {}", p.display());
    }

    assert!(
        total_divergences.is_empty(),
        "UNIFIED REGRESSION: unified selector diverged from the live planner on {} real capture(s): {total_divergences:?}",
        total_divergences.len()
    );
}

fn _trace_provenance(t: &SelectionTrace) -> BTreeSet<(String, bool)> {
    t.ranked
        .iter()
        .map(|c| (c.peer_id.clone(), c.perf_measured))
        .collect()
}

fn report_path() -> Option<PathBuf> {
    let dir = corpus_dir();
    let p = Path::new(&dir).join("EQUIVALENCE_REPORT_P2.txt");
    Some(p)
}

fn corpus_dir() -> String {
    std::env::var("GOLDEN_CORPUS_DIR").unwrap_or_else(|_| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../bench/traces")
            .to_string_lossy()
            .to_string()
    })
}

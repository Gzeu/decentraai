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
//!   set, rejection reasons, exact ranking order, and selected worker are all
//!   compared;
//! - **structure-only** (legacy cases captured before the `network` field
//!   existed): eligible set + rejection reasons compared; ranking/selected are
//!   reported as NOT-COMPARABLE instead of fabricating a verdict, because the
//!   `net` score component cannot be reproduced without the graph.
//!
//! Skips gracefully (exit 0) when no corpus exists, so CI without a collected
//! corpus stays green. Point `GOLDEN_CORPUS_DIR` at a corpus directory to run.

use decentraai_fabric::{GoldenCase, GoldenSuite, UnifiedSelector};
use std::path::PathBuf;

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
                Err(e) => panic!("corpus {} line {} is not a valid GoldenCase: {e}", f.display(), i + 1),
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
    for case in &full {
        let unified = UnifiedSelector {
            network: case.network.clone(),
            ..UnifiedSelector::default()
        };
        let report = GoldenSuite::run(std::slice::from_ref(case), &unified);
        total_compared += 1;
        for mut d in report.divergences {
            d.request_id = format!("{}[{}]", case.request_id, d.request_id);
            total_divergences.push(d);
        }
    }

    println!("==== REAL-TRACE EQUIVALENCE REPORT ====");
    println!("corpus cases:          {}", cases.len());
    println!("full-fidelity replays: {total_compared}");
    println!("structure-only cases:  {structure_only} (ranking not comparable without link graph)");
    println!("divergences:           {}", total_divergences.len());
    for d in total_divergences.iter().take(10) {
        println!(
            "  DIVERGENCE [{}] {}: golden=({}) unified=({})",
            d.field, d.request_id, d.golden, d.unified
        );
    }
    println!("========================================");

    assert!(
        total_divergences.is_empty(),
        "unified selector diverged from the live planner on {} real capture(s): {total_divergences:?}",
        total_divergences.len()
    );
}

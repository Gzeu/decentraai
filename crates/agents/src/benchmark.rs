//! DecentraAI Benchmark Fabric (pure) — the experimental lab foundation.
//!
//! The fabric answers one question with data, not assumptions: **does the
//! collective architecture beat a single agent on the same task?** Every
//! benchmark run (single / RAG / collective) produces a `BenchmarkRun` that is
//! graded deterministically against a gold answer, aggregated into metrics and
//! compared across modes. The runtime half (`decentraai-distributed`) turns
//! these runs into evidence entries, receipts and memory — closing the loop the
//! Evidence RAG reads back.
//!
//! Honesty rules (same as the rest of `crates/agents`):
//! - grading is **deterministic** (`grade_answer`): normalized text matching
//!   against the gold answer, never an LLM judge and never a vibe score;
//! - a task **without** a gold answer cannot be graded — it is `Abstained`,
//!   never guessed (BrowseComp-Plus golds are exact answers, so this stays
//!   strict and useful);
//! - `collective_beats_single` requires a **meaningful margin** (`MIN_MARGIN`),
//!   not a hair-thin difference, and reports the sample counts so a tiny run
//!   cannot masquerade as evidence.
//!
//! No I/O, no async, no external model — pure and unit-testable.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The three execution modes the lab compares. Mirrors the product evolution:
/// A = single agent, B = RAG agent, C = DecentraAI collective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkMode {
    /// Single agent: one model call, no retrieval, no consensus.
    Single,
    /// RAG agent: prompt augmented with retrieved evidence, one model call.
    Rag,
    /// Collective: N agents answer independently, consensus decides.
    Collective,
}

impl BenchmarkMode {
    /// Short tag used in evidence entries and metrics.
    pub fn tag(&self) -> &'static str {
        match self {
            Self::Single => "A",
            Self::Rag => "B",
            Self::Collective => "C",
        }
    }

    /// Human-readable mode name used in evidence tags and lessons.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Rag => "rag",
            Self::Collective => "collective",
        }
    }
}

/// One benchmark task: a prompt + an optional gold answer + optional evidence
/// passages (for RAG mode).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkTask {
    /// Stable id within the benchmark (e.g. BrowseComp-Plus query id).
    pub task_id: String,
    /// The question / instruction given to the agent(s).
    pub prompt: String,
    /// The exact gold answer when the benchmark provides one. `None` = the
    /// task cannot be graded (Abstained).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gold: Option<String>,
    /// Evidence passages injected in RAG mode.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

impl BenchmarkTask {
    /// A task with a gold answer.
    pub fn new(
        task_id: impl Into<String>,
        prompt: impl Into<String>,
        gold: impl Into<String>,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            prompt: prompt.into(),
            gold: Some(gold.into()),
            evidence: Vec::new(),
        }
    }

    /// A task without a gold answer (un-gradable, honest Abstained).
    pub fn ungradable(task_id: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            prompt: prompt.into(),
            gold: None,
            evidence: Vec::new(),
        }
    }

    /// Attaches evidence passages for RAG mode.
    pub fn with_evidence(mut self, evidence: Vec<String>) -> Self {
        self.evidence = evidence;
        self
    }
}

/// The grade of one run. `Abstained` is the honest answer for un-gradable
/// tasks or empty outputs — never a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkVerdict {
    Correct,
    Incorrect,
    Abstained,
}

/// Measured cost/quality fields of one run.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct RunMetrics {
    /// Total tokens the executor reported (input + output).
    pub tokens: u64,
    /// Wall-clock execution time (ms).
    pub latency_ms: u64,
    /// Number of tool calls made during the run.
    pub tool_calls: u32,
}

/// One graded execution of a benchmark task in a given mode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkRun {
    /// Stable run id (e.g. `<mode>:<task_id>:<attempt>`).
    pub run_id: String,
    /// The task executed.
    pub task_id: String,
    /// The mode this run used.
    pub mode: BenchmarkMode,
    /// The model's final text.
    pub output: String,
    /// Deterministic grade against the gold.
    pub verdict: BenchmarkVerdict,
    /// Cost/quality measurements.
    pub metrics: RunMetrics,
    /// Wall-clock timestamp (unix ms).
    pub created_at_ms: u64,
}

/// A per-mode aggregate over its runs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ModeAggregate {
    pub mode: BenchmarkMode,
    /// Number of runs in this mode.
    pub runs: usize,
    /// Number of graded runs (Correct + Incorrect; Abstained excluded).
    pub graded: usize,
    /// Accuracy over graded runs (0.0 when nothing graded — honest).
    pub accuracy: f64,
    /// Average tokens across all runs in the mode.
    pub avg_tokens: f64,
    /// Average latency (ms) across all runs in the mode.
    pub avg_latency_ms: f64,
}

/// The lab's headline question, answered deterministically.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeComparison {
    pub single: ModeAggregate,
    pub rag: ModeAggregate,
    pub collective: ModeAggregate,
    /// `collective.accuracy - single.accuracy` (positive = collective better).
    pub delta: f64,
    /// Whether collective beats single by at least `MIN_MARGIN` on graded
    /// runs AND both modes have at least `MIN_SAMPLES` graded runs.
    pub collective_beats_single: bool,
    /// Human-readable reasoning (what the data says, not a promise).
    pub reasoning: String,
}

/// Minimum accuracy margin for `collective_beats_single` to be true.
pub const MIN_MARGIN: f64 = 0.05;
/// Minimum number of graded runs per mode for a comparison to be meaningful.
pub const MIN_SAMPLES: usize = 5;

/// Normalizes an answer for deterministic grading: lowercase, trim, collapse
/// whitespace, strip punctuation. Keeps digits/letters only for stability
/// across formatting differences (BrowseComp-Plus golds are short exact
/// answers — numbers, names, dates).
pub fn normalize_answer(s: &str) -> String {
    // Any non-alphanumeric character is a separator ("forty-two" == "forty
    // two"); collapse runs of separators to single spaces.
    s.to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Deterministic grading: exact normalized match, or (for short golds)
/// containment either way. `None` gold or empty output → `Abstained`.
pub fn grade_answer(output: &str, gold: Option<&str>) -> BenchmarkVerdict {
    let Some(gold) = gold else {
        return BenchmarkVerdict::Abstained;
    };
    let out = normalize_answer(output);
    let gold = normalize_answer(gold);
    if out.is_empty() || gold.is_empty() {
        return BenchmarkVerdict::Abstained;
    }
    if out == gold {
        return BenchmarkVerdict::Correct;
    }
    // Containment is only meaningful for short answers (a sentence-long
    // output trivially contains a one-word gold; a long gold inside a short
    // output is still a hit when the gold is the exact expected phrase).
    if gold.len() <= 24 && (out.contains(&gold) || gold.contains(&out)) {
        return BenchmarkVerdict::Correct;
    }
    BenchmarkVerdict::Incorrect
}

/// Aggregates one mode's runs into a `ModeAggregate` (accuracy over graded
/// runs only; no graded runs → accuracy 0.0 with `graded: 0`, honest).
pub fn aggregate(mode: BenchmarkMode, runs: &[BenchmarkRun]) -> ModeAggregate {
    let graded: Vec<&BenchmarkRun> = runs
        .iter()
        .filter(|r| {
            r.verdict == BenchmarkVerdict::Correct || r.verdict == BenchmarkVerdict::Incorrect
        })
        .collect();
    let correct = graded
        .iter()
        .filter(|r| r.verdict == BenchmarkVerdict::Correct)
        .count();
    let accuracy = if graded.is_empty() {
        0.0
    } else {
        correct as f64 / graded.len() as f64
    };
    let (tokens, latency): (f64, f64) = if runs.is_empty() {
        (0.0, 0.0)
    } else {
        let t: u64 = runs.iter().map(|r| r.metrics.tokens).sum();
        let l: u64 = runs.iter().map(|r| r.metrics.latency_ms).sum();
        (t as f64 / runs.len() as f64, l as f64 / runs.len() as f64)
    };
    ModeAggregate {
        mode,
        runs: runs.len(),
        graded: graded.len(),
        accuracy,
        avg_tokens: tokens,
        avg_latency_ms: latency,
    }
}

/// Answers the lab's headline question over the three modes' runs. A run
/// belongs to its `mode` field; runs are split deterministically.
pub fn compare_modes(all_runs: &[BenchmarkRun]) -> ModeComparison {
    let single = aggregate(
        BenchmarkMode::Single,
        &by_mode(all_runs, BenchmarkMode::Single),
    );
    let rag = aggregate(BenchmarkMode::Rag, &by_mode(all_runs, BenchmarkMode::Rag));
    let collective = aggregate(
        BenchmarkMode::Collective,
        &by_mode(all_runs, BenchmarkMode::Collective),
    );
    let delta = collective.accuracy - single.accuracy;
    let meaningful =
        collective.graded >= MIN_SAMPLES && single.graded >= MIN_SAMPLES && delta >= MIN_MARGIN;
    let reasoning = if meaningful {
        format!(
            "collective {:.0}% > single {:.0}% (+{:.0}pp) on {} vs {} graded runs",
            collective.accuracy * 100.0,
            single.accuracy * 100.0,
            delta * 100.0,
            collective.graded,
            single.graded
        )
    } else if collective.graded < MIN_SAMPLES || single.graded < MIN_SAMPLES {
        format!(
            "not enough graded runs yet (collective {}, single {}; need {} each)",
            collective.graded, single.graded, MIN_SAMPLES
        )
    } else {
        format!(
            "collective {:.0}% vs single {:.0}% — no meaningful margin (+{:.0}pp, need +{:.0}pp)",
            collective.accuracy * 100.0,
            single.accuracy * 100.0,
            delta * 100.0,
            MIN_MARGIN * 100.0
        )
    };
    ModeComparison {
        single,
        rag,
        collective,
        delta,
        collective_beats_single: meaningful,
        reasoning,
    }
}

fn by_mode(runs: &[BenchmarkRun], mode: BenchmarkMode) -> Vec<BenchmarkRun> {
    runs.iter().filter(|r| r.mode == mode).cloned().collect()
}

/// The honest headline verdict: compares single vs collective **only over
/// tasks that were graded in BOTH modes**.
///
/// `compare_modes` (global) can be contaminated — if the lab ran easy tasks
/// in collective and hard tasks in single, the aggregate would claim a
/// collective win that never actually beat single on the same work. Paired
/// comparison fixes that: each task contributes one graded vote per mode
/// (its first graded run, oldest-first deterministic) and only tasks with a
/// vote in both modes count. The same MIN_SAMPLES / MIN_MARGIN gates apply,
/// now over *shared* tasks.
pub fn paired_compare(all_runs: &[BenchmarkRun]) -> ModeComparison {
    let mut single_votes: BTreeMap<String, BenchmarkVerdict> = BTreeMap::new();
    let mut collective_votes: BTreeMap<String, BenchmarkVerdict> = BTreeMap::new();
    for run in all_runs {
        if run.verdict == BenchmarkVerdict::Abstained {
            continue;
        }
        match run.mode {
            BenchmarkMode::Single => {
                single_votes
                    .entry(run.task_id.clone())
                    .or_insert(run.verdict);
            }
            BenchmarkMode::Collective => {
                collective_votes
                    .entry(run.task_id.clone())
                    .or_insert(run.verdict);
            }
            BenchmarkMode::Rag => {}
        }
    }
    let shared: Vec<&String> = single_votes
        .keys()
        .filter(|k| collective_votes.contains_key(*k))
        .collect();
    let n = shared.len();
    let s_correct = shared
        .iter()
        .filter(|k| single_votes.get(k.as_str()) == Some(&BenchmarkVerdict::Correct))
        .count();
    let c_correct = shared
        .iter()
        .filter(|k| collective_votes.get(k.as_str()) == Some(&BenchmarkVerdict::Correct))
        .count();
    let s_acc = if n == 0 {
        0.0
    } else {
        s_correct as f64 / n as f64
    };
    let c_acc = if n == 0 {
        0.0
    } else {
        c_correct as f64 / n as f64
    };
    let delta = c_acc - s_acc;
    let meaningful = n >= MIN_SAMPLES && delta >= MIN_MARGIN;
    let reasoning = if meaningful {
        format!(
            "collective {:.0}% > single {:.0}% (+{:.0}pp) on {} shared graded tasks",
            c_acc * 100.0,
            s_acc * 100.0,
            delta * 100.0,
            n
        )
    } else if n < MIN_SAMPLES {
        format!(
            "not enough shared graded tasks yet ({}; need {} — run the same tasks in both single and collective)",
            n, MIN_SAMPLES
        )
    } else {
        format!(
            "collective {:.0}% vs single {:.0}% on {} shared tasks — no meaningful margin (+{:.0}pp, need +{:.0}pp)",
            c_acc * 100.0,
            s_acc * 100.0,
            n,
            delta * 100.0,
            MIN_MARGIN * 100.0
        )
    };
    ModeComparison {
        single: ModeAggregate {
            mode: BenchmarkMode::Single,
            runs: n,
            graded: n,
            accuracy: s_acc,
            avg_tokens: 0.0,
            avg_latency_ms: 0.0,
        },
        rag: ModeAggregate {
            mode: BenchmarkMode::Rag,
            runs: 0,
            graded: 0,
            accuracy: 0.0,
            avg_tokens: 0.0,
            avg_latency_ms: 0.0,
        },
        collective: ModeAggregate {
            mode: BenchmarkMode::Collective,
            runs: n,
            graded: n,
            accuracy: c_acc,
            avg_tokens: 0.0,
            avg_latency_ms: 0.0,
        },
        delta,
        collective_beats_single: meaningful,
        reasoning,
    }
}

/// Minimum graded runs per side before a shadow comparison means anything.
pub const SHADOW_MIN_SAMPLES: usize = 8;
/// Minimum accuracy advantage the candidate must show to be recommended.
pub const SHADOW_MIN_ACCURACY_MARGIN: f64 = 0.10;

/// What the deterministic shadow comparison concludes. NOTE: a
/// recommendation NEVER promotes anything — an operator applies the
/// governance transition after reviewing the evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowRecommendation {
    /// Not enough graded evidence yet — keep benchmarking.
    InsufficientEvidence,
    /// Production stays; the candidate did not beat it convincingly.
    KeepProduction,
    /// Candidate won by margin on sufficient samples — operator review.
    OperatorReviewRecommended,
}

/// Deterministic production-vs-candidate verdict for one benchmark corpus.
///
/// Pure: same aggregates → same verdict, always. Latency participates only
/// as a tie-breaker when accuracies are within a hair (≤1 %): quality first,
/// speed second — never speed over correctness.
pub fn compare_shadow_models(
    production: &ModeAggregate,
    candidate: &ModeAggregate,
) -> (ShadowRecommendation, String) {
    let enough = production.graded >= SHADOW_MIN_SAMPLES && candidate.graded >= SHADOW_MIN_SAMPLES;
    if !enough {
        return (
            ShadowRecommendation::InsufficientEvidence,
            format!(
                "not enough graded runs: production={}, candidate={} (need ≥{} each)",
                production.graded, candidate.graded, SHADOW_MIN_SAMPLES
            ),
        );
    }
    let margin = candidate.accuracy - production.accuracy;
    if margin >= SHADOW_MIN_ACCURACY_MARGIN {
        return (
            ShadowRecommendation::OperatorReviewRecommended,
            format!(
                "candidate accuracy {:.3} beats production {:.3} by +{:.3} (≥{:.2})",
                candidate.accuracy, production.accuracy, margin, SHADOW_MIN_ACCURACY_MARGIN
            ),
        );
    }
    // Near-tie on quality: latency decides ONLY inside the hair band.
    if margin.abs() < 0.01 && candidate.avg_latency_ms > 0.0 {
        let faster = candidate.avg_latency_ms < production.avg_latency_ms;
        return (
            if faster {
                ShadowRecommendation::OperatorReviewRecommended
            } else {
                ShadowRecommendation::KeepProduction
            },
            format!(
                "accuracy near-tie ({:+.3}); candidate latency {:.0} ms vs production {:.0} ms",
                margin, candidate.avg_latency_ms, production.avg_latency_ms
            ),
        );
    }
    (
        ShadowRecommendation::KeepProduction,
        format!(
            "candidate accuracy {:+.3} vs production (margin threshold {:.2})",
            margin, SHADOW_MIN_ACCURACY_MARGIN
        ),
    )
}

/// Deterministic in-memory registry of benchmark tasks and runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BenchmarkRegistry {
    tasks: BTreeMap<String, BenchmarkTask>,
    runs: Vec<BenchmarkRun>,
}

impl BenchmarkRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a task (replaces by id).
    pub fn add_task(&mut self, task: BenchmarkTask) {
        self.tasks.insert(task.task_id.clone(), task);
    }

    /// Number of registered tasks.
    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    /// Looks up a task.
    pub fn task(&self, task_id: &str) -> Option<&BenchmarkTask> {
        self.tasks.get(task_id)
    }

    /// Records a run (idempotent on run id).
    pub fn add_run(&mut self, run: BenchmarkRun) {
        if !self.runs.iter().any(|r| r.run_id == run.run_id) {
            self.runs.push(run);
        }
    }

    /// All runs, oldest-first (deterministic).
    pub fn runs(&self) -> &[BenchmarkRun] {
        &self.runs
    }

    /// The lab's headline verdict: paired over tasks graded in both single
    /// and collective modes (contamination-free — see [`paired_compare`]).
    pub fn comparison(&self) -> ModeComparison {
        paired_compare(&self.runs)
    }

    /// The global (per-mode aggregate) comparison over ALL runs. Useful as
    /// secondary data, but the headline verdict must be the paired one.
    pub fn global_comparison(&self) -> ModeComparison {
        compare_modes(&self.runs)
    }

    /// Per-mode run counts.
    pub fn counts(&self) -> BTreeMap<BenchmarkMode, usize> {
        let mut m = BTreeMap::new();
        for r in &self.runs {
            *m.entry(r.mode).or_insert(0) += 1;
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        id: &str,
        task: &str,
        mode: BenchmarkMode,
        output: &str,
        gold: Option<&str>,
    ) -> BenchmarkRun {
        BenchmarkRun {
            run_id: id.into(),
            task_id: task.into(),
            mode,
            output: output.into(),
            verdict: grade_answer(output, gold),
            metrics: RunMetrics {
                tokens: 100,
                latency_ms: 50,
                tool_calls: 0,
            },
            created_at_ms: 1000,
        }
    }

    #[test]
    fn grade_answer_is_deterministic_and_strict() {
        assert_eq!(grade_answer("42", Some("42")), BenchmarkVerdict::Correct);
        assert_eq!(grade_answer(" 42. ", Some("42")), BenchmarkVerdict::Correct);
        assert_eq!(
            grade_answer("The answer is forty-two.", Some("forty two")),
            BenchmarkVerdict::Correct
        );
        assert_eq!(grade_answer("43", Some("42")), BenchmarkVerdict::Incorrect);
        assert_eq!(
            grade_answer("I don't know", Some("42")),
            BenchmarkVerdict::Incorrect
        );
        // No gold → honest Abstained, never guessed.
        assert_eq!(grade_answer("anything", None), BenchmarkVerdict::Abstained);
        // Empty output → Abstained.
        assert_eq!(grade_answer("", Some("42")), BenchmarkVerdict::Abstained);
    }

    #[test]
    fn normalize_answer_collapses_formatting() {
        assert_eq!(normalize_answer("  Hello,   World! "), "hello world");
        assert_eq!(normalize_answer("42."), "42");
    }

    #[test]
    fn aggregate_honest_with_no_graded_runs() {
        let runs = vec![
            run("1", "t", BenchmarkMode::Single, "?", None),
            run("2", "t", BenchmarkMode::Single, "?", None),
        ];
        let a = aggregate(BenchmarkMode::Single, &runs);
        assert_eq!(a.graded, 0);
        assert_eq!(a.accuracy, 0.0);
        assert_eq!(a.runs, 2);
        assert_eq!(a.avg_tokens, 100.0);
    }

    #[test]
    fn compare_modes_decides_collective_only_with_margin_and_samples() {
        // 6 tasks; collective 5/6 correct, single 3/6 → delta 0.33 > margin.
        let mut registry = BenchmarkRegistry::new();
        for i in 0..6 {
            let task = format!("t{i}");
            let gold = Some("g");
            registry.add_run(run(
                &format!("A{i}"),
                &task,
                BenchmarkMode::Single,
                if i < 3 { "g" } else { "x" },
                gold,
            ));
            registry.add_run(run(
                &format!("C{i}"),
                &task,
                BenchmarkMode::Collective,
                if i < 5 { "g" } else { "x" },
                gold,
            ));
        }
        let cmp = registry.comparison();
        assert!(cmp.collective_beats_single);
        assert!(cmp.delta >= 0.05);
        assert_eq!(cmp.single.graded, 6);
        assert_eq!(cmp.collective.graded, 6);
        assert!((cmp.single.accuracy - 0.5).abs() < 1e-9);
        assert!((cmp.collective.accuracy - 5.0 / 6.0).abs() < 1e-9);
    }

    #[test]
    fn compare_modes_refuses_meaningless_margins_and_tiny_samples() {
        // Same accuracy → no claim.
        let mut registry = BenchmarkRegistry::new();
        for i in 0..6 {
            let task = format!("t{i}");
            let gold = Some("g");
            registry.add_run(run(
                &format!("A{i}"),
                &task,
                BenchmarkMode::Single,
                "g",
                gold,
            ));
            registry.add_run(run(
                &format!("C{i}"),
                &task,
                BenchmarkMode::Collective,
                "g",
                gold,
            ));
        }
        let cmp = registry.comparison();
        assert!(!cmp.collective_beats_single);
        assert!(cmp.reasoning.contains("margin"));

        // Fewer than MIN_SAMPLES graded runs → honest "not enough".
        let mut tiny = BenchmarkRegistry::new();
        tiny.add_run(run("A0", "t", BenchmarkMode::Single, "g", Some("g")));
        tiny.add_run(run("C0", "t", BenchmarkMode::Collective, "g", Some("g")));
        let cmp = tiny.comparison();
        assert!(!cmp.collective_beats_single);
        assert!(cmp.reasoning.contains("not enough"));
    }

    #[test]
    fn paired_compare_ignores_tasks_not_shared_between_modes() {
        // The contamination scenario: collective only saw easy tasks (all
        // correct), single saw the same easy tasks PLUS hard ones (all wrong).
        // Global aggregate would claim collective wins (+50pp); the paired
        // verdict must refuse because only the 3 shared easy tasks count.
        let mut registry = BenchmarkRegistry::new();
        for i in 0..3 {
            let task = format!("easy{i}");
            let gold = Some("g");
            registry.add_run(run(
                &format!("A{i}"),
                &task,
                BenchmarkMode::Single,
                "g",
                gold,
            ));
            registry.add_run(run(
                &format!("C{i}"),
                &task,
                BenchmarkMode::Collective,
                "g",
                gold,
            ));
        }
        for i in 0..3 {
            let task = format!("hard{i}");
            let gold = Some("g");
            // single-only hard tasks, all wrong — must NOT count against single
            registry.add_run(run(
                &format!("Ah{i}"),
                &task,
                BenchmarkMode::Single,
                "x",
                gold,
            ));
        }
        let cmp = registry.comparison();
        // shared = 3 easy tasks: both 100% → no margin claim.
        assert!(!cmp.collective_beats_single);
        assert_eq!(cmp.single.graded, 3);
        assert_eq!(cmp.collective.graded, 3);
        assert!((cmp.single.accuracy - 1.0).abs() < 1e-9);
        assert!(cmp.reasoning.contains("margin") || cmp.reasoning.contains("shared"));
    }

    #[test]
    fn registry_is_idempotent_on_run_id_and_sorts_counts() {
        let mut registry = BenchmarkRegistry::new();
        registry.add_task(BenchmarkTask::new("t1", "Q", "g"));
        registry.add_task(BenchmarkTask::new("t2", "Q2", "g2"));
        registry.add_run(run("r1", "t1", BenchmarkMode::Single, "g", Some("g")));
        registry.add_run(run("r1", "t1", BenchmarkMode::Single, "g", Some("g"))); // dup id
        assert_eq!(registry.task_count(), 2);
        assert_eq!(registry.runs().len(), 1);
        assert_eq!(registry.counts()[&BenchmarkMode::Single], 1);
    }

    #[test]
    fn shadow_comparison_is_deterministic_and_never_auto_promotes() {
        let agg = |graded: usize, acc: f64, lat: f64| ModeAggregate {
            mode: BenchmarkMode::Single,
            runs: graded,
            graded,
            accuracy: acc,
            avg_tokens: 0.0,
            avg_latency_ms: lat,
        };

        // Insufficient evidence dominates everything else.
        let (rec, why) = compare_shadow_models(&agg(3, 0.9, 100.0), &agg(2, 1.0, 50.0));
        assert_eq!(rec, ShadowRecommendation::InsufficientEvidence);
        assert!(why.contains("not enough graded"));

        // Clear candidate win by margin.
        let (rec, _) = compare_shadow_models(&agg(10, 0.60, 500.0), &agg(10, 0.75, 900.0));
        assert_eq!(rec, ShadowRecommendation::OperatorReviewRecommended);

        // Candidate slower on a near-tie: production stays.
        let (rec, why) = compare_shadow_models(&agg(10, 0.70, 300.0), &agg(10, 0.705, 800.0));
        assert_eq!(rec, ShadowRecommendation::KeepProduction);
        assert!(why.contains("near-tie"));

        // Candidate faster within the hair band: review recommended — but the
        // CALLER still must run the operator-gated transition; this function
        // returns data, never mutates any registry.
        let (rec, _) = compare_shadow_models(&agg(10, 0.70, 800.0), &agg(10, 0.705, 250.0));
        assert_eq!(rec, ShadowRecommendation::OperatorReviewRecommended);

        // Determinism: same inputs → same verdict + reasoning bytes.
        let p = agg(12, 0.55, 400.0);
        let c = agg(12, 0.70, 400.0);
        assert_eq!(compare_shadow_models(&p, &c), compare_shadow_models(&p, &c));
    }
}

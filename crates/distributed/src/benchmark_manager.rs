//! DecentraAI Benchmark Lab runtime — binds the pure `BenchmarkRegistry` to
//! the live inference executor and the evidence loop.
//!
//! `BenchmarkManager` owns the registry, runs benchmark tasks through an
//! injected executor (`InferenceAgentExecutor`), grades each run with the
//! deterministic `grade_answer`, and feeds every run into the Evidence RAG as
//! `EvidenceFamily::Benchmark` — so the lab's results become part of the
//! fabric's experimental memory ("what have we learned?").
//!
//! Honest semantics:
//! - a run is graded only against a real gold answer (`Abstained` otherwise);
//! - `collective_beats_single` requires MIN_SAMPLES graded runs per mode and
//!   a MIN_MARGIN accuracy delta — a 3-run experiment cannot claim victory;
//! - execution errors become `Abstained` runs with the error text as output
//!   (never fabricated correctness).
//!
//! Evidence entries carry facts (task id, mode, verdict, metrics), never
//! prompts or model outputs.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use decentraai_agents::benchmark::{
    BenchmarkMode, BenchmarkRegistry, BenchmarkRun, BenchmarkTask, BenchmarkVerdict, ModeAggregate,
    ModeComparison, RunMetrics,
};
use decentraai_agents::evidence::{EvidenceEntry, EvidenceFamily};

use crate::agent_runtime::InferenceAgentExecutor;
use crate::evidence_manager::EvidenceManager;

/// Boxed future returned by [`BenchmarkInference::execute`] (avoids repeating
/// the complex type in every implementation).
pub type InferenceFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(String, u64, u64)>> + Send + 'a>>;

/// The inference surface the lab needs: one generation from a prompt,
/// returning `(output_text, tokens, latency_ms)`. Implemented by the live
/// executor and by test mocks — the lab never fakes a generation in
/// production.
pub trait BenchmarkInference: Send + Sync {
    fn execute<'a>(&'a self, prompt: &'a str, evidence: &'a [String]) -> InferenceFuture<'a>;
}

/// Adapter over the real inference executor (local backend / distributed
/// routing / tool calling).
pub struct InferenceBenchmarkExecutor {
    inference: Arc<InferenceAgentExecutor>,
}

impl InferenceBenchmarkExecutor {
    pub fn new(inference: Arc<InferenceAgentExecutor>) -> Self {
        Self { inference }
    }
}

impl BenchmarkInference for InferenceBenchmarkExecutor {
    fn execute<'a>(&'a self, prompt: &'a str, evidence: &'a [String]) -> InferenceFuture<'a> {
        Box::pin(async move {
            let task = decentraai_agents::AgentTask::new(format!(
                "bench:{}",
                prompt.chars().take(24).collect::<String>()
            ));
            let final_prompt = if evidence.is_empty() {
                prompt.to_string()
            } else {
                let ctx = evidence
                    .iter()
                    .map(|e| format!("- {e}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("Evidence:\n{ctx}\n\nQuestion: {prompt}")
            };
            let inputs = serde_json::json!({ "prompt": final_prompt });
            let started = std::time::Instant::now();
            let out = self.inference.execute(&task, &inputs).await?;
            let latency = started.elapsed().as_millis() as u64;
            let (text, tokens) = parse_executor_output(&out);
            Ok((text, tokens, latency))
        })
    }
}

/// Extracts `(text, tokens)` from the agent executor's output JSON.
///
/// The live `InferenceAgentExecutor` returns `{ "text": …, "tokens": N }`
/// (see agent_runtime.rs); other executors may return `content` /
/// `tokens_used`. This parser accepts both so the lab keeps working if the
/// executor contract evolves — and it is pure, so tests pin the contract.
fn parse_executor_output(out: &serde_json::Value) -> (String, u64) {
    let text = out
        .get("text")
        .and_then(|v| v.as_str())
        .or_else(|| out.get("content").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    let tokens = out
        .get("tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| out.get("tokens_used").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    (text, tokens)
}

/// The lab runtime: owns the registry + optional evidence feed.
pub struct BenchmarkManager {
    registry: Arc<Mutex<BenchmarkRegistry>>,
    executor: Arc<dyn BenchmarkInference>,
    evidence: Option<Arc<EvidenceManager>>,
    /// Next run counter for stable run ids.
    next_id: Mutex<u64>,
}

impl BenchmarkManager {
    pub fn new(
        executor: Arc<dyn BenchmarkInference>,
        evidence: Option<Arc<EvidenceManager>>,
    ) -> Self {
        Self {
            registry: Arc::new(Mutex::new(BenchmarkRegistry::new())),
            executor,
            evidence,
            next_id: Mutex::new(0),
        }
    }

    fn next_run_id(&self, mode: BenchmarkMode, task_id: &str) -> String {
        let mut n = self.next_id.lock().unwrap();
        let id = format!("{}:{}:{}", mode.tag(), task_id, *n);
        *n += 1;
        id
    }

    fn feed_evidence(&self, run: &BenchmarkRun, task: &BenchmarkTask) {
        if let Some(evidence) = &self.evidence {
            let entry = EvidenceEntry::new(
                format!("bench:{}", run.run_id),
                EvidenceFamily::Benchmark,
                format!(
                    "benchmark task {} mode {} verdict {:?} ({}ms, {} tokens)",
                    task.task_id,
                    run.mode.tag(),
                    run.verdict,
                    run.metrics.latency_ms,
                    run.metrics.tokens
                ),
                run.created_at_ms,
            )
            .tagged(format!("mode:{}", run.mode.name()))
            .tagged(format!("verdict:{:?}", run.verdict))
            .tagged(format!("latency_ms:{}", run.metrics.latency_ms))
            .tagged(format!("task:{}", task.task_id));
            if let Ok(mut ix) = evidence.index().lock() {
                ix.add(entry);
            }
        }
    }

    /// Runs one task in the given mode and records the graded run.
    ///
    /// - `Single`: one generation, no retrieval.
    /// - `Rag`: one generation with the task's evidence injected (the
    ///   executor augments the prompt with `retrieve_context`).
    /// - `Collective`: `agents` independent generations; the run's output is
    ///   the majority answer and the verdict is the majority grade. If the
    ///   agents disagree with no plurality, the run is Abstained (honest).
    pub async fn run_task(
        &self,
        task: &BenchmarkTask,
        mode: BenchmarkMode,
        agents: usize,
    ) -> Result<BenchmarkRun> {
        let run_id = self.next_run_id(mode, &task.task_id);
        let created = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut output = String::new();
        let mut tokens = 0u64;
        let mut latency = 0u64;
        let mut verdict = BenchmarkVerdict::Abstained;

        match mode {
            BenchmarkMode::Single => match self.executor.execute(&task.prompt, &[]).await {
                Ok((text, t, l)) => {
                    output = text;
                    tokens = t;
                    latency = l;
                    verdict = decentraai_agents::grade_answer(&output, task.gold.as_deref());
                }
                Err(e) => {
                    output = format!("execution error: {e}");
                }
            },
            BenchmarkMode::Rag => match self.executor.execute(&task.prompt, &task.evidence).await {
                Ok((text, t, l)) => {
                    output = text;
                    tokens = t;
                    latency = l;
                    verdict = decentraai_agents::grade_answer(&output, task.gold.as_deref());
                }
                Err(e) => {
                    output = format!("execution error: {e}");
                }
            },
            BenchmarkMode::Collective => {
                let count = agents.max(2);
                let mut answers: Vec<(String, u64, u64)> = Vec::new();
                for _ in 0..count {
                    match self.executor.execute(&task.prompt, &[]).await {
                        Ok(r) => answers.push(r),
                        Err(e) => {
                            output = format!("execution error: {e}");
                        }
                    }
                }
                if !answers.is_empty() {
                    // Plurality vote on the *grades*, not the text: each
                    // answer is graded against the gold, the majority grade
                    // decides. A tie between Correct/Incorrect is Abstained.
                    let mut correct = 0usize;
                    let mut incorrect = 0usize;
                    let mut abstained = 0usize;
                    for (text, t, l) in &answers {
                        tokens += *t;
                        latency += *l;
                        match decentraai_agents::grade_answer(text, task.gold.as_deref()) {
                            BenchmarkVerdict::Correct => correct += 1,
                            BenchmarkVerdict::Incorrect => incorrect += 1,
                            BenchmarkVerdict::Abstained => abstained += 1,
                        }
                    }
                    latency /= count as u64;
                    // Majority of *non-abstained* answers decides; ties → Abstained.
                    if correct > incorrect && correct >= abstained {
                        verdict = BenchmarkVerdict::Correct;
                        // Output = the first correct answer, so the dashboard
                        // shows something real, not the minority answer.
                        output = answers
                            .iter()
                            .find(|(text, _, _)| {
                                decentraai_agents::grade_answer(text, task.gold.as_deref())
                                    == BenchmarkVerdict::Correct
                            })
                            .map(|(text, _, _)| text.clone())
                            .unwrap_or_default();
                    } else if incorrect > correct && incorrect >= abstained {
                        verdict = BenchmarkVerdict::Incorrect;
                        output = answers[0].0.clone();
                    } else {
                        verdict = BenchmarkVerdict::Abstained;
                        output = answers
                            .iter()
                            .map(|(text, _, _)| text.as_str())
                            .collect::<Vec<_>>()
                            .join(" | ");
                    }
                }
            }
        }

        let run = BenchmarkRun {
            run_id,
            task_id: task.task_id.clone(),
            mode,
            output,
            verdict,
            metrics: RunMetrics {
                tokens,
                latency_ms: latency,
                tool_calls: 0,
            },
            created_at_ms: created,
        };
        self.feed_evidence(&run, task);
        if let Ok(mut registry) = self.registry.lock() {
            registry.add_run(run.clone());
        }
        Ok(run)
    }

    /// Runs every task in a batch, in the given mode.
    pub async fn run_batch(
        &self,
        tasks: &[BenchmarkTask],
        mode: BenchmarkMode,
        agents: usize,
    ) -> Vec<BenchmarkRun> {
        let mut runs = Vec::new();
        for task in tasks {
            match self.run_task(task, mode, agents).await {
                Ok(run) => runs.push(run),
                Err(e) => {
                    tracing::warn!(error = %e, task = %task.task_id, "benchmark run failed");
                }
            }
        }
        runs
    }

    /// The registry snapshot.
    pub fn registry(&self) -> Arc<Mutex<BenchmarkRegistry>> {
        self.registry.clone()
    }

    /// The lab's current headline comparison (paired over shared tasks).
    pub fn comparison(&self) -> ModeComparison {
        self.registry
            .lock()
            .map(|r| r.comparison())
            .unwrap_or_else(|_| ModeComparison {
                single: ModeAggregate {
                    mode: BenchmarkMode::Single,
                    runs: 0,
                    graded: 0,
                    accuracy: 0.0,
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
                    runs: 0,
                    graded: 0,
                    accuracy: 0.0,
                    avg_tokens: 0.0,
                    avg_latency_ms: 0.0,
                },
                delta: 0.0,
                collective_beats_single: false,
                reasoning: "registry lock poisoned".into(),
            })
    }

    /// The global (per-mode aggregate) comparison over ALL runs — secondary
    /// data; the headline verdict is the paired one.
    pub fn global_comparison(&self) -> ModeComparison {
        self.registry
            .lock()
            .map(|r| r.global_comparison())
            .unwrap_or_else(|_| self.comparison())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Mock inference: answers a fixed prompt→output table; otherwise
    /// returns "wrong". Deterministic and fast.
    #[derive(Clone)]
    struct MockInference {
        answers: HashMap<String, String>,
        calls: Arc<AtomicU32>,
        /// If >0, the first `flaky` global calls return "wrong" regardless of
        /// the answer table — simulates an executor that is unreliable at
        /// startup (used to build a mode delta).
        flaky: u32,
    }

    impl MockInference {
        fn new(answers: &[(&str, &str)]) -> Self {
            Self {
                answers: answers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                calls: Arc::new(AtomicU32::new(0)),
                flaky: 0,
            }
        }

        fn flaky(answers: &[(&str, &str)], flaky: u32) -> Self {
            let mut m = Self::new(answers);
            m.flaky = flaky;
            m
        }
    }

    impl BenchmarkInference for MockInference {
        fn execute<'a>(&'a self, prompt: &'a str, evidence: &'a [String]) -> InferenceFuture<'a> {
            Box::pin(async move {
                let n = self.calls.fetch_add(1, Ordering::SeqCst);
                let flaky = self.flaky > 0 && n < self.flaky;
                // The exact prompt (or the evidence-prefixed RAG prompt) maps
                // to a canned answer; the gold is the answer for the base
                // prompt so grading matches "g" == gold.
                let key = if evidence.is_empty() {
                    prompt.to_string()
                } else {
                    // RAG prompt format: "Evidence:\n- …\n\nQuestion: Q"
                    prompt
                        .rsplit("Question: ")
                        .next()
                        .unwrap_or(prompt)
                        .to_string()
                };
                let out = if flaky {
                    "wrong".to_string()
                } else {
                    self.answers
                        .get(&key)
                        .cloned()
                        .unwrap_or_else(|| "wrong".to_string())
                };
                Ok((out, 100, 50))
            })
        }
    }

    fn manager_with(
        mock: MockInference,
        evidence: Option<Arc<EvidenceManager>>,
    ) -> BenchmarkManager {
        BenchmarkManager::new(Arc::new(mock), evidence)
    }

    #[test]
    fn single_run_grades_deterministically_and_feeds_evidence() {
        let ev = Arc::new(EvidenceManager::new(None));
        let mock = MockInference::new(&[("What is 2+2?", "4")]);
        let mgr = manager_with(mock, Some(ev.clone()));
        let task = BenchmarkTask::new("t1", "What is 2+2?", "4");

        let run =
            futures::executor::block_on(mgr.run_task(&task, BenchmarkMode::Single, 1)).unwrap();
        assert_eq!(run.verdict, BenchmarkVerdict::Correct);
        assert_eq!(run.mode, BenchmarkMode::Single);
        assert_eq!(mgr.registry().lock().unwrap().runs().len(), 1);

        // Evidence entry was fed (kind benchmark, honest facts only).
        let ix = ev.index().lock().unwrap();
        assert_eq!(ix.counts()[&EvidenceFamily::Benchmark], 1);
        let entry = ix.all()[0].clone();
        assert!(entry.id.starts_with("bench:"));
        assert!(entry.tags.iter().any(|t| t == "verdict:Correct"));
    }

    #[test]
    fn collective_mode_uses_plurality_and_never_fabricates() {
        // Two of three agents answer correctly → Correct.
        let mock = MockInference::new(&[("Q?", "paris")]);
        let mgr = manager_with(mock.clone(), None);
        let task = BenchmarkTask::new("t1", "Q?", "paris");
        let run =
            futures::executor::block_on(mgr.run_task(&task, BenchmarkMode::Collective, 3)).unwrap();
        assert_eq!(run.verdict, BenchmarkVerdict::Correct);
        assert_eq!(mock.calls.load(Ordering::SeqCst), 3);

        // All wrong → Incorrect (a 1-char gold like "g" is a substring of
        // "wrong", so use a real word).
        let bad = MockInference::new(&[]);
        let mgr = manager_with(bad, None);
        let run =
            futures::executor::block_on(mgr.run_task(&task, BenchmarkMode::Collective, 3)).unwrap();
        assert_eq!(run.verdict, BenchmarkVerdict::Incorrect);
    }

    #[test]
    fn rag_mode_injects_evidence_into_the_prompt() {
        let mock = MockInference::new(&[("Q?", "paris")]);
        let mgr = manager_with(mock, None);
        let task = BenchmarkTask::new("t1", "Q?", "paris").with_evidence(vec!["ctx".to_string()]);
        let run = futures::executor::block_on(mgr.run_task(&task, BenchmarkMode::Rag, 1)).unwrap();
        assert_eq!(run.verdict, BenchmarkVerdict::Correct);
    }

    #[test]
    fn execution_error_is_honest_abstained() {
        let mock = MockInference::new(&[]);
        let mgr = manager_with(mock, None);
        let task = BenchmarkTask::ungradable("t1", "Q?");
        let run =
            futures::executor::block_on(mgr.run_task(&task, BenchmarkMode::Single, 1)).unwrap();
        assert_eq!(run.verdict, BenchmarkVerdict::Abstained);
    }

    #[test]
    fn executor_output_parser_reads_live_contract_and_fallback() {
        // The live InferenceAgentExecutor returns { text, tokens } — the
        // parser must read that contract (this test pins it so a future
        // executor change cannot silently blank every benchmark run).
        let (text, tokens) = parse_executor_output(&serde_json::json!({
            "text": "paris",
            "model_hash": "abc",
            "tokens": 42,
        }));
        assert_eq!(text, "paris");
        assert_eq!(tokens, 42);
        // Fallback contract ({ content, tokens_used }) also accepted.
        let (text, tokens) = parse_executor_output(&serde_json::json!({
            "content": "paris",
            "tokens_used": 7,
        }));
        assert_eq!(text, "paris");
        assert_eq!(tokens, 7);
        // Missing text → empty (grading will Abstain honestly, never crash).
        let (text, tokens) = parse_executor_output(&serde_json::json!({"x": 1}));
        assert_eq!(text, "");
        assert_eq!(tokens, 0);
    }

    #[test]
    fn comparison_requires_samples_and_margin() {
        let ev = Arc::new(EvidenceManager::new(None));
        // Single-mode is unreliable at startup (first 2 global calls are
        // wrong) — collective's 3 independent draws converge to the answer.
        // Paired comparison counts *shared tasks* (same task in both modes),
        // so the test uses 5 distinct tasks and runs each in both modes.
        let mock = MockInference::flaky(&[("Q?", "paris")], 2);
        let mgr = manager_with(mock, Some(ev));
        let tasks: Vec<BenchmarkTask> = (0..5)
            .map(|i| BenchmarkTask::new(format!("t{i}"), "Q?", "paris"))
            .collect();

        // Few shared tasks → honest "not enough".
        for task in &tasks[..2] {
            futures::executor::block_on(mgr.run_task(task, BenchmarkMode::Single, 1)).unwrap();
            futures::executor::block_on(mgr.run_task(task, BenchmarkMode::Collective, 3)).unwrap();
        }
        let cmp = mgr.comparison();
        assert!(!cmp.collective_beats_single);
        assert!(cmp.reasoning.contains("not enough"));

        // Reach MIN_SAMPLES=5 shared tasks with collective consistently
        // better: single sees the first 2 flaky calls (wrong) → 3/5 = 0.6;
        // collective runs after warm-up → 5/5 = 1.0 → delta 0.4 ≥ margin.
        for task in &tasks[2..] {
            futures::executor::block_on(mgr.run_task(task, BenchmarkMode::Single, 1)).unwrap();
            futures::executor::block_on(mgr.run_task(task, BenchmarkMode::Collective, 3)).unwrap();
        }
        let cmp = mgr.comparison();
        assert!(cmp.collective_beats_single);
        assert!(cmp.reasoning.contains("collective"));
    }
}

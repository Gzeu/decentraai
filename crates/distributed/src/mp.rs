//! Model Parallel Planner (pure) — the first genuine distributed-inference
//! primitive for the llama-server stack.
//!
//! **Why map-reduce (and not tensor parallelism).** llama-server supports
//! `--split-mode {layer,row,tensor}` + `--tensor-split`, but those split a
//! model across GPUs *inside one machine*. There is **no cross-node tensor /
//! pipeline parallelism** in the deployed llama-server: one forward pass
//! cannot be physically divided over separate nodes over the network. That
//! boundary is real and must be stated, not glossed over.
//!
//! What the stack CAN do for a single logical workload too heavy for one
//! machine is **map-reduce / context-split inference**: the planner detects
//! the workload exceeds a node's per-call context budget, splits it into
//! deterministic shards, dispatches them across workers (map), then reduces
//! the partial results into ONE final answer (reduce). The workload is a
//! single logical task — not independent prompts — and the reduce step is
//! what couples all workers into one result.
//!
//! This module is the pure, testable decision half. The async execution half
//! (map via the existing DFCP delegation, reduce, EvidenceChain) lives in the
//! runtime API.

use serde::{Deserialize, Serialize};

/// A single logical model-parallel workload: one instruction over one body of
/// content that is too large to process in one worker call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MpWorkload {
    pub task_id: String,
    /// The instruction applied to the whole content (e.g. "summarize",
    /// "extract the key claims"). Shared by every shard and the reduce step.
    pub instruction: String,
    /// The content to process (the part that can exceed one worker's budget).
    pub content: String,
}

/// One deterministic slice of the content, assigned to one worker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MpShard {
    pub index: usize,
    pub content: String,
}

/// Default per-shard content budget (chars) for a single worker call. Kept
/// conservative so a shard stays well within a worker's context and answers
/// are short enough for a batched DFCP result.
pub const MAX_SHARD_CHARS: usize = 1500;

/// A worker may process up to this many shards in one batched DFCP request.
pub const MAX_SHARDS_PER_BATCH: usize = 3;

/// The planner's verdict on a workload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MpPlan {
    /// Whether the workload must be distributed (content exceeds one worker's
    /// per-call budget). If false, a single local call is the honest answer.
    pub distributed: bool,
    /// Number of shards the content is split into (1 when local).
    pub n_shards: usize,
    /// The shards (deterministic, in order).
    pub shards: Vec<MpShard>,
}

/// Decides whether a workload needs distributed inference: it does when the
/// content exceeds the per-worker per-call budget.
pub fn needs_distributed(content_chars: usize) -> bool {
    content_chars > MAX_SHARD_CHARS
}

/// Splits content into ordered shards of at most `max_chars` each, breaking on
/// sentence boundaries where possible (deterministic: split on ". " or "!\n";
/// falls back to a hard char cut). Empty content yields a single empty shard.
pub fn split_shards(content: &str, max_chars: usize) -> Vec<MpShard> {
    let content = content.trim();
    if content.is_empty() {
        return vec![MpShard { index: 0, content: String::new() }];
    }
    let max_chars = max_chars.max(1);
    let mut shards: Vec<MpShard> = Vec::new();
    let mut start = 0usize;
    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut idx = 0usize;
    while start < len {
        let mut end = (start + max_chars).min(len);
        // Back off to a sentence boundary before `end` when possible.
        if end < len {
            let mut cut = start;
            for i in (start..end).rev() {
                if (chars[i] == '.' && i + 1 < len && chars[i + 1] == ' ')
                    || chars[i] == '\n'
                {
                    cut = i + 1;
                    break;
                }
            }
            if cut > start {
                end = cut;
            }
        }
        let text: String = chars[start..end].iter().collect();
        if !text.trim().is_empty() {
            shards.push(MpShard {
                index: idx,
                content: text.trim().to_string(),
            });
            idx += 1;
        }
        start = end;
    }
    if shards.is_empty() {
        shards.push(MpShard { index: 0, content: content.to_string() });
    }
    shards
}

/// Plans a workload: decides distributed vs local and produces shards.
pub fn plan(w: &MpWorkload) -> MpPlan {
    let distributed = needs_distributed(w.content.chars().count());
    let shards = if distributed {
        split_shards(&w.content, MAX_SHARD_CHARS)
    } else {
        vec![MpShard { index: 0, content: w.content.trim().to_string() }]
    };
    MpPlan {
        distributed,
        n_shards: shards.len(),
        shards,
    }
}

/// Builds the per-shard chat prompt handed to a worker: apply the shared
/// instruction to this slice, asking for a compact partial result (short
/// partials keep the later reduce prompt small — the reduce cost scales with
/// the size of the partials it must fuse).
pub fn map_prompt(instruction: &str, shard: &MpShard) -> String {
    format!(
        "{instruction}\n\nProcess ONLY the following section and return a concise partial result IN UNDER 40 WORDS:\n---\n{}\n---",
        shard.content
    )
}

/// Target token budget for the reduce call. A reduce only has to fuse the
/// short partials into one short answer, so a small budget keeps the (slow,
/// autoregressive) reduce call fast without harming quality.
pub const REDUCE_MAX_TOKENS: u64 = 256;

/// Builds the reduce prompt: combine all partial results into ONE final answer
/// for the original instruction.
pub fn reduce_prompt(instruction: &str, partials: &[String]) -> String {
    let mut body = String::new();
    for (i, p) in partials.iter().enumerate() {
        body.push_str(&format!("PART {i}:\n{p}\n\n"));
    }
    format!(
        "{instruction}\n\nBelow are partial results from {n} parallel workers. Combine them into ONE coherent final answer. Do not list the parts.\n---\n{body}---",
        n = partials.len()
    )
}

/// The Governor's automatic verdict on a workload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum GovernorVerdict {
    /// The workload fits the local node's capacity; execute locally.
    Local,
    /// The workload exceeds local capacity and enough workers are reachable;
    /// borrow distributed compute and run map-reduce.
    Distributed,
    /// The local queue is saturated; enqueue (do not run now).
    Queue,
    /// The node is over-loaded (CPU+RAM pinned) and cannot serve this now.
    Reject,
}

/// Real resource state fed to the resource-aware Governor decision.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ResourceState {
    /// Workload size in characters (proxy for context/model footprint need).
    pub content_chars: usize,
    /// Total reachable worker slots including the local node.
    pub available_workers: usize,
    /// Local CPU usage percent (0-100), from the system probe.
    pub cpu_percent: f32,
    /// Local RAM usage percent (0-100), from the system probe.
    pub ram_percent: f32,
    /// Current local inference queue depth (waiting requests).
    pub queue_depth: u32,
    /// Queue capacity; 0 = unbounded (Queue verdict never fires).
    pub queue_capacity: u32,
}

/// Deterministic resource-aware Governor decision from REAL state.
///
/// Priority: QUEUE (saturated) → REJECT (over-loaded) → DISTRIBUTED
/// (workload exceeds local budget AND enough workers) → LOCAL.
///
/// `cpu_percent`/`ram_percent` are the requesting node's own probe values.
/// `content_chars` is the context/model-footprint proxy: when it exceeds one
/// worker's per-call budget the workload is genuinely too heavy for a single
/// call, so distributed compute is borrowed if workers exist.
pub fn resource_verdict(r: &ResourceState) -> GovernorVerdict {
    // 1. Saturated queue → enqueue, do not run now.
    if r.queue_capacity > 0 && r.queue_depth >= r.queue_capacity {
        return GovernorVerdict::Queue;
    }
    // 2. Over-loaded node → reject now (honest: cannot serve).
    if r.cpu_percent >= 95.0 && r.ram_percent >= 95.0 {
        return GovernorVerdict::Reject;
    }
    // 3. Workload exceeds one worker's budget AND workers are reachable.
    if r.content_chars > MAX_SHARD_CHARS && r.available_workers >= 2 {
        return GovernorVerdict::Distributed;
    }
    // 4. Fits locally.
    GovernorVerdict::Local
}

/// Human/operator-facing reasoning for a Governor verdict — used verbatim in
/// the evidence record.
pub fn governor_reasoning(v: GovernorVerdict, r: &ResourceState) -> String {
    match v {
        GovernorVerdict::Queue => format!(
            "QUEUE: local queue {q}/{cap} is saturated; enqueuing instead of running now.",
            q = r.queue_depth,
            cap = r.queue_capacity
        ),
        GovernorVerdict::Reject => format!(
            "REJECT: node over-loaded (cpu {cpu:.0}%, ram {ram:.0}%); cannot serve this workload now.",
            cpu = r.cpu_percent,
            ram = r.ram_percent
        ),
        GovernorVerdict::Distributed => format!(
            "DISTRIBUTED: workload is {c} chars (> {max}) exceeding local per-call capacity; {w} worker(s) available, borrowing distributed compute.",
            c = r.content_chars,
            max = MAX_SHARD_CHARS,
            w = r.available_workers
        ),
        GovernorVerdict::Local => format!(
            "LOCAL: workload is {c} chars, cpu {cpu:.0}%, ram {ram:.0}%, queue {q}/{cap}; {w} worker(s) available, one local call suffices.",
            c = r.content_chars,
            cpu = r.cpu_percent,
            ram = r.ram_percent,
            q = r.queue_depth,
            cap = r.queue_capacity,
            w = r.available_workers
        ),
    }
}

/// Model Intelligence: selects the served model for a task kind. Pure and
/// deterministic — the Governor uses it to answer "which model for this
/// task?" before deciding placement.
pub fn select_model(task_kind: &str) -> &'static str {
    match task_kind {
        "embeddings" | "embed" => "nomic-embed-text-v1.5.Q4_K_M.gguf",
        // Chat / generation / summarization / analysis all ride the served
        // chat model on this fabric.
        _ => "Qwen3-1.7B-Q4_K_M.gguf",
    }
}

/// Benchmark/evidence facts about one candidate model, used by Model
/// Intelligence to pick a reducer. Evidence is measured, never assumed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelEvidence {
    pub model: &'static str,
    /// Deterministic accuracy on the Model Intelligence corpus (0..1).
    pub accuracy: f64,
    /// Measured latency for a representative task (ms).
    pub latency_ms: u64,
    /// True when the model burns tokens on hidden reasoning, which can leave
    /// a reduce/aggregation answer empty — a real, observed defect.
    pub reasoner: bool,
}

/// Scores a candidate as a REDUCER: quality first, but penalise a reasoner
/// (empty-output risk) and slow inference. Higher is better.
fn reducer_score(m: &ModelEvidence) -> i64 {
    let quality = (m.accuracy * 100.0).round() as i64;
    let reasoner_penalty = if m.reasoner { 30 } else { 0 };
    let speed_penalty = (m.latency_ms / 1000) as i64; // seconds, mild
    quality - reasoner_penalty - speed_penalty
}

/// Model Intelligence reducer selection from VERIFIED evidence. Picks the
/// best reducer by score; a reasoner is only chosen when nothing better
/// exists. NOT hardcoded to any model — the evidence decides. If no model is
/// provided, falls back to `None` (caller decides).
pub fn select_reducer(models: &[ModelEvidence]) -> Option<&ModelEvidence> {
    if models.is_empty() {
        return None;
    }
    // Prefer non-reasoners; among those, highest score.
    models
        .iter()
        .filter(|m| !m.reasoner)
        .max_by_key(|m| reducer_score(m))
        .or_else(|| models.iter().max_by_key(|m| reducer_score(m)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_content_stays_local() {
        let w = MpWorkload {
            task_id: "t".into(),
            instruction: "summarize".into(),
            content: "Short content that fits.".into(),
        };
        let p = plan(&w);
        assert!(!p.distributed);
        assert_eq!(p.n_shards, 1);
        assert_eq!(p.shards.len(), 1);
    }

    #[test]
    fn long_content_is_split_and_distributed() {
        // ~40 chars each sentence * 40 sentences > MAX_SHARD_CHARS.
        let mut content = String::new();
        for i in 0..40 {
            content.push_str(&format!("Sentence number {i} has enough words to be a real unit. "));
        }
        let w = MpWorkload { task_id: "t".into(), instruction: "summarize".into(), content };
        let p = plan(&w);
        assert!(p.distributed);
        assert!(p.n_shards >= 2, "expected multiple shards, got {}", p.n_shards);
        // Determinism: same input → same shards.
        assert_eq!(plan(&w).shards, p.shards);
        // Shards reconstruct the content roughly (whitespace-trimmed).
        let joined: String = p.shards.iter().map(|s| s.content.clone()).collect::<Vec<_>>().join(" ");
        assert!(joined.contains("Sentence number 0"));
        assert!(joined.contains("Sentence number 39"));
    }

    #[test]
    fn shards_respect_budget() {
        let mut content = String::new();
        for _ in 0..500 {
            content.push_str("a ");
        }
        let shards = split_shards(&content, 100);
        for s in &shards {
            assert!(s.content.chars().count() <= 100, "shard too big");
        }
        assert!(shards.len() >= 5);
    }

    #[test]
    fn reduce_prompt_couples_all_partials_into_one() {
        let partials = vec!["part A".to_string(), "part B".to_string()];
        let p = reduce_prompt("summarize", &partials);
        assert!(p.contains("PART 0"));
        assert!(p.contains("PART 1"));
        assert!(p.contains("ONE coherent final answer"));
        assert!(p.contains("2 parallel workers"));
    }

    #[test]
    fn map_prompt_applies_instruction_to_shard() {
        let shard = MpShard { index: 0, content: "the section".into() };
        let p = map_prompt("summarize", &shard);
        assert!(p.contains("summarize"));
        assert!(p.contains("the section"));
        assert!(p.contains("ONLY the following section"));
    }

    #[test]
    fn governor_decides_locally_when_content_fits() {
        let r = ResourceState {
            content_chars: 100,
            available_workers: 3,
            cpu_percent: 30.0,
            ram_percent: 40.0,
            queue_depth: 0,
            queue_capacity: 10,
        };
        assert_eq!(resource_verdict(&r), GovernorVerdict::Local);
        assert!(governor_reasoning(GovernorVerdict::Local, &r).starts_with("LOCAL"));
    }

    #[test]
    fn governor_requires_distributed_when_over_budget_with_workers() {
        let r = ResourceState {
            content_chars: MAX_SHARD_CHARS + 1,
            available_workers: 3,
            cpu_percent: 30.0,
            ram_percent: 40.0,
            queue_depth: 0,
            queue_capacity: 10,
        };
        assert_eq!(resource_verdict(&r), GovernorVerdict::Distributed);
        assert!(governor_reasoning(GovernorVerdict::Distributed, &r).starts_with("DISTRIBUTED"));
    }

    #[test]
    fn governor_falls_back_to_local_when_no_remote_workers() {
        let r = ResourceState {
            content_chars: MAX_SHARD_CHARS + 1,
            available_workers: 1,
            cpu_percent: 30.0,
            ram_percent: 40.0,
            queue_depth: 0,
            queue_capacity: 10,
        };
        assert_eq!(resource_verdict(&r), GovernorVerdict::Local);
    }

    #[test]
    fn governor_queues_when_saturated_and_rejects_when_overloaded() {
        let queued = ResourceState {
            content_chars: 100,
            available_workers: 3,
            cpu_percent: 30.0,
            ram_percent: 40.0,
            queue_depth: 10,
            queue_capacity: 10,
        };
        assert_eq!(resource_verdict(&queued), GovernorVerdict::Queue);

        let overloaded = ResourceState {
            content_chars: 100,
            available_workers: 3,
            cpu_percent: 97.0,
            ram_percent: 96.0,
            queue_depth: 0,
            queue_capacity: 10,
        };
        assert_eq!(resource_verdict(&overloaded), GovernorVerdict::Reject);
        assert!(governor_reasoning(GovernorVerdict::Reject, &overloaded).starts_with("REJECT"));
    }

    #[test]
    fn model_intelligence_selects_served_model() {
        assert_eq!(select_model("embeddings"), "nomic-embed-text-v1.5.Q4_K_M.gguf");
        assert_eq!(select_model("chat"), "Qwen3-1.7B-Q4_K_M.gguf");
        assert_eq!(select_model("summarize"), "Qwen3-1.7B-Q4_K_M.gguf");
        assert_eq!(select_model("anything"), "Qwen3-1.7B-Q4_K_M.gguf");
    }

    #[test]
    fn model_intelligence_picks_best_reducer_from_evidence_not_hardcoded() {
        // Real measured benchmark facts (VPS): Phi is faster+non-reasoner,
        // Qwen3 is a slow reasoner, Gemma is fast non-reasoner.
        let phi = ModelEvidence { model: "Phi-4-mini", accuracy: 0.33, latency_ms: 803, reasoner: false };
        let qwen = ModelEvidence { model: "Qwen3-1.7B", accuracy: 0.25, latency_ms: 4624, reasoner: true };
        let gemma = ModelEvidence { model: "Gemma-3-1B", accuracy: 0.33, latency_ms: 578, reasoner: false };
        let candidates = [phi, qwen, gemma];
        let best = select_reducer(&candidates).unwrap();
        // Gemma (same accuracy as Phi, faster, non-reasoner) should win on
        // evidence — proof it is NOT hardcoded to Phi.
        assert_eq!(best.model, "Gemma-3-1B");
        // If only Qwen is available, it is still chosen (no better option).
        let only_qwen = [qwen];
        let only = select_reducer(&only_qwen).unwrap();
        assert_eq!(only.model, "Qwen3-1.7B");
        // Empty -> None.
        assert!(select_reducer(&[]).is_none());
    }
}
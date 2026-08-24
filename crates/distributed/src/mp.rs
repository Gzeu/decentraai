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

/// The Governor's automatic verdict on a workload: local only, or distributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum GovernorVerdict {
    /// The workload fits the local node's per-call budget; execute locally.
    LocalCapacitySufficient,
    /// The workload exceeds local capacity and enough workers are reachable;
    /// borrow distributed compute and run map-reduce.
    DistributedComputeRequired,
}

/// Deterministic Governor decision based on REAL availability.
///
/// - content within one worker's budget → local.
/// - content beyond budget AND at least `min_workers` slots are reachable
///   (local + ≥1 remote) → distributed.
/// - content beyond budget but NO remote worker is reachable → local (the
///   honest fallback: splitting without help would just be slow serial work).
///
/// `available_workers` counts the total reachable slots including the local
/// node (so 3 nodes → 3).
pub fn governor_decide(content_chars: usize, available_workers: usize) -> GovernorVerdict {
    if content_chars <= MAX_SHARD_CHARS {
        GovernorVerdict::LocalCapacitySufficient
    } else if available_workers >= 2 {
        GovernorVerdict::DistributedComputeRequired
    } else {
        // No remote capacity to borrow: run locally even though it exceeds
        // one call's budget (the honest, non-fabricated answer).
        GovernorVerdict::LocalCapacitySufficient
    }
}

/// Human/operator-facing reasoning for a Governor verdict — used verbatim in
/// the evidence record.
pub fn governor_reasoning(v: GovernorVerdict, content_chars: usize, available_workers: usize) -> String {
    match v {
        GovernorVerdict::LocalCapacitySufficient if content_chars <= MAX_SHARD_CHARS => format!(
            "LOCAL_CAPACITY_SUFFICIENT: workload is {content_chars} chars (<= {MAX_SHARD_CHARS}) so one local worker call is enough; {available_workers} worker(s) available."
        ),
        GovernorVerdict::LocalCapacitySufficient => format!(
            "LOCAL_CAPACITY_SUFFICIENT: workload is {content_chars} chars (> {MAX_SHARD_CHARS}) but only {available_workers} worker(s) reachable, so distributed compute cannot be borrowed; executing locally."
        ),
        GovernorVerdict::DistributedComputeRequired => format!(
            "DISTRIBUTED_COMPUTE_REQUIRED: workload is {content_chars} chars (> {MAX_SHARD_CHARS}) exceeding local per-call capacity; {available_workers} worker(s) available, borrowing distributed compute."
        ),
    }
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
        assert_eq!(
            governor_decide(100, 1),
            GovernorVerdict::LocalCapacitySufficient
        );
        assert_eq!(
            governor_decide(100, 3),
            GovernorVerdict::LocalCapacitySufficient
        );
        assert!(governor_reasoning(GovernorVerdict::LocalCapacitySufficient, 100, 3)
            .starts_with("LOCAL_CAPACITY_SUFFICIENT"));
    }

    #[test]
    fn governor_requires_distributed_when_over_budget_with_workers() {
        assert_eq!(
            governor_decide(MAX_SHARD_CHARS + 1, 3),
            GovernorVerdict::DistributedComputeRequired
        );
        assert_eq!(
            governor_decide(MAX_SHARD_CHARS + 1, 2),
            GovernorVerdict::DistributedComputeRequired
        );
        assert!(governor_reasoning(GovernorVerdict::DistributedComputeRequired, 2000, 3)
            .starts_with("DISTRIBUTED_COMPUTE_REQUIRED"));
    }

    #[test]
    fn governor_falls_back_to_local_when_no_remote_workers() {
        // Over budget but only 1 worker reachable: honest local fallback.
        assert_eq!(
            governor_decide(MAX_SHARD_CHARS + 1, 1),
            GovernorVerdict::LocalCapacitySufficient
        );
    }
}
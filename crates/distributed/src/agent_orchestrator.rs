//! AgentOrchestrator — the runtime that turns the pure P3–P9 fabric into
//! real agent-to-agent work (P3.5).
//!
//! The pure modules (`delegation`, `workflow`, `verification`, `memory`,
//! `reputation`, `talent_tree`) define *how* to plan, verify and rank; this
//! orchestrator binds them to the live fabric:
//!
//! - **plan** — decompose a master task with the `DelegationPlanner`.
//! - **select** — choose an executor for each stage: an agent from the local
//!   or advertised (remote) view that satisfies the capability, ranked by
//!   reputation (best first, deterministic).
//! - **delegate** — hand the stage to a remote agent over the
//!   [`crate::agent_messenger::AgentMessenger`] (`AgentMessage::Delegate`) and
//!   await the `Reply`.
//! - **verify** — run the stage's per-hop verification.
//! - **learn** — feed the result into the reputation store and, when a scope
//!   is requested, into collective memory.
//!
//! The orchestrator is coordinator-side and works with any agent that speaks
//! the message protocol; it does not require a special runtime on the remote
//! side beyond draining its inbox and replying to `Delegate`.

use anyhow::{Context, Result, bail};
use decentraai_agents::{
    AgentMessage, AgentTask, DelegationPlan, DelegationPlanner, DelegationStage, DelegationVerdict,
    MessageKind, ReputationFactor, ReputationStore, ReputationUpdate, StageResult,
    TaskVerification, WorkflowOutcome, match_agent_semantic,
};
use libp2p::PeerId;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::agent_memory::MemoryStore;
use crate::agent_messenger::AgentMessenger;
use crate::agents::{AgentManager, AgentView};

/// A stage executor chosen by the orchestrator (agent id + hosting peer).
#[derive(Debug, Clone)]
pub struct StageExecutor {
    pub agent_id: String,
    pub peer: PeerId,
}

/// Coordinator-side collective-intelligence orchestrator.
#[derive(Clone)]
pub struct AgentOrchestrator {
    messenger: Arc<AgentMessenger>,
    agents: Arc<AgentManager>,
    /// This node's peer id, stamped on Delegates so remote runtimes know
    /// where to reply.
    local_peer: PeerId,
    reputation: Arc<Mutex<ReputationStore>>,
    /// Optional persistent collective-memory store. When attached, verified
    /// workflow outcomes are written into it (scope `workflow_results`),
    /// so completed work becomes collective knowledge the node can reuse.
    memory_store: Option<Arc<MemoryStore>>,
    /// Max time to wait for a delegated stage's Reply.
    delegate_timeout: Duration,
}

impl AgentOrchestrator {
    /// Wraps the live fabric: messenger for delegation, agent manager for the
    /// capability view, and the local peer id.
    pub fn new(
        messenger: Arc<AgentMessenger>,
        agents: Arc<AgentManager>,
        local_peer: PeerId,
    ) -> Self {
        Self {
            messenger,
            agents,
            local_peer,
            reputation: Arc::new(Mutex::new(ReputationStore::new())),
            memory_store: None,
            delegate_timeout: Duration::from_secs(60),
        }
    }

    /// Attaches a shared reputation store so selection ranks real history.
    pub fn with_reputation_store(&mut self, store: Arc<Mutex<ReputationStore>>) -> &mut Self {
        self.reputation = store;
        self
    }

    /// Attaches the persistent collective-memory store so verified workflow
    /// outcomes are written into it (scope `workflow_results`).
    pub fn with_memory_store(&mut self, store: Arc<MemoryStore>) -> &mut Self {
        self.memory_store = Some(store);
        self
    }

    /// Sets the per-delegation reply timeout.
    pub fn with_delegate_timeout(&mut self, timeout: Duration) -> &mut Self {
        self.delegate_timeout = timeout;
        self
    }

    /// Plans a master task into a routable DAG (pure `DelegationPlanner`).
    pub fn plan(&self, master_task: &AgentTask) -> Result<DelegationPlan> {
        let agents = self
            .agents
            .view()
            .iter()
            .map(|v| v.record.clone())
            .collect::<Vec<_>>();
        DelegationPlanner
            .plan_task(
                master_task,
                &agents,
                format!("plan-{}", master_task.task_id),
                now_ms(),
            )
            .context("planning agent task")
    }

    /// Picks the best executor for a stage among the known agents.
    ///
    /// Ranking (deterministic): agents that satisfy the stage's capability,
    /// local first, then by reputation score desc (unknown reputation = 0, a
    /// tie never penalises a capable agent), then by agent_id asc. A stage
    /// with NO capability requirements (e.g. a synthesis stage) is eligible on
    /// any agent — the orchestrator never invents an executor, but it also
    /// does not block unconstrained stages on a capability match.
    pub fn select_executor(&self, stage: &DelegationStage) -> Option<StageExecutor> {
        let reputation = self.reputation.lock().unwrap();
        let cap_label = stage
            .task
            .required_capabilities
            .first()
            .map(|r| r.capability.label().to_string())
            .unwrap_or_default();
        let mut candidates: Vec<(bool /*local*/, f32 /*score*/, String, PeerId)> = self
            .agents
            .view()
            .into_iter()
            .filter(|v| {
                stage.task.required_capabilities.is_empty()
                    || match_agent_semantic(&v.record, &stage.task.required_capabilities)
                        .is_satisfied()
            })
            .map(|v| {
                let local = !v.remote;
                let score = reputation
                    .get(&v.record.agent_id, &cap_label)
                    .map(|r| r.score())
                    .unwrap_or(0.0);
                (local, score, v.record.agent_id.clone(), v.peer_id)
            })
            .collect();
        candidates.sort_by(|a, b| {
            // Local first, then score desc, then agent_id asc (deterministic).
            b.0.cmp(&a.0)
                .then_with(|| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| a.2.cmp(&b.2))
        });
        candidates
            .into_iter()
            .next()
            .map(|(_, _, agent_id, peer)| StageExecutor { agent_id, peer })
    }

    /// Delegates one stage to a remote (or local-peer) agent and awaits the
    /// `Reply`. Sends `AgentMessage::Delegate` to the hosting peer and drains
    /// the messenger inbox until a `Reply` for this task arrives or the
    /// timeout elapses.
    pub async fn delegate_stage(
        &self,
        executor: &StageExecutor,
        stage: &DelegationStage,
        inputs: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let task_id = stage.task.task_id.clone();
        let payload = inputs.clone();
        let message = AgentMessage::new(
            format!("delegate-{task_id}-{}", stage.stage_id),
            "orchestrator".to_string(),
            executor.agent_id.clone(),
            MessageKind::Delegate,
        )
        .with_task(&task_id)
        .with_payload(payload)
        // Stamp our peer so the remote runtime replies to us.
        .with_from_peer(self.local_peer.to_string())
        .with_created_at_ms(now_ms());

        // Send (re-dialing until the connection settles — a dialed link needs
        // a beat), then await the matching Reply in the orchestrator's inbox.
        // The message carries a task id + nonce; a retry only re-sends while
        // no reply has arrived, so work is not double-committed downstream.
        let deadline = Instant::now() + self.delegate_timeout;
        let mut sent = false;
        loop {
            if !sent {
                match self.messenger.send(executor.peer, message.clone()).await {
                    Ok(()) => sent = true,
                    Err(e) => {
                        tracing::debug!(
                            error = %e, stage = %stage.stage_id, "re-dialing delegate send"
                        );
                    }
                }
            }
            if let Some(reply) = self
                .messenger
                .pop("orchestrator")
                .filter(|m| m.task_id.as_deref() == Some(task_id.as_str()))
            {
                let payload = reply
                    .payload
                    .ok_or_else(|| anyhow::anyhow!("reply for task '{task_id}' has no payload"))?;
                return Ok(payload);
            }
            if Instant::now() > deadline {
                bail!(
                    "timed out waiting for a reply to delegated stage '{}'",
                    stage.stage_id
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Plans a master task and executes it across the fabric (async).
    pub async fn orchestrate(&self, master_task: &AgentTask) -> WorkflowOutcome {
        let plan = match self.plan(master_task) {
            Ok(p) => p,
            Err(e) => {
                return WorkflowOutcome::new(
                    "failed".to_string(),
                    master_task.task_id.clone(),
                    DelegationPlan {
                        plan_id: "failed".into(),
                        master_task_id: master_task.task_id.clone(),
                        stages: Vec::new(),
                        created_at_ms: now_ms(),
                    },
                    execute_plan_failed(&e.to_string()),
                );
            }
        };
        self.run_plan(&plan, None).await
    }

    /// Executes an already-instantiated plan (e.g. one built from a
    /// `WorkflowTemplate::instantiate`) across the fabric by delegating each
    /// stage to its chosen executor and verifying per hop.
    ///
    /// `seed` is merged into every stage's inputs (its fields win only when a
    /// dependency did not already produce them) so the original user prompt
    /// stays available to each stage.
    pub async fn orchestrate_plan(
        &self,
        plan: &DelegationPlan,
        seed: Option<&serde_json::Value>,
    ) -> WorkflowOutcome {
        self.run_plan(plan, seed).await
    }

    /// The shared execution loop: delegate stages in topological order,
    /// verify per hop, collect outputs. Honest Partial on any failure.
    async fn run_plan(
        &self,
        plan: &DelegationPlan,
        seed: Option<&serde_json::Value>,
    ) -> WorkflowOutcome {
        let plan_id = plan.plan_id.clone();
        let task_id = plan.master_task_id.clone();

        let mut outputs: HashMap<String, serde_json::Value> = HashMap::new();
        let mut results: Vec<StageResult> = Vec::new();
        let mut failed: Vec<String> = Vec::new();

        for stage in plan.stages_in_order() {
            let executor = match self.select_executor(stage) {
                Some(ex) => ex,
                None => {
                    failed.push(stage.stage_id.clone());
                    results.push(StageResult {
                        stage_id: stage.stage_id.clone(),
                        agent_id: String::new(),
                        output: None,
                        verified: false,
                        checks: Vec::new(),
                        error: Some("no capable agent available".to_string()),
                    });
                    continue;
                }
            };
            // Build merged inputs from dependency outputs.
            let mut inputs = serde_json::Map::new();
            let mut missing = false;
            for dep in &stage.depends_on {
                if let Some(out) = outputs.get(dep) {
                    inputs.insert(dep.clone(), out.clone());
                } else {
                    missing = true;
                }
            }
            // Merge the seed (e.g. the original user prompt) into the inputs,
            // only where a dependency did not already supply that field.
            if let Some(Value::Object(seed_map)) = seed {
                for (k, v) in seed_map {
                    inputs.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
            if missing {
                failed.push(stage.stage_id.clone());
                results.push(StageResult {
                    stage_id: stage.stage_id.clone(),
                    agent_id: executor.agent_id.clone(),
                    output: None,
                    verified: false,
                    checks: Vec::new(),
                    error: Some("dependency produced no output".to_string()),
                });
                continue;
            }
            let input_value = serde_json::Value::Object(inputs);
            let t0 = Instant::now();
            let capability = stage
                .task
                .required_capabilities
                .first()
                .map(|r| r.capability.label().to_string())
                .unwrap_or_default();
            match self.delegate_stage(&executor, stage, &input_value).await {
                Ok(output) => {
                    let latency_ms = t0.elapsed().as_millis() as u64;
                    let mut checks = Vec::new();
                    let mut verified = true;
                    if stage.verification != TaskVerification::None {
                        let check = verify_value(&output, stage.task.output_schema.as_deref());
                        verified = check.passed;
                        checks.push(check);
                    }
                    if verified {
                        outputs.insert(stage.stage_id.clone(), output.clone());
                    } else {
                        failed.push(stage.stage_id.clone());
                    }
                    // Reputation from real execution: a verified delegated
                    // stage is a success signal; latency feeds the Latency
                    // factor (normalised so faster is better).
                    self.record_execution(&executor.agent_id, &capability, true, latency_ms);
                    results.push(StageResult {
                        stage_id: stage.stage_id.clone(),
                        agent_id: executor.agent_id.clone(),
                        output: Some(output),
                        verified,
                        checks,
                        error: if verified {
                            None
                        } else {
                            Some("output failed verification".into())
                        },
                    });
                }
                Err(e) => {
                    let latency_ms = t0.elapsed().as_millis() as u64;
                    // A failed delegated stage is a reliability signal.
                    self.record_execution(&executor.agent_id, &capability, false, latency_ms);
                    failed.push(stage.stage_id.clone());
                    results.push(StageResult {
                        stage_id: stage.stage_id.clone(),
                        agent_id: executor.agent_id.clone(),
                        output: None,
                        verified: false,
                        checks: Vec::new(),
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        let final_output = outputs.get("synthesis").cloned();
        let verdict = if failed.is_empty() {
            DelegationVerdict::Completed
        } else {
            DelegationVerdict::Partial {
                failed_stages: failed,
            }
        };
        let result = decentraai_agents::DelegationResult {
            plan_id: plan_id.clone(),
            task_id: task_id.clone(),
            verdict: verdict.clone(),
            stages: results,
            final_output,
        };
        // Collective memory: a completed workflow's verified results become
        // collective knowledge (scope `workflow_results`). Best-effort — a
        // memory write must never fail the execution path.
        if matches!(verdict, DelegationVerdict::Completed) {
            if let Some(store) = &self.memory_store {
                write_workflow_to_memory(store, &plan_id, &task_id, &result);
            }
        }
        WorkflowOutcome::new(plan_id, task_id, plan.clone(), result)
    }

    /// Returns the flattened known agents (for dashboards/tests).
    pub fn known_agents(&self) -> Vec<AgentView> {
        self.agents.view()
    }

    /// Records a real execution outcome into the reputation store
    /// (per agent, per capability). Reliability/Quality reflect success;
    /// Latency is normalised so faster is better (1.0 at ~0ms, decaying to
    /// ~0.5 at 30s, floor 0.2 — a slow-but-working agent is not punished to
    /// zero).
    fn record_execution(&self, agent_id: &str, capability: &str, success: bool, latency_ms: u64) {
        let now = now_ms();
        let cap = if capability.is_empty() {
            "_"
        } else {
            capability
        };
        let reliability = if success { 1.0 } else { 0.0 };
        let quality = if success { 1.0 } else { 0.0 };
        let latency_score = {
            let raw = 1.0 - (latency_ms as f64 / 30_000.0);
            raw.clamp(0.2, 1.0) as f32
        };
        let mut store = self.reputation.lock().unwrap();
        store.observe(ReputationUpdate::new(
            agent_id,
            cap,
            ReputationFactor::Reliability,
            reliability,
            now,
        ));
        store.observe(ReputationUpdate::new(
            agent_id,
            cap,
            ReputationFactor::Quality,
            quality,
            now,
        ));
        store.observe(ReputationUpdate::new(
            agent_id,
            cap,
            ReputationFactor::Latency,
            latency_score,
            now,
        ));
    }

    /// A serializable snapshot of the reputation store (per agent, per
    /// capability), for the dashboard. Real, measured history only.
    pub fn reputation_snapshot(&self) -> Vec<serde_json::Value> {
        let store = self.reputation.lock().unwrap();
        store
            .all()
            .into_iter()
            .map(|rep| {
                serde_json::json!({
                    "agent_id": rep.agent_id,
                    "capability": rep.capability,
                    "score": rep.score(),
                    "reasons": rep.reasons(),
                })
            })
            .collect()
    }
}

/// Per-hop value-level verification (mirror of delegation's check).
fn verify_value(
    output: &serde_json::Value,
    schema_hint: Option<&str>,
) -> decentraai_agents::VerificationCheck {
    match schema_hint {
        None => decentraai_agents::VerificationCheck {
            check_kind: decentraai_agents::CheckKind::Schema,
            passed: true,
            detail: "no schema required".to_string(),
        },
        Some(hint) => {
            let hint_value: Result<serde_json::Value, _> = serde_json::from_str(hint);
            match hint_value {
                Ok(serde_json::Value::Object(_)) => {
                    if matches!(output, serde_json::Value::Object(_)) {
                        decentraai_agents::VerificationCheck {
                            check_kind: decentraai_agents::CheckKind::Schema,
                            passed: true,
                            detail: "output is a JSON object per schema hint".to_string(),
                        }
                    } else {
                        decentraai_agents::VerificationCheck {
                            check_kind: decentraai_agents::CheckKind::Schema,
                            passed: false,
                            detail: "output is not a JSON object, but the schema hint requires one"
                                .to_string(),
                        }
                    }
                }
                _ => decentraai_agents::VerificationCheck {
                    check_kind: decentraai_agents::CheckKind::Schema,
                    passed: true,
                    detail: "schema hint not a JSON object — structural check only (honest)"
                        .to_string(),
                },
            }
        }
    }
}

/// Builds a failed delegation result (used when planning fails).
fn execute_plan_failed(reason: &str) -> decentraai_agents::DelegationResult {
    decentraai_agents::DelegationResult {
        plan_id: "failed".into(),
        task_id: String::new(),
        verdict: DelegationVerdict::Failed {
            reason: reason.to_string(),
        },
        stages: Vec::new(),
        final_output: None,
    }
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Writes a completed workflow's verified stage outputs into a collective
/// memory store (scope `workflow_results`). Idempotent by entry id;
/// best-effort by design — a memory failure must never fail the execution.
///
/// The verdict gate lives here (not at the call site) so the invariant
/// "only completed workflows become memory" is testable in isolation.
fn write_workflow_to_memory(
    store: &MemoryStore,
    plan_id: &str,
    task_id: &str,
    result: &decentraai_agents::DelegationResult,
) {
    if !matches!(result.verdict, DelegationVerdict::Completed) {
        return;
    }
    // The scope is registered once; concurrent orchestrations may race, so an
    // existing scope is treated as already registered.
    let scope = decentraai_agents::MemoryScope::new(
        "workflow_results",
        "orchestrator",
        decentraai_agents::MemoryLevel::Team,
    );
    if store.register_scope(&scope).is_err() {
        // Duplicate scope is fine — it already exists.
    }
    let now = now_ms();
    let task_short = task_id.chars().take(12).collect::<String>();
    // One entry per verified stage, plus a summary entry with the final
    // output. Content is a compact JSON of the verified output value.
    for sr in result
        .stages
        .iter()
        .filter(|s| s.verified && s.error.is_none())
    {
        let content = serde_json::json!({
            "stage_id": sr.stage_id,
            "output": sr.output.clone().unwrap_or(Value::Null),
        })
        .to_string();
        let entry = decentraai_agents::MemoryEntry::new(
            format!("wf:{plan_id}:{task_short}:{}", sr.stage_id),
            "workflow_results".to_string(),
            "orchestrator".to_string(),
            "local".to_string(),
            content,
        )
        .tagged("workflow")
        .tagged("verified")
        .created_at(now);
        let _ = store.write("workflow_results", &entry, "orchestrator", true, true, true);
    }
    // Summary entry with the verdict + final output.
    let summary = serde_json::json!({
        "task_id": task_id,
        "verdict": "completed",
        "final_output": result.final_output.clone().unwrap_or(Value::Null),
        "completed_stages": result.stages.iter().filter(|s| s.verified).count(),
    })
    .to_string();
    let summary_entry = decentraai_agents::MemoryEntry::new(
        format!("wf:{plan_id}:{task_short}:summary"),
        "workflow_results".to_string(),
        "orchestrator".to_string(),
        "local".to_string(),
        summary,
    )
    .tagged("workflow")
    .tagged("summary")
    .created_at(now);
    let _ = store.write(
        "workflow_results",
        &summary_entry,
        "orchestrator",
        true,
        true,
        true,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_memory::MemoryStore;
    use decentraai_agents::{DelegationResult, StageResult};
    use serde_json::json;
    use std::sync::Arc;

    fn temp_store(name: &str) -> Arc<MemoryStore> {
        let path = std::env::temp_dir().join(format!(
            "decentraai-mem-test-{}-{}.sqlite",
            std::process::id(),
            name
        ));
        let _ = std::fs::remove_file(&path);
        Arc::new(MemoryStore::open(&path).unwrap())
    }

    fn verified_stage(stage_id: &str) -> StageResult {
        StageResult {
            stage_id: stage_id.into(),
            agent_id: "a:1".into(),
            output: Some(json!({"result": stage_id})),
            verified: true,
            checks: Vec::new(),
            error: None,
        }
    }

    #[test]
    fn completed_workflow_writes_memory_entries() {
        let store = temp_store("completed");
        let result = DelegationResult {
            plan_id: "p1".into(),
            task_id: "t1".into(),
            verdict: DelegationVerdict::Completed,
            stages: vec![verified_stage("research"), verified_stage("synthesis")],
            final_output: Some(json!({"text": "done"})),
        };
        write_workflow_to_memory(&store, "p1", "t1", &result);

        let scopes = store.list_scopes().unwrap();
        assert!(
            scopes.iter().any(|s| s.name == "workflow_results"),
            "workflow_results scope must be registered"
        );
        let entries = store
            .read("workflow_results", "orchestrator", true)
            .unwrap();
        // 2 verified stages + 1 summary entry.
        assert_eq!(
            entries.len(),
            3,
            "expected 3 memory entries, got {}",
            entries.len()
        );
        assert!(
            entries
                .iter()
                .any(|e| e.tags.contains(&"summary".to_string())),
            "a summary entry must exist"
        );
        assert!(
            entries
                .iter()
                .any(|e| e.tags.contains(&"verified".to_string())),
            "verified stage entries must exist"
        );
    }

    #[test]
    fn partial_workflow_is_not_written_to_memory() {
        let store = temp_store("partial");
        let result = DelegationResult {
            plan_id: "p2".into(),
            task_id: "t2".into(),
            verdict: DelegationVerdict::Partial {
                failed_stages: vec!["synthesis".into()],
            },
            stages: vec![verified_stage("research")],
            final_output: None,
        };
        write_workflow_to_memory(&store, "p2", "t2", &result);

        let scopes = store.list_scopes().unwrap();
        assert!(
            !scopes.iter().any(|s| s.name == "workflow_results"),
            "partial workflow must not create the memory scope"
        );
    }

    #[test]
    fn workflow_memory_write_is_idempotent() {
        let store = temp_store("idempotent");
        let result = DelegationResult {
            plan_id: "p3".into(),
            task_id: "t3".into(),
            verdict: DelegationVerdict::Completed,
            stages: vec![verified_stage("research")],
            final_output: Some(json!({"text": "done"})),
        };
        write_workflow_to_memory(&store, "p3", "t3", &result);
        write_workflow_to_memory(&store, "p3", "t3", &result);

        let entries = store
            .read("workflow_results", "orchestrator", true)
            .unwrap();
        assert_eq!(
            entries.len(),
            2,
            "idempotent write must not duplicate entries"
        );
    }
}

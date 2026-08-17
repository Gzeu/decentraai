//! AgentRuntime — the remote side that turns the orchestrator's `Delegate`
//! into real, executed agent work (production agent runtime).
//!
//! The orchestrator (`crate::agent_orchestrator::AgentOrchestrator`) is the
//! coordinator half: it plans, selects executors and delegates stages. This
//! is the *executor* half that runs on the node hosting an agent: it drains
//! the agent's inbox, runs each `AgentMessage::Delegate` through an injected
//! executor (e.g. a local llama-server call or another agent), and replies
//! with `AgentMessage::Reply` addressed back to the delegating peer.
//!
//! The executor is injected via the [`AgentExecutor`] trait so the runtime is
//! testable with synthetic functions and the production node can plug in a
//! real inference backend — the runtime itself never touches the engine.

use anyhow::{Context, Result};
use decentraai_agents::{AgentMessage, AgentTask, MessageKind};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::agent_messenger::AgentMessenger;
use crate::DistributedInference;

/// Executes a delegated task asynchronously and returns the output JSON value.
///
/// This is the seam to the real engine: the runtime calls this with the
/// task's inputs (by value, cloned by the caller); the callback runs the
/// model/tool and returns an output `serde_json::Value`. Async so a real
/// inference backend (e.g. `route_request`) can be awaited inside.
pub type AgentExecutor = dyn Fn(
        AgentTask,
        serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value>> + Send>>
    + Send
    + Sync;

/// The outcome of processing one delegated message.
#[derive(Debug, Clone, PartialEq)]
pub enum ExecutedMessage {
    /// A Delegate was executed and a Reply was sent.
    Replied { task_id: String },
    /// A non-Delegate message was ignored (the runtime only answers Delegate).
    Ignored,
    /// The runtime has no executor configured and refused the Delegate.
    NoExecutor,
}

/// Runs a single agent's inbox, executing Delegates and replying.
#[derive(Clone)]
pub struct AgentRuntime {
    agent_id: String,
    messenger: Arc<AgentMessenger>,
    executor: Option<Arc<AgentExecutor>>,
    poll_interval: Duration,
}

impl AgentRuntime {
    /// A runtime for `agent_id` that answers Delegates via `messenger`.
    pub fn new(agent_id: impl Into<String>, messenger: Arc<AgentMessenger>) -> Self {
        Self {
            agent_id: agent_id.into(),
            messenger,
            executor: None,
            poll_interval: Duration::from_millis(50),
        }
    }

    /// Attaches the executor that actually performs the work.
    pub fn with_executor<F, Fut>(&mut self, executor: F) -> &mut Self
    where
        F: Fn(AgentTask, serde_json::Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<serde_json::Value>> + Send + 'static,
    {
        self.executor = Some(Arc::new(move |t, i| Box::pin(executor(t, i))));
        self
    }

    /// Sets the inbox poll interval.
    pub fn with_poll_interval(&mut self, interval: Duration) -> &mut Self {
        self.poll_interval = interval;
        self
    }

    /// The agent id this runtime serves.
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Whether an executor is configured.
    pub fn can_execute(&self) -> bool {
        self.executor.is_some()
    }

    /// Drains exactly one pending message for this agent and processes it.
    /// Returns `None` when the inbox is empty. Does NOT block on the inbox.
    pub async fn process_one(&self) -> Result<Option<ExecutedMessage>> {
        let Some(msg) = self.messenger.pop(self.agent_id()) else {
            return Ok(None);
        };
        if msg.kind != MessageKind::Delegate {
            return Ok(Some(ExecutedMessage::Ignored));
        }
        let task_id = msg.task_id.unwrap_or_else(|| "untracked".to_string());
        let Some(executor) = &self.executor else {
            tracing::warn!(agent = %self.agent_id, task = %task_id, "refusing Delegate: no executor");
            return Ok(Some(ExecutedMessage::NoExecutor));
        };
        let task = AgentTask::new(&task_id);
        let inputs = msg.payload.unwrap_or(serde_json::Value::Null);
        // Where to reply: the delegating peer recorded by the orchestrator.
        let reply_peer: libp2p::PeerId = msg
            .from_peer
            .as_deref()
            .and_then(|p| p.parse().ok())
            .ok_or_else(|| {
                anyhow::anyhow!("delegate from '{}' has no reply peer", msg.from_agent)
            })?;

        let output = match executor(task, inputs).await {
            Ok(out) => out,
            Err(e) => {
                // Reply with an error-shaped payload so the orchestrator can
                // mark the stage failed rather than hang.
                let reply = AgentMessage::new(
                    format!("reply-{task_id}"),
                    self.agent_id(),
                    "orchestrator",
                    MessageKind::Reply,
                )
                .with_task(&task_id)
                .with_payload(serde_json::json!({ "error": e.to_string() }))
                .with_created_at_ms(now_ms());
                self.messenger
                    .send(reply_peer, reply)
                    .await
                    .with_context(|| format!("sending error reply for '{task_id}'"))?;
                return Ok(Some(ExecutedMessage::Replied { task_id }));
            }
        };

        let reply = AgentMessage::new(
            format!("reply-{task_id}"),
            self.agent_id(),
            "orchestrator",
            MessageKind::Reply,
        )
        .with_task(&task_id)
        .with_payload(output)
        .with_created_at_ms(now_ms());
        // Reply to the delegating peer. A transport failure is logged; the
        // orchestrator surfaces the stage failure via its timeout.
        self.messenger
            .send(reply_peer, reply)
            .await
            .with_context(|| format!("sending reply for '{task_id}'"))?;
        Ok(Some(ExecutedMessage::Replied { task_id }))
    }

    /// Runs the drain loop forever: polls the inbox, executes Delegates,
    /// replies. Never returns under normal operation.
    pub async fn run_forever(&self) {
        loop {
            match self.process_one().await {
                Ok(Some(_)) => {}
                Ok(None) => tokio::time::sleep(self.poll_interval).await,
                Err(e) => {
                    tracing::warn!(agent = %self.agent_id, error = %e, "runtime processing error");
                    tokio::time::sleep(self.poll_interval).await;
                }
            }
        }
    }
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// An [`AgentRuntime`] executor that runs a delegated LLM task through the
/// fabric's real inference path (`DistributedInference::route_request`).
///
/// The delegate's JSON input may carry `prompt` (string) and an optional
/// `model_hash`; when the task names a model (via its required workload) or
/// the input does, that wins; otherwise the executor's default model runs.
/// The returned output is `{ "text": <generated>, "model_hash": ..., "tokens": N }`.
#[derive(Clone)]
pub struct InferenceAgentExecutor {
    distributed: Arc<DistributedInference>,
    default_model_hash: String,
}

impl InferenceAgentExecutor {
    /// An inference executor routing to `distributed` with a fallback model.
    pub fn new(distributed: Arc<DistributedInference>, default_model_hash: String) -> Self {
        Self {
            distributed,
            default_model_hash,
        }
    }

    /// Runs the task through the fabric planner → reservation → worker path.
    pub async fn execute(
        &self,
        task: &AgentTask,
        inputs: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let request = infer_request_from(task, inputs, &self.default_model_hash)?;
        let response = self
            .distributed
            .route_request(request)
            .await
            .context("routing delegated inference")?;
        Ok(serde_json::json!({
            "text": response.output,
            "model_hash": task.required_workload.as_ref().map(|w| w.model_hash.clone()).unwrap_or_else(|| self.default_model_hash.clone()),
            "tokens": response.tokens_used,
        }))
    }
}

/// Pure decision: builds an `InferRequest` for a delegated task from its JSON
/// input and the executor's default model. Testable without a network.
///
/// The input may be:
/// - a JSON object `{ "prompt": "...", "model_hash": "..." }` (model optional),
/// - a JSON string (treated as the prompt),
/// - anything else → an error (no prompt).
fn infer_request_from(
    task: &AgentTask,
    inputs: &serde_json::Value,
    default_model_hash: &str,
) -> Result<decentraai_protocol::InferRequest> {
    let (prompt, input_model): (String, Option<String>) = match inputs {
        serde_json::Value::String(s) => (s.clone(), None),
        serde_json::Value::Object(map) => {
            let prompt = map
                .get("prompt")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("delegate input object is missing a 'prompt' string"))?
                .to_string();
            let model = map.get("model_hash").and_then(|v| v.as_str()).map(str::to_string);
            (prompt, model)
        }
        other => {
            anyhow::bail!("delegate input must be a prompt string or an object, got {other:?}")
        }
    };

    // Model resolution: task's required workload > input > default.
    let model_hash = task
        .required_workload
        .as_ref()
        .map(|w| w.model_hash.clone())
        .or(input_model)
        .unwrap_or_else(|| default_model_hash.to_string());

    let max_tokens = task
        .required_workload
        .as_ref()
        .map(|w| w.max_tokens)
        .or_else(|| (task.budget_max_tokens > 0).then_some(task.budget_max_tokens))
        .unwrap_or(1024);

    Ok(decentraai_protocol::InferRequest::new(
        model_hash,
        prompt,
        max_tokens,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_p2p::P2PNode;
    use decentraai_identity::Identity;

    /// A messenger with a dead transport node; push/pop work, send would fail
    /// (not reached on the NoExecutor/Ignored paths under test).
    fn test_messenger() -> Arc<AgentMessenger> {
        let node = P2PNode::new(
            &Identity::generate(),
            decentraai_p2p::DEFAULT_MAX_MESSAGE_BYTES,
            decentraai_p2p::DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
            None,
        )
        .unwrap();
        Arc::new(AgentMessenger::new(node))
    }

    #[tokio::test]
    async fn runtime_without_executor_refuses_delegate() {
        let messenger = test_messenger();
        let runtime = AgentRuntime::new("a:1", messenger.clone());
        messenger.push_inbound(AgentMessage::new("m", "orch", "a:1", MessageKind::Delegate));
        let result = runtime.process_one().await.unwrap();
        assert_eq!(result, Some(ExecutedMessage::NoExecutor));
    }

    #[tokio::test]
    async fn runtime_ignores_non_delegate() {
        let messenger = test_messenger();
        let runtime = AgentRuntime::new("a:1", messenger.clone());
        messenger.push_inbound(AgentMessage::new("m", "x", "a:1", MessageKind::Ask));
        let result = runtime.process_one().await.unwrap();
        assert_eq!(result, Some(ExecutedMessage::Ignored));
    }

    #[tokio::test]
    async fn runtime_with_empty_inbox_returns_none() {
        let messenger = test_messenger();
        let runtime = AgentRuntime::new("a:1", messenger.clone());
        assert_eq!(runtime.process_one().await.unwrap(), None);
    }

    #[tokio::test]
    async fn can_execute_reflects_executor() {
        let messenger = test_messenger();
        let mut runtime = AgentRuntime::new("a:1", messenger.clone());
        assert!(!runtime.can_execute());
        runtime.with_executor(|_t, _i| async move { Ok(serde_json::Value::Null) });
        assert!(runtime.can_execute());
    }

    // ---- InferenceAgentExecutor: input → request (pure, no network) ----

    #[test]
    fn infer_request_from_string_prompt_uses_default_model() {
        let task = AgentTask::new("t");
        let req = infer_request_from(&task, &serde_json::json!("hello"), "m-default").unwrap();
        assert_eq!(req.prompt, "hello");
        assert_eq!(req.model_hash, "m-default");
        assert_eq!(req.max_tokens, 1024);
    }

    #[test]
    fn infer_request_from_object_honors_model_and_budget() {
        let mut task = AgentTask::new("t");
        task.budget_max_tokens = 2048;
        let req = infer_request_from(
            &task,
            &serde_json::json!({ "prompt": "summarize", "model_hash": "m-obj" }),
            "m-default",
        )
        .unwrap();
        assert_eq!(req.prompt, "summarize");
        assert_eq!(req.model_hash, "m-obj");
        assert_eq!(req.max_tokens, 2048);
    }

    #[test]
    fn infer_request_task_workload_wins_over_input() {
        let mut wl = decentraai_compute::WorkloadRequirements::new("m-task".into(), 256, 0);
        wl.max_tokens = 512;
        let mut task = AgentTask::new("t");
        task.required_workload = Some(decentraai_agents::AgentWorkloadRequirement::from(wl));
        let req = infer_request_from(
            &task,
            &serde_json::json!({ "prompt": "x", "model_hash": "m-input" }),
            "m-default",
        )
        .unwrap();
        assert_eq!(req.model_hash, "m-task");
        assert_eq!(req.max_tokens, 512);
    }

    #[test]
    fn infer_request_rejects_input_without_prompt() {
        let task = AgentTask::new("t");
        assert!(infer_request_from(&task, &serde_json::json!({"nope": 1}), "m").is_err());
        assert!(infer_request_from(&task, &serde_json::json!([1, 2]), "m").is_err());
    }
}

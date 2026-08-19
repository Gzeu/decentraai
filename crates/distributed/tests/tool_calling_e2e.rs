//! E2E: real tool calling in the agent executor.
//!
//! The executor is pointed at a live local backend (mock) and given real tool
//! bindings (mock tool server). The mock backend answers the first call with a
//! `[TOOL_CALL]` block (asking for sentiment), the tool server returns a
//! result, and the backend answers the follow-up (whose prompt carries the
//! `[TOOL_RESULT]` block) with the final text. The test asserts the tool was
//! executed once and its result reached the final output.

use decentraai_agents::{AgentTask, AgentWorkloadRequirement};
use decentraai_compute::WorkloadRequirements;
use decentraai_distributed::agent_runtime::InferenceAgentExecutor;
use decentraai_distributed::tool_calling::ToolBinding;
use decentraai_identity::Identity;
use decentraai_p2p::P2PNode;
use httpmock::prelude::*;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn executor_runs_a_real_tool_call_round_trip() {
    // Tool server: /v1/skills/sentiment returns a deterministic result.
    let tool_mock = MockServer::start_async().await;
    let tool_handle = tool_mock
        .mock_async(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/v1/skills/sentiment");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"label":"positive","score":0.99}"#);
        })
        .await;

    // Backend (llama-server stand-in): first call (no [TOOL_RESULT] in prompt)
    // returns a tool-call block; the follow-up (prompt carries the result)
    // returns the final answer. Matching on the prompt content keeps the mock
    // deterministic — exactly like the real executor's two rounds.
    let backend = MockServer::start_async().await;
    let first_round = backend
        .mock_async(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/v1/chat/completions")
                .body_contains("How do people feel about this");
            then.status(200)
                .header("content-type", "application/json")
                .body(
                    r#"{"choices":[{"message":{"content":"[TOOL_CALL]{\"name\":\"sentiment\",\"arguments\":{\"text\":\"wow\"}}[/TOOL_CALL]"},"finish_reason":"stop"}]}"#,
                );
        })
        .await;
    let final_round = backend
        .mock_async(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/v1/chat/completions")
                .body_contains("[TOOL_RESULT");
            then.status(200)
                .header("content-type", "application/json")
                .body(
                    r#"{"choices":[{"message":{"content":"The sentiment is positive."},"finish_reason":"stop"}]}"#,
                );
        })
        .await;

    let node = P2PNode::new(
        &Identity::generate(),
        decentraai_p2p::DEFAULT_MAX_MESSAGE_BYTES,
        decentraai_p2p::DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
    )
    .unwrap();
    let distributed = Arc::new(
        decentraai_distributed::DistributedInference::new(
            node,
            decentraai_distributed::InferenceConfig::default(),
            None,
            None,
        )
        .unwrap(),
    );
    let mut executor = InferenceAgentExecutor::new(distributed, "m-default".into());
    let live_url = Arc::new(std::sync::Mutex::new(Some(backend.base_url())));
    executor.with_live_backend(live_url);
    executor.with_tools(vec![ToolBinding::new(
        "sentiment",
        "classifies text sentiment (input: text)",
        format!("{}/v1/skills/sentiment", tool_mock.base_url()),
    )]);

    let mut task = AgentTask::new("t1");
    task.required_workload = Some(AgentWorkloadRequirement::from(
        WorkloadRequirements::new("m-default".into(), 256, 0),
    ));
    let out = executor
        .execute(&task, &serde_json::json!({"prompt": "How do people feel about this?"}))
        .await
        .unwrap();

    assert_eq!(out["text"], "The sentiment is positive.");
    let calls = out["tool_calls"].as_array().unwrap();
    assert_eq!(calls.len(), 1, "one tool call must have been executed");
    assert_eq!(calls[0]["tool"], "sentiment");
    assert_eq!(calls[0]["arguments"]["text"], "wow");
    assert_eq!(tool_handle.hits(), 1, "tool server hit exactly once");
    assert_eq!(
        first_round.hits() + final_round.hits(),
        2,
        "backend hit twice: tool-call round + final round"
    );
}
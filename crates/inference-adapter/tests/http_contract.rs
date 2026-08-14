use decentraai_inference_adapter::{
    BackendConfig, BackendError, BackendRequest, InferenceBackend, OpenAiCompatibleBackend,
};
use futures_util::StreamExt;
use httpmock::prelude::*;
use std::time::Duration;

fn request() -> BackendRequest {
    BackendRequest {
        request_id: "req-1".into(),
        prompt: "hello".into(),
        max_tokens: 8,
        temperature: 0.7,
        top_p: 0.9,
    }
}

#[tokio::test]
async fn health_uses_backend_endpoint() {
    let server = MockServer::start_async().await;
    let health = server
        .mock_async(|when, then| {
            when.method(GET).path("/health");
            then.status(200);
        })
        .await;
    let backend = OpenAiCompatibleBackend::new(BackendConfig {
        base_url: server.base_url(),
        ..Default::default()
    })
    .unwrap();
    backend.health().await.unwrap();
    health.assert_async().await;
}

#[tokio::test]
async fn complete_maps_openai_response() {
    let server = MockServer::start_async().await;
    let completion = server.mock_async(|when, then| { when.method(POST).path("/v1/chat/completions"); then.status(200).json_body(serde_json::json!({"choices": [{"message": {"content": "hello back"}, "finish_reason": "stop"}], "usage": {"completion_tokens": 2}})); }).await;
    let backend = OpenAiCompatibleBackend::new(BackendConfig {
        base_url: server.base_url(),
        model: "test-model".into(),
        ..Default::default()
    })
    .unwrap();
    let response = backend.complete(request()).await.unwrap();
    assert_eq!(response.output, "hello back");
    assert_eq!(response.tokens_used, Some(2));
    completion.assert_async().await;
}

#[tokio::test]
async fn maps_backend_http_error() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/health");
            then.status(503).body("backend unavailable");
        })
        .await;
    let backend = OpenAiCompatibleBackend::new(BackendConfig {
        base_url: server.base_url(),
        ..Default::default()
    })
    .unwrap();
    assert!(matches!(
        backend.health().await,
        Err(BackendError::Http { status: 503, .. })
    ));
}

#[tokio::test]
async fn sends_bearer_key_without_logging_it() {
    let server = MockServer::start_async().await;
    let completion = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/chat/completions")
                .header("authorization", "Bearer test-only-key");
            then.status(200)
                .json_body(serde_json::json!({"choices": [{"message": {"content": "ok"}}]}));
        })
        .await;
    let backend = OpenAiCompatibleBackend::new(BackendConfig {
        base_url: server.base_url(),
        api_key: Some("test-only-key".into()),
        ..Default::default()
    })
    .unwrap();
    backend.complete(request()).await.unwrap();
    completion.assert_async().await;
}

#[tokio::test]
async fn request_timeout_is_configurable() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(GET).path("/health");
            then.status(200).delay(Duration::from_millis(50));
        })
        .await;
    let backend = OpenAiCompatibleBackend::new(BackendConfig {
        base_url: server.base_url(),
        request_timeout: Duration::from_millis(1),
        ..Default::default()
    })
    .unwrap();
    assert!(matches!(
        backend.health().await,
        Err(BackendError::Timeout | BackendError::Transport(_))
    ));
}

#[tokio::test]
async fn stream_preserves_order_and_terminal_event() {
    let server = MockServer::start_async().await;
    let stream_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\" world\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let completion = server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/chat/completions");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(stream_body);
        })
        .await;
    let backend = OpenAiCompatibleBackend::new(BackendConfig {
        base_url: server.base_url(),
        ..Default::default()
    })
    .unwrap();
    let mut stream = backend.stream(request()).await.unwrap();
    let first = stream.next().await.unwrap().unwrap();
    let second = stream.next().await.unwrap().unwrap();
    let terminal = stream.next().await.unwrap().unwrap();
    assert_eq!(
        (
            first.request_id.as_str(),
            first.sequence,
            first.text.as_str()
        ),
        ("req-1", 0, "hello")
    );
    assert_eq!(
        (
            second.request_id.as_str(),
            second.sequence,
            second.text.as_str()
        ),
        ("req-1", 1, " world")
    );
    assert_eq!(
        (
            terminal.sequence,
            terminal.text.as_str(),
            terminal.finish_reason.as_deref()
        ),
        (2, "", Some("stop"))
    );
    assert!(stream.next().await.is_none());
    completion.assert_async().await;
}

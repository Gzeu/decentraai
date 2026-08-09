//! OpenAI-compatible API endpoint (M4c): a thin proxy in front of the
//! managed llama-server. It adds local Bearer auth, tracks request
//! activity for the idle-unload lifecycle, and stays deliberately dumb:
//! all inference logic lives in llama.cpp.

use anyhow::{Context, Result};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use rand_core::RngCore;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::ServeManager;

/// Shared proxy state.
#[derive(Clone)]
pub struct ApiState {
    /// Base URL of the managed llama-server (e.g. http://127.0.0.1:41501).
    backend_url: String,
    /// Optional Bearer token; checked on every request when set.
    auth_token: Option<Arc<str>>,
    /// Lifecycle handle; activity is recorded per request.
    manager: Arc<Mutex<ServeManager>>,
    client: reqwest::Client,
}

impl ApiState {
    pub fn new(
        backend_url: String,
        auth_token: Option<String>,
        manager: Arc<Mutex<ServeManager>>,
    ) -> Self {
        Self {
            backend_url,
            auth_token: auth_token.map(Into::into),
            manager,
            client: reqwest::Client::new(),
        }
    }
}

/// Builds the proxy router exposing the OpenAI-compatible surface.
pub fn build_router(state: ApiState) -> Router {
    Router::new()
        .route("/v1/models", get(proxy_handler))
        .route("/v1/completions", post(proxy_handler))
        .route("/v1/chat/completions", post(proxy_handler))
        .with_state(state)
}

/// Binds the API on an ephemeral port of `host` and serves it in the
/// background. Returns the actual bound address.
pub async fn serve_api(state: ApiState, host: &str) -> Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind((host, 0))
        .await
        .with_context(|| format!("binding API on {host}"))?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, build_router(state)).await {
            tracing::warn!(error = %e, "api server stopped unexpectedly");
        }
    });
    Ok(addr)
}

async fn proxy_handler(
    State(state): State<ApiState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(token) = &state.auth_token {
        let expected = format!("Bearer {token}");
        let authorized = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value == expected);
        if !authorized {
            return (
                StatusCode::UNAUTHORIZED,
                "{\"error\":{\"message\":\"missing or invalid API token\",\"type\":\"authentication_error\"}}",
            )
                .into_response();
        }
    }

    state.manager.lock().await.note_activity();

    let url = format!("{}{}", state.backend_url, uri.path());
    let mut request = state.client.request(method, &url);
    if let Some(content_type) = headers.get(header::CONTENT_TYPE) {
        request = request.header(header::CONTENT_TYPE, content_type);
    }
    match request.body(body.to_vec()).send().await {
        Ok(upstream) => {
            let status = StatusCode::from_u16(upstream.status().as_u16())
                .unwrap_or(StatusCode::BAD_GATEWAY);
            let content_type = upstream.headers().get(header::CONTENT_TYPE).cloned();
            let bytes = upstream.bytes().await.unwrap_or_default();
            let mut response = (status, bytes).into_response();
            if let Some(value) = content_type {
                response.headers_mut().insert(header::CONTENT_TYPE, value);
            }
            response
        }
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "{\"error\":{\"message\":\"model backend unavailable (unloaded or crashed); restart decentraai serve\",\"type\":\"server_error\"}}",
        )
            .into_response(),
    }
}

/// Loads the local API token or generates a fresh one with 0600
/// permissions. The token never leaves the machine: it only guards the
/// loopback endpoint from other local processes.
pub fn ensure_api_token(path: &Path) -> Result<String> {
    if path.exists() {
        let token = std::fs::read_to_string(path)
            .with_context(|| format!("reading API token from {}", path.display()))?;
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    let mut bytes = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &token)
        .with_context(|| format!("writing API token to {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LlamaServer, RuntimeConfig};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[cfg(unix)]
    fn write_fake_server(dir: &Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-llama-server");
        std::fs::write(&path, "#!/bin/sh\nexec sleep 60\n").unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[cfg(unix)]
    async fn test_manager(dir: &Path) -> Arc<Mutex<ServeManager>> {
        let binary = write_fake_server(dir);
        let config = RuntimeConfig::new(dir.join("model.gguf"));
        let server = LlamaServer::start(&binary, &config).unwrap();
        Arc::new(Mutex::new(ServeManager::new(server, Duration::from_secs(3600))))
    }

    async fn start_backend() -> (SocketAddr, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let hits = counter.clone();
        let app = Router::new()
            .route(
                "/v1/models",
                get(move || {
                    let hits = hits.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        "{\"object\":\"list\",\"data\":[]}"
                    }
                }),
            )
            .route(
                "/v1/chat/completions",
                post(|body: Bytes| async move {
                    format!("{{\"echo\":{}}}", body.len())
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (addr, counter)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_forwards_models_to_backend() {
        let dir = tempfile::tempdir().unwrap();
        let (backend, hits) = start_backend().await;
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(format!("http://{backend}"), None, manager.clone());
        let api = serve_api(state, "127.0.0.1").await.unwrap();

        let response = reqwest::get(format!("http://{api}/v1/models"))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert!(response.text().await.unwrap().contains("\"list\""));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_enforces_bearer_token() {
        let dir = tempfile::tempdir().unwrap();
        let (backend, hits) = start_backend().await;
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(
            format!("http://{backend}"),
            Some("secret".to_string()),
            manager.clone(),
        );
        let api = serve_api(state, "127.0.0.1").await.unwrap();
        let client = reqwest::Client::new();

        let denied = client
            .get(format!("http://{api}/v1/models"))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 401);

        let allowed = client
            .get(format!("http://{api}/v1/models"))
            .header("Authorization", "Bearer secret")
            .send()
            .await
            .unwrap();
        assert_eq!(allowed.status(), 200);
        assert_eq!(hits.load(Ordering::SeqCst), 1, "backend must only see authorized requests");
        manager.lock().await.shutdown().await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_forwards_post_bodies() {
        let dir = tempfile::tempdir().unwrap();
        let (backend, _) = start_backend().await;
        let manager = test_manager(dir.path()).await;
        let state = ApiState::new(format!("http://{backend}"), None, manager.clone());
        let api = serve_api(state, "127.0.0.1").await.unwrap();

        let payload = "{\"model\":\"test\",\"messages\":[]}";
        let response = reqwest::Client::new()
            .post(format!("http://{api}/v1/chat/completions"))
            .header("Content-Type", "application/json")
            .body(payload)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let echo = response.text().await.unwrap();
        assert!(echo.contains(&payload.len().to_string()));
        manager.lock().await.shutdown().await.unwrap();
    }

    #[test]
    fn token_is_generated_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runtime/api.token");
        let first = ensure_api_token(&path).unwrap();
        let second = ensure_api_token(&path).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[cfg(unix)]
    #[test]
    fn token_file_has_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api.token");
        ensure_api_token(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

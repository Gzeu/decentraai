//! Server-side chat history endpoints (USER DATA, not logging).
//!
//! Re-exported via `pub(crate) use chat::*` in mod.rs.
//!
//! Conversations are scoped per authenticated caller (see
//! [`crate::chat_history::owner_key`]); unauthenticated (`Open`) callers get
//! 401 on every endpoint — without a stable identity there is nothing to
//! scope history to. Unknown and foreign-owned ids share one 404 so ids are
//! not an existence oracle across owners. Error bodies are static strings;
//! message content never appears in responses beyond the owned conversation
//! itself, and never in logs (see `chat_history.rs` invariant docs).

use super::*;
use axum::extract::Path;

/// Resolves the caller's chat-history owner or returns the 401 response.
/// Every endpoint funnels through here so auth scoping cannot drift.
/// Boxed: `Response` is large for a `Result` Err variant (clippy).
fn chat_owner(state: &ApiState, headers: &HeaderMap) -> Result<String, Box<Response>> {
    let auth = state
        .classify(headers)
        .map_err(|e| Box::new(e.into_response()))?;
    crate::chat_history::owner_key(&auth).ok_or_else(|| {
        Box::new(
            (
                StatusCode::UNAUTHORIZED,
                [(header::CONTENT_TYPE, "application/json")],
                "{\"error\":{\"message\":\"chat history requires authentication\",\"type\":\"authentication_error\"}}".to_string(),
            )
                .into_response(),
        )
    })
}

fn chat_store(state: &ApiState) -> Result<crate::chat_history::ChatStore, Box<Response>> {
    match &state.chat_history_path {
        Some(path) => {
            crate::chat_history::ChatStore::load(path).map_err(|_| {
                Box::new(
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        [(header::CONTENT_TYPE, "application/json")],
                        "{\"error\":{\"message\":\"chat history store unavailable\",\"type\":\"server_error\"}}"
                            .to_string(),
                    )
                        .into_response(),
                )
            })
        }
        None => Err(Box::new(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::CONTENT_TYPE, "application/json")],
                "{\"error\":{\"message\":\"chat history is not enabled on this node\",\"type\":\"server_error\"}}"
                    .to_string(),
            )
                .into_response(),
        )),
    }
}

fn json_response(body: serde_json::Value) -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// `GET /v1/conversations` — this caller's conversations, newest first.
/// Metadata only (id, title, timestamps, message count).
pub(crate) async fn list_conversations_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    let owner = match chat_owner(&state, &headers) {
        Ok(owner) => owner,
        Err(response) => return *response,
    };
    let store = match chat_store(&state) {
        Ok(store) => store,
        Err(response) => return *response,
    };
    json_response(serde_json::json!({ "conversations": store.list(&owner) }))
}

/// `GET /v1/conversations/:id` — one full conversation owned by the caller.
/// Unknown and foreign ids share one 404.
pub(crate) async fn get_conversation_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let owner = match chat_owner(&state, &headers) {
        Ok(owner) => owner,
        Err(response) => return *response,
    };
    let store = match chat_store(&state) {
        Ok(store) => store,
        Err(response) => return *response,
    };
    match store.get(&owner, &id) {
        Some(conversation) => json_response(serde_json::json!({ "conversation": conversation })),
        None => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            "{\"error\":{\"message\":\"conversation not found\",\"type\":\"invalid_request_error\"}}".to_string(),
        )
            .into_response(),
    }
}

/// `DELETE /v1/conversations/:id` — deletes one owned conversation.
/// Unknown and foreign ids report `deleted: false` (no oracle).
pub(crate) async fn delete_conversation_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let owner = match chat_owner(&state, &headers) {
        Ok(owner) => owner,
        Err(response) => return *response,
    };
    let mut store = match chat_store(&state) {
        Ok(store) => store,
        Err(response) => return *response,
    };
    match store.delete(&owner, &id) {
        Ok(deleted) => json_response(serde_json::json!({ "deleted": deleted })),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            "{\"error\":{\"message\":\"chat history store unavailable\",\"type\":\"server_error\"}}".to_string(),
        )
            .into_response(),
    }
}

/// `DELETE /v1/conversations` — deletes ALL of the caller's conversations.
/// Reports the count removed.
pub(crate) async fn delete_all_conversations_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    let owner = match chat_owner(&state, &headers) {
        Ok(owner) => owner,
        Err(response) => return *response,
    };
    let mut store = match chat_store(&state) {
        Ok(store) => store,
        Err(response) => return *response,
    };
    match store.delete_all(&owner) {
        Ok(count) => json_response(serde_json::json!({ "deleted": count })),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            "{\"error\":{\"message\":\"chat history store unavailable\",\"type\":\"server_error\"}}".to_string(),
        )
            .into_response(),
    }
}

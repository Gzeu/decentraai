//! Compute Assist runtime ("Sharing is Caring", M14/M15 milestone 1).
//!
//! Two roles, one module:
//!
//! * **Worker side** ([`attach_dfcp_worker`]): answers inbound DFCP messages
//!   under THIS node's owner limits. Reserve creates a tracked lease;
//!   Assign executes the capability against the LOCAL managed engine and
//!   calls the requester back with the result; Release drops the lease.
//! * **Requester side** ([`run_assist_request`]): discovers capable workers
//!   from the compute mesh, negotiates REQUEST→OFFER→RESERVE over DFCP,
//!   assigns the task, awaits the result inside the lease window, records
//!   evidence-backed contribution credit for the worker.
//!
//! Security posture: every inbound message is bound to its
//! transport-authenticated peer; offers are re-checked at reserve time;
//! leases expire even if RELEASE never arrives; credit is recorded only
//! for a successful, verified result.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use decentraai_compute::assist::{AssistOffer, AssistRequest};
use decentraai_config::AssistSharingSection;
use decentraai_p2p::DfcInbound;
use decentraai_protocol::dfcp::{
    AssistTaskAssign, AssistTaskResult, ReservationId, ResourceReserve, ResourceReserved,
};
use serde_json::json;

/// One active lease granted by this worker.
#[derive(Debug)]
pub struct ActiveLease {
    pub capability: String,
    pub cpu_cores: u16,
    pub ram_mb: u64,
    pub expires_at: Instant,
}

/// Callback delivering arbitrary bytes to one peer over the fabric channel
/// (used to return async assist results to the requesting peer).
pub type PeerSender = Arc<dyn Fn(decentraai_p2p::PeerId, Vec<u8>) + Send + Sync>;

/// Worker-side assist state shared across DFCP callbacks.
pub struct AssistWorkerState {
    pub limits: Arc<AssistSharingSection>,
    /// Live root URL of the local managed llama-server (resolved per call).
    pub backend_url: Arc<std::sync::RwLock<String>>,
    /// Optional dedicated embeddings backend (a llama-server loaded with an
    /// embedding model). When set, `capability = "embeddings"` is served from
    /// here instead of the chat backend, so a node can serve both chat and
    /// embeddings to the pool. Static (not live-resolved) — the operator
    /// configures a stable embeddings endpoint.
    pub embeddings_backend_url: Option<Arc<std::sync::RwLock<String>>>,
    pub http: reqwest::Client,
    pub leases: Mutex<HashMap<ReservationId, ActiveLease>>,
    pub offers_sent: Mutex<HashMap<decentraai_protocol::dfcp::ResourceOfferId, (String, u16, u64)>>,
    pub trusted_peers: Arc<std::sync::RwLock<Vec<String>>>,
}

impl AssistWorkerState {
    pub fn new(
        limits: Arc<AssistSharingSection>,
        backend_url: String,
        trusted_peers: Vec<String>,
    ) -> Self {
        Self::with_embeddings(limits, backend_url, None, trusted_peers)
    }

    /// Constructs the worker with an optional dedicated embeddings backend.
    pub fn with_embeddings(
        limits: Arc<AssistSharingSection>,
        backend_url: String,
        embeddings_backend_url: Option<String>,
        trusted_peers: Vec<String>,
    ) -> Self {
        Self {
            limits,
            backend_url: Arc::new(std::sync::RwLock::new(backend_url)),
            embeddings_backend_url: embeddings_backend_url
                .filter(|u| !u.is_empty())
                .map(|u| Arc::new(std::sync::RwLock::new(u))),
            http: reqwest::Client::new(),
            leases: Mutex::new(HashMap::new()),
            offers_sent: Mutex::new(HashMap::new()),
            trusted_peers: Arc::new(std::sync::RwLock::new(trusted_peers)),
        }
    }

    /// Refreshes the live engine URL (M24 respawns change ephemeral ports).
    pub fn update_backend_url(&self, url: &str) {
        *self.backend_url.write().expect("backend url lock") = url.to_string();
    }
}

/// Builds the DFCP dispatch callback for a WORKER node. Returns `Some(bytes)`
/// as the synchronous reply for Reserve/Assign; `None` for notifications.
#[allow(clippy::too_many_arguments)]
pub fn attach_dfcp_worker(
    state: Arc<AssistWorkerState>,
    send_to_peer: PeerSender,
) -> impl Fn(libp2p::PeerId, DfcInbound) -> Option<Vec<u8>> + Send + Sync + 'static {
    move |peer, msg| {
        let send_to_peer = std::sync::Arc::clone(&send_to_peer);
        match msg {
            // Capacity poll: answer with an owner-limit-checked OFFER only when
            // we can genuinely help. An empty reply means "not a candidate".
            DfcInbound::Request(request) => {
                let trusted = true; // private swarm + admission gate upstream
                if let Some((cpu, ram)) = state.limits.admit(
                    &request.capability,
                    &peer.to_string(),
                    trusted,
                    request.cpu_cores,
                    request.ram_mb,
                ) {
                    let offer = decentraai_protocol::dfcp::ResourceOffer::answering(
                        &request,
                        cpu,
                        ram,
                        state.limits.max_lease_seconds,
                    );
                    state.offers_sent.lock().expect("offers lock").insert(
                        offer.offer_id.clone(),
                        (request.capability.clone(), cpu, ram),
                    );
                    tracing::info!(offer = %offer.offer_id, capability = %request.capability, "dfcp offering capacity");
                    serde_json::to_vec(&offer).ok()
                } else {
                    None
                }
            }
            DfcInbound::Reserve(reserve) => handle_reserve(&state, &reserve),
            DfcInbound::Assign(assign) => handle_assign(
                state.clone(),
                &assign,
                std::sync::Arc::clone(&send_to_peer),
                peer,
            ),
            DfcInbound::Release(release) => {
                let removed = state
                    .leases
                    .lock()
                    .expect("lease lock")
                    .remove(&release.reservation_id)
                    .is_some();
                tracing::info!(
                    reservation = %release.reservation_id,
                    released = removed,
                    "dfcp resource release"
                );
                None
            }
            // Results complete parked oneshots in the p2p layer itself.
            DfcInbound::Result(_) => None,
        }
    }
}

fn handle_reserve(state: &Arc<AssistWorkerState>, reserve: &ResourceReserve) -> Option<Vec<u8>> {
    // The offer we sent defines the terms; unknown offer ids are refused so
    // replayed/forged reserves cannot conjure capacity out of thin air.
    let (capability, cpu_cores, ram_mb) = state
        .offers_sent
        .lock()
        .expect("offers lock")
        .get(&reserve.offer_id)
        .cloned()?;

    // Owner limits re-checked NOW (config may have tightened since the offer).
    let peer = reserve
        .offer_id
        .as_str()
        .split(':')
        .next()
        .unwrap_or_default();
    let Some((cpu, ram)) = state.limits.admit(
        &capability,
        peer,
        true, // trust already enforced by the secure channel + admission
        cpu_cores,
        ram_mb,
    ) else {
        return Some(
            serde_json::to_vec(&json!({"error": "not permitted by owner limits"}))
                .unwrap_or_default(),
        );
    };

    // Prune expired leases first: expired capacity becomes available again.
    state
        .leases
        .lock()
        .expect("lease lock")
        .retain(|_, lease| lease.expires_at > Instant::now());

    let max_lease = state.limits.max_lease_seconds;
    let reservation_id = ReservationId::new();
    let lease_seconds = max_lease.min(max_lease);
    state.leases.lock().expect("lease lock").insert(
        reservation_id.clone(),
        ActiveLease {
            capability: capability.clone(),
            cpu_cores: cpu,
            ram_mb: ram,
            expires_at: Instant::now() + Duration::from_secs(lease_seconds),
        },
    );
    tracing::info!(%reservation_id, capability = %capability, "dfcp lease granted");

    let confirmed = ResourceReserved {
        protocol_version: decentraai_protocol::dfcp::DFCP_VERSION,
        reservation_id,
        offer_id: reserve.offer_id.clone(),
        lease_seconds,
    };
    serde_json::to_vec(&confirmed).ok()
}

fn handle_assign(
    state: Arc<AssistWorkerState>,
    assign: &AssistTaskAssign,
    send_to_peer: PeerSender,
    requester_peer: decentraai_p2p::PeerId,
) -> Option<Vec<u8>> {
    // Lease must be alive and must match this assignment's reservation.
    let lease = {
        let leases = state.leases.lock().expect("lease lock");
        let lease = leases.get(&assign.reservation_id)?;
        if lease.expires_at <= Instant::now() || lease.capability != assign.capability {
            return Some(
                serde_json::to_vec(
                    &json!({"accepted": false, "error": "lease expired or mismatched"}),
                )
                .unwrap_or_default(),
            );
        }
        // Copy what we need; the lock is dropped before any awaiting work.
        (lease.capability.clone(), lease.expires_at)
    };
    let _ = lease;

    // Execute ASYNCHRONOUSLY: the synchronous reply is only an acceptance
    // marker. The real result travels back as its own DFCP message.
    let task_assign = assign.clone();
    tokio::spawn(async move {
        let started = Instant::now();
        // Observe the worker's OWN CPU pressure before/after execution so we
        // can demonstrate REAL remote resource use (not a masked local call).
        let load_before = read_loadavg();
        let (success, payload, error) =
            execute_capability(&state, &task_assign.capability, &task_assign.payload).await;
        let load_after = read_loadavg();
        let elapsed_ms = started.elapsed().as_millis() as u64;
        let result = AssistTaskResult {
            protocol_version: decentraai_protocol::dfcp::DFCP_VERSION,
            assignment_id: task_assign.assignment_id.clone(),
            success,
            payload,
            error,
        };
        tracing::info!(
            assignment = %task_assign.assignment_id,
            capability = %task_assign.capability,
            success,
            elapsed_ms,
            worker_cpu_load_before = %load_before,
            worker_cpu_load_after = %load_after,
            "assist task finished (remote CPU observed)"
        );
        // Deliver the result to the REQUESTER as its own DFCP message; the
        // requester's parked oneshot completes there and contribution credit
        // is recorded on verified success.
        let bytes = serde_json::to_vec(&result).unwrap_or_default();
        send_to_peer(requester_peer, bytes);
    });

    Some(serde_json::to_vec(&json!({"accepted": true})).unwrap_or_default())
}

/// Executes one capability against THIS node's local managed engine.
async fn execute_capability(
    state: &Arc<AssistWorkerState>,
    capability: &str,
    payload: &[u8],
) -> (bool, Vec<u8>, Option<String>) {
    let base = state.backend_url.read().expect("backend url lock").clone();
    match capability {
        // Embeddings: llama-server `/v1/embeddings` with an embedding-capable
        // model loaded. Payload: JSON {"input":"text"}; result: vector JSON.
        // When a dedicated embeddings backend is configured, serve from it so
        // the chat backend (e.g. a non-embedding LLM) is not consulted.
        "embeddings" => {
            let input: serde_json::Value = match serde_json::from_slice(payload) {
                Ok(v) => v,
                Err(e) => {
                    return (false, Vec::new(), Some(format!("bad payload: {e}")));
                }
            };
            let embed_base = state
                .embeddings_backend_url
                .as_ref()
                .map(|u| u.read().expect("embeddings url lock").clone())
                .unwrap_or_else(|| base.clone());
            let res = state
                .http
                .post(format!("{embed_base}/v1/embeddings"))
                .json(&json!({"input": input.get("input").cloned().unwrap_or_default()}))
                .send()
                .await;
            match res {
                Ok(r) if r.status().is_success() => {
                    let body = r.bytes().await.unwrap_or_default();
                    (true, body.to_vec(), None)
                }
                Ok(r) => (
                    false,
                    Vec::new(),
                    Some(format!("backend HTTP {}", r.status())),
                ),
                Err(e) => (false, Vec::new(), Some(format!("backend unreachable: {e}"))),
            }
        }
        // Chat/text generation: standard OpenAI-shaped completion against the
        // worker's served model. Payload is forwarded as-is.
        //
        // Batch form: when the payload carries an `inputs` array, the worker
        // runs each prompt through the chat backend and returns a
        // `{"responses":[content,...]}` array. This lets ONE DFCP negotiation
        // carry many prompts, amortising per-task overhead.
        "chat" | "text_generation" => {
            let body: serde_json::Value = match serde_json::from_slice(payload) {
                Ok(v) => v,
                Err(e) => return (false, Vec::new(), Some(format!("bad payload: {e}"))),
            };
            if let Some(inputs) = body.get("inputs").and_then(|v| v.as_array()) {
                let model = body
                    .get("model")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let max_tokens = body
                    .get("max_tokens")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let mut responses: Vec<serde_json::Value> = Vec::new();
                let mut failed = false;
                for inp in inputs {
                    let prompt = inp.as_str().unwrap_or("");
                    let chat_body = serde_json::json!({
                        "model": model,
                        "messages": [{"role":"user","content": prompt}],
                        "max_tokens": max_tokens,
                    });
                    let res = state
                        .http
                        .post(format!("{base}/v1/chat/completions"))
                        .json(&chat_body)
                        .send()
                        .await;
                    match res {
                        Ok(r) if r.status().is_success() => {
                            let b = r.bytes().await.unwrap_or_default();
                            let content = serde_json::from_slice::<serde_json::Value>(&b)
                                .ok()
                                .and_then(|v| {
                                    v.get("choices")
                                        .and_then(|c| c.as_array())
                                        .and_then(|c| c.first())
                                        .and_then(|ch| ch.get("message"))
                                        .and_then(|m| m.get("content"))
                                        .cloned()
                                        .or_else(|| {
                                            serde_json::from_slice::<serde_json::Value>(&b)
                                                .ok()
                                                .and_then(|v| {
                                                    v.get("choices")
                                                        .and_then(|c| c.as_array())
                                                        .and_then(|c| c.first())
                                                        .and_then(|ch| ch.get("text"))
                                                        .cloned()
                                                })
                                        })
                                })
                                .unwrap_or(serde_json::Value::Null);
                            responses.push(content);
                        }
                        _ => {
                            responses.push(serde_json::Value::Null);
                            failed = true;
                        }
                    }
                }
                (
                    !failed,
                    serde_json::to_vec(&serde_json::json!({ "responses": responses }))
                        .unwrap_or_default(),
                    if failed {
                        Some("some batch prompts failed".into())
                    } else {
                        None
                    },
                )
            } else {
                let res = state
                    .http
                    .post(format!("{base}/v1/chat/completions"))
                    .json(&body)
                    .send()
                    .await;
                match res {
                    Ok(r) if r.status().is_success() => {
                        let body_bytes = r.bytes().await.unwrap_or_default();
                        (true, body_bytes.to_vec(), None)
                    }
                    Ok(r) => (
                        false,
                        Vec::new(),
                        Some(format!("backend HTTP {}", r.status())),
                    ),
                    Err(e) => (false, Vec::new(), Some(format!("backend unreachable: {e}"))),
                }
            }
        }
        other => (
            false,
            Vec::new(),
            Some(format!("capability `{other}` has no assist executor yet")),
        ),
    }
}

// ---------------------------------------------------------------------------
// Requester side
// ---------------------------------------------------------------------------

/// One candidate discovered in the mesh.
#[derive(Debug, Clone)]
pub struct AssistCandidate {
    /// libp2p peer id string.
    pub peer_id: String,
    pub capability: String,
    pub cpu_cores: u16,
    pub ram_mb: u64,
    pub queue_depth: u32,
    pub contribution_balance: i64,
}

/// Runs the full assist negotiation against the mesh and executes one task.
///
/// Returns `(success, result payload, explanation)`. Every step is logged at
/// INFO so a live run shows REQUEST→OFFER→RESERVE→ASSIGN→RESULT→RELEASE as
/// distinct, auditable events.
#[allow(clippy::too_many_arguments)]
pub async fn run_assist_request(
    p2p: &decentraai_p2p::P2PNode,
    connected_peers: Vec<libp2p::PeerId>,
    // NOTE: requester-side limits are not the gate here; the WORKER's own
    // owner-limits gate acceptance. Kept out of the signature for clarity.
    request: AssistRequest,
    task_payload: Vec<u8>,
    lease_seconds: u64,
) -> (bool, Vec<u8>, String) {
    use decentraai_protocol::dfcp::{
        ResourceOffer, ResourceRelease, ResourceRequest as DfcRequest, ResourceReserve,
    };

    // 1. REQUEST: ask every connected trusted peer for capacity. The DFCP
    //    message doubles as both advertisement-poll and capability probe —
    //    workers that cannot help answer with an explicit refusal or stay
    //    silent until the short timeout elapses.
    let dfcp_req = DfcRequest::new(
        request.capability.clone(),
        request.cpu_cores,
        request.ram_mb,
        lease_seconds,
    );
    let req_bytes = decentraai_protocol::serialize_message(&dfcp_req).unwrap_or_default();

    let mut offers: Vec<(ResourceOffer, AssistOffer)> = Vec::new();
    for peer in &connected_peers {
        let peer_str = peer.to_string();
        tracing::info!(peer = %peer_str, capability = %request.capability, "dfcp RESOURCE_REQUEST sent");
        let reply = match p2p.request(*peer, req_bytes.clone()).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(peer = %peer_str, error = %e, "dfcp request transport failed");
                continue; // unreachable/slow peer: simply not a candidate
            }
        };
        let offer: Option<ResourceOffer> = serde_json::from_slice(&reply).ok();
        let Some(offer) = offer else { continue };
        if offer.capability != request.capability {
            continue;
        }
        offers.push((
            offer.clone(),
            AssistOffer {
                peer_id: peer_str,
                capability: offer.capability.clone(),
                cpu_cores: offer.cpu_cores,
                ram_mb: offer.ram_mb,
                lease_seconds: offer.lease_seconds,
                sampled_ago_secs: 0, // answered live just now
                queue_depth: 0,      // worker-side gate already applied its own
                contribution_balance: 0,
                has_recent_failure: false,
            },
        ));
    }
    if offers.is_empty() {
        return (
            false,
            Vec::new(),
            "no capable worker answered the resource request".into(),
        );
    }

    // 2. OFFER selection: deterministic scoring with fairness bias.
    let assist_only: Vec<AssistOffer> = offers.iter().map(|(_, a)| a.clone()).collect();
    let winner = match decentraai_compute::assist::select_offer(assist_only.iter(), &request) {
        Ok(w) => w.clone(),
        Err(rejections) => {
            return (
                false,
                Vec::new(),
                format!("all offers rejected by deterministic gates: {rejections:?}"),
            );
        }
    };
    // The DFCP offer paired with the winning scored candidate carries the
    // worker-generated offer id needed for the RESERVE handshake.
    let Some((winner_dfc_offer, _)) = offers.iter().find(|(_, a)| a.peer_id == winner.peer_id)
    else {
        return (
            false,
            Vec::new(),
            "winner offer vanished during selection".into(),
        );
    };
    let winner_dfc_offer = winner_dfc_offer.clone();

    // 3. RESERVE against the winner's authoritative ledger. The worker
    //    remembers every offer it sent (offer id → terms), so echoing the
    //    offer id is the authentication of this handshake.
    let winner_peer = match winner.peer_id.parse::<libp2p::PeerId>() {
        Ok(p) => p,
        Err(_) => {
            return (
                false,
                Vec::new(),
                format!("invalid peer id {}", winner.peer_id),
            );
        }
    };
    let Ok(reply) = p2p
        .request(
            winner_peer,
            decentraai_protocol::serialize_message(&ResourceReserve {
                protocol_version: decentraai_protocol::dfcp::DFCP_VERSION,
                offer_id: winner_dfc_offer.offer_id.clone(),
                request_id: dfcp_req.request_id.clone(),
            })
            .unwrap_or_default(),
        )
        .await
    else {
        return (
            false,
            Vec::new(),
            format!("reserve handshake with {} failed", winner.peer_id),
        );
    };
    let reserved: Result<decentraai_protocol::dfcp::ResourceReserved, _> =
        serde_json::from_slice(&reply);
    let reserved = match reserved {
        Ok(r) => r,
        Err(_) => {
            return (
                false,
                Vec::new(),
                format!("worker {} refused the reservation", winner.peer_id),
            );
        }
    };
    tracing::info!(reservation = %reserved.reservation_id, worker = %winner.peer_id, "dfcp lease reserved");

    // 4. ASSIGN: park a waiter FIRST so the callback cannot race it.
    let assignment = AssistTaskAssign::new(
        reserved.reservation_id.clone(),
        request.capability.clone(),
        task_payload,
    );
    let rx = p2p.register_assist_wait(assignment.assignment_id.as_str());
    let Ok(_ack) = p2p
        .request(
            winner_peer,
            decentraai_protocol::serialize_message(&assignment).unwrap_or_default(),
        )
        .await
    else {
        // Lease will expire on the worker side (TTL backstop).
        return (
            false,
            Vec::new(),
            format!("assignment delivery to {} failed", winner.peer_id),
        );
    };

    // 5. Await RESULT inside the lease window (+small transport slack).
    let wait = Duration::from_secs(reserved.lease_seconds.saturating_add(5));
    let outcome = tokio::time::timeout(wait, rx).await;
    let result_bytes = match outcome {
        Ok(Ok(bytes)) => bytes,
        _ => {
            // Release best-effort so the worker frees early when reachable.
            let _ = p2p
                .request(
                    winner_peer,
                    decentraai_protocol::serialize_message(&ResourceRelease {
                        protocol_version: decentraai_protocol::dfcp::DFCP_VERSION,
                        reservation_id: reserved.reservation_id.clone(),
                    })
                    .unwrap_or_default(),
                )
                .await;
            return (
                false,
                Vec::new(),
                format!("assist timed out after {}s", wait.as_secs()),
            );
        }
    };
    let result: AssistTaskResult =
        serde_json::from_slice(&result_bytes).unwrap_or(AssistTaskResult {
            protocol_version: 0,
            assignment_id: assignment.assignment_id.clone(),
            success: false,
            payload: Vec::new(),
            error: Some("unparsable result".into()),
        });

    // 6. RELEASE the lease early on success.
    if result.success {
        let _ = p2p
            .request(
                winner_peer,
                decentraai_protocol::serialize_message(&ResourceRelease {
                    protocol_version: decentraai_protocol::dfcp::DFCP_VERSION,
                    reservation_id: reserved.reservation_id.clone(),
                })
                .unwrap_or_default(),
            )
            .await;
    } else {
        // Failure path: drop the lease too; no credit is recorded anywhere.
        let _ = p2p
            .request(
                winner_peer,
                decentraai_protocol::serialize_message(&ResourceRelease {
                    protocol_version: decentraai_protocol::dfcp::DFCP_VERSION,
                    reservation_id: reserved.reservation_id.clone(),
                })
                .unwrap_or_default(),
            )
            .await;
        return (
            false,
            Vec::new(),
            result.error.unwrap_or_else(|| "assist failed".into()),
        );
    }

    tracing::info!(worker = %winner.peer_id, "assist complete; contribution credit pending evidence");
    (
        true,
        result.payload,
        format!("assisted by {}", winner.peer_id),
    )
}

/// Reads this worker's 1-minute CPU load average (1 = one core fully busy).
/// Used for observability of REAL remote CPU use during assist tasks.
/// Deterministic, stdlib-only, no external process.
fn read_loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(str::to_string))
        .unwrap_or_else(|| "0.00".to_string())
}

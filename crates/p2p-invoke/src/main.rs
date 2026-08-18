//! Standalone client that exercises the REAL distributed inference path.
//!
//! This binary dials a running `decentraai distributed --model` worker and
//! sends a prompt through the full stack: P2P `InferRequest` → worker queue →
//! `OpenAiCompatibleBackend::stream()` (backed by a real llama-server) →
//! streamed `InferProgress` frames → final `InferResponse`. It is the
//! coordinator-side user endpoint used to validate the vertical slice and is
//! also the reference for how the runtime API should drive the router.
//!
//! Usage:
//!   decentraai-p2p-invoke --peer <multiaddr> --model-hash <hash> [--prompt <text>]
//!
//! The `--peer` address must carry the worker's `/p2p/<peer-id>` suffix, e.g.
//! `/ip4/127.0.0.1/tcp/43123/p2p/12D3KooW...`. The model hash is the BLAKE3
//! of the GGUF file the worker loaded. Ctrl-C sends an `InferCancel` and the
//! worker aborts generation and replies `InferFailed(cancelled)`.

use anyhow::{Context, Result, bail};
use decentraai_identity::Identity;
use decentraai_p2p::{P2PNode, RequestHandler};
use decentraai_protocol::{InferMessage, InferRequest};
use libp2p::PeerId;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing_subscriber::EnvFilter;

/// Forwards every inbound inference frame (progress / final / error) into a
/// channel so the main task can print streamed tokens and detect the terminal
/// message. Non-inference frames (e.g. WorkerAnnouncement broadcasts from the
/// worker's discovery loop) are ignored.
struct IncomingHandler {
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl RequestHandler for IncomingHandler {
    fn handle(&self, request: &[u8]) -> Result<Vec<u8>> {
        if decentraai_protocol::deserialize_message::<InferMessage>(request, request.len()).is_ok()
        {
            let _ = self.tx.send(request.to_vec());
        }
        Ok(Vec::new())
    }
}

/// Parses `<addr>/p2p/<peer-id>` and returns the peer id. The full string is
/// passed to `dial()` later, so we only need to extract the PeerId for the
/// request-response calls.
fn parse_peer_target(addr: &str) -> Result<PeerId> {
    let parts: Vec<&str> = addr.split("/p2p/").collect();
    if parts.len() != 2 {
        bail!("peer address must include a /p2p/<peer-id> suffix, got: {addr}");
    }
    let peer = parts[1];
    let peer_id: PeerId = peer.parse().context("parsing /p2p/ peer id")?;
    Ok(peer_id)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args[1] == "-h" || args[1] == "--help" {
        println!(
            "Usage: decentraai-p2p-invoke --peer <multiaddr-with-p2p-suffix> (--model-hash <hash> | --model <gguf-path>) [--prompt <text>] [--max-tokens <n>] [--timeout-ms <n>]"
        );
        return Ok(());
    }

    let get = |name: &str| -> Option<String> {
        args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone())
    };

    let peer_addr =
        get("--peer").context("missing --peer (worker multiaddr with /p2p/<peer-id> suffix)")?;
    // Either the exact model hash or a path to the GGUF whose BLAKE3 is the
    // model hash (the same computation the worker performs at startup).
    let model_hash = match get("--model-hash") {
        Some(hash) => hash,
        None => {
            let path = get("--model").context("need --model-hash or --model")?;
            let bytes = std::fs::read(&path).with_context(|| format!("reading model {}", path))?;
            tracing::info!(bytes = bytes.len(), "computing BLAKE3 model hash");
            blake3::hash(&bytes).to_hex().to_string()
        }
    };
    let prompt = get("--prompt").unwrap_or_else(|| "Hello".to_string());
    let max_tokens: u32 = get("--max-tokens").unwrap_or_else(|| "96".into()).parse()?;
    let timeout_ms: u64 = get("--timeout-ms")
        .unwrap_or_else(|| "90000".into())
        .parse()?;

    let worker_peer = parse_peer_target(&peer_addr)?;

    // A fresh ephemeral identity for this client so it never touches node
    // key material; the worker only needs the PeerId to stream back to us.
    let identity = Identity::generate();

    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let handler = IncomingHandler { tx };
    let node = P2PNode::new(
        &identity,
        1024 * 1024,
        decentraai_p2p::DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(Arc::new(handler)),
    )?;
    let bound = node.listen("/ip4/0.0.0.0/tcp/0").await?;
    tracing::info!(local = %node.local_peer_id(), %bound, "client listening");
    tracing::info!(%worker_peer, "dialing worker");
    node.dial(&peer_addr).await?;

    // Give the connection time to settle before the first request.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let mut req = InferRequest::new(model_hash, prompt, max_tokens);
    req = req.with_sender(node.local_peer_id());
    req = req.with_streaming(true);
    req.timeout_ms = u32::try_from(timeout_ms).unwrap_or(u32::MAX);
    let request_id = req.request_id;

    // Send the RAW InferRequest (not wrapped in an InferMessage envelope):
    // the worker's swarm task routes bare InferRequest bytes to its on_infer
    // callback (exactly what the coordinator router sends). An envelope would
    // instead fall through to the generic handler and never reach on_infer.
    let payload = decentraai_protocol::serialize_message(&req)?;
    tracing::info!(%request_id, "sending inference request");

    let response_bytes = node.request(worker_peer, payload).await?;
    let first: InferMessage =
        decentraai_protocol::deserialize_message(&response_bytes, response_bytes.len())?;
    match &first {
        InferMessage::InferAccepted { request_id, .. } => {
            tracing::info!(%request_id, "worker accepted request")
        }
        InferMessage::InferResponse(resp) => {
            println!(
                "immediate final response: success={} output={:?} error={:?}",
                resp.success, resp.output, resp.error
            );
            return Ok(());
        }
        InferMessage::InferFailed { error, .. } => bail!("worker rejected request: {error}"),
        other => bail!("unexpected immediate response: {other:?}"),
    }

    // Stream progress frames until the terminal event, or the user cancels.
    // On Ctrl-C we flip `cancelled` and send InferCancel, but keep listening:
    // the worker aborts generation and confirms with InferFailed(cancelled),
    // which is our terminal frame.
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut output = String::new();
    let cancel_worker = node.clone();
    let cancel_flag = cancelled.clone();
    let cancel_request_id = request_id;
    let cancel_task = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        cancel_flag.store(true, Ordering::SeqCst);
        tracing::warn!(%cancel_request_id, "user cancelled; sending InferCancel");
        let cancel = InferMessage::InferCancel {
            request_id: cancel_request_id,
            reason: "user abort".to_string(),
        };
        if let Ok(bytes) = decentraai_protocol::serialize_message(&cancel) {
            let _ = cancel_worker.request(worker_peer, bytes).await;
        }
    });

    println!("--- response ---");
    let stream_fut = async {
        loop {
            // After a cancel, only wait a short grace period for the worker's
            // InferFailed(cancelled) confirmation before giving up.
            let deadline = if cancelled.load(Ordering::SeqCst) {
                Duration::from_secs(10)
            } else {
                Duration::from_millis(timeout_ms)
            };
            match tokio::time::timeout(deadline, rx.recv()).await {
                Ok(Some(bytes)) => {
                    let msg: InferMessage =
                        decentraai_protocol::deserialize_message(&bytes, bytes.len())?;
                    match msg {
                        InferMessage::InferProgress(p) => {
                            output.push_str(&p.partial_output);
                            print!("{}", p.partial_output);
                            let _ = std::io::Write::flush(&mut std::io::stdout());
                        }
                        InferMessage::InferResponse(resp) => {
                            println!();
                            println!(
                                "--- done (success={} tokens={} elapsed_ms={}) ---",
                                resp.success, resp.tokens_used, resp.processing_time_ms
                            );
                            if resp.success && resp.output.len() > output.len() {
                                print!("{}", &resp.output[output.len()..]);
                                let _ = std::io::Write::flush(&mut std::io::stdout());
                            }
                            return Ok(());
                        }
                        InferMessage::InferFailed {
                            error, retryable, ..
                        } => {
                            println!();
                            if cancelled.load(Ordering::SeqCst) {
                                println!("--- cancelled by user (worker aborted: {error}) ---");
                                return Ok(());
                            }
                            println!("--- failed (retryable={retryable}): {error} ---");
                            bail!("inference failed: {error}");
                        }
                        _ => {}
                    }
                }
                Ok(None) => bail!("worker closed the stream without a terminal event"),
                Err(_) if cancelled.load(Ordering::SeqCst) => {
                    bail!("cancelled by user")
                }
                Err(_) => bail!("timed out after {timeout_ms}ms waiting for worker"),
            }
        }
    };

    let result = stream_fut.await;
    cancel_task.abort();
    result
}

//! End-to-end transfer tests: real nodes on loopback exchanging
//! manifests and chunks through the libp2p request/response channel.

use anyhow::Result;
use decentraai_identity::Identity;
use decentraai_manifest::{CHUNK_SIZE, Manifest, scan};
use decentraai_p2p::reputation::ReputationStore;
use decentraai_p2p::transfer::{download, download_multi};
use decentraai_p2p::{
    DEFAULT_MAX_CHUNK_MESSAGE_BYTES, DEFAULT_MAX_MESSAGE_BYTES, P2PNode, RegistryServer,
    RequestHandler, StaticFileServer,
};
use decentraai_protocol::{
    CURRENT_PROTOCOL_VERSION, CatalogRequest, CatalogResponse, ChunkRequest, ChunkResponse,
    ManifestRequest, ManifestResponse, announcement_bytes, deserialize_message, serialize_message,
};
use libp2p::PeerId;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;

/// Deterministic bytes starting with the GGUF magic (required by `scan`).
fn test_bytes(len: usize) -> Vec<u8> {
    assert!(len >= 4, "test data must fit the GGUF magic");
    let mut data = b"GGUF".to_vec();
    data.extend((4..len).map(|i| (i % 251) as u8));
    data
}

/// Spins up a serving node (with handler) and a client node, connected
/// over loopback via an explicit dial with the server's PeerId.
async fn node_pair(handler: Option<Arc<dyn RequestHandler>>) -> (P2PNode, P2PNode) {
    let server = P2PNode::new(
        &Identity::generate(),
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        handler,
    )
    .unwrap();
    let addr = server.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();

    let client = P2PNode::new(
        &Identity::generate(),
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
    )
    .unwrap();
    client
        .dial(&format!("{addr}/p2p/{}", server.local_peer_id()))
        .await
        .unwrap();
    (server, client)
}

async fn spawn_server(handler: Option<Arc<dyn RequestHandler>>) -> (P2PNode, String) {
    let server = P2PNode::new(
        &Identity::generate(),
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        handler,
    )
    .unwrap();
    let addr = server.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();
    let peer_id = server.local_peer_id();
    (server, format!("{addr}/p2p/{peer_id}"))
}

/// A reputation store with a high ban threshold: neutral for tests that
/// do not exercise banning.
fn test_reputation(dir: &Path) -> ReputationStore {
    ReputationStore::load(&dir.join("reputation.json"), 100, Duration::from_secs(300)).unwrap()
}

/// Retries the download while the freshly dialed connection settles.
/// Progress persists across attempts thanks to the resume bitmap.
async fn download_with_retry(
    client: &P2PNode,
    peer: PeerId,
    manifest_id: &str,
    dir: &Path,
    reputation: &mut ReputationStore,
) -> Result<PathBuf> {
    let mut last_err = None;
    for _ in 0..50 {
        match download(client, peer, manifest_id, dir, reputation).await {
            Ok(path) => return Ok(path),
            Err(e) => {
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    Err(last_err.unwrap())
}

/// Serializes a ManifestResponse. Takes ownership because `Manifest` is not Clone.
fn manifest_bytes(manifest: Manifest) -> Vec<u8> {
    serialize_message(&ManifestResponse {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        manifest,
    })
    .unwrap()
}

#[tokio::test]
async fn end_to_end_download_matches_source() {
    let dir = TempDir::new().unwrap();
    let source_path = dir.path().join("model.gguf");
    let data = test_bytes(CHUNK_SIZE + 123);
    std::fs::write(&source_path, &data).unwrap();

    let manifest = scan(&source_path).unwrap();
    let manifest_id = manifest.model_id.clone();
    let handler = Arc::new(StaticFileServer::new(
        manifest_bytes(manifest),
        source_path.clone(),
        CHUNK_SIZE as u64,
    ));

    let (server, client) = node_pair(Some(handler)).await;
    let mut reputation = test_reputation(dir.path());
    let out_dir = dir.path().join("client");
    let path = download_with_retry(
        &client,
        server.local_peer_id(),
        &manifest_id,
        &out_dir,
        &mut reputation,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert_eq!(path.file_name().unwrap().to_str().unwrap(), "model.gguf");
    let staging = out_dir.join("staging");
    assert!(!staging.join(format!("{manifest_id}.part")).exists());
    assert!(!staging.join(format!("{manifest_id}.done")).exists());
    assert_eq!(
        reputation.score(&server.local_peer_id()),
        2.0,
        "two verified chunks must credit the peer twice"
    );
}

/// Serves a valid manifest but garbage chunk data.
struct CorruptChunkHandler {
    manifest_response: Vec<u8>,
}

impl RequestHandler for CorruptChunkHandler {
    fn handle(&self, request: &[u8]) -> Result<Vec<u8>> {
        if deserialize_message::<ManifestRequest>(request, request.len()).is_ok() {
            return Ok(self.manifest_response.clone());
        }
        if let Ok(req) = deserialize_message::<ChunkRequest>(request, request.len()) {
            return serialize_message(&ChunkResponse {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                chunk_index: req.chunk_index,
                chunk_data: vec![0xFF; 64],
            });
        }
        anyhow::bail!("unrecognized request")
    }
}

#[tokio::test]
async fn corrupted_chunk_is_rejected() {
    let dir = TempDir::new().unwrap();
    let source_path = dir.path().join("model.gguf");
    std::fs::write(&source_path, test_bytes(CHUNK_SIZE + 1)).unwrap();

    let manifest = scan(&source_path).unwrap();
    let manifest_id = manifest.model_id.clone();
    let handler = Arc::new(CorruptChunkHandler {
        manifest_response: manifest_bytes(manifest),
    });

    let (server, client) = node_pair(Some(handler)).await;
    let mut reputation = test_reputation(dir.path());
    let out_dir = dir.path().join("client");
    let err = download_with_retry(
        &client,
        server.local_peer_id(),
        &manifest_id,
        &out_dir,
        &mut reputation,
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("failed verification"),
        "expected a verification failure, got: {err}"
    );
    assert!(
        reputation.failures(&server.local_peer_id()) >= 1,
        "corrupt chunks must be recorded as failures"
    );
}

#[tokio::test]
async fn corrupted_download_is_quarantined() {
    let dir = TempDir::new().unwrap();
    let source_path = dir.path().join("model.gguf");
    std::fs::write(&source_path, test_bytes(CHUNK_SIZE + 1)).unwrap();

    let manifest = scan(&source_path).unwrap();
    let manifest_id = manifest.model_id.clone();
    let handler = Arc::new(CorruptChunkHandler {
        manifest_response: manifest_bytes(manifest),
    });

    let (server, client) = node_pair(Some(handler)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut reputation = test_reputation(dir.path());
    let out_dir = dir.path().join("client");
    let err = download(
        &client,
        server.local_peer_id(),
        &manifest_id,
        &out_dir,
        &mut reputation,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("failed verification"));

    // The staging artifact moved to quarantine with metadata.
    let quarantine = out_dir.join("quarantine");
    let meta_path = quarantine.join(format!("{manifest_id}.quarantine.json"));
    assert!(meta_path.exists(), "quarantine metadata must exist");
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(meta_path).unwrap()).unwrap();
    assert_eq!(meta["manifest_id"].as_str().unwrap(), manifest_id);
    assert_eq!(
        meta["peer"].as_str().unwrap(),
        server.local_peer_id().to_string()
    );
    assert!(
        meta["reason"]
            .as_str()
            .unwrap()
            .contains("failed verification")
    );
    assert!(
        !out_dir
            .join("staging")
            .join(format!("{manifest_id}.part"))
            .exists(),
        "staging must be emptied by the quarantine move"
    );

    // The security events reached the audit log.
    let audit = std::fs::read_to_string(out_dir.join("logs/audit.jsonl")).unwrap();
    assert!(audit.contains("chunk_verification_failed"));
}

#[tokio::test]
async fn misbehaving_peer_gets_banned() {
    let dir = TempDir::new().unwrap();
    let source_path = dir.path().join("model.gguf");
    std::fs::write(&source_path, test_bytes(CHUNK_SIZE + 1)).unwrap();

    let manifest = scan(&source_path).unwrap();
    let manifest_id = manifest.model_id.clone();
    let handler = Arc::new(CorruptChunkHandler {
        manifest_response: manifest_bytes(manifest),
    });

    let (server, client) = node_pair(Some(handler)).await;
    // Let the dialed connection settle before direct (non-retried) calls.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut reputation = ReputationStore::load(
        &dir.path().join("reputation.json"),
        2,
        Duration::from_secs(3600),
    )
    .unwrap();
    let out_dir = dir.path().join("client");
    let peer = server.local_peer_id();

    let first = download(&client, peer, &manifest_id, &out_dir, &mut reputation)
        .await
        .unwrap_err();
    assert!(first.to_string().contains("failed verification"));
    assert!(!reputation.is_banned(&peer));

    let second = download(&client, peer, &manifest_id, &out_dir, &mut reputation)
        .await
        .unwrap_err();
    assert!(second.to_string().contains("failed verification"));
    assert!(
        reputation.is_banned(&peer),
        "peer must be banned at the threshold"
    );

    // The third attempt is refused locally, before any network traffic.
    let third = download(&client, peer, &manifest_id, &out_dir, &mut reputation)
        .await
        .unwrap_err();
    assert!(third.to_string().contains("banned"));

    let audit = std::fs::read_to_string(out_dir.join("logs/audit.jsonl")).unwrap();
    assert!(audit.contains("peer_banned"));
}

/// Delegates to StaticFileServer while counting chunk requests.
struct CountingHandler {
    inner: StaticFileServer,
    chunk_requests: Arc<AtomicUsize>,
}

impl RequestHandler for CountingHandler {
    fn handle(&self, request: &[u8]) -> Result<Vec<u8>> {
        if deserialize_message::<ChunkRequest>(request, request.len()).is_ok() {
            self.chunk_requests.fetch_add(1, Ordering::SeqCst);
        }
        self.inner.handle(request)
    }
}

#[tokio::test]
async fn download_resumes_from_bitmap() {
    let dir = TempDir::new().unwrap();
    let source_path = dir.path().join("model.gguf");
    let data = test_bytes(CHUNK_SIZE * 2);
    std::fs::write(&source_path, &data).unwrap();

    let manifest = scan(&source_path).unwrap();
    let manifest_id = manifest.model_id.clone();
    let counter = Arc::new(AtomicUsize::new(0));
    let handler = Arc::new(CountingHandler {
        inner: StaticFileServer::new(
            manifest_bytes(manifest),
            source_path.clone(),
            CHUNK_SIZE as u64,
        ),
        chunk_requests: counter.clone(),
    });

    let (server, client) = node_pair(Some(handler)).await;

    // Simulate an interrupted download: chunk 0 already staged and marked done.
    let out_dir = dir.path().join("client");
    let staging = out_dir.join("staging");
    std::fs::create_dir_all(&staging).unwrap();
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(staging.join(format!("{manifest_id}.part")))
            .unwrap();
        file.set_len(data.len() as u64).unwrap();
        file.write_all(&data[..CHUNK_SIZE]).unwrap();
    }
    std::fs::write(staging.join(format!("{manifest_id}.done")), [1u8, 0u8]).unwrap();

    let mut reputation = test_reputation(dir.path());
    let path = download_with_retry(
        &client,
        server.local_peer_id(),
        &manifest_id,
        &out_dir,
        &mut reputation,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "only the missing chunk should have been requested"
    );
    assert_eq!(
        reputation.successes(&server.local_peer_id()),
        1,
        "resumed chunks must not be credited twice"
    );
}

#[tokio::test]
async fn registry_server_serves_scanned_models() {
    let dir = TempDir::new().unwrap();
    let models_dir = dir.path().join("models");
    std::fs::create_dir_all(&models_dir).unwrap();
    let data = test_bytes(CHUNK_SIZE * 2 + 7);
    std::fs::write(models_dir.join("tiny.gguf"), &data).unwrap();

    let mut registry = decentraai_registry::ModelRegistry::new(models_dir.clone()).unwrap();
    registry.scan_directory(&models_dir).unwrap();
    let manifest = scan(models_dir.join("tiny.gguf")).unwrap();
    let manifest_id = manifest.model_id.clone();

    let handler = Arc::new(RegistryServer::new(registry));
    let (server, client) = node_pair(Some(handler)).await;
    let mut reputation = test_reputation(dir.path());
    let out_dir = dir.path().join("client");
    let path = download_with_retry(
        &client,
        server.local_peer_id(),
        &manifest_id,
        &out_dir,
        &mut reputation,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert_eq!(path.file_name().unwrap().to_str().unwrap(), "tiny.gguf");
    assert_eq!(reputation.score(&server.local_peer_id()), 3.0);
}

#[tokio::test]
async fn registry_server_answers_catalog_requests() {
    let dir = TempDir::new().unwrap();
    let models_dir = dir.path().join("models");
    std::fs::create_dir_all(&models_dir).unwrap();
    std::fs::write(models_dir.join("tiny.gguf"), test_bytes(CHUNK_SIZE + 5)).unwrap();

    let mut registry = decentraai_registry::ModelRegistry::new(models_dir.clone()).unwrap();
    registry.scan_directory(&models_dir).unwrap();
    let manifest_id = scan(models_dir.join("tiny.gguf")).unwrap().model_id;

    let handler = Arc::new(RegistryServer::new(registry));
    let (server, client) = node_pair(Some(handler)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let request = serialize_message(&CatalogRequest {
        protocol_version: CURRENT_PROTOCOL_VERSION,
    })
    .unwrap();
    let raw = client
        .request(server.local_peer_id(), request)
        .await
        .unwrap();
    let catalog: CatalogResponse = deserialize_message(&raw, DEFAULT_MAX_MESSAGE_BYTES).unwrap();
    assert_eq!(catalog.protocol_version, CURRENT_PROTOCOL_VERSION);
    assert_eq!(catalog.manifests.len(), 1);
    assert_eq!(catalog.manifests[0].model_id, manifest_id);
    assert_eq!(catalog.manifests[0].file_name, "tiny.gguf");
}

#[tokio::test]
async fn multi_provider_download_splits_chunks() {
    let dir = TempDir::new().unwrap();
    let source_path = dir.path().join("model.gguf");
    let data = test_bytes(CHUNK_SIZE * 4);
    std::fs::write(&source_path, &data).unwrap();

    let manifest = scan(&source_path).unwrap();
    let manifest_id = manifest.model_id.clone();
    let response_bytes = manifest_bytes(manifest);

    let (server_a, addr_a) = spawn_server(Some(Arc::new(StaticFileServer::new(
        response_bytes.clone(),
        source_path.clone(),
        CHUNK_SIZE as u64,
    ))))
    .await;
    let (server_b, addr_b) = spawn_server(Some(Arc::new(StaticFileServer::new(
        response_bytes,
        source_path.clone(),
        CHUNK_SIZE as u64,
    ))))
    .await;

    let client = P2PNode::new(
        &Identity::generate(),
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
    )
    .unwrap();
    client.dial(&addr_a).await.unwrap();
    client.dial(&addr_b).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut reputation = test_reputation(dir.path());
    let out_dir = dir.path().join("client");
    let path = download_multi(
        &client,
        &[server_a.local_peer_id(), server_b.local_peer_id()],
        &manifest_id,
        &out_dir,
        &mut reputation,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert_eq!(
        reputation.successes(&server_a.local_peer_id()),
        2,
        "round-robin assigns two of four chunks to each provider"
    );
    assert_eq!(reputation.successes(&server_b.local_peer_id()), 2);
}

#[tokio::test]
async fn multi_provider_falls_back_after_corruption() {
    let dir = TempDir::new().unwrap();
    let source_path = dir.path().join("model.gguf");
    let data = test_bytes(CHUNK_SIZE * 2);
    std::fs::write(&source_path, &data).unwrap();

    let manifest = scan(&source_path).unwrap();
    let manifest_id = manifest.model_id.clone();
    let response_bytes = manifest_bytes(manifest);

    let (corrupt, addr_corrupt) = spawn_server(Some(Arc::new(CorruptChunkHandler {
        manifest_response: response_bytes.clone(),
    })))
    .await;
    let (honest, addr_honest) = spawn_server(Some(Arc::new(StaticFileServer::new(
        response_bytes,
        source_path.clone(),
        CHUNK_SIZE as u64,
    ))))
    .await;

    let client = P2PNode::new(
        &Identity::generate(),
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
    )
    .unwrap();
    client.dial(&addr_corrupt).await.unwrap();
    client.dial(&addr_honest).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut reputation = test_reputation(dir.path());
    let out_dir = dir.path().join("client");
    let path = download_multi(
        &client,
        &[corrupt.local_peer_id(), honest.local_peer_id()],
        &manifest_id,
        &out_dir,
        &mut reputation,
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert!(
        reputation.failures(&corrupt.local_peer_id()) >= 1,
        "the corrupt provider must be recorded"
    );
    assert!(
        reputation.successes(&honest.local_peer_id()) >= 1,
        "the honest provider must serve the fallback chunks"
    );
}

#[tokio::test]
async fn manifest_announcement_fires_callback() {
    let dir = TempDir::new().unwrap();
    let source_path = dir.path().join("model.gguf");
    std::fs::write(&source_path, test_bytes(CHUNK_SIZE + 1)).unwrap();
    let manifest = scan(&source_path).unwrap();

    let (server, mut client) = node_pair(None).await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    client.set_on_manifest_announcement(move |peer, m| {
        let _ = tx.send((peer, m.file_name));
    });

    // The dial needs a moment to settle before the server sees the client
    // as connected; broadcast only reaches connected peers. Re-announce
    // until the callback fires.
    let payload = announcement_bytes(&manifest, None).unwrap();
    let mut seen = None;
    for _ in 0..50 {
        server.announce(payload.clone());
        if let Ok(item) = rx.try_recv() {
            seen = Some(item);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (peer, name) = seen.expect("announcement callback must fire");
    assert_eq!(peer, server.local_peer_id());
    assert_eq!(name, "model.gguf");
}

#[tokio::test]
async fn announced_model_auto_downloads_and_verifies() {
    let dir = TempDir::new().unwrap();
    let source_path = dir.path().join("model.gguf");
    let data = test_bytes(CHUNK_SIZE + 123);
    std::fs::write(&source_path, &data).unwrap();

    let manifest = scan(&source_path).unwrap();
    let handler = Arc::new(StaticFileServer::new(
        manifest_bytes(manifest.clone()),
        source_path.clone(),
        CHUNK_SIZE as u64,
    ));

    let server = P2PNode::new(
        &Identity::generate(),
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        Some(handler),
    )
    .unwrap();
    let addr = server.listen("/ip4/127.0.0.1/tcp/0").await.unwrap();

    let mut client = P2PNode::new(
        &Identity::generate(),
        DEFAULT_MAX_MESSAGE_BYTES,
        DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
        None,
    )
    .unwrap();
    client
        .dial(&format!("{addr}/p2p/{}", server.local_peer_id()))
        .await
        .unwrap();

    let out_dir = dir.path().join("client");
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_dir_for_cb = out_dir.clone();
    let rep_path = dir.path().join("reputation.json");
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let client_handle = client.clone();
    let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let started_for_cb = started.clone();
    client.set_on_manifest_announcement(move |peer, m| {
        // Only the first delivery spawns a download; re-announcements while
        // the connection settles must not start duplicate transfers.
        if started_for_cb.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let client_handle = client_handle.clone();
        let out_dir = out_dir_for_cb.clone();
        let rep_path = rep_path.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut reputation =
                ReputationStore::load(&rep_path, 100, Duration::from_secs(300)).unwrap();
            let result = download(&client_handle, peer, &m.model_id, &out_dir, &mut reputation)
                .await;
            let _ = tx.send(result);
        });
    });

    // Re-announce until the connection settles and the first callback fires.
    let payload = announcement_bytes(&manifest, None).unwrap();
    for _ in 0..50 {
        if started.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        server.announce(payload.clone());
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let outcome = tokio::time::timeout(Duration::from_secs(20), rx.recv())
        .await
        .expect("auto-download must complete")
        .expect("download must return a path")
        .expect("download must succeed");
    assert_eq!(
        outcome.file_name().unwrap().to_string_lossy(),
        "model.gguf"
    );
    assert_eq!(
        std::fs::read(&outcome).unwrap(),
        data,
        "the announced model must match its source byte-for-byte"
    );
}

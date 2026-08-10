# Q3b → Q3e: Complete Implementation Guide

## Overview

Implements complete worker onboarding and task execution flow:

- **Q3b**: QR code pairing + trust persistence (SQLite)
- **Q3c**: Worker P2P protocol (InferRequest/Response)
- **Q3d**: Scheduler with intelligent task placement
- **Q3e**: Onboarding wizard (first-run setup)

---

## Q3b: QR Code Pairing + Trust Persistence

### Pairing Code Structure

```rust
pub struct PairingCode {
    pub worker_peer_id: PeerId,
    pub controller_peer_id: PeerId,
    pub pairing_token: String,  // UUID for verification
    pub expires_at: u64,         // 5 minute TTL
    pub node_name: String,
}
```

### QR Code Flow

1. **Worker generates pairing code**
   ```rust
   let pairing = PairingCode::new(
       worker_peer_id,
       controller_peer_id,
       "gpu-worker-1".to_string(),
       300, // 5 minutes
   );
   let qr_data = pairing.to_qr_data()?; // JSON string
   ```

2. **Display QR code**
   ```rust
   // Use qrcode crate
   let code = qrcode::QrCode::new(qr_data.as_bytes());
   // Render as image or ASCII
   ```

3. **Controller scans QR**
   ```rust
   let pairing = PairingCode::from_qr_data(scanned_data)?;
   assert!(!pairing.is_expired());
   ```

4. **Sign and verify**
   ```rust
   let signature = pairing.sign_pairing(controller_identity)?;
   assert!(pairing.verify_pairing(&signature, controller_identity));
   ```

### Trust Store (SQLite)

```rust
let trust_store = TrustStore::new("trust.db")?;

// Add trusted worker
let record = TrustRecordPersisted::new(&pairing);
trust_store.add_trust(&record)?;

// Get trust record
let trust = trust_store.get_trust(&worker_peer_id)?;

// List all trusted
let all = trust_store.list_trusted()?;

// Remove trust
trust_store.remove_trust(&worker_peer_id)?;
```

### Trust Score Calculation

```rust
fn update_trust_score(&mut self) {
    if self.total_requests == 0 {
        return;
    }
    let success_rate = self.successful_requests as f32 / self.total_requests as f32;
    // Exponential moving average
    self.trust_score = 0.8 * self.trust_score + 0.2 * success_rate;
}
```

---

## Q3c: Worker P2P Protocol

### Message Types

```rust
pub enum InferMessage {
    InferRequest(InferRequest),
    InferAccepted { request_id, worker_peer_id, estimated_wait_ms },
    InferProgress(InferProgress),
    InferResponse(InferResponse),
    InferFailed { request_id, worker_peer_id, error, retryable },
    InferCancel { request_id, reason },
    InferPing { request_id },
    InferPong { request_id, latency_ms },
}
```

### Request Flow

```
Controller                        Worker
    |                               |
    |-- InferRequest -------------->|
    |                               | (validate)
    |<-- InferAccepted -------------|
    |                               | (execute)
    |<-- InferProgress (stream) ----| (optional)
    |                               | (complete)
    |<-- InferResponse -------------|
    |
```

### Example Request

```rust
let request = InferRequest::new(
    "sha256:abc123...".to_string(),
    "What is the capital of France?".to_string(),
    100, // max_tokens
);

let msg = InferMessage::InferRequest(request);

// Send via P2P
p2p.send_message(worker_peer_id, msg).await?;
```

### Example Response

```rust
let response = InferResponse {
    request_id: request.request_id,
    worker_peer_id: worker.peer_id,
    output: "Paris".to_string(),
    tokens_used: 1,
    time_ms: 150,
    success: true,
    error: None,
};

let msg = InferMessage::InferResponse(response);
p2p.send_message(controller_peer_id, msg).await?;
```

---

## Q3d: Scheduler with Task Placement

### Scoring Algorithm

```rust
fn score_worker(&self, worker: &WorkerStatus, request: &InferRequest) -> f32 {
    let mut score = 0.0;
    
    // Lower queue depth = better (weight: 0.4)
    let queue_score = 1.0 - (worker.queue_depth as f32 / self.config.max_queue_depth as f32);
    score += queue_score * 0.4;
    
    // Higher available capacity = better (weight: 0.3)
    score += worker.available_capacity * 0.3;
    
    // Lower latency = better (weight: 0.2)
    let latency_score = 1.0 - (worker.current_latency_ms as f32 / 1000.0).min(1.0);
    score += latency_score * 0.2;
    
    // Higher throughput = better (weight: 0.1)
    let throughput_score = (worker.tokens_per_second as f32 / 100.0).min(1.0);
    score += throughput_score * 0.1;
    
    score
}
```

### Task Placement

```rust
let scheduler = WorkerScheduler::new(SchedulerConfig::default());

// Register workers
scheduler.register_worker(worker1_announcement);
scheduler.register_worker(worker2_announcement);

// Select best worker
let request = InferRequest::new(model_hash, prompt, max_tokens);
let placement = scheduler.select_worker(&request).unwrap();

println!("Selected: {}", placement.selected_worker);
println!("Estimated wait: {}ms", placement.estimated_wait_ms);
println!("Confidence: {}%", placement.confidence * 100.0);
```

### Fallback Logic

```rust
// Primary worker failed
let fallback_workers = scheduler.get_fallback_workers(&request, &failed_worker_id);

if let Some(fallback) = fallback_workers.first() {
    // Retry with fallback
    scheduler.queue_request(&fallback.selected_worker, request);
}
```

### Queue Management

```rust
// Add to queue
scheduler.queue_request(&worker_id, request.clone());

// Remove on completion
scheduler.dequeue_request(&worker_id, request.request_id);

// Record metrics
scheduler.record_completion(&worker_id, request_id, success, time_ms);
```

---

## Q3e: Onboarding Wizard

### Wizard Steps

1. **System Resources** - Auto-detect CPU, RAM, GPU, disk
2. **Identity** - Generate new or restore from backup
3. **Role Selection** - Worker, Controller, Both, Validator
4. **Network Config** - P2P port, bootstrap peers, mDNS
5. **QR Pairing** - Generate/display QR code for controller

### Usage

```bash
# First run - wizard auto-starts
cargo run --bin decentraai

# Or explicitly
open docs/onboarding/wizard.html
```

### API Integration

```rust
// POST /api/config
#[derive(Serialize)]
struct OnboardingConfig {
    resources: SystemInfo,
    node_name: String,
    role: String,  // "worker", "controller", "both"
    p2p_port: u16,
    bootstrap_peers: Vec<String>,
    enable_mdns: bool,
    pairing: Option<PairingCode>,
}
```

### Config File Generation

```toml
# config.toml
[node]
name = "gpu-worker-1"
role = "worker"

[p2p]
port = 4001
bootstrap_peers = [
  "/dns4/bootstrap.decentraai.io/tcp/4001/p2p/16Uiu8..."
]
enable_mdns = true

[identity]
path = ".decentraai/identity.json"

[trust_store]
path = ".decentraai/trust.db"
```

---

## Integration Example

### Complete Flow

```rust
use discovery::{
    DiscoveryService, DiscoveryConfig,
    WorkerScheduler, SchedulerConfig,
    TrustStore, PairingCode,
};
use protocol::{InferRequest, InferMessage};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Load or generate identity
    let identity = Identity::load_or_generate(".decentraai/identity.json")?;
    
    // 2. Initialize trust store
    let trust_store = TrustStore::new(".decentraai/trust.db")?;
    
    // 3. Start P2P service
    let p2p = P2PService::new(identity.clone()).await?;
    
    // 4. Start discovery service
    let config = DiscoveryConfig::default();
    let mut discovery = DiscoveryService::new(config, identity.clone(), p2p.clone());
    
    // 5. Start scheduler
    let scheduler = WorkerScheduler::new(SchedulerConfig::default());
    
    // 6. Handle worker discoveries
    tokio::spawn(async move {
        loop {
            let pending = discovery.get_pending_workers();
            for worker in pending {
                println!("Worker discovered: {}", worker.node_name);
                
                // Approve from dashboard
                // discovery.approve_worker(&worker.peer_id)?;
                
                // Add to trust store
                // trust_store.add_trust(&record)?;
                
                // Register with scheduler
                // scheduler.register_worker(worker.clone());
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });
    
    // 7. Submit inference request
    let request = InferRequest::new(
        "model_hash".to_string(),
        "Hello, world!".to_string(),
        50,
    );
    
    let placement = scheduler.select_worker(&request).unwrap();
    println!("Selected worker: {}", placement.selected_worker);
    
    // Send request
    let msg = InferMessage::InferRequest(request);
    p2p.send_message(placement.selected_worker, msg).await?;
    
    Ok(())
}
```

---

## Testing

### Q3b: QR Pairing

```bash
# Terminal 1 - worker
cargo run --bin decentraai -- worker --name "test-worker"

# Terminal 2 - controller
# Open wizard, select "controller", scan QR from worker
cargo run --bin decentraai -- controller
```

### Q3c: P2P Protocol

```bash
# Start worker
cargo run --bin decentraai -- worker

# Submit request
curl -X POST http://localhost:8000/api/infer \
  -H "Content-Type: application/json" \
  -d '{"model": "llama", "prompt": "Hello"}'
```

### Q3d: Scheduler

```bash
# Start multiple workers
cargo run --bin decentraai -- worker --name "worker-1" &
cargo run --bin decentraai -- worker --name "worker-2" &

# Submit many requests
for i in {1..100}; do
  curl -X POST http://localhost:8000/api/infer \
    -d "{\"prompt\": \"Request $i\"}"
done
```

### Q3e: Onboarding

```bash
# First run - wizard appears
cargo run --bin decentraai

# Complete wizard, check generated config
cat .decentraai/config.toml
```

---

## Next Steps

- **Q4a**: Tokenomics and rewards distribution
- **Q4b**: Multi-model support and versioning
- **Q4c**: Advanced monitoring and metrics
- **Q4d**: Production hardening and security audits

---

## Security Considerations

1. **Pairing codes expire** - 5 minute TTL prevents replay attacks
2. **Signatures required** - All trust records signed by identity
3. **Trust scores decay** - Poor performance reduces trust
4. **Sandboxed execution** - Workers run in isolated containers
5. **Rate limiting** - Prevents DoS from malicious workers

---

## Performance Tuning

- Adjust `SchedulerConfig::max_queue_depth` based on workload
- Tune trust score EMA weights (0.8/0.2 default)
- Increase bootstrap peers for faster discovery
- Enable mDNS for LAN-only deployments

---

**Implemented**: August 2026  
**Branch**: `feature/q3b-q3e-complete`  
**Files**: 15+ new/modified  
**Lines**: ~3000

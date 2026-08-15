# M9: Distributed Inference Architecture

## Overview

M9 implements distributed inference across peer GPUs, enabling request routing to available workers and automatic fallback when workers fail. Requests are routed based on real-time capacity, and workers are compensated in reputation for their contributions.

## Architecture

### Components

```
┌─────────────────────────────────────────────────────────────────────┐
│                            Client Node                                   │
├─────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────────────┐   │
│  │   API        │    │   Scheduler  │    │      P2P Node        │   │
│  │   Endpoint   │───▶│  (discovery) │───▶│   (p2p crate)        │   │
│  └──────────────┘    └──────────────┘    └──────────┬───────────┘   │
│                                                        │               │
│                                                        ▼               │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │                        P2P Network (libp2p)                       │  │
│  │   ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐           │  │
│  │   │  mDNS   │  │  Noise  │  │  Yamux  │  │   TCP   │           │  │
│  │   │Discovery│  │Security │  │Muxing  │  │Transport│           │  │
│  │   └─────────┘  └─────────┘  └─────────┘  └─────────┘           │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                                                        │               │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │                        Worker Nodes                               │  │
│  │                                                                   │  │
│  │   ┌──────────────┐    ┌──────────────┐    ┌──────────────┐    │  │
│  │   │  Inference    │◀───│    P2P       │◀───│    Runtime    │    │  │
│  │   │   Handler     │    │    Node      │    │ (llama-server)│    │  │
│  │   └──────────────┘    └──────────────┘    └──────────────┘    │  │
│  │                                                                   │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                                                                          │
└─────────────────────────────────────────────────────────────────────┘
```

### Data Flow

#### 1. Worker Discovery and Registration

```
Worker Node                          Network                         Client Node
     │                                   │                               │
     │─── Broadcast WorkerAnnouncement ──▶│                               │
     │                                   │                               │
     │                                   ▼                               │
     │                                   │                               │
     │◀─── mDNS Discovery ─────────────────│                               │
     │                                   │                               │
     │◀─── Direct Connection ───────────────│───────────────────────────────▶│
     │                                   │                               │
     │─── WorkerAnnouncement ──────────────▶│───────────────────────────────▶│
     │                                   │                               │
     │                                   │                               ▼
     │                                   │                      Scheduler registers worker
```

Workers periodically broadcast their `WorkerAnnouncement` containing:
- Peer ID
- Node name
- Loaded models (by hash)
- Available capacity (0.0 - 1.0)
- Current queue depth
- Tokens per second throughput
- Current latency

#### 2. Request Routing

```
Client Node                          Network                         Worker Node
     │                                   │                               │
     │ ── InferRequest -------------------▶│─────────────────────────────▶│
     │                                   │                               │
     │◀── InferAccepted ──────────────────│──────────────────────────────┘│
     │                                   │                               │
     │◀── InferProgress (streaming) ───────│◀──────────────────────────────│
     │                                   │                               │
     │◀── InferResponse ──────────────────│◀──────────────────────────────│
```

When a client receives an inference request:
1. Scheduler selects the best worker using multi-factor scoring
2. Request is sent via P2P to the selected worker
3. Worker processes request and returns response
4. On failure, scheduler selects fallback worker and retries

#### 3. Fallback Mechanism

```
Client Node
     │
     ├─▶ Worker A ──▶ Success ─────────────────────▶ Return response
     │
     └─▶ Worker A ──▶ Failure ─────────────────────▶ Select fallback
              │                                      │
              └──────────────────────────────────────┘
                                                       │
                                                       ▼
                                              Worker B ──▶ Success
```

### Message Types

#### WorkerAnnouncement
Announced by workers to advertise their capabilities:

```rust
pub struct WorkerAnnouncement {
    pub peer_id: PeerId,
    pub node_name: String,
    pub loaded_models: Vec<String>,      // Model hashes
    pub available_capacity: f32,       // 0.0 - 1.0
    pub queue_depth: u32,
    pub tokens_per_second: u32,
    pub current_latency_ms: u32,
}
```

#### WorkerStatus
Maintained by scheduler for each known worker:

```rust
pub struct WorkerStatus {
    pub peer_id: PeerId,
    pub loaded_models: Vec<String>,
    pub queue_depth: u32,
    pub available_capacity: f32,
    pub current_latency_ms: u32,
    pub tokens_per_second: u32,
}
```

#### InferRequest
Sent from client to worker:

```rust
pub struct InferRequest {
    pub request_id: Uuid,
    pub model_hash: String,
    pub prompt: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub timeout_ms: u32,
    pub stream: bool,
    pub priority: u8,
}
```

#### InferMessage (P2P Protocol)
Enum wrapping all inference-related messages:

```rust
pub enum InferMessage {
    InferRequest(InferRequest),
    InferAccepted { request_id: Uuid, worker_peer_id: PeerId, estimated_wait_ms: u32 },
    InferProgress(InferProgress),
    InferResponse(InferResponse),
    InferFailed { request_id: Uuid, worker_peer_id: PeerId, error: String, retryable: bool },
    InferCancel { request_id: Uuid, reason: String },
    InferPing { request_id: Uuid },
    InferPong { request_id: Uuid, latency_ms: u32 },
}
```

## Scheduling Algorithm

### Multi-Factor Scoring

Workers are scored based on four factors:

| Factor | Weight | Description |
|--------|--------|-------------|
| Queue Depth | 40% | Lower is better (normalized against max_queue_depth) |
| Available Capacity | 30% | Higher is better (0.0 - 1.0) |
| Latency | 20% | Lower is better (normalized against 1000ms) |
| Throughput | 10% | Higher is better (normalized against 100 tokens/sec) |

**Score Formula:**
```
score = (queue_score * 0.4) + (capacity * 0.3) + (latency_score * 0.2) + (throughput_score * 0.1)
```

Where:
- `queue_score = 1.0 - (queue_depth / max_queue_depth)`
- `latency_score = 1.0 - (current_latency_ms / 1000.0).min(1.0)`
- `throughput_score = (tokens_per_second / 100.0).min(1.0)`

### Eligibility Criteria

A worker is eligible for a request if:
1. The worker has the requested model loaded (`loaded_models.contains(model_hash)`)
2. Available capacity > 0.1 (10%)
3. Queue depth < 10

## Request Flow

### Client-Side Flow

```
┌─────────────────────────────────────────────────────────────┐
│  Client Node: Inference Request Handling                        │
├─────────────────────────────────────────────────────────────┤
│                                                                  │
│  Start                                                         │
│    │                                                           │
│    ▼                                                           │
│  ┌─────────────────┐                                          │
│  │ Validate       │                                          │
│  │ Request        │─ Yes ─────────────────────────────────────▶│
│  └─────────────────┘                                          │
│    │ No                                                     │
│    ▼                                                         │
│  Return Error                                                │
│    │                                                         │
│    ▼                                                         │
│  ┌─────────────────┐                                          │
│  │ Select Best     │                                          │
│  │ Worker          │                                          │
│  └─────────────────┘                                          │
│    │                                                         │
│    ▼ No workers available                                      │
│  Return 503 Service Unavailable                                │
│    │                                                         │
│    ▼ Yes workers                                              │
│  ┌─────────────────┐                                          │
│  │ Send Request to │                                          │
│  │ Selected Worker │──────────────────────────────────────────▶│
│  └─────────────────┘    P2P Request                           │
│    │                                                           │
│    ▼                                                           │
│  ┌─────────────────┐                                          │
│  │ Await Response │◀──────────────────────────────────────────│
│  │ (with timeout)  │    P2P Response or Timeout                 │
│  └─────────────────┘                                          │
│    │                                                           │
│    ├─ Success ────────────────────────────────────────────────▶│
│    │                                                           │
│    └─ Failure/Timeout ─────────────────────────────────────────▶│
│                                  │                                │
│                                  ▼                                │
│                         ┌─────────────────┐                       │
│                         │ Get Fallback    │                       │
│                         │ Workers         │                       │
│                         └─────────────────┘                       │
│                                  │                                │
│                                  ▼ If fallbacks available         │
│                         ┌─────────────────┐                       │
│                         │ Select Next     │                       │
│                         │ Fallback Worker │                       │
│                         └─────────────────┘                       │
│                                  │                                │
│                                  ▼                                │
│                         Retry Request (max 3 attempts)            │
│                                  │                                │
│                                  ▼ No more fallbacks              │
│                         Return Error to Client                   │
│                                                                  │
└─────────────────────────────────────────────────────────────┘
```

### Worker-Side Flow

```
┌─────────────────────────────────────────────────────────────┐
│  Worker Node: Inference Request Processing                      │
├─────────────────────────────────────────────────────────────┤
│                                                                  │
│  Start                                                         │
│    │                                                           │
│    ▼                                                           │
│  ┌─────────────────┐                                          │
│  │ Receive        │                                          │
│  │ InferRequest    │                                          │
│  └─────────────────┘                                          │
│    │                                                           │
│    ▼                                                           │
│  ┌─────────────────┐                                          │
│  │ Check Model     │─ No ────────────────────────────────────▶│
│  │ Availability    │       InferFailed (retryable: false)      │
│  └─────────────────┘                                          │
│    │ Yes                                                      │
│    ▼                                                         │
│  ┌─────────────────┐                                          │
│  │ Check Capacity  │─ No ────────────────────────────────────▶│
│  │ & Queue Depth   │       InferFailed (retryable: true)       │
│  └─────────────────┘                                          │
│    │ Yes                                                      │
│    ▼                                                         │
│  ┌─────────────────┐                                          │
│  │ Update Queue    │                                          │
│  │ Depth           │                                          │
│  └─────────────────┘                                          │
│    │                                                           │
│    ▼                                                           │
│  ┌─────────────────┐                                          │
│  │ Send           │                                          │
│  │ InferAccepted  │──────────────────────────────────────────▶│
│  └─────────────────┘                                          │
│    │                                                           │
│    ▼                                                           │
│  ┌─────────────────┐                                          │
│  │ Process        │                                          │
│  │ Request        │                                          │
│  │ (llama-server) │                                          │
│  └─────────────────┘                                          │
│    │                                                           │
│    ▼                                                           │
│  ┌─────────────────┐                                          │
│  │ Return         │                                          │
│  │ InferResponse  │──────────────────────────────────────────▶│
│  └─────────────────┘                                          │
│    │                                                           │
│    ▼                                                           │
│  ┌─────────────────┐                                          │
│  │ Decrement Queue │                                          │
│  │ Depth           │                                          │
│  └─────────────────┘                                          │
│    │                                                           │
│    ▼                                                           │
│  Update WorkerStatus (latency EMA, capacity)                    │
│    │                                                           │
│    ▼                                                           │
│  Broadcast updated WorkerAnnouncement                          │
│                                                                  │
└─────────────────────────────────────────────────────────────┘
```

## Reputation System Integration

Workers are paid in reputation for successful inference requests. The reputation system tracks:

- **Successful requests**: Increase worker reputation
- **Failed requests**: Decrease worker reputation
- **Response time**: Faster responses increase reputation
- **Quality**: Higher confidence scores increase reputation

### Reputation Calculation

```rust
// On successful request completion
let reputation_delta = base_reward
    * confidence
    * (1.0 / (1.0 + response_time_ms / 1000.0))  // Faster = more
    * quality_factor;  // Based on output quality metrics

// On failed request
let reputation_penalty = base_penalty
    * (1.0 - confidence)  // Expected failures penalized less
    * retry_count;  // Repeated failures penalized more
```

## Configuration

### SchedulerConfig

```rust
pub struct SchedulerConfig {
    pub max_queue_depth: u32,           // Default: 10
    pub min_available_capacity: f32,  // Default: 0.1
    pub enable_load_balancing: bool,    // Default: true
    pub fallback_timeout_ms: u32,       // Default: 5000
    pub max_retries: u32,               // Default: 3
    pub retry_backoff_ms: u32,          // Default: 1000
}
```

### Network Configuration

```yaml
# In node.config.yaml
p2p:
  listen_addresses:
    - /ip4/0.0.0.0/tcp/8080
    - /ip6/::/tcp/8080
  
  # Worker discovery settings
  worker:
    announcement_interval_ms: 10000     # Broadcast status every 10 seconds
    discovery_interval_ms: 5000        # Check for new workers every 5 seconds
    stale_worker_timeout_ms: 30000     # Remove workers not heard from in 30s
    
  # Request routing settings
  routing:
    max_concurrent_requests: 100
    request_timeout_ms: 30000
    max_message_size: 1048576          # 1 MB
```

## P2P Protocol Extensions

### Message Types

The existing `/decentraai/message/1` protocol is extended with new message types:

```rust
// In decentraai_protocol
pub enum InferMessage {
    // ... existing types ...
    WorkerAnnouncement(WorkerAnnouncement),
    WorkerStatusUpdate(WorkerStatus),
    InferRequest(InferRequest),
    // ... etc
}
```

### Protocol Numbers

| Protocol | Purpose |
|----------|---------|
| `/decentraai/message/1` | Control plane (manifests, catalog) |
| `/decentraai/infer/1` | Inference requests and responses |
| `/decentraai/worker/1` | Worker discovery and status |

## Implementation Plan

### Phase 1: Worker Discovery (Task 1-2)
1. Add `WorkerAnnouncement` broadcast to P2P node
2. Add worker registration to scheduler
3. Implement periodic status updates from workers

### Phase 2: Request Routing (Task 3)
1. Integrate scheduler with API endpoint
2. Add P2P request sending to selected worker
3. Handle worker responses and forward to client

### Phase 3: Fallback Mechanism (Task 4)
1. Implement retry logic with fallback workers
2. Add exponential backoff for retries
3. Handle different failure modes (retryable vs non-retryable)

### Phase 4: Testing (Task 5)
1. Write unit tests for scheduler scoring
2. Write integration tests for worker discovery
3. Write end-to-end tests with multiple nodes

## Testing Strategy

### Unit Tests
- Scheduler scoring algorithm
- Worker eligibility checks
- Fallback worker selection
- Request queueing and dequeuing

### Integration Tests
- Worker discovery via mDNS
- P2P message serialization/deserialization
- Request/response round-trip

### End-to-End Tests
- 2-node inference: client + 1 worker
- 3-node inference: client + 2 workers (load balancing)
- Fallback test: client + failing worker + fallback worker
- Concurrent requests test: multiple simultaneous requests

## Monitoring

The following metrics are exposed for distributed inference:

| Metric | Type | Description |
|--------|------|-------------|
| `distributed.requests_total` | Counter | Total distributed requests |
| `distributed.requests_success` | Counter | Successful requests |
| `distributed.requests_failed` | Counter | Failed requests |
| `distributed.request_latency_ms` | Histogram | Request latency |
| `distributed.worker_count` | Gauge | Number of active workers |
| `distributed.queue_depth` | Gauge | Current queue depth per worker |
| `distributed.fallbacks_total` | Counter | Number of fallback requests |
| `distributed.reputation_delta` | Counter | Reputation changes |

## Security Considerations

### Authentication
- All P2P messages are signed with the node's Ed25519 identity
- Workers must be authenticated before accepting requests
- Request signatures are verified before processing

### Rate Limiting
- Maximum concurrent requests per worker
- Request timeout enforcement
- Queue depth limits prevent overload

### Validation
- Model hash verification before processing
- Request size limits
- Response validation

### Reputation-Based Trust
- Workers with low reputation get fewer requests
- Banned workers are excluded from scheduling
- Reputation decays over time (to allow recovery)

## Performance Considerations

### Optimization Strategies
1. **Connection Pooling**: Maintain persistent connections to frequently-used workers
2. **Request Batching**: Batch multiple small requests to the same worker
3. **Caching**: Cache worker capabilities to avoid repeated lookups
4. **Asynchronous Processing**: Non-blocking request handling
5. **Load Shedding**: Reject requests when system is overloaded

### Scalability
- Horizontal scaling: Add more worker nodes
- Vertical scaling: Workers can serve multiple models
- Geographic distribution: Workers in different regions

## Future Enhancements

1. **Model Preloading**: Predictive loading of models based on request patterns
2. **Geographic Routing**: Route requests to nearest worker
3. **Model-Specific Pricing**: Different reputation rates for different models
4. **Priority Queues**: Support for priority-based request handling
5. **Federated Learning**: Use inference results to improve models across network

---

# Research Note — Distributed Inference & Related Patterns (Phase 6)

Date: 2026-08-16. Classification: ADOPT / ADAPT / EXPERIMENT / WATCH / IGNORE.
Anchored to the current repo state: DecentraAI runs llama-server as an external
subprocess (never FFI); all distributed flags
(`tensor_parallel` / `expert_routing` / `prefill_decode_separation`) are gated
`false` for every engine DecentraAI actually runs (pinned by tests in
`crates/fabric/src/engine.rs`).

## llama.cpp RPC / tensor-split

- SOURCE: upstream `ggml-org/llama.cpp` `tools/rpc/README.md` (master).
- WHAT IT DOES: a separate `ggml-rpc-server` process exposes host devices over
  TCP/RDMA; the main llama-cli/server distributes model weights + KV cache across
  local and remote devices in proportion to available memory (`--rpc
  host:port,...`, optional `--tensor-split` and local tensor cache).
- CURRENT DECENTRAAI STATE: the distributed `EngineKind::Vllm/Sglang` advertise
  `tensor_parallel: true` in their capability table, but DecentraAI's production
  engine is llama-server (single-worker, whole-request routing). Distributed
  execution scaffolding (plan types `Sequential`/`FanOut`) exists but is only
  emitted when an engine advertises the split capability, which no running engine
  does. The fabric routes whole requests to one worker.
- GAP: a real `ggml-rpc-server`/`--rpc` tensor-split path, with a 
  two-node latency/throughput measurement.
- BENEFIT: single large model larger than one node's VRAM could run across nodes.
- RISK: the RPC backend is explicitly **proof-of-concept, fragile and insecure**
  ("Never run the RPC server on an open network or in a sensitive environment!").
  Version skew across llama.cpp builds, network latency/bandwidth sensitivity,
  and failure behavior are unvalidated.
- COST: spawn/manage `ggml-rpc-server` per node; build variants per accelerator;
  substantial testing on real hardware.
- RECOMMENDATION: **EXPERIMENT** (isolated, LAN-only, behind the existing
  capability gate). Do NOT enable as a default; requires real two-node
  measurements first (ROADMAP gate).

## GPU scheduling / KV-cache locality

- SOURCE: repository evidence (M19 `NetworkGraph`, M20 `SessionAccount`,
  `KvPlanner`, `prefill_decode_separation` gate).
- WHAT IT DOES: KV-aware routing already steers continuations to the worker that
  holds the session's KV prefix, and the planner folds measured network cost into
  scoring.
- CURRENT STATE: M19/M20 verified on two-node LAN.
- GAP / BENEFIT: none required now; mature patterns (prefill/decode separation,
  expert routing) remain parked behind gates until a real engine advertises them.
- RECOMMENDATION: **ADAPT** (already adopted); keep the gates honest.

## OpenTelemetry GenAI + MCP + agent control plane

- SOURCE: OTel GenAI semantic conventions (Phase 8), MCP read-only tools.
- CURRENT STATE: `gen_ai.*` Prometheus projection + MCP `decide` / fabric graph /
  worker capability / intent tools are live.
- RECOMMENDATION: **ADAPT** (adopted); add OTel trace export only when an external
  collector is actually used.

## Self-healing distributed systems

- SOURCE: repository (Phase H `recovery_timeline`, `adapt`/`orchestrate`,
  bounded reconnect, request retry).
- RECOMMENDATION: **ADOPT** (already implemented); keep advisory-only and honest.

## Model routing

- SOURCE: planner + unified decision (`decide`).
- RECOMMENDATION: **ADAPT**; extend to explicit `intent -> capability -> model ->
  variant -> worker -> reservation` once a mutation path is justified.

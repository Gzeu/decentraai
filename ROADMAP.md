# DecentraAI MVP Roadmap

## 1. Foundation (done)
- [x] Workspace, CI, and YAML config loader
- [x] `decentraai init` bootstrap
- [x] System probe + admission checks (`decentraai doctor`)
- [x] Model registry scan/list commands

## 2. Identity and Networking (done)
- [x] Ed25519 identity management with secure persistence
- [x] Message schema and canonical signing
- [x] Model manifest generation with Merkle root
- [x] libp2p transport with Noise + mDNS
- [x] Chunk transfer engine with per-chunk verification and resume
- [x] End-to-end test: two nodes exchange a real model

## 3. Runtime (done)
- [x] M4a: llama-server process manager crate (`decentraai-runtime`)
- [x] M4b: inference admission gate + `decentraai serve start`
- [x] M4c: OpenAI-compatible API endpoint with Bearer auth and idle unload
- [x] Fixed configurable API port + friendly root info page

## 4. Swarm Intelligence (done)
- [x] Reputation and peer scoring (M5a: bans, decay, persistence)
- [x] Manifest announcements + registry-backed serving (M5b)
- [x] Deterministic multi-provider scheduler (M5c: ranked waves + fallback)

## 5. Hardening (done)
- [x] Quarantine workflow for corrupted artifacts (metadata + reason)
- [x] RAM admission fix (reject below the configured reserve)
- [x] Security audit log (bans, admission rejections, verification failures)
- [x] M8: packaging — `scripts/install.sh` + `docs/deployment.md`
  (systemd unit, firewall, security checklist, troubleshooting)

## 6. Sharing and UX (done)
- [x] M7a: peer catalog + `decentraai pull` (share models with one command)
- [x] M7b: web dashboard on the API port
- [x] M7c: dashboard v2 — real inference metrics (tokens, tok/s, recent
  calls, uptime, RAM/GPU), self-poll fix (watching the page no longer
  inflates the counter or blocks idle unload)

## 7. Subscriptions: free, tiered by contribution (in progress)
- [x] P1: token registry (`db/tokens.json`, hashed) + `decentraai token`
  CLI + tiered auth in the proxy (per-tier model allowlist, sliding-window
  rate limit, usage counters, audits)
- [ ] P2: chat UI in the dashboard (model selector filtered by tier)
- [ ] P3: admin dashboard (create/revoke tokens, usage per token)
- [ ] P4: contribution-based tier suggestions from catalog + reputation
- [ ] P5: invites (`decentraai join <invite>`)

## 8. Operations and scale (in progress)
- [x] Q1: generation defaults (sampling + system prompt merged into
  requests), interactive model picker with memory-fit verdicts,
  dashboard lists every indexed model
- [x] Q2: fair FIFO queue for inference requests — one request at a
  time reaches the backend with full resources, 503/504 on full/timeout,
  Queue card on the dashboard shows serving + waiting live
- [ ] Q3: remote backend (`serve start --backend http://host:port`) —
  a weaker station keeps auth/tiers/queue while a stronger machine runs
  the model
- [ ] Q4: onboarding wizard (`decentraai setup`) writing a validated
  config on first run

## 9. Distributed Inference (M9) - IN PROGRESS
- [x] M9-1: Design distributed inference architecture (worker discovery, request routing)
- [x] M9-2: Implement worker registration with real-time capacity reporting
- [x] M9-3: Add request routing to peer GPUs based on capacity
- [x] M9-4: Implement fallback mechanism when workers fail
- [x] M9-5: Queue management with FIFO processing and timeout support
- [ ] M9-6: P2P protocol extensions for WorkerAnnouncement and InferRequest
- [ ] M9-7: Inference request handler for workers (real llama-server adapter)
- [ ] M9-8: Real-time capacity updates from runtime
- [ ] M9-9: Reputation-based compensation for workers

## 10. Complete End-to-End Flow (M10)

### Acceptance Criteria
- [ ] Node starts with `decentraai init` and validated configuration
- [ ] Worker is paired via QR or token and approved from dashboard
- [ ] Worker publishes models, capacity and real-time status
- [ ] Client sends prompt from frontend
- [ ] Router selects eligible worker
- [ ] Request transmitted via authenticated P2P
- [ ] Worker calls real llama-server (not mock handler)
- [ ] Streaming response to frontend via SSE/WebSocket
- [ ] Timeout, retry and fallback work correctly
- [ ] Queue depth, latency, P50/P95/P99 and success rate in dashboard
- [ ] Each request produces audit event with request ID, worker ID, model hash, status
- [ ] Offline worker detected and excluded from routing
- [ ] E2E test can start two local nodes and reproduce full flow

### Implementation Phases

#### Phase 1: Common Contracts
- [ ] Stabilize NodeConfig, WorkerAnnouncement, InferRequest, InferResponse
- [ ] Stabilize WorkerStatus, TaskPlacement
- [ ] Define error codes and retry semantics
- [ ] Request lifecycle: received → queued → assigned → running → completed/failed/timeout
- [ ] Add mandatory request_id, trace_id, created_at, deadline_at, model_hash, sender_peer_id, assigned_worker_id

#### Phase 2: Real Data Plane
- [ ] Fix queue manager: shared state, not clone
- [ ] Install effective inference handler in ChainedHandler
- [ ] Adapter for llama-server/OpenAI-compatible API
- [ ] Streaming incremental tokens
- [ ] Backpressure and bounded queues
- [ ] Retry only for transient errors
- [ ] Circuit breaker for unstable workers
- [ ] Idempotency for resent requests
- [ ] Server-side limits for timeout, tokens, prompt size

#### Phase 3: Trust and Security
- [ ] Verify WorkerAnnouncement signature
- [ ] Compare announcement.peer_id with transport peer ID
- [ ] Pairing with expiration and revocation
- [ ] Replay protection via nonce/sequence number
- [ ] Capability-based authorization: which models each worker can serve
- [ ] Rate limiting per token and per peer
- [ ] Limit prompt size and output size
- [ ] Secret management without tokens in config or logs
- [ ] Audit for login, pairing, revoke, routing and inference
- [ ] Role separation: admin, operator, client, worker

#### Phase 4: Control Plane and Frontend
- [ ] Onboarding: create node, generate identity, pairing QR, health check
- [ ] Chat: conversations, streaming, stop generation, retry, model selection
- [ ] Workers: approve/revoke, status, models, capacity, latency
- [ ] Models: registry, hash, quantization, context size, availability
- [ ] Network: peers, trust, latency, connection errors
- [ ] Observability: logs, metrics, traces, alerts
- [ ] Admin: tokens, roles, quotas, audit events
- [ ] Settings: node config, inference defaults, limits, retention

### Scoring Rubric (Target: 9/10)

| Domain | Target for 9/10 |
|--------|-----------------|
| Core inference | Real request to llama-server, streaming and cancellation |
| Distributed routing | Worker selection, fallback, timeout and circuit breaker |
| Security | Verified identity end-to-end, pairing and authorization |
| Reliability | Restart recovery, bounded queues, idempotency and health checks |
| API | Versioned contracts, OpenAPI, consistent errors and rate limits |
| Frontend | Chat, workers, models, logs, metrics and admin integrated |
| Observability | Metrics, structured logs, audit and correlation IDs |
| Testing | Unit, integration, adversarial and two-node E2E |
| Deployment | Docker Compose for local and reproducible deployment |
| Documentation | Real quickstart, architecture diagrams, operations runbook |
| Developer experience | CI for Rust + frontend + security + E2E |

### Recommended PR Structure

1. **fix(M9)**: shared queue state and request lifecycle
2. **feat(inference)**: real llama-server adapter with streaming
3. **feat(security)**: authenticated worker announcements
4. **feat(api)**: node control plane and SSE inference endpoint
5. **feat(frontend)**: connect chat and worker management to live API
6. **feat(reliability)**: retries, fallback, circuit breakers and recovery
7. **test(e2e)**: two-node distributed inference scenario
8. **ci**: add frontend, security, integration and E2E gates
9. **docs**: production quickstart and operations runbook
10. **release**: M10 production vertical slice

# DecentraAI Fabric Communication Protocol (DFCP)

**Status:** Draft v0.1
**Milestone:** M14
**Branch:** `feature/m14-dfcp`

## 1. Purpose

DFCP defines the node-to-node communication contract used by DecentraAI to share and coordinate **compute resources**, not only model artifacts.

The protocol must allow a node to:

- advertise available CPU, RAM, GPU/VRAM, KV, storage and network capacity;
- request additional resources when local capacity is insufficient or inefficient;
- offer capacity to another trusted node;
- assign executable work to one or more workers;
- return results and execution evidence;
- maintain liveness and capacity freshness;
- preserve node-owner sharing policies and trust boundaries.

## 2. Core principle

> **Resource Sharing, not Model Sharing.**
>
> Models remain where they are most useful. Compute, memory and supporting work can be coordinated across the fabric when the planner determines that distribution is beneficial.

Human-facing principle:

> **Share what you have. Use what you need. Verify every contribution.**

## 3. Scope of v0.1

DFCP v0.1 is a protocol contract only. It does not yet implement:

- distributed tensor execution;
- transparent shared virtual memory;
- WAN-untrusted scheduling;
- economic settlement or cryptocurrency;
- arbitrary code execution on peer nodes.

The first implementation target is **trusted resource coordination** for auxiliary compute and future distributed execution.

## 4. Node identity and trust

Every DFCP message is associated with the existing DecentraAI node identity and authenticated transport.

A node may only consume resources according to the existing trust/admission policy. Resource sharing must never bypass:

- peer authentication;
- trust state;
- owner sharing policy;
- capability/readiness checks;
- admission controls;
- request authorization.

## 5. Message types

### HELLO

Initial protocol negotiation and node capability summary.

Required fields:

- `protocol_version`
- `node_id`
- `message_id`
- `timestamp_ms`
- `capabilities`
- `sharing_policy`

### RESOURCE_ADVERTISE

Periodic or event-driven publication of currently available resources.

Required resource dimensions:

- CPU logical/available capacity;
- RAM total/available;
- GPU identity and utilization;
- VRAM total/available;
- KV capacity and usage;
- storage capacity;
- network bandwidth and measured latency;
- served models;
- execution capabilities.

Advertisements are **time-bounded observations**, not permanent promises.

### RESOURCE_REQUEST

A node asks the fabric for additional capacity.

Example needs:

- CPU assistance;
- RAM/KV capacity;
- GPU assistance;
- preprocessing;
- embeddings;
- reranking;
- RAG retrieval;
- future model-parallel execution.

The request must contain a unique `request_id`, constraints, deadline, priority and failure policy.

### RESOURCE_OFFER

A peer proposes concrete capacity for a resource request.

An offer must include:

- `request_id`
- `offer_id`
- provider `node_id`
- offered resources
- expected latency
- validity/expiry
- policy constraints
- optional execution capabilities

### TASK_ASSIGN

Authorizes execution of a bounded task against an accepted resource offer.

Tasks must be identified independently from the parent request so retries and duplicates remain observable.

### TASK_RESULT

Returns task completion state, result metadata and evidence references.

Raw prompts, secrets, bearer tokens and private keys must never be emitted as telemetry.

### HEARTBEAT

Liveness and freshness message carrying lightweight resource health information.

## 6. Resource model

A resource advertisement is modeled as a snapshot:

```text
ResourceSnapshot {
  cpu: CpuCapacity,
  memory: MemoryCapacity,
  gpu: [GpuCapacity],
  kv: KvCapacity,
  storage: StorageCapacity,
  network: NetworkCapacity,
  models: [ServedModel],
  capabilities: [ExecutionCapability]
}
```

Capacity is divided into:

- **total** — physically or logically available;
- **allocated** — reserved by active work;
- **free** — currently schedulable;
- **shareable** — owner-authorized portion;
- **observed_at** — timestamp of measurement.

Scheduler decisions must use fresh snapshots and reservations, not stale advertisements alone.

## 7. Workload model

A workload declares requirements independently of any specific worker:

```text
WorkloadRequirements {
  cpu,
  ram,
  vram,
  gpu_compute,
  kv,
  bandwidth,
  max_latency_ms,
  deadline_ms,
  model,
  capabilities,
  priority,
  distribution_mode
}
```

`distribution_mode` values for v0.1:

- `local_only`
- `assist`
- `distributed`

Future values may include `pipeline_parallel` and `tensor_parallel`.

## 8. Compute-assist model

A node running the primary inference may request assistance without moving the primary model.

Examples:

```text
PRIMARY NODE
  inference
    ├── embeddings  -> Worker B
    ├── preprocessing -> Worker C
    ├── retrieval -> Worker D
    └── KV spill/cache -> Worker E
```

This is the preferred first implementation because it reduces network synchronization cost compared with per-token model sharding.

## 9. Reservation semantics

Resource offers become usable only after a reservation is created.

Reservation requirements:

- unique reservation ID;
- owner/provider node identity;
- resource quantities;
- creation timestamp;
- TTL/deadline;
- parent request ID;
- idempotent release/expiry;
- no double allocation.

A stale advertisement must never be treated as a reservation.

## 10. Failure and retry model

DFCP must assume nodes can disappear during work.

Required semantics:

- bounded retry;
- retryable vs terminal failure;
- explicit cancellation;
- reservation release on failure;
- no duplicate credit for retried tasks;
- task/result correlation;
- parent request remains traceable across retries.

## 11. Observability and evidence

Every accepted distributed task must be correlatable through:

```text
request_id
  -> reservation_id
  -> task_id
  -> provider_node_id
  -> result
  -> execution evidence
```

The existing DecentraAI SelectionTrace/EvidenceChain/verified contribution infrastructure should be extended rather than replaced.

## 12. Scheduling principle

The fabric should not distribute work merely because other nodes are available.

The planner should compare local and distributed execution using a benefit model that considers:

```text
benefit =
  compute_gain
+ memory_gain
+ kv_locality
+ parallelism_gain
- network_latency_cost
- transfer_cost
- serialization_cost
- contention_risk
- failure_risk
```

If local execution is faster and satisfies the workload, keep it local.

If local execution is impossible or materially worse, create a distributed plan.

## 13. Security invariants

DFCP must preserve these invariants:

1. A peer cannot allocate resources it was not offered.
2. A node cannot consume resources outside owner policy.
3. A task cannot be credited without verifiable execution evidence.
4. Retries cannot double-credit work.
5. Expired reservations cannot be reused.
6. Stale advertisements cannot create implicit capacity.
7. Secrets and raw model prompts/results are not placed in operational telemetry.
8. Remote execution remains opt-in through existing trust/admission controls.

## 14. Implementation sequence

### M14.1 — Protocol types

Create typed Rust message structures and serialization tests.

### M14.2 — Capability advertisement

Extend existing compute advertisements with explicit free/shareable capacity and freshness.

### M14.3 — Resource request/offer

Implement trusted peer request/offer exchange without task execution.

### M14.4 — Reservation integration

Connect DFCP offers to the existing reservation ledger.

### M14.5 — Compute-assist vertical slice

Demonstrate one inference request where a primary node delegates auxiliary CPU work to a trusted peer.

### M14.6 — Evidence integration

Record provider identity, reservation, task lifecycle and result in the existing evidence chain.

### M14.7 — Failure/recovery

Kill or disconnect an assisting worker and verify bounded retry/fallback with no duplicate credit.

## 15. Future protocol directions

Later milestones may add:

- distributed memory segments;
- remote KV ownership/migration;
- DAG execution;
- pipeline parallelism;
- tensor parallelism;
- cross-node model execution;
- adaptive cost/latency optimization;
- contribution-aware scheduling;
- federation across independently operated fabrics.

## 16. Definition of done for M14

M14 is complete only when a reproducible two-node test demonstrates:

1. authenticated node discovery;
2. real resource advertisement;
3. resource request/offer exchange;
4. reservation;
5. task assignment;
6. task result;
7. worker failure handling;
8. evidence correlation;
9. no duplicate reservation or contribution credit;
10. all existing workspace tests remain green and clippy remains clean.

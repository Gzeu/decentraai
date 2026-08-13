# DecentraAI Product Status

Updated for the current `feat/roadmap-scaffold` implementation and forward product direction.

## Product

DecentraAI is a decentralized AI compute network: people connect computers/GPUs and DecentraAI turns that capacity into verified, automatically orchestrated distributed AI infrastructure.

**This is not a GPU marketplace, not a cloud server product, and not a llama-server wrapper.** Engines such as llama.cpp/llama-server, vLLM or SGLang are execution backends. DecentraAI is the orchestration layer that discovers heterogeneous compute, verifies and provisions models, plans execution, schedules workloads, and adapts to network and runtime conditions.

## Current implementation status

- M10 — Real distributed inference: DONE
- M11 — Capability-aware compute sharing: DONE
- M12 — Real hardware advertisement: DONE
- M13 — Capability-aware routing and reservations: DONE
- M14 — On-demand model provisioning: DONE
- M15 — Worker-side reservation enforcement: DONE
- M16 — Live compute metrics: DONE
- M17 — Contribution-based tier suggestions: IN PROGRESS

## Proven real path

```text
Coordinator
  -> P2P InferRequest
  -> Worker queue
  -> OpenAI-compatible backend
  -> real llama-server
  -> real GGUF model
  -> streamed InferProgress
  -> terminal InferResponse
```

Cancellation is supported through the real P2P path and the worker remains usable after a cancelled request.

## Compute orchestration

Workers advertise real compute capabilities including GPU/VRAM/RAM/CPU, model availability, health, load and live performance. The scheduler uses capability matching and resource reservations, with worker-side enforcement preventing overcommit.

## Model provisioning

Workers can provision a required model on demand when policy allows. The verified transfer pipeline downloads and verifies the model before indexing and serving it. The CLI can spawn a real `llama-server` for the provisioned model.

## Metrics

Live runtime metrics include queue depth, measured tokens/sec, measured latency, completed/failed request totals, and capacity/reservation state. Compute advertisements carry live performance data so scheduling can account for actual worker performance.

## M17 direction

M17 completes the contribution layer around verified compute served, using hardware, time served and verified requests to drive tier suggestions. After M17, the product path continues toward a complete Ubuntu click-to-run node lifecycle: hardware detection, model detection/provisioning, runtime startup, trust/pairing, network discovery, compute contribution, inference, streaming and recovery.

# Forward Technology Roadmap

These milestones extend the current implementation without changing the product goal. They are intentionally focused on intelligent distributed execution rather than building a marketplace or hosting service.

## M18 — Distributed Execution Engine

Move from selecting one capable worker to constructing an execution plan when a workload cannot or should not run on one node.

- Model/layer placement across heterogeneous workers.
- Pipeline and tensor-parallel execution primitives where supported by the backend.
- Execution graph with worker-to-worker data flow.
- Bandwidth- and latency-aware placement.
- Dynamic worker availability and failure-aware replanning.
- Keep the engine abstraction backend-neutral; do not couple DecentraAI to one inference server.

## M19 — Network-Aware Scheduler

Make network conditions first-class scheduling inputs.

- Peer-to-peer latency and bandwidth measurements.
- Topology and link-quality awareness.
- Congestion-aware routing.
- Transfer-cost estimation for model and intermediate state movement.
- Reliability and historical performance in placement scoring.
- Choose an execution plan, not merely a worker.

## M20 — KV-Aware Inference Fabric

Treat KV cache as a schedulable distributed inference resource.

- Prefill/decode separation where the selected backend supports it.
- KV-cache locality and reuse.
- KV transfer cost in scheduling.
- Context-aware routing.
- Cache-aware worker selection.
- Pluggable KV storage/transport so the architecture can evolve with inference engines.

## M21 — Distributed MoE / Expert Fabric

Extend execution planning to sparse expert models.

- Distributed expert placement.
- Expert-aware routing.
- Load-aware expert selection.
- Minimize cross-node expert traffic.
- Rebalance experts as node capacity changes.

## M22 — Multi-Engine Runtime

Decouple DecentraAI orchestration from any single inference runtime.

```text
                 DecentraAI
                     |
        +------------+------------+
        |            |            |
    llama.cpp       vLLM        SGLang
        |            |            |
      worker       worker       worker
```

The orchestration layer should select an execution backend according to workload, hardware and deployment constraints.

## M23 — Autonomous Execution Planner

Unify compute, model, network and runtime signals into an execution planner.

```text
request
  -> workload requirements
  -> available compute
  -> model locality
  -> network topology
  -> runtime performance
  -> KV/cache state
  -> trust/reliability
  -> execution plan
  -> adaptive execution
```

The planner should continuously adapt when workers slow down, disappear, recover, or become more capable.

## M24 — Resilient Decentralized AI Fabric

Complete the network-level behavior required for a real decentralized infrastructure.

- Automatic worker admission and trust lifecycle.
- Failure detection and recovery.
- Execution-plan failover.
- Model integrity and supply-chain verification.
- Secure peer communication.
- Abuse/resource protection.
- Reputation based on verified compute actually served.
- Privacy and tenancy boundaries.

## Long-term product outcome

The final user experience should be:

```text
INSTALL / CLICK
    -> detect hardware
    -> detect or obtain compatible model
    -> verify model
    -> start local runtime
    -> create node identity
    -> establish trust
    -> discover network
    -> advertise compute
    -> READY

PROMPT
    -> understand workload
    -> select execution plan
    -> reserve compute
    -> provision model if required
    -> run local or distributed inference
    -> stream result
    -> observe performance
    -> recover/replan on failure
    -> release resources
```

The user should not need to understand workers, VRAM, model placement, inference engines, P2P routing, KV cache, or execution topology.

**DecentraAI's goal is to make heterogeneous computers behave like one intelligent, verified AI compute fabric.**

## Technology radar

The roadmap should continuously track advances in:

- distributed inference and model sharding;
- pipeline/tensor parallelism;
- prefill/decode disaggregation;
- KV-cache transport, locality and scheduling;
- topology-aware distributed execution;
- heterogeneous GPU/CPU inference;
- llama.cpp, vLLM, SGLang and other inference runtimes;
- decentralized trust, verification and reputation;
- privacy-preserving and secure compute.

New technology should be adopted when it improves the DecentraAI execution fabric, but the product scope remains orchestration of decentralized AI compute rather than a marketplace or a single inference server.

## Validation policy

The production inference path uses real GGUF models and real inference runtimes. Mocks are reserved for isolated tests only. Tests are focused gates; production implementation and real integrations are the primary deliverables.

## Development branch

Primary work branch: `feat/roadmap-scaffold`.

Keep PR #25 draft until the complete production evidence and final integration gates are satisfied.

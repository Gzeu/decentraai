# DecentraAI Product Status

Updated for the current `feat/roadmap-scaffold` implementation.

## Product

DecentraAI is a decentralized AI compute network: people connect computers/GPUs and DecentraAI turns that capacity into verified, automatically orchestrated distributed AI infrastructure.

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

## Validation policy

The production inference path uses real GGUF models and real `llama-server`. Mocks are reserved for isolated tests only. Tests are focused gates; production implementation and real integrations are the primary deliverables.

## Development branch

Primary work branch: `feat/roadmap-scaffold`.

Keep PR #25 draft until the complete production evidence and final integration gates are satisfied.

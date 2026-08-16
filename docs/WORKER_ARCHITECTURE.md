# Worker Architecture

This document describes the **lightweight worker** model in DecentraAI: what a
worker is, what it links, what it advertises, how it is born/lives/dies, how it
abstracts the platform, and how it will (eventually) adapt to constrained
devices.

It is grounded in the actual source, in particular:

- `crates/node-cli/src/bin/decentraai-worker.rs` — the standalone worker binary.
- `crates/distributed/src/compute.rs` — `ComputeManager` worker-side methods.
- `crates/distributed/src/lib.rs` — `DistributedInference::register_as_worker` /
  `register_worker_backend` (the full inbound path).
- `crates/compute/src/capability.rs` — `ComputeCapability` / `ServedModel`.
- `crates/compute/src/availability.rs` — `ComputeAvailability` / `ComputeAdvertisement`.
- `crates/system-probe`, `crates/runtime` — probes and the engine adapter.
- `crates/fabric/src/plan.rs` / `engine.rs` / `advisory.rs` — gated execution shapes.

> **Honesty rule.** Everything marked *today / current* is implemented and
> measured. Everything under *Future / plan* is a contract or direction, NOT
> implemented. Nothing in this document should be read as claiming a feature
> works when it does not.

---

## 1. Control plane vs worker plane

`decentraai-distributed` provides both halves. Which half a node exercises is a
matter of which methods it calls, not which crate it links.

### Control Node owns (decisions, orchestration, history)

- Planner / scheduler — decides *who* runs *what* (`route_request`,
  `route_request_streamed`, network probe, worker reaper).
- Model hub + registry scan — acquiring and indexing models.
- Dashboard / API / MCP — the user-facing control surface.
- Token/API auth — `decentraai-tokens`.
- Orchestration, policy, history, fabric coordination.
- Contribution / compensation accounting.
- Metrics server.

### Lightweight Worker needs only

- **Identity** — reuse the node's Ed25519 key (`identity/key.pem`); derive the
  libp2p keypair from it. Never regenerates a random identity.
- **Join the fabric** — listen on a P2P address, connect to peers.
- **Advertise** — capability/resources/engine/models + live availability
  (heartbeat).
- **Heartbeat** — re-broadcast the advertisement on a fixed interval.
- **Receive authorized work** — serve signed `InferRequest` frames from peers it
  is trusted by.
- **Execute on a local engine** — run a real `llama-server` subprocess (no
  silent mock).
- **Return results** — stream progress to the requester, emit exactly one
  terminal event (`InferResponse` / `InferFailed`).
- **Report real measurements** — measured `tokens_per_second`, latency, queue
  depth, load.
- **Expose version/platform** — `node_version`, `node_id`, `node_name`.

A worker does **not** run the planner, hub, registry scan, dashboard, MCP,
tokens, decisions, or orchestration. See the crate-level doc header in
`decentraai-worker.rs`.

---

## 2. Worker dependency boundary

The standalone worker links (directly or transitively through
`decentraai-distributed` / `decentraai-runtime`):

- `decentraai-distributed` (worker-side `ComputeManager` +
  `register_worker_backend`)
- `decentraai-runtime` — `LlamaServer`, `find_llama_server`
- `decentraai-identity`
- `decentraai-system-probe`
- and transitively: `compute`, `config`, `discovery`, `fabric`, `inference-adapter`,
  `manifest`, `p2p`, `protocol`, `registry`, `audit`.

It does **not** run in its serving path:

- `decentraai-hub` (model hub)
- `decentraai-tokens` (API auth)
- the dashboard / API / MCP

These are pulled in only transitively (via `runtime`/`distributed`) and are never
exercised by the worker code path.

> The worker still signs its advertisements (P3) and verifies inbound request
> signatures; it reuses the existing identity/trust/auth machinery rather than
> duplicating it.

---

## 3. Worker contract / advertisement

A worker broadcasts a `ComputeAdvertisement` = static `ComputeCapability` +
live `ComputeAvailability` (+ a few envelope fields), serialized and announced
over P2P. `announced_at_ms` timestamps each beat.

### Envelope
- `peer_id` — libp2p peer id.
- `node_id` — compact stable id (e.g. `dca-8f2a3c`) derived from the peer id.
- `node_name` — human-readable name.
- `node_version` — build/version string; empty/unknown on old nodes.
- `accepts_remote_inference` — honest opt-in for remote sharing
  (`config.inference.allow_remote_inference`); old nodes default to `false`.

### Capability (static)
- `cpu_cores`, `ram_mb` (total).
- `gpu: Option<GpuSpec>` — name, `vram_mb`, driver; `None` = CPU-only.
- `engine` — e.g. `llama_server`.
- `served_models: Vec<ServedModel>` — model_hash, file_name, size_mb,
  `est_ram_mb`, `est_vram_mb`, `context_tokens` (real `--ctx-size`; `0` = unknown).
- `available_models` — on-disk models not currently loaded.
- `can_provision` — whether the worker will fetch a missing model on demand.

### Availability (per-heartbeat, real/measured)
- `available_ram_mb`, `available_vram_mb: Option<u64>`.
- `load_percent`, `queue_depth`, `tokens_per_second`, `current_latency_ms`.
- `status: WorkerHealth` — `Ready` / `Busy` / `Degraded` / `Unhealthy` /
  `Offline` (the last is coordinator-side only).

### Honesty contract
- **Values are real and measured** from `SystemSnapshot::collect()` +
  `probe_gpu()` + measured runtime metrics at each beat.
- **UNKNOWN stays UNKNOWN** — never fabricated. `gpu: None` for CPU-only, a
  missing/unknown `node_version` stays empty, `context_tokens: 0` for unknown KV
  capacity. Memory budgets (`est_ram_mb`/`est_vram_mb`) are conservative
  estimates, not exact allocations.
- A worker advertises only what it actually serves (a real model + real engine).

### Today vs Future fields
| Field | Today | Future |
| --- | --- | --- |
| identity / node_id / node_name | ✅ | |
| node_version / platform | ✅ (version) | platform string |
| CPU / RAM / GPU / VRAM | ✅ | |
| engines / models / capabilities | ✅ | |
| availability (load, queue, tps, latency) | ✅ | |
| battery state | | 🔒 mobile plan |
| thermal state | | 🔒 mobile plan |
| foreground / background | | 🔒 mobile plan |
| network quality | | 🔒 mobile plan |
| user contribution limit | | 🔒 plan (adaptive contribution) |

The 🔒 rows are **plans**, not implemented; see §6.

---

## 4. Worker lifecycle

Evidence-backed states, each mapped to a real mechanism:

| State | Mechanism |
| --- | --- |
| **DISCOVERED** | Signed advertisement (`advertisement_wire_bytes`) broadcast over mDNS; coordinators see the peer + its capability. |
| **TRUSTED** | A coordinator adds the peer id to its trust store (same mechanism used for any node). |
| **CONNECTED / READY** | Worker is listening on a P2P address and advertising (heartbeat running). |
| **BUSY** | Serving or queued; reported via `availability.status` / queue depth. |
| **OFFLINE** | Heartbeat lapses past the stale window; coordinator marks `Offline` (`reap_unhealthy`). |

- The first advertisement is sent immediately, then the broadcaster re-advertises
  on `advertisement_interval_ms`.
- **UPDATING / VERIFIED are NOT emitted** — there is no remote update mechanism.
  A worker never claims a state it cannot back with evidence.

---

## 5. Platform abstraction

The worker path is effectively **platform-neutral**. Clean boundaries keep the
core worker logic portable while probes/engines are swappable per platform.

### Platform-independent core
- Identity, P2P join, advertisement, request handling, admission, reservation
  ledger, metrics, version reporting.

### Platform-specific probes — `decentraai-system-probe`
- `SystemSnapshot::collect()` via `sysinfo`; optional `probe_gpu()` via
  `nvidia-smi` that **degrades gracefully** (no GPU → `gpu: None`, never a
  fabricated value).

### Engine adapters — `decentraai-runtime`
- `LlamaServer::spawn` / `find_llama_server` start a real `llama-server`
  subprocess via cross-platform `std::process`; killed on drop.

Because these are the only platform-touching pieces, a future Windows / ARM /
Android port needs only its own probe + engine adapter — the core worker code
does not change.

### Practical packaging boundary (by platform)

| Platform | Packaging | Notes |
|---|---|---|
| Linux | `cargo build --release --bin decentraai-worker`; systemd user unit optional | `scripts/install-app.sh` installs both binaries; llama-server from distro/built PATH |
| Windows | build the same bin; run as a console service or scheduled task | llama-server.exe on PATH/`--binary`; process model is cross-platform |
| ARM (e.g. Raspberry Pi) | same Rust bin; optional container | CPU-only probe path; no nvidia-smi (degrades to `gpu: None`) |
| Android / mobile | FUTURE — not supported | needs its own NPU/thermal/battery probe + engine adapter; the worker contract is unchanged |

The worker **contract** (advertisement fields, join flow, signed P2P) is
identical across platforms; only probes and engine adapters differ. No single
update mechanism is assumed (see `docs/deployment.md`).

---

## 6. Mobile readiness & adaptive contribution

### Mobile advertisement extensions (CONTRACT / PLAN — not implemented)
Future advertisement fields that a constrained/mobile device would add:
- battery state (level, charging)
- thermal pressure
- foreground / background (app visibility)
- network quality
- CPU / GPU / NPU
- available memory
- user contribution limits

**These are plans only.** There is no fabricated mobile telemetry today — a
worker advertises only real, measured values.

### How the scheduler would later adapt (direction, not implemented)
When a device reports low battery / high thermal / busy / degraded network / or
the user disables contribution, the scheduler should **reduce or stop**
workload routed to it — driven entirely by what the worker honestly advertises.

### Adaptive contribution direction
- **Desktop → high**, **Laptop → medium**, **Phone → limited**.
- The worker advertises its **honest** capacity; the coordinator does not assume
  a fixed capacity.
- Today the worker already advertises real `availability` (load, queue, tps,
  latency). Future work adds **capacity / contribution limits** so the worker
  can bound how much remote work it will take.

This direction is documented here as a contract; it is not yet implemented.

---

## 7. Distributed inference boundary

DecentraAI does **not** implement `llama.cpp` RPC / tensor split / pipeline split
— a monolithic GGUF is not split across HTTP backends today.

Where multi-worker execution could later plug in (all currently **experimental /
future**, gated behind capabilities no engine advertises by default):

- **Fabric planner `PlanKind`** — `Single` (always produced/executable),
  `Sequential` (gated on `prefill_decode_separation`), `FanOut` (gated on
  callers that have such work). `Sequential`/`FanOut` are supported by the
  executor but only emitted when an engine advertises the relevant capability.
- **`supports_staging()`** (`EngineCapabilities::prefill_decode_separation`) —
  the gate that lets an engine opt into multi-stage plans. `llama-server` does
  not advertise staging today.
- **`fan_out_candidacy` advisory** (`crates/fabric/src/advisory.rs`) — a
  coordinator-side heuristic that identifies workers eligible for fan-out
  (trusted, healthy, serves the model, `supports_staging()`).

None of these represent working distributed inference — they are the seams where
it could later attach. Until an engine actually advertises the capability and
real hardware proves it, `PlanKind::Single` is the only plan produced.

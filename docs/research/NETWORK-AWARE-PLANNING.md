# Network-Aware Planning for DecentraAI

Status: MIXED (VERIFIED for RDMA/topology-aware scheduling in llm-d, GKE, and disaggregated serving; INFERRED for DecentraAI’s P2P fabric).[cite:110][cite:111][cite:117][cite:143][cite:146]

This document focuses on how DecentraAI can incorporate network measurements and topology into execution planning.

## 1. Network Metrics in Modern Runtimes (VERIFIED)

### 1.1 RDMA and Topology-Aware Scheduling

- llm-d and GKE use topology labels (e.g., `cloud.google.com/gce-topology-block`) to colocate pods within the same high-speed network fabric, improving performance for multi-host replicas and expert-parallel deployments.[cite:143][cite:146]
- NVIDIA’s disaggregated serving documentation emphasizes RDMA and NVLink for KV transfer between prefill and decode workers.[cite:110][cite:111][cite:117]

### 1.2 Practical Measurements

Common metrics:

- **RTT**: round-trip time, measured via ping or TCP handshakes.
- **Bandwidth**: measured via tools like iperf or inferred from sustained transfer rates.
- **Jitter**: variability in latency, relevant for real-time streaming.
- **Packet loss**: impacts reliability and quality, particularly over Wi‑Fi.
- **Connection stability**: measured via error rates and reconnection frequency.

These are all practically measurable by a fabric like DecentraAI.

## 2. NetworkFacts for DecentraAI (INFERRED)

Extend DecentraAI with a `NetworkFacts` structure per worker and per link:

- Per-link metrics:
  - `rtt_ms: f64` (MEASURED via probes).
  - `bandwidth_mbps: f64` (MEASURED via periodic tests).
  - `jitter_ms: f64` (MEASURED as variance).
  - `packet_loss_pct: f64`.
  - `link_type: LinkType` (LAN, Wi‑Fi, WAN, RDMA, NVLink).
- Provenance:
  - MEASURED (from probes).
  - INFERRED (from OS/hardware metadata).
  - UNKNOWN (no measurements yet).

`NetworkGraph` already exists; it can be populated with these metrics.

## 3. Network-Bound vs Compute-Bound Strategies (INFERRED)

ExecutionStrategy selection must account for whether a strategy is likely to be network-bound:

- **Single-worker strategies**: mostly compute-bound; network only affects client–worker path.
- **Disaggregated prefill/decode**: sensitive to KV transfer cost; can become network-bound if KV is large and bandwidth is low.[cite:110][cite:111][cite:117]
- **Speculative draft/verify**: network cost is smaller (tokens and possibly hidden states), but high RTT can still hurt acceptance benefits.
- **Distributed KV cache sharing**: network-bound when caches are large and stored on remote devices.

Planner should estimate per-strategy network cost:

- `network_cost_bytes = KV_size_bytes + draft_payload_bytes`.
- `network_time = network_cost_bytes / bandwidth + RTT`.

Strategies with `network_time` comparable to or larger than compute time should be downgraded.

## 4. Measurement Regimes (INFERRED)

### 4.1 Active Probing

- Periodically run:
  - ICMP pings between nodes.
  - TCP handshake latency tests.
  - iperf-like bandwidth tests (short, low-impact).

### 4.2 Passive Observation

- Record actual transfer times for KV and draft payloads during inference.
- Use these MEASURED values to refine estimates for similar paths.

### 4.3 Adaptation

- If network metrics degrade (e.g., RTT spikes, packet loss increases), planner should:
  - Prefer single-worker or local strategies.
  - Reduce reliance on disaggregated or speculative cross-node strategies.

## 5. Recommendations (GO / EXPERIMENT / WAIT)

- **GO NOW**:
  - Implement NetworkFacts and per-link measurement probes between DecentraAI nodes.
  - Integrate network metrics into ExecutionStrategy scoring.

- **EXPERIMENT FIRST**:
  - Run controlled experiments on 1 GbE, 2.5 GbE, 10 GbE, and Wi‑Fi to quantify when disaggregated and speculative strategies become network-bound.

- **WAIT**:
  - Complex topology-aware replication schemes until simpler network-aware planning is well understood.

Network-aware planning is a necessary foundation for all distributed strategies; DecentraAI should build this measurement and scoring layer early.

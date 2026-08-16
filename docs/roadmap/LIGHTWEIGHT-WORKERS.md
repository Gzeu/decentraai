# Lightweight Workers & Mobile Fabric

## Status

**ROADMAP — PLANNED**

This initiative extends DecentraAI so that a full DecentraAI control plane does not need to run on every device contributing compute.

The control plane remains authoritative for identity, trust, policy, capability matching, resource-aware decisions, orchestration, history and observability. A participating device may run only a lightweight worker/client that connects to the fabric and exposes the resources and execution capabilities it can honestly provide.

## Product Goal

Allow additional devices to become fabric workers without installing the full DecentraAI control plane.

Target device classes include:

- Linux desktops and servers
- Windows PCs
- macOS systems
- ARM boards such as Raspberry Pi
- NAS / appliance-class systems where a supported runtime is available
- Android phones and tablets
- future mobile or embedded devices

A device should contribute only what it can safely and reliably provide. The fabric must never assume that every worker supports the same engines, models, capabilities or resource classes.

## Architecture

```text
                         DECENTRAAI CONTROL NODE

                 Hub / Planner / Registry / Policy
                 Decision / Orchestration / MCP / UI
                                |
                         Worker Protocol
                                |
        +-----------------------+-----------------------+
        |                       |                       |
   Full Worker             Full Worker          Lightweight Worker
   Linux/Windows           Linux/macOS           Mobile/Embedded
        |                       |                       |
      GPU/CPU                GPU/CPU/NPU             CPU/GPU/NPU
```

The worker is a participant in the existing fabric, not a second control plane.

## Lightweight Worker Responsibilities

The first worker contract should be intentionally small:

- persistent node identity
- authenticated connection to the control plane/fabric
- trust/policy participation using existing mechanisms
- heartbeat and liveness
- capability advertisement
- resource advertisement
- engine/model advertisement
- task reception
- execution adapter
- result reporting
- measured execution telemetry
- graceful availability changes

The worker must not duplicate:

- the Fabric planner
- Model Hub
- registry authority
- authentication/identity authority
- resource estimation system
- recovery/orchestration engine
- dashboard
- MCP control plane

## Mobile Worker

A mobile device should be treated as a **Lightweight Worker**, not as a full DecentraAI node.

A mobile worker may advertise, where the platform actually exposes the information:

- CPU capacity
- RAM availability
- GPU/NPU capability
- supported inference engines
- supported model/capability classes
- network characteristics
- battery state
- thermal state
- foreground/background availability
- device/user policy

Unknown values remain `UNKNOWN`; the worker must never fabricate measurements.

## Adaptive Contribution

Mobile and constrained devices must not be treated like desktop GPU workers.

The fabric should be able to reduce or suspend workload assignment based on real worker state, for example:

```text
NORMAL
  |
  +--> available capacity
  |
  v
LIMITED
  |
  +--> battery / thermal / load / network pressure
  |
  v
SUSPENDED
```

The exact policy must be derived from platform-supported measurements and explicit worker policy. DecentraAI must not silently drain a user's battery, overheat a device, or assume background execution is available.

## Workload Distribution

### Phase 1 — Whole-request distribution

Prioritize distributing independent requests across workers:

```text
Request queue
     |
     +---- Phone A
     +---- Phone B
     +---- Laptop
     +---- Desktop GPU
```

Each worker executes a complete request. The planner determines the appropriate share based on capability, resource availability, policy, trust, network and historical performance.

This is the preferred first implementation because it avoids requiring model-sharding support on every mobile platform.

### Phase 2 — Adaptive fan-out

Where the existing `FanOut`/staging architecture and workload semantics support it, a workload may be divided across multiple eligible workers.

Example:

```text
             workload
          /     |      \
       30%     20%      50%
        /       |         \
   Phone A   Phone B    Desktop
```

Distribution must remain explainable and bounded by actual worker capacity.

### Phase 3 — Distributed model execution

Model/tensor splitting across workers is a separate experimental capability.

Potential llama.cpp RPC/tensor-split integration must remain behind explicit capability advertisement and policy gates until real multi-node benchmarks demonstrate that it is beneficial.

It must not be assumed that adding more workers automatically makes one inference faster.

## Worker Capability Contract

A worker advertisement should continue to reuse the authoritative existing compute structures.

Conceptually:

```text
Worker
├── identity
│   ├── peer_id
│   ├── node_id
│   └── node_name
├── resources
│   ├── CPU
│   ├── RAM
│   ├── VRAM
│   ├── disk
│   └── network
├── platform
├── engines
├── models
├── capabilities
├── policy
├── trust
├── availability
└── measured performance
```

Identity fields must never be conflated.

## Security Requirements

A lightweight worker must not become a privilege escalation path.

Required principles:

- reuse existing node identity and trust
- reuse existing authentication/policy mechanisms
- no new token authority on the worker
- no arbitrary remote shell execution
- explicit worker opt-in for contribution
- explicit remote-inference policy
- least privilege for worker credentials
- signed/verified fabric messages remain authoritative
- model downloads continue using existing integrity verification
- resource limits prevent worker exhaustion
- worker can disconnect/revoke itself cleanly

Mobile onboarding must never grant the control plane access to unrelated phone data, files, microphone, camera or other private resources merely because the device contributes compute.

## Lifecycle

Target lifecycle:

```text
DISCOVERED
   |
TRUSTED / APPROVED
   |
CONNECTED
   |
WORKER READY
   |
AVAILABLE
   |
LIMITED
   |
SUSPENDED / OFFLINE
```

Version readiness should integrate with the existing `node_version` advertisement and future update-readiness work.

## Update Strategy

The worker and control plane should be version-aware.

The existing `node_version` advertisement is the first building block.

Future update workflow:

```text
Worker connects
     |
version advertised
     |
compare with fabric/control version
     |
CURRENT / OUTDATED / UNKNOWN
     |
optional approved update workflow
     |
restart worker
     |
verify new version
```

Remote updates must be explicit, authenticated and platform-specific. Do not implement arbitrary command execution as an update mechanism.

## Cross-Platform Strategy

The core worker protocol should remain platform-neutral.

Platform-specific adapters should handle:

- process lifecycle
- resource probing
- GPU/NPU discovery
- battery/thermal telemetry
- background execution restrictions
- local inference runtime
- networking constraints
- update/install mechanism

The first supported implementation should target one additional non-Linux/Windows environment before broadening to all mobile platforms.

## Roadmap

### L1 — Worker Protocol Extraction

- [ ] Identify the minimum existing code needed by a worker.
- [ ] Separate worker concerns from control-plane concerns without duplicating logic.
- [ ] Define the minimal connection/heartbeat/capability/resource contract.
- [ ] Preserve existing signed protocol and identity semantics.

### L2 — Standalone Worker Binary

- [ ] Build a `decentraai-worker` target that does not require the full dashboard/Hub/control plane.
- [ ] Join an existing fabric using existing identity/trust mechanisms.
- [ ] Advertise real resources, engines, models and capabilities.
- [ ] Receive and execute an eligible whole request.
- [ ] Report measured results.

### L3 — Cross-Platform Worker

- [ ] Validate an additional non-Linux/Windows platform.
- [ ] Add platform-specific resource probing.
- [ ] Add platform-specific engine adapters where available.
- [ ] Document unsupported capabilities honestly.

### L4 — Mobile Worker

- [ ] Android worker/client proof of concept.
- [ ] CPU/GPU/NPU capability advertisement where accessible.
- [ ] battery-aware availability.
- [ ] thermal-aware availability.
- [ ] foreground/background policy.
- [ ] explicit user opt-in and contribution limits.

### L5 — Adaptive Fabric Contribution

- [ ] Whole-request fan-out across multiple workers.
- [ ] deterministic workload limits per worker.
- [ ] historical performance-aware allocation.
- [ ] recovery when a mobile worker disappears.
- [ ] dashboard visibility for worker contribution and state.

### L6 — Experimental Distributed Inference

- [ ] Capability-gated model/tensor split.
- [ ] llama.cpp RPC experiment on a dedicated branch.
- [ ] two-node benchmark against whole-request routing.
- [ ] network bandwidth/latency measurements.
- [ ] only adopt if measured results justify the complexity.

## Acceptance Criteria

The initiative is successful when a new supported device can:

1. install only the worker component;
2. establish a trusted connection to an existing DecentraAI fabric;
3. advertise its real identity/resources/capabilities;
4. appear in the Fabric Digital Twin;
5. participate in `CAN I RUN THIS?` decisions;
6. receive eligible work without exposing the full control plane;
7. report real execution measurements;
8. be limited or suspended without breaking the fabric;
9. disconnect/reconnect without losing identity;
10. remain unable to perform control-plane operations it was not authorized to perform.

For mobile devices additionally:

- user contribution must be explicitly enabled;
- battery/thermal/resource constraints must be respected;
- unsupported platform capabilities must remain `UNKNOWN`;
- no private device data is exposed to the fabric.

## Current Status

**Planned.** The existing DecentraAI compute advertisement, identity, trust, policy, capability, resource, version and execution systems provide the foundation. The next implementation step is extraction of a minimal standalone worker from the existing authoritative components, followed by a real second-platform proof of concept.

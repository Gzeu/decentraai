# DecentraAI Control Plane v2

This branch is a visual/runtime integration track for the Command Deck. The target is a living distributed-AI control plane, not a generic admin dashboard.

## Design goals

- Make the live fabric the primary visual surface.
- Keep every status, worker, capability, model and execution indicator grounded in real runtime state.
- Preserve the single-binary embedded HTML/CSS/JS constraint.
- Use Canvas 2D for topology and motion rather than external UI dependencies.
- Keep existing views and operator functionality intact while upgrading information hierarchy.
- Make P8 Dataset/Skill and provenance visible as first-class runtime concepts.

## Target information hierarchy

1. Fabric state / topology
2. Active execution and planner state
3. Workers and models
4. Skills / capabilities / provenance
5. Queue, recovery, network and diagnostics
6. Secondary historical metrics

## Visual direction

- Near-black navy base
- Cyan as primary fabric/accent signal
- Indigo/violet for capability/remote semantics
- Emerald for verified/healthy states
- Amber for pressure/warnings
- Red only for actionable failure
- Fine borders, restrained glow, compact technical typography
- Dense but readable operator surface

## Runtime sources

The existing dashboard already derives the live surface from read-only endpoints including `/status`, `/v1/peers`, `/v1/compute`, `/v1/network`, and `/v1/execution`. The redesign must continue to use those sources and must not introduce fake values.

## P8 surface

The Skills/Cabilities area should expose the real Dataset → Skill → Capability relationship. Dataset existence does not prove a capability; provenance must remain visible, and Power != Permission.

## Implementation phases

### Phase 1 — Shell

- navigation hierarchy
- typography and spacing tokens
- topbar / status strip
- panel/card primitives
- responsive behavior

### Phase 2 — Living Fabric

- primary Canvas topology
- planner/worker state transitions
- active request pipeline
- worker health and compute metadata
- event stream

### Phase 3 — Intelligence

- Agents view
- Skills view
- capability coverage
- provenance indicators
- Talent Tree bridge

### Phase 4 — Operations

- queue/workload
- inference metrics
- recovery
- network
- model registry

### Phase 5 — Verification

- compile
- tests
- live browser verification
- compare against the real Command Deck screenshots
- open a dedicated PR for review

## Non-goals

- no model outputs in telemetry
- no secrets in the UI
- no fake runtime counters
- no CDN or runtime frontend package dependency
- no redesign of backend domain logic

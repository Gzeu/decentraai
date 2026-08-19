# DecentraAI Control Plane v3 — Living Fabric

> **Documentation-only design specification.**
>
> This document defines the next visual/UX direction for the DecentraAI dashboard. It does **not** require or imply any Rust, API, protocol, storage, or runtime changes.

## 1. Purpose

The current DecentraAI runtime has grown beyond a simple node dashboard. The product surface now needs to represent the system as a **living distributed AI fabric**:

- trusted nodes and P2P topology
- real remote inference and execution paths
- persistent agents and capabilities
- models and serving state
- semantic retrieval and collective memory
- measured reputation
- audit and runtime health

The UI should therefore behave like a **control plane for the fabric**, not like a collection of independent admin pages.

## 2. Design principle

### The UI is the fabric's visual control plane

The primary screen should answer, at a glance:

1. **What is alive?** — nodes, agents, models, links.
2. **What is happening now?** — current execution and activity stream.
3. **Why was this node/agent/model selected?** — planner/capability context.
4. **Can I trust the result?** — verified/trusted/audited states.
5. **What does the fabric know?** — RAG, memory, skills, reputation.

Avoid a generic SaaS-card dashboard where topology, execution and intelligence are disconnected.

## 3. Information architecture

### Primary navigation

**Fabric**
- Overview
- Topology
- Execution
- Agents
- Models
- Skills / RAG
- Memory
- Reputation

**System**
- Hardware
- Inference
- Security
- Audit

The navigation is intentionally organized around the **fabric mental model** rather than implementation crates.

## 4. Overview screen

The Overview becomes the default landing surface and is composed of four zones:

### A. Living Fabric

A runtime-derived topology canvas showing:

- node identity
- connection state
- active model
- active agent
- resource pressure
- trust state
- request/execution flow

A link should become visually active while a request is traveling through the fabric.

### B. Current Execution

A compact execution card should expose the most recent active route:

`Desktop → Laptop → Agent → Model → Result`

Useful fields:

- selected worker
- planner decision
- agent/capability
- model
- retrieval context
- streaming state
- latency
- terminal state

### C. Live Activity

A chronological event stream for operational understanding:

- request received
- planner selection
- agent capability match
- retrieval performed
- inference started
- stream active
- memory persisted
- reputation updated
- inference completed / failed

The feed must remain high-signal. Do not expose secrets, prompts, bearer tokens, private keys or raw model output in telemetry.

### D. Fabric Health

Small status strip with the minimum high-value state:

- healthy nodes / total nodes
- verified links
- active executions
- success rate
- latency summary
- current model availability

## 5. Visual language

### Overall tone

**Technical, premium, calm, precise.**

Target feeling:

> "Operating system for a distributed AI fabric"

Not:

> "generic analytics dashboard"

### Palette

- background: deep navy / near-black
- surface: blue-black panels
- primary signal: electric cyan
- secondary signal: indigo / blue
- verified/healthy: restrained mint
- warning: amber
- critical: red

Bright color is reserved for state and interaction. Most of the interface should remain quiet.

### Typography

Use a modern sans-serif with strong hierarchy:

- compact uppercase labels for telemetry
- high-contrast titles for primary state
- restrained monospace only for identifiers, hashes, ports and technical values

### Geometry

- medium corner radius
- thin borders
- low-noise shadows
- generous internal spacing
- dense information without visual clutter

## 6. Node language

Each node is a first-class visual entity.

Minimum visible state:

- node name / identity
- online / degraded / offline
- trust state
- active agent
- served model
- resource pressure
- current execution role

Selecting a node should reveal its details without leaving the fabric context.

## 7. Agent language

Agents should not be presented as simple rows in a table.

The agent detail view should expose:

- identity
- role
- capabilities
- provenance / verification state
- reputation
- execution count
- current task
- persistence state

Capabilities should be explainable through the Dataset → Skill → Capability → Talent chain where that evidence exists.

## 8. Execution language

Execution is the bridge between the fabric graph and the user's request.

Represent execution as a trace:

```text
Request
  ↓
Planner
  ↓
Worker selection
  ↓
Agent / capability match
  ↓
Model
  ↓
RAG / memory context
  ↓
Inference
  ↓
Verification / audit
  ↓
Result
```

The trace should support a compact mode for normal users and an expanded diagnostic mode for operators.

## 9. Trust and truth states

The UI should distinguish implementation truth from future intent.

Recommended status vocabulary:

| State | Meaning |
|---|---|
| **LIVE** | Currently observed at runtime |
| **VERIFIED** | Backed by runtime/test evidence |
| **READY** | Implemented, but not necessarily exercised in current session |
| **FOUNDATION** | Supporting groundwork exists |
| **PARKED** | Intentionally not claimed complete |

Do not visually imply completion for roadmap items that are still foundation-only.

## 10. Topology behavior

The topology canvas should prioritize operational clarity over decorative animation.

- stable nodes remain visually calm
- active links animate only during real traffic
- failed routes become explicit
- hover reveals link/node metadata
- selected execution highlights the exact route
- topology and execution remain synchronized

The graph is not a decorative network illustration; it is an operator's live map.

## 11. Chat integration

Chat should become an entry point into the fabric rather than a detached conversation page.

A completed request can expose:

```text
Request
→ planner decision
→ selected node
→ selected agent
→ model
→ retrieval count
→ latency
→ verification
```

Technical details remain collapsible so the default experience stays clean.

## 12. Data discipline

The UI concept must stay honest about the data source.

### Good

- runtime-derived topology
- measured latency
- real request state
- actual agent records
- actual reputation measurements
- real retrieval results

### Avoid

- synthetic node activity
- decorative fake metrics
- invented capabilities
- fake worker counts
- simulated execution histories presented as live state

## 13. Responsive behavior

### Desktop

Three-column operator layout:

`Navigation | Fabric canvas | Activity / diagnostics`

### Laptop / smaller desktop

Collapse the activity rail into a drawer while keeping the fabric and current execution visible.

### Mobile

Mobile is a monitoring surface, not the primary authoring interface:

- current health
- current execution
- node list
- active alerts
- compact activity stream

## 14. Accessibility

- keyboard navigation for all controls
- no state communicated by color alone
- reduced-motion mode for topology animations
- readable minimum text size for telemetry
- visible focus states
- clear contrast for primary and secondary text
- status text adjacent to state indicators

## 15. Non-goals

This design specification does **not** propose:

- backend/domain rewrites
- new API contracts
- new storage models
- new protocol messages
- external frontend dependencies
- fake demo data
- replacement of working runtime behavior

It is a visual/UX layer over the existing system.

## 16. Prototype reference

Static concept image:

`docs/ui/assets/control-plane-v3-living-fabric.svg`

The reference image is intentionally a **design artifact only**. It illustrates hierarchy, topology, execution, activity and health; it is not a claim that every displayed value is currently exposed by the runtime.

## 17. Suggested implementation order

When implementation is eventually approved, do it incrementally:

1. Overview information hierarchy
2. Living Fabric topology treatment
3. Current Execution trace
4. Live Activity feed
5. Agent / Model visual language
6. Trust-state system
7. Accessibility and responsive polish

No runtime work is required to approve this design direction.
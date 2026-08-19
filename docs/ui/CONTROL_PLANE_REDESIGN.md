# DecentraAI Control Plane Redesign

## Goal

Evolve the current dashboard into a high-signal, futuristic control plane while preserving the existing runtime data and domain boundaries.

This is a UI presentation change. Do not replace real runtime values with decorative/mock values.

## Visual direction

- Deep navy/near-black background
- Cyan/teal primary accent
- Green for healthy/ready states
- Amber for degraded/untrusted states
- Violet/indigo for capability and intelligence layers
- Subtle glow and depth; avoid excessive neon/cyberpunk styling
- Dense technical typography with strong hierarchy
- Rounded control-plane panels with clear grouping
- Responsive layout for desktop first, then smaller screens

## Information architecture

### Overview

- Fabric health
- Live topology
- Workers/nodes
- Agents
- Skills/capabilities summary
- Active execution / workload
- Inference / queue
- Recent events
- Recovery status

### Topology

Make the live fabric graph the visual centerpiece:

- Local node centered
- Remote nodes/workers around it
- P2P links
- Ready/offline/untrusted/connected states
- CPU/RAM/VRAM/load/latency/model metadata when available
- Live updates from existing runtime state

### Agents

Present collective intelligence rather than isolated agent cards:

- Local vs remote agents
- Capability coverage
- Provenance-aware claims
- Models/tools
- Workflow controls
- Talent/capability relationship when available

### Skills

Present the P8 pipeline directly:

Model → Dataset → Skill → Capability → Talent → Agent Power

Show:

- registered skills
- applicable skills
- unlocked capabilities
- verified evidence
- provenance
- model requirement
- dataset reference
- prerequisites
- resource requirements

Never invent verified evidence in the UI.

### Workers

Treat workers as a compute mesh:

- CPU
- RAM
- VRAM
- GPU
- model
- queue
- in-flight work
- latency
- tok/s
- trust state
- contribution/reputation where available

## Trust rules

- Power != Permission
- Dataset existence != capability proof
- Skill existence != capability proof
- Do not duplicate domain capability calculations in frontend code
- Do not expose secrets
- Do not add a central dependency for presentation-only features

## Implementation strategy

### Phase 1 — visual system

- Shell/sidebar/header
- Shared cards
- status pills
- typography
- spacing
- color tokens
- responsive grid

### Phase 2 — live fabric

- topology visualization
- worker/node cards
- connection state
- live metrics

### Phase 3 — intelligence surfaces

- agents
- skills
- capabilities
- Talent Tree relationships
- provenance

### Phase 4 — operations

- workload
- inference
- queue
- recovery
- events

### Phase 5 — polish

- subtle transitions
- responsive behavior
- command palette integration
- accessible focus/keyboard states
- performance pass

## Non-goals

- No backend rewrite
- No new database
- No replacement of existing P2P/fabric logic
- No invented metrics
- No Canva dependency

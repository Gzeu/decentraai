# Implementation Report — Sentient Command Deck

Prototype lives only on `feat/governor-command-deck-prototype` under `prototypes/governor-command-deck/`.
Not merged. Production DecentraAI is untouched.

## 1. Visual concept

An intelligent operating system, not a chatbot and not an admin dashboard.

Obsidian field, soft white type, cool gray secondary, electric cyan as intelligence, restrained emerald for health, amber for attention, violet for research. Energy comes from state, spacing, and motion — not neon, cards, or charts.

IBM Plex Sans for UI. IBM Plex Mono only for telemetry, IDs, models, and logs.

## 2. Interaction model

Chat is the Governor's brain. Execution is a live spine, not a job table.
Inspectors slide in only when an object is selected.
Command palette (`⌘K`) is the power-user path: surfaces, providers, models, replay, pause, cancel, memory, evolution, incident preview.

The state machine retunes ambient energy: IDLE breathes, ACTIVE pulses, INCIDENT is a restrained amber warning.

## 3. Component architecture

Named concepts in `index.html` (ready to extract later):

GovernorShell, GovernorStatus, ChatSurface, ChatMessage, ActivityCapsule, ExecutionTimeline, ExecutionEvent, ToolCall, SkillActivity, DelegationActivity, WorkerStatus, ProviderStatus, EvolutionPanel, CapabilityGap, Experiment, MemoryInspector, CommandPalette.

No imports from `crates/` or production services.

## 4. Mock execution scenario

Elevated Desktop inference latency → OBSERVING → THINKING → `fabric-diagnostics` → MCP `get_worker_status` → Desktop delegation → RESULT 1.8s → VERIFYING → capability gap #42 → RESEARCHING → experiment v2 vs v1 → +18% throughput → PROPOSAL to promote skill v2 → deterministic ALLOW (simulated) → LEARNING.

Replay with `R` or the palette.

## 5. Design decisions

- Two surfaces only. Pulse is a hairline strip, not a dashboard.
- Proposals stay proposals until a simulated Rust decision.
- Capsules and timeline events expand in place.
- Memory is a typed list with confidence, not a graph.
- Evolution is a living gap thread, not project management.
- Desktop-first 1440×1024; inspectors become sheets on narrower widths.

## 6. What should later connect to real DecentraAI APIs

- Governor state and transcript
- MCP tool calls and results
- Skill invocation
- Worker delegation / Fabric execution
- Verification receipts
- Memory read/write
- Capability gaps, experiments, proposals
- Provider and model inventory
- Pause / cancel of real runs

## 7. What should remain frontend-only

- Visual system and motion
- Surface layout and inspector chrome
- Command palette UX
- Capsule / timeline presentation
- Keyboard map
- Mock scenario player used for demos

## 8. Screenshots / preview

The preview **is** `index.html`. Open it locally. No fabricated screenshots.

## 9. Recommended next iteration

Keep this branch as the visual contract. Do not merge.
When the direction is accepted, extract the component model into a real frontend and wire Chat → MCP → Execution → Fabric → Memory behind the same surfaces. Do not let operational UI regress into an admin dashboard.

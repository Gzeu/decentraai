# Implementation Report — Sentient Command Deck

Prototype lives only on `feat/governor-command-deck-prototype` under `prototypes/governor-command-deck/`.
Not merged. Production DecentraAI is untouched.

## 1. Visual concept

Governor Presence: an intelligent operating system, not a chatbot and not an admin dashboard.

The first read is identity, utterance, and path. Obsidian field, soft white type, cool gray secondary, electric cyan intelligence, restrained emerald health, amber attention, violet research. Energy comes from state and motion — not neon, cards, or charts.

IBM Plex Sans for UI. IBM Plex Mono only for telemetry, IDs, models, and logs.

## 2. Interaction model

Chat is the Governor's brain. Execution is a live spine, not a job table.
Inspectors slide in only when asked.
The palette is `Ask Governor...` — questions go to Chat; named commands are power controls.

Product path (fewer, calmer nodes):

`OBSERVING → DIAGNOSING → DELEGATING → VERIFYING`

Internal states still exist and retune ambient energy. Proposals stay proposals until a simulated Rust decision.

## 3. Component architecture

GovernorShell, GovernorPresence, GovernorStatus, ChatSurface, ChatMessage, ActivityCapsule, ExecutionTimeline, ExecutionEvent, ToolCall, SkillActivity, DelegationActivity, WorkerStatus, ProviderStatus, EvolutionPanel, CapabilityGap, Experiment, MemoryInspector, CommandPalette.

No imports from `crates/` or production services.

## 4. Mock execution scenario

Elevated Desktop inference latency → OBSERVING → THINKING → `fabric-diagnostics` → MCP `get_worker_status` → Desktop delegation → RESULT 1.8s → VERIFYING → capability gap #42 → RESEARCHING → experiment v2 vs v1 → +18% throughput → PROPOSAL to promote skill v2 → deterministic ALLOW (simulated) → LEARNING.

Replay with `R` or Replay execution.

## 5. Design decisions

- Presence first. Pulse metrics are no longer a dashboard strip.
- Two surfaces only. Chat / Execute sit under the utterance.
- Capsules keep a discreet signature: MCP, Skill, Worker, Memory, Proposal.
- State changes tempo, wash, path, and utterance weight — not layout chrome.
- Desktop-first 1440×1024.

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

- Presence composition and motion
- Capsule signatures
- Command palette UX
- Keyboard map
- Mock scenario player

## 8. Screenshots / preview

The preview **is** `index.html`. Open it locally. No fabricated screenshots.

## 9. Recommended next iteration

Keep this branch as the visual contract. Do not merge.
When Presence feels right, extract the component model and wire Chat → MCP → Execution → Fabric → Memory behind the same two surfaces.

## Governor Presence iteration

Tested in this pass, still mock-only:

1. **State transitions** — IDLE breathes; OBSERVING / THINKING raise the utterance; EXECUTING / DELEGATING energize the path and spine; VERIFYING turns emerald; LEARNING turns violet; INCIDENT is restrained amber.
2. **Execution capsules** — MCP, Skill, Worker/Delegation, Memory, Proposal each have a quiet mark and hue. No icon pack, no neon.
3. **Command palette** — `Ask Governor...` plus Run capability, Inspect worker, Inspect memory, Run benchmark, Research capability, Replay execution, Pause execution, Open incident, Switch provider, Switch model.

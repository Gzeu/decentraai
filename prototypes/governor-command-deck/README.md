# Sentient Command Deck

Isolated visual contract for the DecentraAI Core Governor.

**Branch only.** Do not merge to `main`. No production imports.

## Preview

Must be served over HTTP (ES modules):

```bash
python3 -m http.server 5173 --directory prototypes/governor-command-deck
```

Open `http://localhost:5173`.

## Layout

```
index.html              shell
css/deck.css            visual system
js/state.js             mock data + state machine
js/components.js        named UI concepts
js/app.js               scenario + interaction
js/api.js               API contract adapter (mock -> real)
```

Named concepts in `js/components.js`:

GovernorShell, GovernorPresence, GovernorStatus, ChatSurface, ChatMessage, ActivityCapsule, ExecutionTimeline, ExecutionEvent, ToolCall, SkillActivity, DelegationActivity, WorkerStatus, ProviderStatus, EvolutionPanel, CapabilityGap, Experiment, MemoryInspector, CommandPalette.

## Invariant

Chat = brain. Execution = hands.
`AI proposes → deterministic policy decides → workers execute`

## API Contract (`js/api.js`)

Single seam between UI and future real backend. UI calls these; only this file changes when the real endpoints land.

| Function | Returns | Future endpoint |
|---|---|---|
| `getGovernorState()` | identity + pressure + sharing + provider + workers | `GET /v1/governor/state` |
| `sendChat(message)` | reply + execution trace | `POST /v1/governor/chat` |
| `getExecution()` | events + active flag | `GET /v1/governor/execution` |
| `getWorkers()` | WorkerStatus[] with contribution_balance | `GET /v1/compute` |
| `getMemory()` | typed Obsidian notes (scoped) | `GET /v1/governor/memory` |
| `getSkills()` | Skill Registry entries | `GET /v1/skills` |
| `getProviders()` | LOCAL + OX_ALPHA + LAGUNA availability/routing | `GET /v1/intel/providers` |
| `getModels()` | models on fabric nodes | `GET /v1/models` |
| `getCapabilityGaps()` | evolution backlog | `GET /v1/governor/gaps` |
| `cancelExecution(event_id)` | cancellation confirmation | `POST /v1/governor/execution/cancel` |

Mock mode is active by default (`USE_MOCK = true`). Real mode: set `window.DECENTRAAI_API = { baseUrl, apiKey }`, flip to false.

Response schemas are documented via JSDoc in api.js and match real DecentraAI shapes (Agent OS RBAC, DFCP negotiation stages, Sharing is Caring credit flow, Fabric Intelligence pressure score).

## Shortcuts

`⌘K` Ask Governor · `1` Chat · `2` Execute · `Space` pause · `R` replay · `M` memory · `E` evolution · `Esc` close

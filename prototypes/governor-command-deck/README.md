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
```

Named concepts in `js/components.js`:

GovernorShell, GovernorPresence, GovernorStatus, ChatSurface, ChatMessage, ActivityCapsule, ExecutionTimeline, ExecutionEvent, ToolCall, SkillActivity, DelegationActivity, WorkerStatus, ProviderStatus, EvolutionPanel, CapabilityGap, Experiment, MemoryInspector, CommandPalette.

## Contract

Chat = brain. Execution = hands.
`AI proposes → deterministic Rust decides → workers execute`

## Shortcuts

`⌘K` Ask Governor · `1` Chat · `2` Execute · `Space` pause · `R` replay · `M` memory · `E` evolution · `Esc` close

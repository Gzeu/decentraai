# Sentient Command Deck

Isolated visual/UX prototype for the DecentraAI Core Governor.

**Branch only.** Do not merge to `main`. This folder does not import or modify production DecentraAI code.

## Preview

Open `index.html` in a desktop browser, or from the repo root:

```bash
python3 -m http.server 5173 --directory prototypes/governor-command-deck
```

Then visit `http://localhost:5173`.

Target frame: 1440 × 1024.

## Presence

The first thing on screen is the Governor, not a dashboard.

- Identity + online + active model
- Current utterance
- Product path: OBSERVING → DIAGNOSING → DELEGATING → VERIFYING
- Then Chat (brain) or Execution (hands)

`AI proposes → deterministic Rust decides → workers execute` stays visible on proposals.

## Contract

Two primary surfaces: **Chat** and **Execution**. Everything else is an inspector or a command.

All data is mocked. No network calls.

## Shortcuts

| Key | Action |
| --- | --- |
| `⌘K` / `Ctrl+K` | Ask Governor / command palette |
| `1` | Chat |
| `2` | Execution |
| `Space` | Pause / resume |
| `R` | Replay execution |
| `M` | Inspect memory |
| `E` | Research capability |
| `Esc` | Close overlay |
| `[` `]` | Previous / next execution event |

Palette commands: Run capability, Inspect worker, Inspect memory, Run benchmark, Research capability, Replay execution, Pause execution, Open incident, Switch provider, Switch model.

## Isolation

Do not touch `crates/`, production configuration, Agent OS, MCP, Fabric, DFCP, workers, memory implementation, or authentication.

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

## Contract

`AI proposes → deterministic Rust decides → workers execute`

Two primary surfaces: **Chat** (brain) and **Execution** (hands). Everything else is an inspector or a command.

All data is mocked. No network calls.

## Shortcuts

| Key | Action |
| --- | --- |
| `⌘K` / `Ctrl+K` | Command palette |
| `1` | Chat |
| `2` | Execution |
| `Space` | Pause / resume scenario |
| `R` | Replay latency scenario |
| `M` | Memory inspector |
| `E` | Evolution inspector |
| `P` | Pause |
| `Esc` | Close overlay |
| `[` `]` | Previous / next execution event |

## Isolation

This prototype must not touch:

- `crates/`
- production configuration
- Agent OS, MCP, Fabric, DFCP, workers
- memory implementation or authentication

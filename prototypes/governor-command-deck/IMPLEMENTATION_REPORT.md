# Implementation Report — Sentient Command Deck

Lives only on `feat/governor-command-deck-prototype`.
Not merged. Production DecentraAI is untouched.

## 1. Visual concept

Governor Presence first: identity, utterance, path. Not a chatbot. Not an admin dashboard.

## 2. Interaction model

Two surfaces. Inspectors on demand. Palette is `Ask Governor...`.
Product path: `OBSERVING → DIAGNOSING → DELEGATING → VERIFYING`.
Proposals wait for a simulated Rust decision.

## 3. Component architecture

Extracted, still frontend-only:

| File | Role |
| --- | --- |
| `index.html` | GovernorShell mount |
| `css/deck.css` | visual system |
| `js/state.js` | mock fabric + state machine |
| `js/components.js` | Presence, Chat, Execution, inspectors, palette |
| `js/app.js` | scenario player + keyboard |

No `crates/` imports. No network calls.

## 4. Mock scenario

Desktop latency → diagnostics → MCP `get_worker_status` → Desktop → 1.8s → verify → gap #42 → bench +18% → propose skill v2 → simulated ALLOW.

## 5. Design decisions

Presence over dashboards. Discreet capsule signatures. State changes energy, not chrome.

## 6. Later API connections

Governor transcript, MCP, skills, Fabric/workers, verification, memory, gaps, providers/models, pause/cancel.

## 7. Remain frontend-only

Presence, signatures, palette UX, keyboard, demo scenario.

## 8. Preview

`python3 -m http.server 5173 --directory prototypes/governor-command-deck`

## 9. Next

Keep as visual contract. Wire Chat → MCP → Execution → Fabric → Memory only after this composition is accepted.

# DecentraAI — TODO / next steps

> Living list of the next work items, in priority order. Update as items land.
> The single source of truth for *what's done* is `ROADMAP.md`; this file is
> the forward-looking task list.

## Active / next (in order)

- [ ] **Two-node LAN validation** (Collective Intelligence live on 2 nodes)
  - [x] Desktop: `git pull --rebase && bash scripts/upgrade-node.sh` (DONE
        2026-08-18; Desktop-ul e vizibil ca agent remote pe Laptop —
        `dca-NGE65Z:generalist` in `/v1/agents`).
  - [x] **Vizibilitate reciproca verificata live de pe Desktop** (DONE
        2026-08-18, dupa rebuild cu fix-ul p2p `80829be`): `/v1/compute` = 2
        workers (ambele `remote=True`), `/v1/agents` = 2 (unul remote),
        `/v1/fabric` = 2 noduri (Laptop `ONLINE`, `trusted: true`), link LAN
        1000 Mbps. Cauza reala era bug-ul p2p (SignedComputeAdvertisement
        inghitit in agent branch), NU un binar vechi.
  - [ ] Laptop: `bash scripts/validate-lan.sh` — dovedeste remote execution
        end-to-end (orchestrator pe un nod, executa pe celalalt prin
        `route_request`). Referinta: `docs/NODE_UPGRADE.md`.
- [ ] **Collective memory written from workflows + UI** — when a workflow
  completes, write its verified results into the SQLite `MemoryStore` scopes
  (ownership + access per the pure P5 model) and surface a Memory view in the
  dashboard (`/v1/memory`).
- [ ] **Reputation fed from real results + UI** — after each verified delegated
  stage, feed the outcome into the `ReputationStore` so executor selection
  ranks real history (not synthetic samples); surface reputation in the
  dashboard.
- [ ] **Retrieval tool in execution** — a workflow/agent that *calls* semantic
  retrieval during generation (RAG at runtime, not just the endpoint).
- [ ] **CLI `decentraai agent workflow run`** — trigger `/v1/agents/orchestrate`
  from the CLI (not just dashboard/curl) for scripting.
- [ ] **Node `node.model` on the Desktop** — serve Mistral-7B there for a
  higher-quality remote executor.

## Product / polish

- [ ] Dashboard: surface reputation + talent tree views (currently CLI-only).
- [ ] `decentraai agent` — add `memory` and `economy` inspection subcommands.
- [ ] Persist agent records (AgentManager) to disk so restarts keep local
      agents stable.

## Backlog (larger / later)

- [ ] Fully autonomous agent runtime that chains tools (not only inference).
- [ ] Self-optimization loop wired to live fabric observations.
- [ ] Economy ledger wired to Quota/Compensation for real (non-monetary)
      contribution settlement.

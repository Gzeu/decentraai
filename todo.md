# DecentraAI — TODO / next steps

> Living list of the next work items, in priority order. Update as items land.
> The single source of truth for *what's done* is `ROADMAP.md`; this file is
> the forward-looking task list.

## Active / next (in order)

- [ ] **Two-node LAN validation** (Collective Intelligence live on 2 nodes)
  - [x] Desktop: `git pull --rebase && bash scripts/upgrade-node.sh` (DONE
        2026-08-18; Desktop-ul e vizibil ca agent remote pe Laptop —
        `dca-NGE65Z:generalist` in `/v1/agents`).
  - [ ] **Desktop compute worker nu apare pe Laptop** — Desktop-ul e agent-visible
        dar `/v1/compute` pe Laptop listeaza doar worker-ul local. De rezolvat
        (Desktop: `node.model` la un model diferit de Llama — ex. Mistral — ca
        sa forteze rutarea remote; verifica `allow_remote_inference: true`;
        reporneste node-ul), apoi `bash scripts/validate-lan.sh` pe Laptop.
        Referinta: `docs/NODE_UPGRADE.md` (corectat de Pylon).
- [ ] **Collective memory written from workflows** — when a workflow completes,
  write its verified results into the SQLite `MemoryStore` scopes (ownership +
  access per the pure P5 model), so the fabric accumulates real knowledge.
- [ ] **Reputation fed from real results** — after each verified delegated
  stage, feed the outcome into the `ReputationStore` so executor selection
  ranks real history (not synthetic samples).
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

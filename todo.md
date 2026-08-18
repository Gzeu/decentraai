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
- [x] **Collective memory written from workflows + UI** — DONE 2026-08-18
  (commit `3b55830`): workflow outcomes cu verdict Completed sunt scrise in
  SQLite `MemoryStore` (scope `workflow_results`, MemoryLevel::Team): verified
  stage outputs + summary, idempotent, best-effort. `AgentOrchestrator`
  `.with_memory_store()` wired in node. Dashboard `/v1/memory` arata rezultate
  reale. 3 teste noi (completed/partial/idempotent).
- [x] **Reputation fed from real results + UI** — DONE 2026-08-18: `record_execution`
  alimenteaza `ReputationStore` din `run_plan` (succes/esec + latenta),
  `select_executor` rank-uiește cu score real (local first, score desc,
  deterministic), `/v1/reputation` + dashboard `renderReputation` (score +
  reasons cu sample counts). Verificat live: 3 entries cu reliability/quality/
  latency reale.
- [x] **Retrieval tool in execution** — DONE 2026-08-18 (commit `5a14936`):
  `augment_prompt_with_retrieval()` pur — un task cu input `retrieve` face RAG
  la runtime si augmenteaza promptul cu docs din index (fara docs pastreaza
  base). Wiring complet: `RetrievalManager` → `InferenceAgentExecutor.with_retrieval()`
  → CLI `--retrieve` → seed → stage inputs. Eliminata si dubla scriere memory
  in orchestrate handler. 2 teste noi.
- [x] **CLI `decentraai agent workflow run`** — DONE 2026-08-18 (commits
  `02ae867` + `2ff46fb`): `decentraai agent workflow-run` ruleaza
  `/v1/agents/orchestrate` din CLI (verdict + output), cu `--template` si
  `--retrieve`. CLI complet cu meniu coerent (commit `e944545`): `rag index/
  query` + `memory list` + `build_local_client()`.
- [ ] **Node `node.model` on the Desktop** — serve Mistral-7B there for a
  higher-quality remote executor.

## Product / polish

- [x] Dashboard: reputation + talent tree views — DONE 2026-08-18: Talents
  (commit `8d2c5eb`), Reputation + Memory views live in Mesh.
- [ ] `decentraai agent` — add `memory` and `economy` inspection subcommands.
- [ ] Persist agent records (AgentManager) to disk so restarts keep local
      agents stable.

## Backlog (larger / later)

- [ ] Fully autonomous agent runtime that chains tools (not only inference).
- [ ] Self-optimization loop wired to live fabric observations.
- [ ] Economy ledger wired to Quota/Compensation for real (non-monetary)
      contribution settlement.
# Review — Collective Intelligence Plane (crates/agents + runtime-ul agent din distributed)

Sursă: subagent de adâncime (2026-08-21), read-only. Afirmația-cheie (matcher-ul
unificat fără apelanți de producție, PolicyEngine fără apelanți) a fost
re-verificată în cod de coordonator. Referit din `docs/TECHNICAL_REVIEW.md`.

## 1. Responsabilitate & suprafață publică

**Substratul pur de agent (`crates/agents`, 26 fișiere, 12.449 LOC).** Model de
domeniu fără dependențe (compute + hub doar), serde-serializabil:
`AgentRecord`/`AgentRegistry` (agent.rs, registry.rs), matcher-ul unificat de
capability (matcher.rs), task/tool/advertisement (task.rs, tool.rs,
advertisement.rs), mesaj P2 + inbox mărginit (message.rs), DAG de delegare P3
(delegation.rs), verificare/consens P4 (verification.rs), model de memorie P5
(memory.rs), reputație P6 (reputation.rs), engine de politică P7 (policy.rs),
arbore de talente P8 (talent_tree.rs), workflow-uri P9 (workflow.rs),
self-optimizer P10 (selfopt.rs), economie P11 (economy.rs), cunoaștere/decizie/
receipt P12 (knowledge.rs, decision.rs, receipt.rs), evidence RAG + benchmark +
retrieval + dataset (evidence.rs, benchmark.rs, retrieval.rs, dataset.rs),
receipt semnat P13 (signed_receipt.rs). Totul funcții pure; 221 de teste.

**Runtime bindings (`decentraai-distributed` agent_\*).** Jumătatea cu stare:
`AgentManager` (agents.rs) — vederi local+remote cu suport pentru
advertismente semnate și eviction stale; `AgentMessenger` (agent_messenger.rs)
— punte la canalul request/response libp2p cu self-delivery;
`AgentOrchestrator` (agent_orchestrator.rs) — plan → selecție executor
ranked-by-reputation (local first) → delegate → verificare per-hop → reputație
→ memorie colectivă; `AgentRuntime` + `InferenceAgentExecutor`
(agent_runtime.rs) — drenează inbox-uri, rulează Delegates contra backend-ului
local live sau `route_request`, cu retrieval RAG și tool-calling mărginit;
`MemoryStore` (agent_memory.rs) — store SQLite persistent cu aplicare a
accesului.

**Buclele de cunoaștere/evidență/benchmark.** `KnowledgeRuntime`
(knowledge_runtime.rs) — bucla P12: receipt → credit compensație idempotent
(doar verificat) → obiect de cunoaștere → decizie colectivă (delegată la
`evaluate_consensus`) → feedback de memorie în scopul `collective.knowledge`;
`EvidenceManager` (evidence_manager.rs) — sincronizează idempotent indexul pur
din execuții/receipts/decizii/memorie, cu interogare structurală vs semantică
onestă și lecții derivate; `BenchmarkManager` (benchmark_manager.rs) — rulează
task-uri single/RAG/collective prin executorul live, gradează determinist,
hrănește fiecare run în evidence; `benchmark_datasets.rs` (BrowseComp-Plus);
`retrieval_manager.rs` + `embedding.rs` + `tool_calling.rs` — RAG și tool-uri.

## 2. Abstractions-cheie — wired vs pure-logic-only

**Genuin wire-uit end-to-end (apelanți de runtime existenți):**
- AgentRecord/AgentManager + advertismente semnate — `spawn_agent_broadcaster`
  (node-cli main.rs:6373), verificare de semnătură în p2p_handler.rs:151–165,
  teste E2E două-noduri (distributed/tests/agent_e2e.rs).
- AgentMessenger + AgentRuntime + InferenceAgentExecutor — daemon-ul spawn-ează
  un runtime de producție per agent local (main.rs:1872–1883), `POST
  /v1/agents/orchestrate` (api.rs:7154–7246) instantiază
  `research_report_template` și apelează `orchestrator.orchestrate_plan`.
- MemoryStore (SQLite, cu aplicare a accesului) — `db/agent_memory.sqlite`
  (main.rs:1913); `can_read`/`can_write` aplicate per operație
  (agent_memory.rs:255–266, 331–337).
- ReputationStore — **parțial wire-uit**: orchestratorul hrănește store-ul
  in-memory cu rezultate reale per stage (agent_orchestrator.rs:413–448,
  folosit în select_executor:126–162). Dar **nu e niciodată persistat**, iar
  vizualizarea CLI `agent reputation` e explicit "built from synthetic sample
  data" (main.rs:578–579). Reputația p2p (p2p/reputation.rs) și
  ContributionProfile (compute) sunt sisteme separate, bridged manual.
- Template-uri de workflow — wire-uite (research_report via API);
  verification-agnostice by design (workflow.rs:13–19).
- KnowledgeRuntime — wire-uit (main.rs:2223–2231; API: GET /v1/knowledge,
  POST receipt/decide). Profilele se seed-uiesc la wiring din stare compute
  măsurată, niciodată din corp HTTP (api.rs:6558–6567 — verificat).
- Evidence RAG — wire-uit, sync lazy la request (api.rs:6919, 6969). Calea
  semantică doar cu embeddings reale; fallback structural etichetat
  `mode:"structural"` (evidence_manager.rs:198–219; evidence.rs:246–277).
- Benchmark Lab — wire-uit (main.rs:1893–1901, attach 2244–2246). `comparison()`
  e `paired_compare` peste task-uri gradate în AMBELE moduri
  (benchmark.rs:313–401, registry 445–447), MIN_SAMPLES=5 / MIN_MARGIN=0.05
  (benchmark.rs:177–179).
- Tool calling + retrieval RAG în executor — wire-uit (main.rs:1861–1869,
  agent_runtime.rs:306–382).

**Pure-logic-only (zero apelanți de runtime — verificat prin grep):**
- `match_agent` (matcher-ul unificat, matcher.rs:127) — apelat DOAR de
  propriile teste (matcher.rs:269–461). Orchestratorul live folosește doar
  `match_agent_semantic` (agent_orchestrator.rs:140) — deci gate-ul fizic
  (compute matcher: trust/RAM/VRAM/rezervări/health) și gate-ul model-allowlist
  **nu sunt aplicate pe calea de delegare**.
- PolicyEngine (P7) — zero apelanți în afara policy.rs + re-export.
  `AgentPolicies.allow_remote` e setat (main.rs:5498–5501) dar niciodată
  aplicat: `select_executor` ignoră starea agentului și `allow_remote`, iar
  `AgentRuntime::process_one` acceptă orice Delegate (agent_runtime.rs:110–171).
- SelfOptimizer (P10), EconomyLedger/CapabilityOffer/BookingRequest/negotiate
  (P11) — zero apelanți de runtime (hit-urile `negotiate` din api.rs/mcp.rs sunt
  negociere de protocol MCP, nu economie).
- `safety_penalty`, `best_for_capability`, `resolve_disagreement`,
  `VerificationLedger`, `check_output_schema`, `run_workflow` — fără apelanți
  în afara crate-ului agents. `evaluate_consensus` e apelat doar de calea
  knowledge decide, **nu** de verificarea de workflow/delegare.
- `TaskVerification::Critic`/`Consensus` — variante moarte la runtime:
  orchestratorul tratează orice cerință `!= None` ca schema check
  (agent_orchestrator.rs:333).
- SignedComputeReceipt / verify_receipt_signature (P13) — CLI-only
  (`decentraai receipt sign|verify`, main.rs:3611–3706); nu face parte din
  fluxul live (receipt-urile KnowledgeRuntime sunt nesemnate).
- MemoryRegistry (pur) — nefolosit; runtime-ul folosește MemoryStore.
- TalentTree — doar inspecție read-only (dashboard/CLI); nu conduce planning-ul.

## 3. Integrare & cuplare

- **Dependențe agents verificate**: doar decentraai-compute, decentraai-hub,
  libp2p, serde, serde_json, thiserror, ed25519-dalek, blake3 — fără
  tokio/reqwest/anyhow; pretenția "pure, no I/O" ține.
- **Separarea runtime/pur e bună ca formă** (distributed ține tot I/O-ul), dar
  granița e leaky: orchestratorul **re-implementează** logică pură în loc să o
  apeleze — `run_plan` (agent_orchestrator.rs:263–401) duplică `execute_plan`
  (delegation.rs:485–632), `verify_value` (agent_orchestrator.rs:470–508)
  duplică `check_value_schema` (delegation.rs:41–84) aproape verbatim. Sortarea
  topologică a lui Kahn e scrisă a treia oară în workflow.rs:172–218. Cei doi
  executori de DAG pot drift-ui (deja diferă subtil: seed-merge, final_output).
- **Duplicare peste plan**: trei sisteme de reputație (p2p `PeerScore`,
  compute `ContributionProfile`/`CompensationLedger`, agents `AgentReputation`)
  — domenii înrudite dar distincte, bridged manual fără vocabular comun.
  Cosinus scris de două ori (retrieval.rs, evidence.rs:296–314). Matcher-ul
  unificat (headline P0) și selecția semantică-only a runtime-ului = două
  noțiuni paralele de eligibilitate.
- **Store-uri SQLite**: `db/agent_memory.sqlite` (MemoryStore) și
  `db/reputation.json` (p2p). Controlul de acces e aplicat la stratul corect —
  fiecare read/write re-rulează `can_read`/`can_write` pur cu fapte de trust
  din partea apelantului (agent_memory.rs:255–266, 331–337); deciziile de acces
  nu sunt niciodată cache-uite. Bun.

## 4. Semnale de maturitate

- **Documentație**: excelentă — fiecare modul are doc "de ce" focusat;
  invariantele de onestitate sunt declarate și de obicei implementate.
- **Erori**: thiserror/anyhow cu context la granițe, consistent în agents.
  Dar `lock().unwrap()` e omniprezent în runtime (agents.rs:67–202,
  agent_memory.rs:151–385, retrieval_manager.rs:51–77), în timp ce
  knowledge_runtime.rs folosește deliberat `map_err(poisoned)` (179, 205, 258)
  — stil inconsistent; un lock otrăvit panichează nodul în majoritatea store-urilor.
- **unwrap/expect**: agents 252 total, **19 în producție** (delegation.rs:233,
  236, 252, 292–295, 306, 311 în Kahn; workflow.rs:182–183, 198;
  talent_tree.rs:342–421 — 10 `.expect` în `seed_talent_tree`) — toate
  "impossible-by-construction", dar încălcând regula repo-ului. Distributed țintă:
  ~39 în producție, aproape toate lock-uri Mutex.
- **Disciplina de verdict onest — verificată în cod**: fără evidență →
  confidence 0.0 (knowledge.rs:171–192, testat knowledge.rs:319–327);
  receipt-uri eșuate nu creditează (receipt.rs:109–118,
  knowledge_runtime.rs:585–601); consensul cere N opinii înainte de decizie
  (verification.rs:397–406); acordurile cu confidence zero sunt notate, nu
  crezute (verification.rs:408–427); RAG-ul semantic nu fabrică niciodată un
  scor (evidence.rs:246–277); compararea benchmark e paired + "not enough
  samples" onest (benchmark.rs:263–297, 332–371); reputația necunoscută = 0.0
  ca "unknown, not a penalty" (reputation.rs:183–210).
- **Determinism**: BTreeMap peste tot, tie-break-uri id asc, output-uri sortate
  — consistent și testat. Consens/ranking/lecții sunt funcții pure de input.

## 5. Mirosuri & riscuri concrete

- **God-fișiere**: api.rs 16.720, node-cli main.rs 7.339, distributed/compute.rs
  4.654 — logica de runtime a planului agent trăiește în aceste monoliți.
- **Suprafață moartă / promisiuni ne-wire-uite**: narațiunea AGENTS.md/ROADMAP
  ("matcher unificat … cross-wired", verificare per-hop P4, politică P7,
  self-opt P10, economie P11) e înaintea codului: matcher, PolicyEngine,
  SelfOptimizer, economia, verificarea bazată pe consens și receipt-urile
  semnate sunt pure-logic-only sau CLI-only. Etichetarea e onestă în notele de
  subsol, dar decalajul claim-to-code e larg.
- **Drift de nume**: docs-urile numesc reputația de agent "P6"; CLI-ul
  `agent reputation` arată date demo sintetice în timp ce store-ul real
  alimentat din delegații e doar in-memory; "reputation fed from real results"
  e doar pe jumătate adevărat.
- **Bypass al gate-ului fizic**: calea de delegare nu verifică niciodată
  trust/capacitate/prezență de model — un coordonator poate delega către un
  agent remote al cărui nod nu poate servi modelul, contrazicând etosul
  "verify before use" la stratul de agent.
- **Logică de DAG triplicată și implementări duale de verificare** — riscul
  cel mai mare de drift.

## 6. Verdicturi (1–5)

| Zonă | Scor | Justificare |
|---|---|---|
| Substrat pur de agent (agents) | **4** | Cel mai bine documentat și testat cod din repo; penalizat doar de cele 19 unwrap-uri de producție și suprafața mare ne-wire-uită (P7/P10/P11 + jumătățile pure ale P4/P6) |
| Runtime bindings (distributed agent_\*) | **3** | Genuin live (daemon, API, E2E două-noduri) cu căi de eșec oneste, dar duplică logica pură, sare gate-ul matcher-ului fizic, e plin de lock().unwrap() |
| Buclele knowledge/evidence/benchmark | **4** | Bucla închisă e reală, idempotentă, onestă (verificat în cod + teste); gap-uri: receipt-urile semnate (P13) și consensul în verificare nu sunt wire-uite în buclă |

## Top 5 riscuri pe termen lung (planul intelligence)

1. **Matcher-ul unificat neaplicat la runtime** — `match_agent` (matcher.rs:127)
   nu are apelant de producție; delegația selectează pe claim-uri semantice
   doar (agent_orchestrator.rs:140), deci gate-urile trust/capacitate/model
   allowlist nu se aplică silențios muncii de agent. Fie wire-uiește matcher-ul
   complet în `select_executor`, fie demotează pretenția din docs.
2. **Executori de DAG duplicați vor drift-ui** — `run_plan` vs `execute_plan`
   și `verify_value` vs `check_value_schema` sunt implementări duplicat ale
   acelorași decizii; `execute_plan`/`run_workflow` pur nu sunt apelate
   niciodată de runtime. Consolidează pe un singur executor (injectează seed-ul
   în calea pură).
3. **Promisiunile moarte devin datorie de trust** — P7 (allow_remote/egress/
   sandbox) și verificarea prin consens P4 sunt declarate dar neaplicate; când
   agenții primesc mai multă capacitate (tool-uri, egress), lipsa aplicării
   devine gap de securitate, nu doar de docs.
4. **Reputația = trei sisteme deconectate, unul efemer** — AgentReputation
   alimentat din delegații trăiește doar în memoria orchestratorului (pierdut
   la restart), p2p și compute sunt separate, vizualizarea CLI e sintetică. E
   nevoie de un feed de reputație unificat și persistat (rezultate reale doar)
   înainte ca ranking-ul să pretindă că conduce ceva pe termen lung.
5. **Wiring monolit (api.rs 16,7k, main.rs 7,3k) + stare agent in-memory** —
   starea de runtime a planului agent (reputație, records, advertismente) e
   împrăștiată prin daemon fără persistență pentru reputație și fără sursă
   unică deduplicată; pe măsură ce colectivul crește, cusătura pur/runtime va
   drift-ui și mai mult.

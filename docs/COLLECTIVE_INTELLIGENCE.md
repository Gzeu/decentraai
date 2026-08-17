# Collective Intelligence — arhitectură & fundație

> Status: **propunere de arhitectură** (Pylon, 2026-08-17). Auditul repo-ului e făcut pe
> commit `8479eb4`. Nu s-a scris cod pentru această direcție — doar analiză + design.
> Următorul pas natural: agrearea scope-ului Phase 1 cu George, apoi implementare.

## 1. Rezumat executiv

DecentraAI are deja ~80% din substratul necesar pentru Collective Intelligence, dar sub
nume de „node" și „worker", nu sub nume de „agent". Direcția corectă NU este un sistem
paralel nou, ci **generalizarea primitivelor existente** (capability → agent capability,
task → agent task, planner → orchestrator, worker → agent host).

Decizia arhitecturală centrală propusă:

> **Un agent este un context logic de execuție (identity + capabilities + policies +
> memory + reputation) care rulează PE un node existent — nu un proces nou.**

Consecință: un node poate găzdui mii de agenți logici. Scalarea la milioane de agenți
rămâne o problemă de transport/coordonare, nu o problemă de arhitectură de bază.
Arhitectura fundamentală (node → capabilities → plan → reserve → execute → verify →
release) nu se schimbă pe măsură ce numărul de agenți crește — se schimbă doar
dimensiunea registrelor și politicile.

## 2. Audit — ce există deja (verificat în cod, commit 8479eb4)

### 2.1 Componente reutilizabile direct

| Componentă | Locație | Ce oferă pentru CI |
|---|---|---|
| Identity Ed25519 + PeerId | `crates/identity` | identitatea de bază a unui agent (node) |
| Semantic capability taxonomy | `crates/hub` `CapabilityKind` (26 tipuri) | limbajul semantic al capabilities-urilor |
| Intent → capability | `crates/hub::intent` `capabilities_for_intent` | sămânța task-routing-ului |
| Capability matching (provenance-aware) | `crates/hub::requirements` | verificare requirement vs claims |
| Physical capability + models | `crates/compute` `ComputeCapability` / `ServedModel` | resursele fizice ale unui agent |
| Task primitives | `InferRequest` / `WorkloadRequirements` / `TaskPlacement` | modelul de task de bază |
| Planner / routing | `crates/fabric` `ExecutionPlanner`, `plan_and_reserve` | delegarea + selecția worker |
| Expert/split routing | `crates/fabric::expert` `ExpertRegistry/Router` | primitiv de task-DAG pe 2 niveluri |
| Reservations + admission | `crates/compute` `ReservationLedger`/`Admission` | resource budgets per task |
| Registry + heartbeat | `ComputeRegistry` / `ComputeAdvertisement` | descoperirea + liveness |
| Trust store | `crates/discovery::pairing` `TrustStore` (SQLite) | trust persistent, scor EMA |
| Reputation | `CircuitBreaker` + `CompensationLedger` (reputation_mult) | scor de încredere per worker |
| Contribution + tiers | `ContributionProfile` / `QuotaLedger` / tiers | economics sintetice, non-monetare |
| Resilience | `FallbackHandler` / `adapt()` / reconnect / reaper | recuperare + retry sigur |
| Observability | `ExecutionDecision` / `/v1/execution` / audit / dashboard | colectiv graph pe execuție |
| Memory (parțial) | `SessionAccount` / `KVCacheState` (M20) | memorie de sesiune, continuare KV |
| Discovery extern | `crates/hub::catalog` (HuggingFace) | pipeline DISCOVER→EVALUATE→REGISTER (parțial) |
| Authn/z + rate limit | `crates/tokens`, `ApiState::require_master` | policy layer pentru API |

### 2.2 Stratul „Next-Gen" (nu e reflectat în AGENTS.md)

ROADMAP secțiunile 43–92 conțin deja, marcate DONE, o serie de primitive care sunt
practic un agent-workflow simplu pe execuție de inferență:

- **Unified fabric decision + execute** (`/v1/decision`, `/v1/execute`): decide → reserve →
  execute, cu confirmare explicită pentru mutații, dry-run, recovery în decizie.
- **Capability-only execute** (fără intent parsing): „rulează ce poate face X".
- **Sessions/continuation** (KV locality observability) — memorie de context între request-uri.
- **Batch allocation + adaptive fan-out** — împărțirea unui set de task-uri între workeri.
- **Standalone lightweight worker** + **mobile worker contract** — deja proiectat, nu doar node universal.

Acest strat e cea mai bună fundație de pornire: deja seamănă cu „un orchestrator delegă
task-uri unor entități capabile". Ce lipsește este generalizarea de la *inferență* la
*task generic*, și de la *worker (node fizic)* la *agent (context logic)*.

### 2.3 Gap-uri (ce NU există)

1. **Entitatea Agent** — nu există un `AgentRecord`/`AgentRegistry`. Există doar workeri
   (node-uri fizice). Fără lifecycle, policies, goals, relationships per-agent.
2. **Task generic** — task-urile sunt doar `InferRequest` (prompt → tokens). Nu există
   task cu schemă de intrare/ieșire arbitrară, tool calls, verificare, output structurat.
3. **Agent-to-agent messaging** — există doar request/response de inferență + broadcast
   de ad-uri. Nu există mesaje agentice tipate (ask, delegate, reply, verify, ping-pong
   semantic).
4. **Delegation DAG** — plannerul face Single/Sequential/FanOut pentru inferență. Nu există
   graph de task-uri cu contracte între noduri (Research→Financial→Synthesis→Critic).
5. **Result verification / consensus** — nu există critic, cross-check, confidence, evidence.
6. **Collective memory** — există doar memorie de sesiune KV. Fără memory multi-nivel
   (agent/team/network) cu ownership + privacy + retention.
7. **Tool registry** — capability-urile sunt hardware+model. Nu există tools/MCP/OCR/etc.
   ca capabilities first-class.
8. **Talent tree** — nimic. Nici capability graph dinamic, nici upgrade discovery.
9. **Marketplace in-network** — hub e doar HuggingFace. Nu există listing de capability
   între peeri (advertisement-ul acoperă doar compute).
10. **Self-optimization** — fără feedback loop care ajustează model/tool placement pe baza
    rezultatelor.

## 3. Harta de mapare (viziune → primitives existente)

```
Viziunea ta                    Substrat existent                          Gap
-----------------------------  ----------------------------------------  ---------------
Agent identity                 Identity (Ed25519/PeerId)                 role + lifecycle
Agent capabilities             ComputeCapability + CapabilityKind        unificarea limbajelor
Agent goals/policies           tokens/tiers + quota + matcher            policies per agent
Agent memory                   SessionAccount (KV)                       memory multi-nivel
Agent reputation               TrustStore + breaker + compensation        agregare per agent
Task primitives                InferRequest + WorkloadRequirements       task generic + schemas
Task routing / delegation      ExecutionPlanner + plan_and_reserve       DAG + contracts
Resource scheduling            ComputeScheduler + ReservationLedger      budgets per agent
Discovery (peers)              mDNS + ComputeAdvertisement               capability discovery semantic
Discovery (external)           hub::catalog (HF)                         pipeline evaluare + register
Result validation              — (doar verificare hash/chunk)            critic + consensus
Collective memory              —                                         memorie partajată cu politici
Agent economy                  QuotaLedger + CompensationLedger           price/SLA per capability
Self-optimization              adapt() (per-request)                     loop global pe fabric
Observability                  /v1/execution + audit + dashboard          graph de agenți + relații
Security                      semnare + replay + trust + tokens          permissions per capability
```

## 4. Arhitectura propusă (evoluată)

Diagrama originală e un funnel unidirectional. O îmbunătățesc pe trei axe:

1. **Identitate + Trust + Policy devin fundația, nu un strat** — tot ce e deasupra se
   sprijină pe ele; nimic nu poate fi „mai inteligent decât permisiunile lui".
2. **Verificare la fiecare hop, nu doar la final** — fiecare delegare are un pas de
   validation (nu doar Synthesis→Critic).
3. **Memory + Observability sunt cross-cutting** — nu coloane laterale, ci planuri
   verticale care ating toate straturile.

```
                     ┌──────────────────────────────────────────────┐
                     │          OBSERVABILITY (colectiv graph)       │
                     ├──────────────────────────────────────────────┤
                     │          COLLECTIVE MEMORY (agent/team/fabric)│
                     ├──────────────────────────────────────────────┤
                     │  SELF-OPTIMIZATION (policy loop, sandboxed)   │
                     └──────────────┬───────────────────────────────┘
                                    ↓
                  ┌─────────────────────────────────────────────┐
                  │          AGENT NETWORKS (teams, DAGs)        │
                  │   plan ─ delegate ─ verify ─ synthesize      │
                  └───────────────┬─────────────────────────────┘
                                  ↓
                  ┌─────────────────────────────────────────────┐
                  │            AGENTS (logical contexts)        │
                  │  identity · capabilities · goals · policies  │
                  │  memory · reputation · relationships          │
                  └───────────────┬─────────────────────────────┘
                                  ↓
   ┌──────────────┬──────────────┼──────────────┬──────────────┐
   ↓              ↓              ↓              ↓              ↓
 COMPUTE        MODELS         TOOLS          DATA          MEMORY
 (nodes/       (GGUF +       (MCP, OCR,    (datasets,    (owned
  reservations) manifest)     embeddings)   knowledge)     scopes)
   └──────────────┴──────────────┼──────────────┴──────────────┘
                                  ↓
                  ┌─────────────────────────────────────────────┐
                  │      CAPABILITY DISCOVERY LAYER             │
                  │  discover → evaluate → benchmark → register │
                  └───────────────┬─────────────────────────────┘
                                  ↓
        ┌─────────────────────────────────────────────────────────┐
        │  IDENTITY · TRUST · AUTH · PERMISSIONS · OWNERSHIP      │
        │  (Ed25519 · TrustStore · tokens/tiers · sandbox)        │
        └─────────────────────────────────────────────────────────┘
```

**Agent Power ≠ Permission** rămâne invariant: un agent cu capabilities mai mari nu
dobândește automat drepturi mai mari — permisiunile sunt declarate în policies și
aplicate la fiecare hop (admission, tool gate, budget, sandbox).

### 4.1 Abstractions core (extensibile, nu fixe)

```
AgentRecord {
  agent_id          // derivat din Identity + rol (ex: PeerId + "research")
  node              // node-ul gazdă (unde rulează contextul)
  capabilities      // semantic (CapabilityKind[]) + execution (models/tools)
  role              // generalist | specialist | planner | critic | ... (extensibil)
  policies          // permissions, budgets, sandbox, allowed_models/tools
  memory_scopes     // owned memory (vezi §4.4)
  reputation        // scor agregat (vezi §4.6)
  relationships     // teams, peers, dependencies
  state             // lifecycle: registered → ready → busy → suspended → retired
}

AgentTask {
  task_id, parent_id      // suportă DAG-uri
  input_schema/output     // arbitrar, nu doar prompt→text
  required_capabilities   // semantic requirements
  required_resources      // WorkloadRequirements (există)
  budget                  // resurse maxime, deadline
  verification            // {none | self | critic | consensus} + confidence floor
  priority, owner         // mapping la tokens/tiers existent
}

AgentMessage {             // peste transportul libp2p existent
  from, to, kind          // ask | delegate | reply | verify | ping
  payload, schema         // serde-typed, size-capped (pattern protocol)
  nonce, signature        // reuse semnarea canonică existentă
}

CapabilityAdvertisement {  // extinde ComputeAdvertisement existent
  physical: ComputeCapability,     // există
  semantic: Vec<CapabilityClaim>,  // există în hub — se aduce în ad
  tools: Vec<ToolDescriptor>,      // NOU: MCP/OCR/embeddings/etc.
  policies_hint,                   // cost, availability windows
}
```

### 4.2 Delegație = generalizează bucla existentă

Bucla curentă din `route_request` (sign → requirements → plan_and_reserve → dispatch →
stream → release → adapt) devine:

```
discover → match → plan(DAG) → reserve → execute(per-hop) → verify → learn → release
```

Verificarea per-hop e pasul nou: după ce un agent execută un sub-task, rezultatul trece
printr-un verificator (self-check cu schemă + optional critic/consensus) înainte să
alimenteze următorul stage. `ExecutionPhase` existent se extinde cu `Verify`/`Disagree`.

### 4.3 Unificarea limbajelor de capability

Există două limbaje care NU sunt cross-wired:
- `ComputeCapability` (fizic: RAM/VRAM/GPU/engine/models)
- `CapabilityKind`/`ModelCapabilities` (semantic: Coding/OCR/Vision/...)

Propunere: **un singur `AgentCapability` = { semantic: Vec<CapabilityClaim>, execution:
ExecutionCapability }**, unde `ExecutionCapability` extinde `ComputeCapability` cu
`tools: Vec<ToolDescriptor>`. Matcher-ul devine compozițional: semantic requirements se
rezolvă prin `hub::requirements::match_requirements`, physical prin
`CapabilityMatcher` existent. Asta transformă `required_capability: Option<String>`
(astăzi un singur string) în `required_capabilities: Vec<CapabilityRequirement>` +
`required_execution: WorkloadRequirements`.

### 4.4 Collective memory — niveluri + politici

Design: memory e un set de **scopes** cu ownership. Scopurile nu sunt toate shared.

| Nivel | Proprietar | Vizibilitate | Exemplu |
|---|---|---|---|
| Agent memory | agent | privat (default) | context, learnings proprii |
| Team memory | echipa de agenți | membrii echipei | rezultate intermediate, convenții |
| Network memory | rețeaua (trusted) | noduri cu trust | cataloage de capability, benchmark |
| Fabric knowledge | întreaga rețea (opt-in) | public cu politici | modele verificate, lecții |

Politici obligatorii per scope: ownership, access (cine citește/scrie), retention (TTL),
privacy (ce e logat — respectă invariantul „prompts/outputs never logged" pentru scope-urile
private), provenance (cine a scris, cu ce sursă), trust floor (cine poate contribui).

Implementare fără I/O nou masiv: memorie persistentă pe node-ul gazdă (SQLite — avem deja
rusqlite) + sync opțional între noduri prin mesaje tipate, respectând ownership. Nu se
construiește un „global brain" în Phase 1.

### 4.5 Reputation — model robust

Se unifică ceea ce există deja (TrustStore EMA + CircuitBreaker + CompensationLedger +
ContributionProfile) într-un **AgentReputation** cu factori separați, nu un scor unic opac:

```
AgentReputation {
  reliability   // success_rate ponderat pe task-uri verificate (există: verified/total)
  quality       // scor mediu de verificare (rezultate care au trecut critic/consensus)
  latency       // percentile pe execuții (există: RuntimeMetrics)
  uptime        // disponibilitate în fereastră (există: availability/health)
  safety        // zero încălcări de policy/sandbox (NOU: penalizează doar încălcări)
  provenance    // claims verificate vs inferate (există în hub, se aduce în registru)
}
```

Decizii:
- Scorurile sunt **per (agent, capability)** — un agent poate fi excelent la OCR și
  mediocru la coding. Scorul global e doar o proiecție.
- **Doar eșecuri criptografice/policy încalcă safety** (invariant existent: rețeaua nu
  pedepsește erorile de rețea).
- Reputation alimentează planner-ul ca factor de scor, cu **weight configurabil**, nu ca
  filtru hard (filtrele hard = trust + capability match).
- Extensibil: fiecare factor are o definiție de calcul izolată, testabilă pur (pattern
  compute crate).

### 4.6 Talent tree — graph dinamic de capabilities

Nu gamification: este **capability graph** cu dependențe (capability A deblochează B).
Proprietăți cerute:
- Dinamic — se adaugă noduri fără schimbare de cod la nivel de graph (enum + registry).
- Fără niveluri hardcodate, fără final.
- Fiecare nod are: prerequisite capabilities, resurse necesare (estimate), provenance
  (verified/experimental), cost (sintetic — quota), confidence.

Exemplu (doar ilustrativ — se construiește din capability claims reale):
```
Embeddings → SemanticSearch → RAG → KnowledgeAgent
ToolCalling → MCP → MultiToolAgent → AutonomousAgent
CodingModel → CodeReview → RepoAgent
```

Talent tree-ul nu e motor de „level up" — e un **lookup table cu precondiții** pe care
planner-ul îl consultă ca să știe ce capability-uri compuse sunt realizabile pe resursele
curente. Se alimentează din Capability Discovery Layer (mai jos).

### 4.7 Capability discovery layer (pipeline extern)

Pipeline-ul propus de tine (discover → evaluate → compatibility → resource estimation →
benchmark → quality score → register → available) se construiește pe:
- `hub::catalog` (discover: HuggingFace) — există.
- `hub::capability::classify` (evaluate semantic) — există.
- `manifest` + registry (compatibility/resource estimation: GGUF size, RAM/VRAM estimates
  din `ServedModel::estimate_*`) — există.
- **benchmark + quality score — NOU**: un harness de benchmark (puteem începe cu
  `llama-bench`/prompt-uri standard) care produce scoruri atribuibile.
- **register — NOU**: transformă un model extern într-o capability înregistrată în
  fabric (advertisement semantic), nu doar un fișier în registry local.

### 4.8 Security model (deliberat de la început)

Separare strictă, păstrând tot ce există (semnare, replay guard, trust, tokens):

| Concept | Mecanism |
|---|---|
| Identity | Ed25519 per node (există); agent_id derivat + semnat de node |
| Authentication | semnare canonică per mesaj (există `sign_manifest` pattern) |
| Authorization | tokens/tiers + per-agent policies (NOU: policy engine) |
| Capabilities | claims semnate de node-ul gazdă, provenance obligatoriu |
| Permissions | per-agent allowlist de tools/models/nodes (NOU) |
| Resource ownership | ReservationLedger + admission (există), extins cu budgets |
| Network access | trust + mDNS; sandbox de rețea pentru agenți (NOU) |
| Tool access | tool gate per agent, cu aprobare pentru tools riscante (NOU) |

**Sandbox (Control Exploration)**: agenții în mod `EXPLORATION`/`EXPERIMENTAL` primesc
un context de execuție cu limite hard: resurse (quota), network (doar whitelist),
filesystem (doar director propriu), secrets (interzis — niciodată), host access (interzis
pe noduri străine). Normal = limitat la policies; Explorare = sandbox extins dar măsurat;
Experimental = doar pe nodul gazdă, cu audit complet.

## 5. Faze propuse (reordonate față de propunerea ta — justificare)

Ordinea ta (identity → comm → delegation → scheduling → talent tree → workflows → memory
+ reputation → self-opt → scale) are o problemă: **resource scheduling și memory/reputation
sunt deja parțial implementate**, iar **verificarea** e fundamentală pentru memory și
reputation de încredere — deci trebuie înaintea lor.

| Fază | Conținut | Baza existentă | Verdict |
|---|---|---|---|
| **P0 — Agent substrate** | `AgentRecord` + `AgentRegistry` (local, pe node); unifică capability; `AgentTask` generic cu schemas; lifecycle | identity, compute, hub, protocol | Construit pe existent, e piatra de temelie |
| **P1 — Discovery semantic** | extinde `ComputeAdvertisement` cu semantic claims + tools; agent registry broadcast | p2p ad-uri, hub taxonomy | Subțire, dar deblochează tot |
| **P2 — Agent messaging** | `AgentMessage` tipat (ask/delegate/reply/verify) peste request-response | p2p, protocol, semnare | Esențial pentru P3 |
| **P3 — Delegation DAG** | generalizează plannerul: task-DAG cu contracts + verify per hop; orchestrator (master agent) | fabric planner, expert, batch | Inima sistemului |
| **P4 — Verification + consensus** | critic agents, cross-check, confidence, dispute resolution | — (nou, pur, testabil) | Înainte de reputație, obligatoriu |
| **P5 — Collective memory** | memory scopes cu ownership/privacy/retention; team + network memory | SessionAccount, SQLite | După verification (memoria de încredere) |
| **P6 — Agent reputation** | unifică TrustStore + breaker + compensation într-un AgentReputation per-capability | tot ce există | Agregare + weight în planner |
| **P7 — Policy + budgets per agent** | policy engine: permissions, budgets, sandbox, exploration modes | tokens, quota, matcher | Securitatea la scară |
| **P8 — Talent tree + discovery pipeline** | capability graph + benchmark/register | hub, manifest | Conținut peste P1 |
| **P9 — Collective workflows** | workflow-uri compuse (Research→Finance→Docs→Synthesis→Critic) ca șabloane pe P3 | planner | Demonstrația de produs |
| **P10 — Self-optimization** | policy loop: ajustează placement/models pe baza rezultatelor măsurate | adapt(), metrics, quota | Ultimul, cere P4+P6 |
| **P11 — Agent economy** | price/SLA per capability, buying/delegation între agenți | quota, compensation | Modular, târziu, NU acum |

### Recomandare de scope

- **Implementăm acum (P0–P3)**: sunt generalizări ale codului existent, cu valoare
  imediată și risc controlat. P0+P1 înseamnă „un node poate anunța capabilities semantice
  + tools" — extensie naturală a advertisement-ului.
- **Experimental (P4–P7)**: pure, testabile, dar cu design de făcut. P4 și P7 înaintea
  P5/P6 (verificare + securitate înainte de trust/memory).
- **Construit pe baze open-source (P8, discovery)**: benchmark poate folosi instrumente
  existente (llama-bench, harness-uri); nu reinventăm.
- **Open-source deja existent de folosit**: MCP (tools), llama.cpp (engine), HuggingFace
  (catalog) — le integrăm, nu le duplicăm.
- **Nu acum (P10–P11)**: necesită maturitate P4+P6; se proiectează modular (economy
  folosește deja quota/compensation ca abstracții).

## 6. Invariants care se păstrează (nedeclarabile, obligatorii)

1. **Verify before use** — se extinde de la artefacte la rezultate de task.
2. **Doar eșecurile criptografice/policy pedepsesc** — erorile de rețea sau de execuție
   NU ating reputation safety.
3. **Determinism** — canonical serialization, ranking deterministic, persistence tmp+sync+
   rename (pattern existent).
4. **Secrets stay local** — policy engine interzice agenților accesul la secrets;
   invarianții de 0600/never-log se aplică și pentru memory scopes private.
5. **Prompts/outputs never logged** — memory scopes private NU ajung în audit; auditul
   înregistrează doar evenimente de securitate.
6. **Engine = subproces** — agenții orchestrează engine-uri externe; nu FFI.
7. **Agent Power ≠ Permission** — capabilities mari nu acordă drepturi mari; policies
   sunt declarative și aplicate la fiecare hop.

## 7. Riscuri & decizii deschise

| Decizie | Opțiuni | Recomandare |
|---|---|---|
| Agent = proces nou vs context logic | proces per agent (greu, scala proastă) vs context pe node | **context logic pe node**; proces doar pentru execution sandbox |
| Memory shared = sync global vs per-node | global brain (complicat, privacy risk) vs per-node + opt-in sync | **per-node, sync opt-in prin mesaje tipate** |
| Tools în capabilities | enum fix (rigid) vs descriptor serde extensibil | **descriptor structurat extensibil** (pattern hub `PipelineTag::Other`) |
| Reputation în scoring | filtru hard (risc) vs weight configurabil | **weight configurabil** în planner, filtrul hard rămâne trust+capability |
| Cine orchestrează DAG | coordinator central (pattern existent) vs agent orchestrator distribuit | **începem cu orchestrator pe node-ul coordinator** (pattern existent), distribuit mai târziu |
| Verificarea rezultatelor | doar self-check (schemă) vs critic agent obligatoriu | **self-check default, critic la cerere (confidence floor)** |

## 8. Definiția „done" pentru Phase 1 (propunere de contract)

P0 + P1 sunt considerate gata când:
- un node poate rula mai multe `AgentRecord`-uri logice cu capability claims semnate;
- advertisement-ul transportă claims semantice + tools descriptors;
- `hub::requirements` matcher-ul e wired în matcher-ul de execuție (un singur verdict
  compozițional);
- dashboard-ul arată agenții (pe lângă workers) într-o vizualizare simplă;
- teste: unit pentru `AgentRegistry`/capability unificat + E2E pentru advertisement
  semantic între 2 noduri reale;
- ROADMAP/README actualizate în aceeași push.
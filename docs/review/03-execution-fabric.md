# Review — Execution Fabric (crates/fabric, compute + părțile de execuție din distributed)

Sursă: subagent de adâncime (2026-08-21), read-only. Afirmațiile-cheie au fost
re-verificate în cod de coordonator (trei selectori, fallback legacy,
dependența fabric→compute one-way). Referit din `docs/TECHNICAL_REVIEW.md`.

## 1. Responsabilitate & suprafață publică

**`decentraai-fabric` (6.146 LOC, 10 module)** — planner-ul de execuție
engine-aware, pur (fără I/O, fără async): `plan.rs`/`planner.rs` (ExecutionPlan,
ExecutionPlanner), `network.rs` (NetworkGraph + cost de transfer), `kv.rs`
(planner KV-cache-aware), `expert.rs` (fabric de expert/distributed-MoE),
`engine.rs` (EngineKind + ABI de capability), `decision.rs` (decizii
autonome), `batch.rs`, `advisory.rs`. Toate tipurile serde-serializabile;
deciziile deterministe și unit-testabile.

**`decentraai-compute` (5.963 LOC, 18 module)** — domeniul pur de compute
sharing: `capability.rs`, `availability.rs`, `requirements.rs`, `matcher.rs`
(CapabilityMatcher), `reservation.rs`/`scheduler.rs` (ReservationLedger,
ComputeScheduler), `registry.rs` (ComputeRegistry), `fabric_graph.rs`
(FabricGraph/CapabilityGraph/ComputeGraph — fabric v2, §130),
`placement.rs` (PlacementEngine — scoring compozit + hard gates),
`contribution.rs`/`contribution_state.rs`/`compensation.rs`/`credits.rs`/
`quota.rs`/`loadbalance.rs`/`resource_contribution.rs` (economia de contribuție).
Fără I/O; totul serde-serializabil.

**`decentraai-distributed` — doar fișierele de execuție**: `compute.rs`
(ComputeManager, 4.654 linii), `router.rs`, `lib.rs` (route_request/
route_request_streamed), `session.rs`, `queue.rs`, `breaker.rs`, `fallback.rs`,
`worker.rs`, `tracker.rs`, `rate_limit.rs`, `replay.rs`, `probe.rs`.

## 2. Abstractions-cheie & flux

**Direcția de dependență e curată**: `fabric → compute` one-way, `compute` are
zero dependențe interne, `distributed` depinde de ambele. Fără cicluri.

**Ownership-ul ține în cod**: "planner owns who, scheduler enforces capacity"
e literal adevărat — `fabric::ExecutionPlanner::plan` (planner.rs:386)
selectează, `compute::ComputeScheduler::reserve_worker` (scheduler.rs:185)
re-validatează prin `CapabilityMatcher` și înregistrează în ledger, iar
worker-ul re-verifică la admitere (lib.rs:844). Trei straturi care cad de acord
pe headroom. Fără cale de scurgere de rezervări găsită.

**Fluxul verificat end-to-end**: `route_request_streamed` (lib.rs:1333) →
`requirements_for` → `record_decision` → `plan_and_reserve` (compute.rs:1580) →
`send_request_streamed` (router.rs:161) → worker `on_infer` admission +
ReplayGuard + rate limit → `stream_request_to_terminal` (lib.rs:1805) →
release (ambele părți curate).

**Retry bounded, idempotency-safe**: `DistributedError::is_retryable()`
(lib.rs:231-251, cu raționament atent documentat) — doar `P2PError` e
retryable; respingerea definitivă a worker-ului sau cancel nu se re-trimit
niciodată (generare non-idempotentă: fără dublare de token/KV accounting).
Calea de streaming rămâne single-attempt + fallback legacy (retry mid-stream
ar duplica output parțial).

## 3. Integrare & cuplare

`distributed` consumă `compute` de 80 de ori și `fabric` pe calea
plan_and_reserve. Runtime-ul expune deciziile via `/v1/execution`,
`/v1/placement/plan` (engine determinist contra grafului live, §130.3) și
`/v1/fabric/graphs` (operator+).

## 4. Semnale de maturitate

- Docs bune, cu raționamente "de ce" per termen de scoring (testul
  `continuation_is_steered_to_prefix_host_by_locality_score`, planner.rs:846-860).
- `compute` e cel mai bine testat dintre crates-urile pur pure (110 teste).
- Verdicturi: **compute 5/5** (domeniu pur, determinist, complet), **fabric
  4/5** (excelent dar cu scaffolding mort), **distributed-execution 3/5**
  (cale fabric reală și E2E-testată, dar god-fișiere + duplicare de scheduler +
  ocoliri de politică pe calea legacy).

## 5. Mirosuri & riscuri concrete

1. **TREI selectori vii cu 5–6 formule de scoring divergente.** Planner-ul
   fabric (scoring compozit network+KV+afiinitate+cache), scheduler-ul compute
   (rezervări) și legacy `discovery::scheduler` (select_worker, discovery/
   scheduler.rs:151-165) care la egalitate folosește `max_by` **fără tie-break
   PeerId — non-determinism**, încălcând invariantul de determinism declarat.
2. **Fallback-ul legacy re-emite cereri după `RequestTimeout`**
   (lib.rs:1315-1329), **ocolind politica at-most-once** `is_retryable()`
   (lib.rs:249) — work non-idempotent poate fi duplicat pe această cale.
3. **`SessionAccount` crește fără bound** (session.rs:46-134, fără TTL) —
   starea de afinitate KV crește nelimitat.
4. **`InferFailed.retryable=true` e semnal mort**: router-ul aplatizează toate
   `InferFailed` în non-retryable `AllWorkersFailed` — două vocabularuri de
   retry care nu se acordă.
5. **~1.500 de linii de scaffolding mort/parkat**: `StrategyKind`/
   `TrustTier`/`PerformanceProfile`, tipuri de placement market,
   `orchestrate`/`Observation`, `rebalance_advisory`/`replan_decision`; plus
   câmpuri de config moarte (`retry_backoff_ms`, `use_reputation` — vezi și
   fix-ul din c08c39d pentru partea de config wire-uită).
6. **Gated-inert (onest, per AGENTS.md)**: expert routing pinat off
   (engine.rs:309-329 — niciun engine nu anunță capabilitatea, fallback
   whole-model e ce rulează); split-ul prefill/decode ireachabil
   (`decision::evaluate` e mereu apelat cu `allow_fanout=false`, compute.rs:1378).
7. **`RequestFacts` construit copy-paste de 5× cu `transfer_mib` hardcodat la
   0** — termenul de rețea M19 (transfer cost) nu e niciodată dimensionat pe
   calea live.
8. **`distributed/compute.rs` (4.654 linii) e god-fișier** — 149 de funcții,
   ~60 de `Mutex::lock().unwrap()` pe site-uri de request-path, unwrap-uri în
   producție la lib.rs:466/1255/1531, queue.rs:144.

## 6. Verdict (1–5)

| Crate | Scor | Justificare |
|---|---|---|
| compute | **5** | Domeniu pur, determinist, complet, bine testat; graful fabric + placement engine (130) sunt cea mai bună piesă nouă |
| fabric | **4** | Planner excelent, scoring explicabil, degradare grațioasă onestă; penalizat pentru scaffolding mort (~1.500 linii) |
| distributed (execuție) | **3** | Calea fabric reală și E2E-testată, retry idempotency-safe documentat; dar god-fișiere, duplicare de scheduler, ocolire de politică pe calea legacy, sesiuni fără TTL |

## Top 5 riscuri pe termen lung (planul execution)

1. **Unifică selectorii** — trei formule de scoring divergente (una chiar
   non-deterministă la egalitate) erodează predictibilitatea; un singur selector
   (planner-ul fabric), legacy `discovery::scheduler` șters.
2. **Supune fallback-ul legacy aceleiași politici `is_retryable`** — re-emiterea
   după RequestTimeout poate duplica generare non-idempotentă.
3. **Sesiuni cu TTL** — `SessionAccount` fără eviction = scurgere de memorie
   pe coordonator.
4. **Acordă vocabularul de retry worker↔coordonator** — `retryable=true` e
   ignorat; fie consumat, fie scos.
5. **Șterge scaffolding-ul mort + unifică `RequestFacts`** — `transfer_mib=0`
   face termenul de rețea M19 decorativ pe calea live.

# Review tehnic DecentraAI — arhitectură, modularitate, integrare, maturitate

Data: 2026-08-21 · Commit-ul analizat: `4176d98` (pre-consolidare) · Status actualizat la: `c08c39d`

Acest document consolidează review-ul arhitectural complet al codebase-ului
DecentraAI (analiză read-only: fără teste noi, fără implementări). A fost
produs prin inspecție directă (graful de dependențe, god-modulele, verificarea
în cod a invariantelor, count-uri) plus cinci review-uri de adâncime pe planuri,
executate de subagenți; afirmațiile-cheie din rapoarte au fost re-verificate în
cod înainte de includere. Rapoartele brute sunt salvate în `docs/review/`.

---

## Verdictul pe scurt

**DecentraAI este un prototip de foarte înaltă calitate, nu un sistem
production-hardened.** Construit în 12 zile (567 de commit-uri, 2026-08-09 →
21), 20 de crates, ~94.000 LOC Rust, ~1.190 de teste, zero `unsafe`, CI
complet. Maturitatea reală e concentrată în nucleul pur decizional
(fabric/compute/agents) și în disciplina de onestitate ("verificat vs dedus vs
sugestie"); fragilitatea reală e concentrată în stratul de servicii
(runtime/API/dashboard) și în lanțuri de încredere implementate dar
neverificate pe căile live.

Maturitate per plan (1–5): **data plane 3** · **service plane 2** ·
**execution fabric 4** · **intelligence plane 3.5** · **inginerie
cross-cutting 4**.

---

## 1. Ce este real, verificat, funcțional

### Nucleul pur este excelent

`crates/agents` (12,5k LOC), `crates/fabric` (6,1k) și `crates/compute` (6k)
sunt genuine pure în cod (zero `std::fs`/`tokio`/`reqwest` în producție) —
toate deciziile (planning, scoring, rezervări, consens, reputație, cunoaștere,
benchmark) sunt funcții deterministe, serde-serializabile, testabile cu intrări
sintetice. Testele le confirmă (agents 221, compute 110, fabric 122).
Separarea "planner owns who, scheduler enforces capacity" este literal
adevărată în cod (fabric `planner.rs:386` → compute `scheduler.rs:185` →
re-verificare worker-side la admitere).

> **Caveat (corectat în addendum):** puritatea e adevărată la nivel de *cod*,
> nu de *graf de dependențe*. `compute` depinde de libp2p doar pentru
> `PeerId`, `agents` depinde de `hub` (crate I/O) doar pentru tipuri pure.
> Vezi §6, decizia limitatoare #6 și ROADMAP §132.4 (open).

### Invariantul "verify before use" este real pe calea de download

`crates/p2p/src/transfer.rs` face exact ce promite: BLAKE3 per chunk, gate
Merkle final + hash de fișier complet înainte de rename atomic, carantină cu
metadata, reputație atinsă **doar** de eșecuri criptografice (erorile de rețea
nu ating scorurile). Path-safety pentru nume controlate de peer e bine făcută
și testată (`validate_artifact_component`, transfer.rs:347–370).

### Disciplina de onestitate este reală, nu doar declarată

Verificat în cod: fără evidență → confidence 0.0 (`knowledge.rs:171`);
receipt-urile eșuate nu creditează; RAG-ul semantic nu inventează scoruri
(fallback structural etichetat `mode:"structural"`); verdictul benchmark-ului e
*paired* peste task-uri evaluate în ambele moduri (MIN_SAMPLES=5 /
MIN_MARGIN=0.05); reputația necunoscută = 0, nu penalizare. E cel mai valoros
activ al proiectului.

### Validare live reală

Validare pe două noduri fizice (Laptop i5 `dca-GriBWu` ↔ Desktop i7
`dca-NGE65Z`, `docs/TWO_NODE_VALIDATION.md`): inferență remote bidirecțională,
chat SSE remote, probe RTT reale, execuție prin fabric. E2E-uri automate pe
loopback (p2p e2e_transfer 675 linii, compute_e2e 880 linii).

### Inginerie de proces solidă

CI: fmt + clippy `-D warnings` + test + build + cargo-audit + gitleaks +
validare de roadmap (tracker de 345 de pași). 0 teste `#[ignore]`. Docs `//!`
cu "de ce" și threat-model la nivel de modul. Changelog, deploy systemd,
scripturi de upgrade idempotente.

---

## 2. Ce este fragil sau doar schelet

### God-modulele sunt arhitectura

- `crates/runtime/src/api.rs` — **16.720 linii**, ~170 funcții, 92 de rute,
  165 de teste inline, `ApiState` cu **19 câmpuri `Option<Arc<...>>`** (unu
  per subsistem opțional).
- `crates/node-cli/src/main.rs` — 7.339 linii; `node_start` = compoziție de
  ~1.000 de linii cu 20+ variabile `let mut ... Option`.
- `crates/distributed/src/compute.rs` — 4.654 linii, 149 de funcții
  (ComputeManager: trust, advertismente, contribuții, quota, compensație,
  credite, execuții, fabric graphs).
- `dashboard.rs` (4.993) + `dashboard_v2.rs` (1.137) — **două dashboard-uri
  paralele** (`/ui2` permanent), HTML+CSS+JS în raw-string-uri Rust (~5.000 de
  linii de JS, `innerHTML` de 226 de ori, protejat manual cu `esc()`).

Contrast: când proiectul vrea, produce module mici și impecabile (`queue.rs` —
FIFO cu release RAII prin `Drop`; `inference-adapter` — trait curat).
God-modulele sunt consecința vitezei, nu a lipsei de talent.

### Lanțuri de încredere implementate dar neverificate în producție

1. **Injecție de modele neautentificate prin anunțuri.** `verify_manifest_signature`
   are zero apelanți în producție (doar testele protocol); `sign_manifest` e
   chemat doar la emiterea anunțului. Handler-ul `ManifestAnnouncement`
   (p2p/src/lib.rs:771) livrează manifestul **fără verificare de semnătură**,
   iar `ShareMode::Auto` (node-cli/main.rs:2832) descarcă automat de la orice
   peer mDNS. Un peer LAN poate împinge octeți arbitrari în `models/`, pe care
   nodul îi re-anunță și îi poate încărca în llama-server. Lipsesc și
   semnătura verificată și ancora de încredere (trusted set-ul scheduler-ului).
2. **Matcher-ul unificat nu e wired.** `match_agent` (matcher.rs:127) are doar
   teste ca apelanți; orchestratorul folosește doar `match_agent_semantic`
   (agent_orchestrator.rs:140) — gate-urile fizice (trust, RAM/VRAM, model
   allowlist) **nu se aplică pe calea de delegare**.
3. **PolicyEngine (P7) are zero apelanți.** `allow_remote` e setat dar
   neaplicat; `AgentRuntime::process_one` acceptă orice `Delegate`.
4. **Envelope semnate duplicate.** `SignedComputeAdvertisement` și
   `SignedAgentAdvertisement` sunt envelope identice (protocol/src/lib.rs:268–350),
   cu 4 funcții sign/verify aproape identice, deosebite prin sniffing
   (p2p_handler.rs:151–177).
5. **Două concepte de PeerId.** `identity::PeerId` (hex blake3) neconsumat
   în afara crate-ului; rețeaua folosește `libp2p::PeerId`; CLI-ul printează
   ambele (main.rs:2757–2759).

### Trei selectori vii, cu formule divergente

Planner-ul fabric (scoring compozit network+KV+afiinitate), scheduler-ul
compute (rezervări) și legacy `discovery::scheduler` (la egalitate folosește
`max_by` **fără tie-break PeerId — non-determinism**). Fallback-ul legacy
(lib.rs:1315–1329) re-emite cereri după `RequestTimeout`, **ocolind politica
at-most-once** `is_retryable()`; semnalul worker-side `InferFailed.retryable=true`
e mort (router-ul aplatizează tot în non-retryable).

### Bugs deterministe în data plane

- **Carantina strică resume-ul permanent:** `quarantine_staging` redenumește
  doar `.part`, lasă `.done`; la retry, chunk-urile "verificate" sunt sărite,
  regiunile rămân zero-filled, hash-ul final eșuează pentru totdeauna până la
  ștergerea manuală a bitmap-ului. Carantina se declanșează și pe erori pure de
  rețea, contrazicând propriul doc.
- **Codec-ul bufferează până la 96 MiB per frame de control** înainte de
  cap-ul de parsare de 1 MiB — amplificare de memorie (DoS) inbound; fără
  bound de fluxuri concurente.
- **`RegistryServer` re-hashează fiecare model complet la fiecare cerere** —
  O(dimensiune model) per cerere.
- **`Manifest.file_name` are dublu sens** (basename vs relative path din
  registry) — modelele din subdirectoare sunt servite dar refuzate de
  downloader: nedescărcabile end-to-end.

### Suprafața HTTP e inconsistent protejată

`/v1/token` **întoarce master token-ul oricărui apelant local** (api.rs:5941,
folosit de dashboard); patru endpoint-uri P14 (contribution_state,
credits_balance, credits_events, verified_compute_history) sunt complet
neautentificate; `/metrics` e deschis. Coerent sub modelul loopback-only, dar
decizie nerostită. Protecția e per-handler (22+ site-uri `require_master`),
fără extractor/middleware, fără inventar de endpoint-uri protejate.

### Ciclu de viață subproces fragil

SIGKILL fără reap la Drop, fără SIGTERM grațios, `allocate_port` TOCTOU
(bind-0/release/re-bind), stderr copiilor e null, și doar llama-server are
supervisor (`ensure_healthy`) — OCR/STT/skills/TTS căzuți rămân morți până la
restart. Plus 4 probe de health blocante (2s fiecare) în handler-ul `/status`
(api.rs:5202–5217) pe thread-uri de executor.

---

## 3. Integrarea componentelor: hub-of-hubs, fără fațadă

Graful de dependențe: `node-cli` → 17 crates, `runtime` → 13, `distributed` →
13. **Nu e hub-and-spokes, e hub-de-hub-uri**: runtime depinde de distributed,
deci fan-in-ul tranzitiv al control plane-ului e tot workspace-ul. Nu există
niciun tip `Node`/`ControlPlane`/`FabricClient` — compoziția e `ApiState` (19
`Option<Arc>`) + `node_start` cu `attach_*`.

- **Subsisteme re-implementate în paralel:** rate limiting (3 copii de
  sliding-window în api.rs + unul keyed-by-peer în distributed), cozi (runtime
  `queue.rs` vs distributed `RequestQueueManager`), probe de health (TCP
  hand-rolled vs reqwest), pattern-ul tmp+sync+rename copiat în 4 crates.
- **Patru suprafețe paralele peste aceeași stare:** REST (92 rute) + MCP
  (~30 de tool-uri) + CLI (22 comenzi) + dashboard.
- **Două sisteme de config care nu se întâlnesc:** YAML-ul operator (64 de
  knobs, strict validat) vs `distributed::config::InferenceConfig` — toate
  site-urile live foloseau `InferenceConfig::default()` (max_retries hardcodat
  la 3). **FIXED în c08c39d** (vezi §5).

---

## 4. Unde ne îndreptăm: direcția proiectului

Traiectoria e consecventă: de la "fabric de inferență distribuită" către
"infrastructură de inteligență colectivă", cu fabric-ul ca strat de execuție.

1. **Fabric v2 — graphs + placement engine** (§130): fabric-ul raționează
   despre noduri ca entități de resurse, cu planner explicator.
2. **Colectiv inteligent ca produs** — cunoaștere, evidență, benchmark,
   workflow-uri, memorie colectivă.
3. **Productizare** — `decentraai node` daemon unu-proces, instalare app,
   auto-upgrade, tool-uri locale (TTS/OCR/STT/skills).
4. **Economie de contribuție** — credite, quota, consumer keys, compensație
   idempotentă.

Pericolul de direcție: **feature velocity (60 feat vs 24 fix în ultimele 120
de commit-uri) + tracker de 345 de pași** transformă roadmap-ul în sursă de
adevăr, iar codul rămâne în urmă. Docs-urile declarau 9 crates/106+ teste;
realitatea: 20 crates/1.190. Matcher-ul unificat, PolicyEngine,
self-optimizerul, economia, consensul în verificare, receipt-urile semnate
(P13) sunt declarate "DONE"/"wired" dar sunt pure-logic-only sau CLI-only în
cod. Reputația de agenți (alimentată din delegații reale) e doar în memorie —
se pierde la restart.

---

## 5. Status la `c08c39d` (consolidare livrată)

| Constatare din review | Severitate | Status |
|---|---|---|
| Config distribut mort: `InferenceConfig::default()` peste tot, `max_retries` hardcodat la 3 | Medie | ✅ FIXED `c08c39d` — `InferenceSection` + `from_section()` wire-uit în toate cele 4 site-uri; knobs documentate în `node.example.yaml`; test de regresie |
| `configs/node.schema.json` era stub (7 chei `{"type":"object"}`) | Medie | ✅ FIXED `c08c39d` — schema completă (456 linii) care oglindește `NodeConfig`; `node.example.yaml` validează cu jsonschema |
| CHANGELOG: 5× `[Unreleased]` | Joasă | ✅ FIXED `c08c39d` — o singură secțiune cu subsecțiuni |
| `docs/ARCHITECTURE.md`: crates fantomă (`policy-engine`, `chunk-store`, …) | Joasă | ✅ FIXED `c08c39d` — layout real + notă |
| Numere de teste greșite în docs (1018/1184/106+) | Joasă | ✅ FIXED `c08c39d` — corectate la 1190 |
| Dead deps: `fabric→anyhow`, `providers→tokio` | Joasă | ✅ FIXED `c08c39d` |
| `inference-adapter` outlier (edition 2021, MIT, deps non-workspace) | Joasă | ✅ FIXED `c08c39d` — edition 2024, `license.workspace`, `thiserror.workspace`, `tokio.workspace` |
| `compute→libp2p` doar pentru `PeerId`; `agents→hub` doar pentru tipuri pure | Medie | ⏳ OPEN — fix real = crate-frunză `decentraai-types` (refactor mare; documentat în ROADMAP §132.4) |
| CI fără llama-server real (httpmock/fake) | Medie | ⏳ OPEN — ROADMAP §132.4 |
| `hub` download (cel mai sensibil I/O) fără teste I/O reale | Medie | ⏳ OPEN — ROADMAP §132.4 |
| **Injecție de modele neautentificate** (semnătură de manifest neverificată + `ShareMode::Auto` fără trust) | **Critică** | ✅ FIXED `79d9156` — `ManifestAnnouncement` poartă `signer_public_key`; swarm-ul verifică anti-spoof (pk → peer conectat) + Ed25519 peste manifest; forjat e drop-uit înainte de consumatori; `require_signed_announcements` (era dead config) e acum aplicat în `run_share_worker` (`ShareMode::Auto` descarcă doar semnate când e setat); test unit + E2E forged-drop |
| Carantină → bitmap stale → artefact permanent nedescărcabil; carantină și pe erori de rețea | **Critică** | ✅ FIXED `b0b386e` — `quarantine_staging` mută acum și `.done` (bitmap-ul), deci retry-ul pornește curat; carantina se declanșează doar pe eșec crypto, nu pe erori de rețea; 2 teste de regresie |
| Codec 96 MiB per frame inbound înainte de cap-ul de 1 MiB (DoS) | Înaltă | ✅ FIXED `b70971d` — codec direcțional: `max_request_bytes` = cap control (1 MiB), `max_response_bytes` = cap chunk; un frame de 90 MiB ca REQUEST e respins la codec înainte de alocare; test `codec_request_cap_is_control_sized_response_cap_is_chunk_sized` |
| Matcher-ul unificat (`match_agent`) ne-wired în delegare | Înaltă | ✅ FIXED `66e5a1c` — `AgentOrchestrator` are un `execution_gate` opțional; `select_executor` cere semantic AND gate; node-cli setează gate-ul cu `match_agent` complet (semantic + model allowlist + compute physical) pentru agenți locali (adv sincron), remote = semantic-only onest (physical UNKNOWN, nu bloc); test `execution_gate_filters_physically_ineligible_local_agents` |
| PolicyEngine (P7) zero apelanți | Înaltă | ✅ FIXED `52d80f3` — `AgentRuntime` are un `policy_gate` opțional verificat înainte de fiecare Delegate (task refuzat = reply cu eroare policy, executorul nu rulează niciodată); node-cli atașează `PolicyEngine.check_model` (model allowlist + working state) pe fiecare agent local; test `policy_gate_denies_before_executor_runs` |
| Trei selectori + fallback legacy care ocolește `is_retryable()` + `SessionAccount` fără TTL | Înaltă | ⏳ OPEN |
| God-module: api.rs 16,7k / compute.rs 4,6k / main.rs 7,3k / 2 dashboard-uri | Înaltă | ⏳ OPEN |
| `ApiState` 19 `Option<Arc>` + `attach_*` fără fațadă | Medie | ⏳ OPEN |
| `/v1/token` dă master token-ul oricui; 4 endpoint-uri P14 neautentificate | Înaltă (loopback-trust) | ✅ FIXED `b7c8ecd` — cele 4 endpoint-uri P14 (`/v1/contribution`, `/v1/credits/balance`, `/v1/credits/events`, `/v1/verified-compute/history`) cer acum `require_operator_or_admin` (coerent cu `/v1/fabric/graphs`); `/v1/token` rămâne deliberat deschis DOAR pe loopback (bootstrap token al dashboard-ului) — decizie documentată în cod + test de regresie 401 pe cele 4 |
| 177 `unwrap()` + 27 `.expect()` + 3 `unreachable!` în producție; poison inconsistent | Medie | ⏳ OPEN |
| Subprocese: SIGKILL-only, fără reap, fără supervisor pentru tool-uri, stderr null | Medie | ⏳ OPEN |
| Probe de health blocante în `/status` | Medie | ⏳ OPEN |
| Reputație fragmentată (3 sisteme), cea de agenți doar în memorie | Medie | ⏳ OPEN |
| Executori de DAG duplicați (`run_plan` vs `execute_plan`) | Medie | ⏳ OPEN |
| `RequestFacts` copy-paste 5× cu `transfer_mib=0` (termenul M19 nedimensionat live) | Medie | ⏳ OPEN |
| `chunk_size_mb` validat dar neconsumat (hardcodat 4 MiB) | Joasă | ⏳ OPEN |
| `RegistryServer` re-hash per cerere; `Manifest.file_name` dublu sens | Medie | ⏳ OPEN |
| Fără semver/versiune de protocol (totul 1.0.0, compat prin `#[serde(default)]`) | Medie | ⏳ OPEN |
| ~507 pachete externe, 2× SQLite bundled, libp2p kad/relay/dcutr necondiționat, fără cargo-udeps | Joasă | ⏳ OPEN |

---

## 6. Deciziile care pot limita pe termen lung

1. **Trust-by-proximity nerostit.** Loopback-only + `/v1/token` dă tokenul
   master oricui local + endpoint-uri P14 deschise = "procesele locale sunt de
   încredere" ca model de securitate nedeclarat. Problemă la primul
   multi-tenant sau relay public.
2. **Viteza de feature peste închiderea inelelor de încredere.** 60/24
   feat/fix, tracker de 345 de pași, trei surse de adevăr despre numărul de
   teste. Ritmul produce "DONE declarat, wired parțial" — datorie de încredere
   la primul incident real.
3. **HTML/CSS/JS în raw-string-uri Rust, în două variante.** Fără tooling
   frontend, fără validare build-time, XSS stocat posibil prin câmpuri remote
   neescapate, orice schimbare de status oglindită în două fișiere de mii de
   linii.
4. **`Option<Arc<T>>` ca pattern de compoziție.** Tipul nu poate garanta care
   subsisteme există; feature matrix-ul e implicit — 2^n stări netestate.
5. **Toate crates la 1.0.0, fără semver.** Blocant la primul consumator extern
   sau la splitarea worker-ului standalone (`decentraai-worker` e planificat).
6. **Scurgere de dependențe în crates-urile "pure"** (`compute→libp2p` pentru
   `PeerId`, `agents→hub` pentru tipuri pure) — proprietatea "miez pur
   embeddable" nu se susține la nivel de graf; un consumator extern al lui
   `compute` ar trage libp2p. Fix: crate-frunză `decentraai-types`.
7. **`std::sync::Mutex::lock().unwrap()` în cod async** (~118 locuri în
   distributed) — poison-panic doar pe un task de request-path = nodul cade;
   tratarea poison e inconsistentă.
8. **Zgomot de docs istorice** (30+ fișiere, ~6.500 linii în docs/, majoritatea
   rapoarte de milestone) care maschează drift-ul dintre narațiune și cod.
   Docs-urile vii (ARCHITECTURE.md) descriau 3 planuri; realitatea are 5+.

---

## 7. Următorul nivel arhitectural

Nu e un milestone nou de feature-uri — e o **fază de consolidare pe 4 axe**:

**A. Închide lanțurile de încredere (prioritate maximă).**
(a) `verify_manifest_signature` + ancora de încredere (trusted set-ul
scheduler-ului) pe calea announcement→auto-download; (b) repară carantina
(invalidează bitmap-ul `.done` la carantină; carantină doar pe eșecuri
criptografice); (c) cap-uri per-direcție la codec + bound de fluxuri inbound;
(d) wire matcher-ul unificat complet în `select_executor` — sau demotează
pretenția din docs. Fără ele, "verify before use" e decorativ pe jumătate din
suprafețe.

**B. Sparge god-modulele prin module per-domeniu, nu prin crates noi.**
`api.rs` → `api/` cu module: `auth` (extractori axum), `proxy`, `fabric`,
`admin`, `mcp`, `views`. `dashboard_v2` devine singurul dashboard. Regulă:
niciun fișier nou peste ~2.000 de linii.

**C. Introdu o fațadă de control plane** (`Node`/`ControlPlane`) care să
înlocuiască cele 19 `Option<Arc>`: subsisteme obligatorii cu feature flags
explicite. Extrage "kernel-ul comun" (rate limiting, coadă, atomic-persist,
health probe) într-un singur loc.

**D. Unifică execuția.** Un singur selector (planner-ul fabric), legacy
`discovery::scheduler` șters; un singur executor de DAG (pure `execute_plan`/
`run_workflow`); fallback-ul legacy supus aceleiași politici `is_retryable`;
sesiuni cu TTL; `RequestFacts` construit o singură dată; ~1.500 de linii de
scaffolding mort șters.

Apoi: **protocolul ca contract versionat** (deny_unknown_fields pe tot planul
de inferență, câmp de versiune pe `InferRequest`, registry de scheme),
**reputația unificată și persistată**, **persistența conversațiilor**.

### Recomandare impact/risc pentru pasul următor

1. **Lanțurile de încredere (A)** — cel mai bun raport impact/risc: sunt
   bug-uri de securitate/disponibilitate cu scop clar, diff-uri mici, ușor de
   acoperit cu teste de regresie. Începe cu carantina/resume (bug determinist,
   test deja existent de extins) și cu semnătura de manifest + gate de trust
   pe `ShareMode::Auto`.
2. **Crate-frunza `decentraai-types`** — impact mediu, risc mic (mutare
   mecanică), deblochează pretenția de puritate; cel mai bun "quick win"
   structural.
3. **Spargerea god-modulelor (B)** — impactul cel mai mare pe termen lung,
   dar riscul cel mai mare (atinge fiecare handler); fă-o după A, în trepte
   mecanice, cu teste ca plasă de siguranță.

---

## Referințe

- Rapoartele brute ale subagenților: `docs/review/01-data-plane.md`,
  `02-service-plane.md`, `03-execution-fabric.md`, `04-intelligence-plane.md`,
  `05-cross-cutting.md`
- ROADMAP §130 (Fabric v2), §132 (Consolidare, DONE), §132.4 (open items)
- `docs/ARCHITECTURE.md`, `docs/THREAT_MODEL.md`,
  `docs/TWO_NODE_VALIDATION.md`

# Review — Service Plane / Host (crates/runtime, node-cli, providers, inference-adapter, system-probe, config, hub, discovery, p2p-invoke)

Sursă: subagent de adâncime (2026-08-21), read-only. Afirmațiile-cheie au fost
re-verificate în cod de coordonator (god-module, `/v1/token`, endpoint-uri P14
neautentificate, probe blocante). Referit din `docs/TECHNICAL_REVIEW.md`.

Corecții de scop: `serve.rs` nu există — manager-ul llama-server e
`crates/runtime/src/lib.rs` (1.096 linii); `tool_calling.rs` trăiește în
`crates/distributed/src/tool_calling.rs` (221 linii), nu în runtime; Tool
Runtime subprocess = `crates/runtime/src/tools.rs`.

## 1. Responsabilitate & suprafață publică

**`decentraai-runtime` (26.872 LOC, 8 module + 5 scripturi Python embeduite)** —
control plane-ul. Deține ciclul de viață al subprocesului llama-server
(`LlamaServer`, `ServeManager` incl. supervisor-ul M24, `TtsServer`/`TtsManager`),
gate-ul de admitere (`ensure_admitted`), proxy-ul OpenAI-compatible cu auth pe
tier-uri/rate limit/quota (`api.rs`), coada FIFO (`queue.rs`), **două**
dashboard-uri embeduite (`dashboard.rs`, `dashboard_v2.rs`), un server MCP
read-only (`mcp.rs`) și jumătatea HTTP a control plane-ului de provideri
(`providers_api.rs`). Suprafață: `build_router` expune ~70 rute (api.rs:4897)
plus `ApiState`, `ServeManager`, `LlamaServer`, `RuntimeConfig`,
`ensure_api_token`, `queue::InferenceQueue`.

**`decentraai-cli` (8.256 LOC; binarul `decentraai`)** — 22 comenzi clap:
`init`, `setup`, `doctor`, `config`, `registry`, `model`, `swarm`, `serve`,
`pull`, `token`, `worker`, `distributed`, `trust`, `tier`, `consumer-key`,
`agent`, `receipt`, `node`, `rag`, `memory`, `open`, `invite`, `join`,
`upgrade`, `bench`, `contribution`. `node` (main.rs:1315–2314, ~1.000 de
linii) e daemon-ul productizat: auto-provision → identitate → detectare model →
spawn llama-server → wiring fabric distribuit → agent host → dashboard/API.

**`decentraai-providers` (3.316 LOC, 5 module)** — "Model Fabric": domeniu
tipat (`ProviderKind`/`ConnectedModel`/`Pricing`/`SharingPolicy`/`ModelBudget`),
trait `ProviderAdapter` + `OpenAICompatibleProvider` (adapter.rs:46),
`CredentialStore` in-memory (manager.rs:8–11 — documentat intenționat),
circuit breaker/health (health.rs), `ProviderManager` cu persistență atomică
tmp+rename la `db/providers.json`. Fără I/O în domeniu.

**`decentraai-inference-adapter` (572 LOC, modul unic)** — abstracția de
engine. Trait `InferenceBackend` (`health`/`complete`/`stream`, lib.rs:178) cu
o implementare `OpenAiCompatibleBackend` peste reqwest; `BackendConfig` cu
timeouts, size caps, `EngineKind`, și un `LiveBackendUrl` resolver care
urmează portul la respawn (lib.rs:102–135). Cel mai curat crate mic din plan.

**`decentraai-system-probe` (387 LOC)** — `SystemSnapshot::collect()`
(sysinfo), `derive_budget`, `admit_inference` (floor RAM, politică GPU/
temperatură/VRAM; lib.rs:133–161), plus un probe de baterie Linux. Logica pură
separată de I/O; bine testat.

**`decentraai-config` (1.182 LOC)** — `NodeConfig` tipat YAML cu `validate()`
strict (lib.rs:466–608): 16+ reguli (bind loopback-only, cuplare tier/auth,
range-uri de porturi, engine allowlist, range-uri de sampling).
`is_known_engine` (lib.rs:608) previne typo-uri silențioase de engine.

**`decentraai-hub` (2.046 LOC, 6 module)** — catalog HuggingFace + download
verificat. Threat-model documentat (lib.rs:8–19): SHA-256 pinat înainte de
download, staging `.part`, rename atomic, mod TOFU-lite onest când digest-ul nu
e cunoscut (download.rs:9–14). Plus `capability.rs`/`requirements.rs`/`intent.rs`
(metadata hub → capability compute).

**`decentraai-discovery` (954 LOC)** — **denumit greșit**: nu are mDNS (ăla
trăiește în `decentraai-p2p`). Conținut real: pairing Ed25519 cu QR
(`pairing.rs`), `TrustStore` SQLite, `WorkerScheduler` (`scheduler.rs`). Legacy
din era M3; node-cli folosește azi doar `TrustStore` (main.rs:1609).

**`decentraai-p2p-invoke` (233 LOC)** — binar diagnostic/validare: dialează un
worker distribuit și conduce un `InferRequest` streamed real (main.rs:1–16).
Parsing de args hand-rolled (main.rs:77), identitate efemeră, Ctrl-C →
`InferCancel`. Harness de test, nu produs — etichetat clar.

## 2. Abstractions-cheie

- **`ApiState`** (api.rs:226–350) — god-struct de ~30 de câmpuri: nucleu
  (`backend_url`, `auth_token`, `Arc<Mutex<ServeManager>>`, `InferenceQueue`,
  contoare), apoi ~20 de `Option<Arc<…>>` (compute, p2p, distributed, agents,
  orchestrator, skills, embedding, retrieval, memory, talent_tree, providers,
  tts, ocr, stt, skills_tool, knowledge, evidence, benchmark). Constructor cu 9
  args și `#[allow(clippy::too_many_arguments)]` (api.rs:353); restul prin
  `attach_*` (node-cli main.rs:2150–2260). "God object asamblat prin setter
  injection".
- **`ServeManager`** (lib.rs:370–528) — ciclul de viață al engine-ului:
  `note_activity` resetează idle clock, `unload_if_idle` oprește copilul,
  `ensure_healthy` (M24) probează și respawn-ează din restart spec, contor
  `respawns`. Design solid, testat.
- **`LlamaServer`/`TtsServer`/`ToolServer`** — pattern repetat: spawn copil pe
  port efemer loopback → `wait_until_ready` health probe → kill-on-drop
  backstop (lib.rs:360, tools.rs:110). `ToolServer` e genericizat o dată
  (tools.rs:27) dar re-înfășurat de trei ori (OCR/STT/skills).
- **`Auth` + `GateError`** (api.rs:158, 140) — granița de auth.
  `classify()` rezolvă token-ul la `Open`/`Master`/`Subscriber{tier,role}`/
  `Consumer{quota…}`; `GateError` e eroare by-value convertită la Response la
  marginea handler-ului. `require_master`/`require_operator_or_admin`
  (api.rs:624, 641).
- **`InferenceQueue`** (queue.rs:45) — FIFO cu `QueueTicket` RAII (Drop
  eliberează slotul la succes/eroare/disconnect), wake-uri `Notify`, cap pe
  waiting room și timeout. Curat, testat.
- **Trait de provider** (providers/adapter.rs:46) — `ProviderAdapter: Send +
  Sync` cu `test_connection`/`discover_models`/`complete`/`stream`;
  `OpenAICompatibleProvider` deleagă la `inference_adapter::OpenAiCompatibleBackend`.
- **Wiring-ul daemon-ului** (node-cli main.rs:1315–2314) — compoziție
  secvențială de ~1.000 de linii: watcher auto-upgrade → setup/identitate →
  detectare model → `LlamaServer::start` + `OpenAiCompatibleBackend` cu
  `live_engine_url` (Arc<Mutex>) ca sursă unică de adevăr pentru URL → compute
  manager cu signing key → agent manager → `DistributedP2PHandler` → `P2PNode`
  → `DistributedInference` → Tool Runtime spawn → `AgentRuntime` per agent →
  `ServeManager` + supervisor M24 → task-uri broadcaster/probe/reaper →
  `ApiState` + ~20 attach → `serve_api`. Fiecare subsistem e `Option<…>`/
  best-effort; nodul degradează grațios.

## 3. Integrare & cuplare

Fan-out (Cargo.toml):

```
node-cli  (17 crates): agents audit compute config discovery distributed hub identity
                        inference-adapter manifest p2p protocol providers registry runtime
                        system-probe tokens
runtime   (13): agents audit compute config distributed fabric hub inference-adapter p2p
                providers registry system-probe tokens
distributed (13): agents audit compute config discovery fabric hub identity inference-adapter
                  p2p protocol registry system-probe
p2p (5) · providers (2) · agents (2) · discovery (2) · protocol (2) · system-probe (1) · fabric (1)
leaves (0): config manifest identity registry audit tokens compute inference-adapter hub
```

**Nu e hub-of-spokes; e hub-de-hub-uri.** Două god-hub-uri — `runtime` și
`distributed` — și `runtime` depinde de `distributed`, deci fan-in-ul
tranzitiv al control plane-ului e tot workspace-ul. Nimic nu stă între control
plane și crates-urile de fabric.

Abstracția lipsă e observabilă: `runtime` și `distributed` **re-implementează
aceleași subsisteme**: rate limiting (`ApiState::check_rate_limit` +
`check_execute_rate_limit` + `check_consumer_rate_limit` — trei copii ale
aceleiași ferestre, api.rs:703/734/820 — plus `distributed::rate_limit`), cozi
(runtime `queue.rs` vs `distributed::queue`), sesiuni/retry/fallback
(`distributed::fallback`, `replay`, `session`) — nimic în spatele unui trait pe
care runtime să-l consume generic. Crate-uri care ar putea/ar trebui să fie în
spatele unor traits: admiterea (`system_probe`), planner-ul fabric
(`fabric`/`distributed::router`), tool runtime-ul (tools.rs), manager-ii de
subproces. Nu există fațadă `ControlPlane`/`Fabric`/`ToolHost` — fiecare
subsistem nou = +1 câmp + attach + view + handler în cele două god-fișiere.
`RuntimeConfig`/`BackendConfig`/`EngineKind` sunt concepte duplicat
(runtime lib.rs:59 vs inference-adapter lib.rs:23–66).

## 4. Semnale de maturitate

- **Docs**: punctul forte al planului. Docs de modul cu threat-model explicit
  (hub lib.rs:8–19, providers/manager, api.rs:1–15, tools.rs, mcp.rs, queue.rs,
  inference-adapter), comentarii "de ce" omniprezente (rationale-ul ETXTBSY
  lib.rs:278–284, nota false-ready main.rs:1949–1956).
- **Erori**: anyhow + `.context()` la granițe, `bail!` cu mesaje acționabile,
  erori tipate mici (`GateError`, `BackendError`, `QueueFull`/`WaitTimeout`),
  pattern-ul RAII `ConsumerQuotaGuard` (api.rs:84–123) e genuin atent.
- **unwrap/expect în producție — numere verificate**: runtime are 886 de
  potriviri totale, dar ~80% în module de test; în producție ≈ 32, majoritatea
  `std::sync::Mutex::lock().unwrap()` (panic-on-poison). Instanțe concrete:
  lib.rs:320 (`child.expect("engine spawn retries exhausted")` — după buclă de
  retry), lib.rs:501 (`self.server.take().unwrap()` — guardat), lib.rs:739,
  tools.rs:215/282/368 (redundante), queue.rs:108 (`expect("front exists")`),
  node-cli main.rs:1959/2110/2013/6021/6022 (invariant-guarded) + main.rs:3313,
  3344, 5775 (flags CLI). **Fără unwrap necondiționat pe cale fierbinte, dar
  convenția declarată e încălcată în ~10 site-uri.**
- **Suprafață HTTP**: corpuri JSON consistente `{"error":{"message","type"}}`;
  helper-e 401/403/429 (api.rs:9640–9685); cap-uri la granița proxy
  (`MAX_PROMPT_BYTES=200_000`/`MAX_OUTPUT_TOKENS=8192`, api.rs:62–66);
  injectare de eveniment de eroare SSE mid-stream (`sse_safe_stream`,
  api.rs:8529). **Acoperire auth inconsistentă**: `/status`, `/metrics`
  (documentate auth-neutral), dashboard-ul și `/v1/token` sunt deschise —
  dar `/v1/token` **întoarce master token-ul oricărui apelant**
  (api.rs:5941–5946), iar JS-ul dashboard-ului îl fetch-uiește
  (dashboard.rs:1446). Sub modelul loopback-only asta e coerent, dar e o
  decizie nerostită. Endpoint-urile P14 `contribution_state`, `credits_balance`,
  `credits_events`, `verified_compute_history` (api.rs:6592/6610/6634/6654)
  **nu iau headere deloc** — vedem conturi și balanțe neautentificat.
  Rate limiting per-token/consumer există, dar fără limiter global și fără
  auth pe `/metrics`.
- **Validare config**: 16+ reguli, dar un singur `validate()` monolit cu
  mesaje string (fără coduri de eroare) și gap-uri: `queue_max_requests`,
  `idle_model_unload_minutes`, `request_timeout_seconds`, `max_context_tokens`
  nu sunt validate. `KNOWN_SKILLS` trăiește în config (config/lib.rs:520)
  *și* e re-encodat ca match în node-cli (main.rs:1833–1840) — două surse de
  adevăr.
- **Teste**: grele — ~7.400 de linii de teste doar în api.rs; queue (4),
  lib.rs (15), tools, system-probe (10), config, inference-adapter (httpmock).

## 5. Mirosuri & riscuri concrete

1. **God-module.** `api.rs` = 16.720 linii, 80 de handlere, fără structură de
   submodule (doar `providers_api.rs`, `mcp.rs` sunt scoase); `dashboard.rs`
   4.993; `node-cli/src/main.rs` 7.339 cu un `node_start` de 1.000. Fiecare
   milestone nou = mai multe handlere + mai multe câmpuri `ApiState` în același
   fișier — fișierul e arhitectura.
2. **Două dashboard-uri paralele.** `dashboard_v2.rs` (1.137) e o a doua
   implementare, "visual refresh", cu propriile template-uri; `/ui2` e
   permanent disponibil (api.rs:5056); alegerea rădăcinii e flag de config.
   Orice schimbare de câmp de status trebuie oglindită în două fișiere JS
   raw-string mari.
3. **HTML-in-Rust embedding.** `DASHBOARD_HTML` = raw-string de 1.389 de linii
   (dashboard.rs:41), `JS_TEMPLATE` ~3.500 (1430+), cu replace de token-uri la
   serve-time (`__SHARE__`/`__MODEL__` api.rs:9720–9744, `__API_PORT__`/
   `/*__JS__*/` api.rs:5062). Fără validare build-time, fără syntax
   highlighting, fără minificare; eroare de JS prinsă doar în browser. JS
   interpolează cu `innerHTML` de 226 de ori contra unui helper `esc()` folosit
   de 279 — disciplina e manuală; orice câmp remote neescapat = XSS stocat pe
   browser-ul operatorului.
4. **Logică duplicată.** (a) trei limiter-e sliding-window în api.rs + unul în
   distributed; (b) patru `*Manager`/`healthy()` aproape identice (TtsManager
   lib.rs:702, OcrManager/SttManager/HfSkillsManager tools.rs:190/253/335) cu
   același `self.server.as_ref().unwrap().server.port()` redundant; (c) cap-urile
   `200_000`/`8192` ca literali în runtime (api.rs:62/66), 4+ literali
   `BackendConfig` în node-cli (main.rs:1488, 1524, 1567, 3010) și default-uri
   în inference-adapter (lib.rs:129–130) — fără constantă partajată, drift
   garantat; (d) `python3.13` hardcodat (lib.rs:631, tools.rs:61); (e)
   `probe_health` TCP hand-rolled (lib.rs:560) vs probe reqwest (tools.rs:134).
5. **I/O blocant în handlere async.** `/status` apelează `state.tts.healthy()`,
   `ocr.healthy()`, `stt.healthy()`, `skills_tool.healthy()` (api.rs:5202–5217);
   fiecare rulează un `probe_health` blocant cu timeout de 2s (lib.rs:560–578,
   PROBE_TIMEOUT lib.rs:51) — un nod ocupat poate stagna un thread de executor
   până la ~8s per poll `/status`. Dashboard-ul poll-uiește `/status` periodic.
6. **Riscuri de management de subproces.** (a) kill-on-drop e `start_kill()`
   fără wait/reap (lib.rs:360–365, tools.rs:110–114) — fereastră de zombie;
   (b) **fără SIGTERM grațios nicăieri** — SIGKILL pe unix, llama-server nu
   flushează stare la unload; (c) `allocate_port` (lib.rs:185) e TOCTOU
   bind-0/release/re-bind clasic; (d) **tool-urile n-au supervisor**: doar
   llama-server are `ensure_healthy` (M24); un OCR/STT/skills/TTS căzut rămâne
   mort până la restart; (e) toți copiii au `Stdout/Stderr::null` (lib.rs:306,
   tools.rs:77) — un server Python care moare după spawn e aproape
   nediagnosticabil.
7. **Auth-by-handler.** 22 de site-uri `require_master` + 32
   `require_operator_or_admin` + 9 `classify` explicite — pattern repetat per
   handler în loc de extractori/middleware axum; unele handlere (`/v1/token`,
   `/metrics`, P14) îl sar cu totul, fără inventar de endpoint-uri protejate.
   `classify` **reîncarcă TokenStore/ConsumerKeyStore de pe disc la fiecare
   cerere** (api.rs:572, 602) — corect-by-construction la revocare, dar un read
   de fișier per request pe calea fierbinte.
8. **Valori magice / hardcodate.** `EXECUTE_RATE_LIMIT_PER_MINUTE=10`
   (api.rs:56), `RECENT_REQUEST_LIMIT=12`, `DASHBOARD_EVENT_LIMIT=10`, cap
   OCR 50 MiB (api.rs:6168), `1024` fake model size (main.rs:1580),
   `model_size_bytes/4 + 1024` RAM estimate (main.rs:2045), `python3.13`,
   timeout de delegare 600s (main.rs:1937) — toate inline.
9. **Gap-uri de observabilitate.** Fără middleware tower-http/tracing, fără
   span-uri per request, fără log de request-uri (doar `tracing::info/warn` pe
   căi alese); `/metrics` e text Prometheus asamblat manual (api.rs:5264–5411)
   cu escaping de label-uri prin concatenare de string-uri — corect azi,
   nescalabil; `tracing::error!` lipsește în mai multe ramuri de eșec
   (e.g. eșecul spawn-ului `serve_api` loghează doar warn, api.rs:5080–5082).
10. **Drift-ul crate-ului `discovery`.** Cod de epoca pairing/scheduling (QR
    pairing, `WorkerScheduler`) care nu mai corespunde nici numelui, nici
    descoperirii reale (mDNS libp2p în `p2p`); node-cli folosește doar
    `TrustStore`. Suprafață moartă cu model propriu de trust care se suprapune
    cu `reputation`/`trust.db`.

## 6. Verdict (1–5)

| Crate | Scor | Justificare |
|---|---|---|
| runtime | **2** | Bogat funcțional și documentat, dar god-fișier de 16,7k, subsisteme duplicat, probe blocante în handlere async, acoperire auth inconsistentă; cel mai mare debit de refactor din workspace |
| node-cli | **2** | Monolit de 7,3k cu funcție de wiring de 1.000 de linii și `expect`-uri de invariant; ok ca suprafață UX, dar nemaintenabil la ritmul actual |
| providers | **3** | Domeniu curat fără I/O + trait de adapter + handling onest de credențiale; mic, focusat |
| inference-adapter | **4** | Abstracția de model: trait mic + implementare testată, timeouts/size caps/live-URL resolution; aproape exemplar |
| system-probe | **4** | Logică pură curat separată de I/O, bine testată; mic și gata |
| config | **3** | Tipat + validat cu reguli reale, dar `validate()` monolit cu erori string și câteva range-uri nevalidate |
| hub | **3** | Threat-model documentat și TOFU-lite onest; plumbing HTTP/download hand-rolled, mapping-ul de capability doar moderat testat |
| discovery | **2** | Crate legacy denumit greșit (pairing/scheduler); doar `TrustStore` e folosit azi; suprapunere cu modelele de trust p2p/reputation |
| p2p-invoke | **3** | Ok ca harness de diagnostic etichetat; args hand-rolled, fără clap, dar nu e cod de produs |

## Top 5 riscuri pe termen lung (planul service)

1. **God-fișierele sunt arhitectura.** `api.rs` (16,7k), `dashboard.rs`+
   `dashboard_v2.rs` (6,1k), `node-cli/src/main.rs` (7,3k) concentrează
   creșterea fiecărui milestone în trei fișiere; dashboard-ul v2 paralel dublează
   suprafața. Orice schimbare de auth/stare/observabilitate le atinge pe toate.
2. **Două hub-uri suprapuse fără abstracție partajată.** `runtime` și
   `distributed` implementează ambele rate limiting, cozi, sesiuni, retry,
   stare worker; `runtime → distributed` dă cuplaj tranzitiv aproape pe tot
   workspace-ul. Drift-ul între limiter-e/cozi e deja vizibil; control plane-ul
   are nevoie de o fațadă îngustă (trait `FabricClient`) în loc de 20 de
   `Option<Arc<…>>`.
3. **Suprafață HTTP inconsistent și parțial neautentificat.** `/v1/token`
   livrează master token-ul oricărui apelant local, patru endpoint-uri P14 sunt
   complet deschise, iar protecția e aplicată manual per handler fără inventar.
4. **Ciclul de viață al subproceselor e fragil.** SIGKILL-only, fără reap la
   Drop, cursa de port bind-0, stderr null, și — critic — fără supervizare
   pentru procesele de tool (doar llama-server are `ensure_healthy`). Un tool
   căzut degradează silențios setul de capability-uri anunțat până la restart.
5. **I/O blocant și observabilitate hand-rolled pe calea de request.** Patru
   probe de health blocante de 2s per apel `/status`, reîncărcare de pe disc a
   token store-ului per cerere, Prometheus construit prin concatenare de
   string-uri și fără tracing de request — latență și debuggabilitate afectate
   la primul load concurent.

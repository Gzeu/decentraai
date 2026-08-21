# Review — Inginerie Cross-Cutting (workspace, API hygiene, erori, teste, docs, config, ops)

Sursă: subagent de adâncime (2026-08-21), read-only. Toate numerele din grep/read
pe arborele curent (fără build). Referit din `docs/TECHNICAL_REVIEW.md`.

## 1. Workspace & sănătatea grafului de dependențe

**20 de membri de workspace** (`Cargo.toml:2-23`), ~90.5K LOC în `src/`. Graful
de dependențe e un DAG strict — **fără cicluri** (verificat: niciun edge nu
indică "în sus"; nimic nu depinde de `node-cli`; `runtime→distributed` există
dar `distributed→runtime` nu; `p2p-invoke` nu e depins de nimeni).

```
leaves (9):  audit  compute  config  hub  identity  inference-adapter  manifest  registry  tokens
layer 2:     protocol→{identity,manifest}   system-probe→config   agents→{compute,hub}
             fabric→compute   providers→{inference-adapter,hub}   discovery→{identity,protocol}
layer 3:     p2p→{audit,identity,manifest,protocol,registry}   p2p-invoke→{identity,p2p,protocol}
layer 4:     distributed→13 dependențe interne   runtime→13 dependențe interne
top:         node-cli→17 dependențe interne      [frunză a grafului]
```

- **God integratori**: `node-cli` (17), `distributed` (13), `runtime` (13).
  Fiecare feature nou fan-out prin `distributed`+`runtime`; `node-cli` e un
  monolit de 8.256 LOC care wire-uiește tot.
- **Încălcări de strat (reale)**: `compute` — documentat "pure, no I/O, no
  async" (compute/src/lib.rs:1-6) — depinde de **libp2p** doar pentru `PeerId`
  (matcher.rs:6, scheduler.rs:12, reservation.rs:12); workspace
  feature-unification trage tot stack-ul libp2p (tokio/kad/relay/dcutr) în
  crate-ul "pur". `agents` — "pure, no I/O" — depinde de `hub` (crate I/O cu
  reqwest+tokio) doar pentru tipurile pure `hub::capability`/`hub::requirements`
  (agents/src/matcher.rs:24-25, talent_tree.rs:38). Acele tipuri ar trebui să
  stea într-un crate-frunză de domeniu.
- **Semver: niciuna.** Toate cele 20 de crates pinat `version = "1.0.0"` via
  `[workspace.package]` (Cargo.toml:26-27). Compatibilitatea wire e gestionată
  ad hoc cu `#[serde(default)]` (e.g. `node_id`, `accepts_remote_inference`),
  fără negocieri de versiune sau cale de deprecare.
- **Outlier inconsistent**: `inference-adapter` folosește edition **2021**,
  licență **MIT** și `thiserror = "1"` / `tokio = "1"` non-workspace
  (inference-adapter/Cargo.toml:3-16) — **FIXED în c08c39d** (edition 2024,
  license.workspace, thiserror.workspace, tokio.workspace).
- **Igienă de dependențe**: 527 de intrări în Cargo.lock (~507 pachete externe)
  pentru un proiect LAN-first. Feature-urile libp2p compilează necondiționat
  `kad, relay, dcutr, identify, ping` (Cargo.toml:45) deși DHT/relay sunt off
  implicit. `rusqlite` cu **bundled** (compile C) în `discovery`
  (discovery/Cargo.toml:9) pentru pairing trust store (pairing.rs:271-290) — și
  din nou în `distributed` pentru MemoryStore; două crates compilează SQLite.
  **Dependențe declarate nefolosite**: `fabric→anyhow` (fabric/Cargo.toml:12 —
  **FIXED c08c39d**), `distributed→decentraai-config` (își definește propriul
  `config.rs`), `providers→{tokio,chrono,futures-util}` (tokio scos în c08c39d).
  Fără check CI (fără cargo-udeps).

## 2. Igienă API public

Per-crate `pub fn` / `pub item` / `pub use` (src): distributed 361/78/12,
agents 297/142/24, compute 134/72/16, runtime 120/29/0, providers 76/36/4,
fabric 75/53/9, tokens 41/20/2, p2p 33/15/1, protocol 30/18/1, discovery 25/6/3,
hub 23/19/5, manifest 23/10/0, registry 13/3/0, identity 10/2/0,
inference-adapter 7/11/0, node-cli 6/3/0, system-probe 5/5/1, config 5/25/1,
audit 2/1/0, p2p-invoke 0/0/0.

- **Niciun crate nu are `#![warn(missing_docs)]`/`#![deny(...)]`** — acoperirea
  de docs e voluntară. Calitatea e bimodală: crates-urile de domeniu noi sunt
  bine documentate (agents 1.446 linii `///`; distributed 1.599; fabric 691;
  compute 633 — de ex. raționamentul de idempotency pe `is_retryable`,
  distributed/src/lib.rs:231-251). Frunzele vechi sunt goale: identity 8 linii
  de docs pentru 12 item-uri pub, manifest 26 pentru 33, registry 44 pentru 16,
  system-probe 15 pentru 10, audit 4 pentru 3.
- **Scurgeri interne**: `runtime/src/api.rs` expune o suprafață de 120 de
  funcții pub dintr-un fișier cu un handler pentru toate (11 site-uri de gate
  `require_master`, api.rs:1094/1137/1200/1246/1335/1375/1434/2131/2741…);
  `node-cli/src/main.rs` e un binar de 8K linii a cărui "API" e un singur
  `main`. `p2p-invoke` are 0 item-uri pub și 0 teste — binar manual de
  validare, nereferit de niciun alt crate.
- Pozitiv: `config` re-exportă exact un helper (`pub use
  helpers::ensure_mode_0600`, config/src/lib.rs:1122); `tokens` are o suprafață
  strânsă, purpose-built.

## 3. Cultura de erori

Trei stiluri, în general coerente:
- **anyhow + `.context()` la granițele de I/O**: node-cli (83 context / 54
  bail), p2p (17/27), hub (16/10), registry (15/5), tokens (6/7), identity
  (10/1). Conform convenției (AGENTS.md §4).
- **thiserror tipat** în crates-urile frunză/domeniu: agents (38 refs),
  providers (28), config (8), inference-adapter (7), manifest (4), plus
  taxonomia distributed de mai jos.
- **Enum-uri de outcome în loc de `Result`** în crates-urile pure de decizie:
  `compute` returnează `MatchOutcome`/`MatchReason` (matcher.rs:11-33), `fabric`
  returnează `PlanResult` (planner.rs:326) — design bun pentru cod de decizie.

**Taxonomia distributed**: `DistributedError` (distributed/src/lib.rs:207-275)
e o taxonomie reală cu `is_retryable()` (doar `P2PError` e retryable — semantică
at-most-once pentru generare non-idempotentă, documentată) și mapping stabil
`code() → InferErrorCode`. Minor: doc-comentariul de pe `is_retryable` (232-248)
conține un paragraf pe jumătate editat, stricat.

**Convenția "Never `unwrap()` outside tests" e încălcată la scară**: 177 de
`unwrap()` non-test + 27 `.expect(` + 3 `unreachable!` în `src/` în afara
`#[cfg(test)]` (0 `panic!`). Pe crates: distributed 118 (aproape toate
`Mutex::lock().unwrap()` — risc de panic-on-poison pe fiecare lock de
request-path: compute.rs:512-619, agent_memory.rs:151-385, agents.rs:67-180,
agent_messenger.rs:65-112), runtime 32 (23 în api.rs, mai ales lock-uri +
`consumer_keys_path.as_ref().unwrap()` la api.rs:571), agents 10
(delegation.rs:233-311, workflow.rs:182-198 — unwrap-uri de invariant
HashMap, defensibile), discovery 10 (pairing.rs:326-449
`self.conn.as_ref().unwrap()`), node-cli 6. **Tratarea poison e
inconsistentă**: distributed/src/lib.rs:978 folosește
`unwrap_or_else(|e| e.into_inner())` în timp ce fiecare site vecin folosește
`.unwrap()`.

**Înghițirea erorilor, concret**: `let _ = p2p_clone.request(...)` la livrarea
răspunsurilor `InferFailed` (timeout/respingere de capacitate,
distributed/src/lib.rs:951, 983) — un send eșuat lasă requester-ul în așteptare
până la propriul timeout; `let _ = ledger.settle(id, tokens_used)`
(runtime/src/api.rs:107) și `let _ = ledger.release(id)` (api.rs:119) — eșecuri
de contabilitate a compensației aruncate; `let _ = decentraai_audit::record(...)`
la 5+ site-uri (api.rs:1176/1225/1298/1357/4114) — documentat best-effort, dar
cu zero observabilitate când scrierea de audit eșuează; `let _ = store.write(...)`
(agent_orchestrator.rs:580, 600) — eșecuri de persistență a memoriei colective
mute; `let _ = cancel_queue.cancel_request(...)` (lib.rs:596). Și
`ModelRegistry::load(&path).ok()` la api.rs:1610/1737/1913 — un registry corupt
degradează silențios 3 endpoint-uri la "no models".

## 4. Cultura de teste

**Număr real: 1.192 de teste** (`#[test]`+`#[tokio::test]`, src + tests). Per
crate: runtime 237 (134+103 tokio), agents 221, distributed 186 (incl.
tests/agent_e2e.rs, compute_e2e.rs, tool_calling_e2e.rs, lifecycle.rs), fabric
122, compute 110, hub 53, node-cli 41, providers 34, protocol 32, p2p 30 (incl.
tests/e2e_transfer.rs — libp2p real două-noduri pe loopback), config 27,
registry 24, tokens 26, inference-adapter 13, discovery 12, identity 9,
system-probe 9, manifest 5, audit 1, **p2p-invoke 0**. `#[ignore]`: **0** în
tot repo-ul — nimic nu e sărit.

**Claim-uri vs cod — toate trei sunt greșite**: README.md:445 "1018 tests"
(stale cu 174); ROADMAP.md:3293 "1184 tests" (stale cu 8); AGENTS.md:122
"106+ tests" (stale cu ~11×). Numărul real: **1192** (corectat în c08c39d la
1190). Drift-ul a rămas neobservat pentru că CI nu are aserțiune pe numărul de
teste.

**Calitatea e genuin bună, nu cargo-cult**: testele fabric explică *de ce*
există fiecare termen de scoring (`continuation_is_steered_to_prefix_host_by_locality_score`,
planner.rs:846-860); testele de queue din runtime verifică proprietăți
comportamentale (FIFO, respingere waiting-room plin, drop eliberează slot,
timeout — queue.rs:187-247); config are un test per regulă de validare
(config/src/lib.rs:623-1118); runtime-ul spawn-ează **subprocese fake de
engine** ca să testeze ciclul real spawn/health/kill/idle-unload/crash-restart
(lib.rs:1009-1075, incl. nota de retry ETXTBSY la 278-283).

**Gap-uri reale**: binarul real `llama-server` nu rulează niciodată în CI
(doar validare LAN live); calea de download a lui `hub` (stream→`.part`→
digest→rename, cel mai sensibil I/O din sistem) are doar teste triviale
(hub/src/download.rs:199-208 — `hex_formats_lowercase`) — toate cele 53 de
teste hub sunt logică pură de tipuri în `capability.rs`.

## 5. Drift docs & artefacte de proces

- **Număr de crates**: AGENTS.md §2 spune "9 workspace crates" (AGENTS.md:23)
  și listează 11; sunt **20**. Nouă crates lipsesc complet din secțiunea de
  layout (agents, distributed, fabric, hub, discovery, providers, tokens,
  inference-adapter, p2p-invoke) — deși proza ulterioară descrie agents/
  distributed/fabric pe larg.
- **README "Core Modules"** (README.md:293-310): 12 din 20 de crates, LOC
  foarte stale — runtime "8,000+" vs real 26.872; distributed "3,000+" vs
  15.947; agents "4,000+" vs 12.449; p2p "3,500+" vs 2.141; fabric "2,500+" vs
  6.146. Lipsește compute (5.963), hub, providers, protocol, discovery, tokens,
  inference-adapter, p2p-invoke.
- **docs/ARCHITECTURE.md:55-66 descrie crates care nu există** —
  `policy-engine`, `chunk-store`, `transfer-engine`, `inference-runtime`,
  `inference-router` — document de design timpuriu niciodată actualizat
  (**FIXED c08c39d** — layout real + notă).
- **docs/IMPLEMENTATION_MATRIX.md** marchează item-uri P0 ca `DESIGN`/`PARTIAL`
  care sunt livrate (adapter real de inferență, E2E LAN două-noduri, wiring de
  handlere).
- **CHANGELOG.md are CINCI secțiuni `[Unreleased]`** (liniile 7, 28, 65, 98,
  138) și una `[1.0.0]` — fiecare push adaugă un bloc Unreleased nou în loc să
  le îmbine (**FIXED c08c39d** — o singură secțiune cu subsecțiuni).
- **ROADMAP.md e un hibrid de 3.338 de linii** (50 de secțiuni de top):
  raport istoric de milestone + tracker viu + totaluri de teste. Markerii de
  status se contrazic: §7/§8 au header "in progress" cu toate checkbox-urile
  `[x]` (ROADMAP.md:42-60, 139-155) — și AGENTS.md susține "ROADMAP.md is fully
  done (M0–M8)". 38 de `(DONE)` vs 6 markeri in-progress; secțiunile 23-50 sunt
  incremente "Next-Gen Phase" cu etichete WIRED/PARTIAL. Singura parte
  validată în CI e numerotarea step-ID a `docs/ROADMAP_345_EXECUTION_TRACKER.md`
  (1..345, scripts/validate_roadmap.py).
- **Zgomot istoric**: ~15 din 45 de fișiere din `docs/` sunt înregistrări
  milestone-DONE (COMPLETE_BUILD_REPORT_M10_M17, M10_AGENT_CHECKLIST,
  M10_FILE_MAP, M10_IMPLEMENTATION_BLUEPRINT, M10_PRODUCTION_VISION, M11_*,
  Q3B-Q4D_*, TWO_NODE_VALIDATION, WP-001.4…).
- **Ce rulează CI** (.github/workflows/): `ci.yml` — fmt, `clippy --workspace
  --all-targets -D warnings`, `cargo test --workspace`, build, plus cargo-audit
  și gitleaks; `m10-validation.yml` — la fel + clippy `--all-features` +
  validare OpenAPI YAML (docs/api/m10-openapi.yaml) + grep de pattern-uri de
  secrete; `inference-adapter.yml` — check izolat al adapter-ului;
  `roadmap-validation.yml` — step-ID-urile tracker-ului;
  `build-dashboard.yml` — un grep de branding (`grep -q 'DecentraAI — Command
  Deck' crates/runtime/src/dashboard.rs`). Notabil: fără aserțiune pe numărul
  de teste, fără coverage, fără check de dependențe nefolosite, fără check de
  drift în docs.

## 6. Config & operațiuni

- **Knobs**: ~64 de chei în `configs/node.example.yaml` peste 13 secțiuni —
  suprafață de operator foarte mare pentru un singur nod.
- **Validarea e genuin strictă**: `deny_unknown_fields` pe fiecare secțiune
  (config/src/lib.rs:17,56,142,165…) plus `validate()` cu ~20 de reguli incl.
  securitate: bind non-loopback cere auth (config/src/lib.rs:509-516),
  tier-urile cer `api_auth_required: true` (:521-526), inferența remote cere
  `private_swarm` (:546-550), porturi privilegiate respinse (:551-555),
  engine whitelist (:556-562), range-uri temperature/top_p (:571-585). 27 de
  teste acoperă regulile; config-ul exemplu e el însuși fixture de test
  (config/src/lib.rs:631).
- **Dar `configs/node.schema.json` e un stub**: 7 chei de top, fiecare doar
  `{"type":"object"}`, zero proprietăți imbricate, zero reguli
  (configs/node.schema.json:1-17) — schema JSON nu spune nimic despre niciun
  knob, deci orice consumator extern (IDE, tooling, join flow) nu primește
  validare. Validatorul Rust și schema au drift-uit până la divergență completă
  (**FIXED c08c39d** — schema completă de 456 linii; node.example.yaml
  validează cu jsonschema).
- **Două sisteme de config deconectate**: YAML validat `NodeConfig`
  (decentraai-config) vs `distributed::config::InferenceConfig`
  (distributed/src/config.rs, ~15 knobs: announcement_interval_ms,
  max_retries, retry_backoff_ms, request_timeout_ms…). Fiecare site de
  construcție din căile live folosea **`InferenceConfig::default()`**
  (node-cli/src/main.rs:1626, 1752, 5893, 6006; de-centraai-worker.rs:182,196)
  — deci "retry up to `config.max_retries`" din AGENTS.md era hardcodat la 3 și
  **niciun knob de tuning distribuit nu era setabil de operator**; cele două
  config-uri nu se întâlneau niciodată. (Un pic de YAML curge: config.network
  .max_message_bytes → P2PNode::new, main.rs:6004.) **FIXED c08c39d** —
  `InferenceSection` + `from_section()` în toate cele 4 site-uri.
- **Fluxul de secrete/token-uri e solid**: master token API `runtime/api.token`
  generat cu `OsRng`, scris apoi chmod 0600 (runtime/src/api.rs:9810-9832);
  token-uri de abonament `dsk_<64hex>` stocate doar ca hash-uri BLAKE3
  (tokens/src/lib.rs:1-16); credențial de invitație 0600 (main.rs:3801-3809);
  `.gitignore` acoperă `*.key/*.pem/identity/*.sqlite/.decentraai/`; gitleaks +
  grep de pattern-uri de secrete în CI. Scrierile de audit sunt best-effort by
  design.

## 7. Top 10 riscuri cross-cutting pe termen lung (clasate)

1. **Arhitectură neaplicată → gravitația integratorilor.** 17/13/13 dependențe
   interne pe node-cli/distributed/runtime; stratificarea documentată nu e
   aplicată de nimic. Cele trei god-crates vor continua să absoarbă feature-uri
   până când un split (e.g. api.rs de 26,9K LOC în runtime) devine
   nemaintenabil.
2. **Pattern-ul `Mutex::lock().unwrap()` panic-on-poison (118× non-test în
   distributed).** Un handler care panichează pe orice cale de request (queue
   sweep, memory store, agents registry) doboară task-ul tokio; poison e tratat
   corect într-un singur loc (lib.rs:978). Un singur lock otrăvit = DoS pe nod.
3. **Două sisteme de config care nu se întâlnesc niciodată.** YAML operator
   (64 de knobs, strict validat) vs parametrii de tuning distribuit (mereu
   `default()`); max_retries/timeouts/announcement cadence erau config
   ireachabil. **FIXED c08c39d** (from_section în 4 site-uri) — dar rămâne de
   verificat că restul knobs-urilor sunt realmente consumați.
4. **Scurgeri de dependențe în crates-urile pure** (compute→libp2p,
   agents→hub): "pure/no-I/O" e adevărat pentru codul crate-ului, nu pentru
   graful său; fiecare trage un stack async complet în cod de domeniu și
   blochează proprietatea dorită (miez pur testabil, embeddable). Fix: mută
   tipurile keyed-by-PeerId / hub::capability + requirements într-un
   crate-frunză.
5. **Eșec silențios pe căi de integritate și bani.** `let _ =
   ledger.settle/release` (compensație), `let _ = p2p_clone.request` pentru
   reply-uri de eșec, `ModelRegistry::load().ok()` degradând 3 endpoint-uri,
   eșecuri de audit neobservabile. Fiecare e "best-effort" documentat;
   colectiv fac integritatea sistemului greu de observat.
6. **Datoria de documentație ca hazard de corectitudine.** AGENTS.md (9 crates /
   106+ teste), README (1018; tabel stale de 12 crates), ROADMAP (1184;
   secțiuni "in progress" complet bifate), ARCHITECTURE.md (crates fantomă),
   IMPLEMENTATION_MATRIX (statusuri DESIGN pentru P0 livrate), CHANGELOG (5
   Unreleased). Niciun CI nu prinde nimic din asta; cele trei claim-uri de
   număr de teste sunt toate greșite (real: 1192). Parțial FIXED c08c39d
   (CHANGELOG, ARCHITECTURE.md, numere).
7. **Fără semver / protocol wire versionat.** Toate crates-urile înghețate la
   1.0.0; compat prin acumulare de câmpuri `#[serde(default)]` (node_id,
   accepts_remote_inference…) — calea de creștere e câmpuri opționale
   nelimitate și default-uri silențioase (default-ul conservator e corect, dar
   nu există negociere de versiune sau cale de deprecare).
8. **Amprentă de dependențe & igienă.** ~507 pachete transitive pentru un tool
   LAN; feature-uri libp2p kad/relay/dcutr/identify necondiționate; două crates
   cu SQLite bundled; deps nefolosite în fabric/distributed/providers (parțial
   FIXED c08c39d); fără check udeps-style în CI. Riscul de supply-chain și
   timp de build crește cu fiecare milestone.
9. **Integrarea engine-adevărat e testată doar live.** CI nu rulează niciodată
   un llama-server real (httpmock în loc); ciclul de viață al subprocesului de
   engine e bine testat cu fakes, dar contractul real (SSE, telemetrie KV,
   `--embedding`, comportament la crash al binarului real) e validat doar prin
   rulări manuale LAN.
10. **Zero `#[ignore]` e cu două tăișuri.** Disciplină bună, dar cu 237 de
    teste runtime incl. 103 async pe timere/subprocese, suita e expusă la
    creșterea de hack-uri anti-flake (retry-ul ETXTBSY din lib.rs:278-320 e
    exact așa ceva); nu există semnal CI pe durata suitei, iar lipsa unei
    aserțiuni pe numărul de teste e de ce drift-ul "1184/1018" a trecut
    neobservat.

**Echilibru (pozitiv)**: validare strictă de config, taxonomie reală de
retry/idempotency, teste semnificative incl. E2E pe loopback real, handling
solid de secrete, zero `unsafe`, etichetare onestă "wired vs verified". Riscurile
de mai sus sunt în majoritate despre *drift* — docs, plumbing de config,
convenții — nu despre design de nucleu stricat.

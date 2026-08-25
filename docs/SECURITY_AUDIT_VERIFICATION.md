# Security Audit Verification — Gzeu/decentraai

**Verificat de:** Pylon (audit verification pass, read-only)
**Data:** 2026-08-25
**Metodă:** fiecare finding din auditul extern a fost localizat în codul real de
pe `main` (`dd8021e` + hardening `9af9f63`, `e60f14d`) prin code search, nu din
raportul auditului. Fără blind fixes. Fără modificări de runtime.

**Reguli de clasificare:** CONFIRMED (problema există, cu exploit path concret),
PARTIAL (mecanismul există dar cu o lacună reală), FALSE_POSITIVE,
ALREADY_FIXED.

---

## Tabel de verificare

| # | Finding | Severity claimed | Actual status | Evidence (cod) | Required action | Test care ar demonstra problema |
|---|---------|------------------|---------------|----------------|-----------------|--------------------------------|
| 1 | Quota enforcement la ingress | CRITICAL | **PARTIAL — confirmed gap pe governor_execute** | Rate limit + reserve există pe toate căile consumer (`api.rs:5994/6058/6120/8665/12940`); **DAR governor_execute nu apelează niciodată `settle()` pe `ConsumerQuotaGuard`** — guard-ul e drop-uit la final, iar Drop face *release*, nu consume | Apela `guard.settle(1)` după execuție reușită; altfel un agent rulează DISTRIBUTED nelimitat fără să consume quota | Rulează 2× governor_execute cu același `dca_`; al doilea trebuie refuzat când available=0. Acum: ambele trec |
| 2 | RESERVE → ASSIGN atomicity | CRITICAL | **ALREADY_MITIGATED (TTL backstop) + PARTIAL (release best-effort lipsește la ASSIGN failure)** | `intel_assist.rs:587–607`: ASSIGN eșuat → requester NU trimite RELEASE (comentariu: „Lease will expire on the worker side"); worker-side prune expiră lease-urile la fiecare REQUEST (`intel_assist.rs:190–196`) și ASSIGN validează expiry (`:230`) | Best-effort RELEASE și la ASSIGN delivery failure (nu doar la result timeout) | Oprește workerul între RESERVE și ASSIGN → lease-ul trebuie să expire în ≤ max_lease_seconds și capacitatea să revină |
| 3 | DFCP message-size enforcement | CRITICAL | **FALSE_POSITIVE (deja corect)** | `p2p/lib.rs:1489–1502` `read_frame`: citește length-prefix-ul (4B), respinge `len > max` **înainte** de a aloca buffer-ul; `deserialize_message` (`protocol/lib.rs:141–153`) verifică `len > max_size` înainte de `from_slice`; transport cap: `max_request_bytes=max_message_bytes` (1 MiB config), response cap mai mare pentru chunks | Nicio acțiune | Trimite frame cu length prefix > cap → conexiunea e respinsă cu InvalidData, fără alocare |
| 4 | Credential leakage / Debug / Display | HIGH | **ALREADY_SAFE** | Consumer secret: plaintext afișat o dată la creare, stocat doar ca BLAKE3 hash (`tokens/consumer.rs:16–18, 54–55`); `master_token()` expus doar pentru self-call intern loopback (9404), niciodată logat sau inclus în răspunsuri/evidence (verificat prin grep); evidence entries conțin doar fapte (latency, worker, status) | Nicio acțiune | Grep CI: token/hash nu apare în logs, răspunsuri sau evidence text |
| 5 | Contribution cryptographic verification | HIGH | **CONFIRMED (weakness)** — creditul se bazează pe output non-empty, nu pe criptografie | `governor_execute`: `verified=true` setat de caller; credit acordat pe shards `Completed` = output non-empty; **EvidenceEntry nu are semnătură cryptographică** (`agents/evidence.rs` — niciun câmp signature); `compute.evidence_chain(execution_id)` leagă execution+credit+trace, dar nu e semnat | Semnează evidence entries cu Ed25519 (identity deja există); credit doar pe observații cu receipt semnat | Modifică output-ul unui worker fără a invalida evidența → astăzi trece; cu semnături, nu |
| 6 | Lease watchdog / disconnect cleanup | HIGH | **ALREADY_MITIGATED (TTL)** + gap minor | Worker-side: prune TTL la fiecare REQUEST ofertă (`intel_assist.rs:190–196`) + expiry check la ASSIGN (`:230`); requester-side: RELEASE best-effort la result timeout (`:615–625`). **Gap:** disconnect brusc fără RELEASE → lease rămâne până la TTL (bounded, max_lease_seconds ≤ 120) | Opțional: cleanup la ConnectionClosed event | Oprește workerul cu lease activ → capacitatea trebuie să revină în ≤ max_lease_seconds (120s) |
| 7 | Execution ID propagation | MEDIUM | **FIXED (consolidare)** | `/v1/governor/execute` + `/v1/model-parallel` returnează `execution_id=gov:{task_id}`; același id în toate evidence entries (`gov:{id}:…`) și economy credit (`gov-{id}-…`); receipts P12/P14 au propriul `execution_id` cu `compute.evidence_chain(id)` care leagă execution+credit+worker_balance+selection_trace | Nicio acțiune | Dat un execution_id, reconstruiește decision→model→workers→reduce→credit din evidence + receipts |
| 8 | P2P timeouts | MEDIUM | **ALREADY_SET** | `request_response::Config::default().with_request_timeout(backend_request_timeout())` (`p2p/lib.rs:604–605`, env-overridable); swarm idle timeout 600s (`:642`); M15/M17 self-calls: 240s explicit | Nicio acțiune | Peer care acceptă conexiunea dar nu răspunde → request eșuează cu timeout, nu blochează event loop-ul |
| 9 | unwrap/expect pe căile critice | MEDIUM | **PARTIAL (low risk)** | `runtime/intel_assist.rs`: 9 `.expect("… lock")` — toate pe `std::sync::Mutex` lock-uri scurte, non-poisoning-prone (fără panic în critic path); `p2p/lib.rs`: 2 `.expect("pending assists mutex")` în event loop; **niciun unwrap pe I/O de rețea**; governor_execute folosește `expect("evidence lock")` pe MemoryStore Mutex | Înlocuiește cu handling explicit doar dacă un poison devine posibil (panic în interiorul lock-ului) | Panic test în interiorul unui lock → poison propagă; acum ar fi crash-on-reuse, nu deadlock |
| 10 | Lock ordering / async deadlocks | MEDIUM | **PARTIAL (design risk, nu bug confirmat)** | `p2p/lib.rs:477–517,1333`: setters sincrone folosesc `futures::executor::block_on(mutex.lock())` — apelate înainte de pornirea event loop-ului (safe); event loop folosește `.lock().await` pe aceleași mutex-uri — block_on pe un mutex deja blocat de event loop ar bloca, dar setters nu rulează concurent cu loop-ul; `runtime/api.rs`: `std::Mutex`-uri scurte (ledger, rate windows, recent requests) fără I/O sub lock; **nu s-a identificat ciclu de ordonare** | Documentează ordinea lock-urilor; evită block_on în cod nou | Stress test: workflow + pressure tick + proxy chat simultan → verifică lipsa deadlock-ului |

---

## Rezumat

| Status | Count | Findings |
|--------|-------|----------|
| CONFIRMED (fix needed) | 2 | #1 settle lipsește la governor_execute (quota bypass), #5 credit fără cryptographic proof |
| PARTIAL | 3 | #2 release best-effort la ASSIGN failure, #9 poison recovery, #10 lock ordering documentation |
| FALSE_POSITIVE / ALREADY_SAFE | 3 | #3 message-size (corect: length-prefix cap înainte de alocare), #4 credentials (hash-only storage, token header-only), #7 execution_id (fixed) |
| ALREADY_SET | 2 | #6 lease TTL backstop, #8 p2p timeouts |

## Observații suplimentare

- Consolidările M15/M16/M17 au **rezolvat deja** două dintre prioritățile
  auditului: execution_id propagation (#7, fixată în faza de observability) și
  timeouts pe self-call-uri (parte din #8). Hardening-ul anterior (rate limit
  înainte de quota, MAX_WORKFLOW_STAGES, idle timeout 600s) acoperă și el
  suprafața de ingress.
- Finding #1 (settle) este cel cu impact economic real: fără settle, quota
  consumer este rezervată și eliberată, deci **nu se consumă niciodată** — un
  agent poate rula nelimitat. Fix-ul este o linie (`guard.settle(1)` după
  execuție reușită) plus un test.
- Finding #5 necesită o decizie de design (semnarea evidence cu Ed25519
  identity, deja prezentă în noduri) — nu un quick fix.
- Nu s-au găsit: credential leakage în logs/răspunsuri, SSRF în self-call-uri
  (URL fix loopback, port u16), alocare neterminată înainte de size check,
  deadlocks confirmați.

## Ordine recomandată de fix (după aprobare)

1. `governor_execute`: `consumer_guard.settle(1)` pe execuție validă + test.
2. Semnătura Ed25519 pe EvidenceEntry (Model Colony evidence) — decizie de
   design, nu urgent.
3. RELEASE best-effort și la ASSIGN delivery failure.

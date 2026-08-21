# Review — Data Plane / Trust Core (crates/identity, manifest, protocol, registry, p2p, audit, tokens)

Sursă: subagent de adâncime (2026-08-21), read-only. Verdictul-cheie a fost
re-verificat în cod de coordonator. Referit din `docs/TECHNICAL_REVIEW.md`.

## 1. Responsabilitate & suprafață publică

- **identity** (280 linii, frunză). Keypair Ed25519 cu persistență atomică 0600
  (`atomic_write_0600`, identity/src/lib.rs:89-127: tmp+fsync+rename+fsync
  director-părinte+chmod forțat), `PeerId` (blake3-of-pubkey hex), sign/verify.
  Graniță curată; dar propriul `PeerId` **nu e consumat în afara crate-ului** —
  rețeaua folosește `libp2p::PeerId`, deci există un concept de identitate
  paralel.
- **manifest** (144 linii live + 336 moarte). `Manifest` (tip wire partajat),
  `scan`/`scan_with_name` (GGUF magic + BLAKE3 streaming), `merkle_root`,
  `write_atomic`. `CHUNK_SIZE` fix la 4 MiB (manifest/src/lib.rs:10).
  **`src/model_registry.rs` nu e declarat în lib.rs și n-ar compila dacă ar fi**
  (folosește `anyhow`, care nu e dependență) — design vechi, mort.
- **protocol** (816 + 398 linii). Scheme wire (planul catalog: `deny_unknown_fields`,
  base64, `deserialize_message` cu cap de dimensiune), semnare canonică
  (`canonical_manifest_bytes`, `sign_manifest`/`verify_manifest_signature`,
  `canonical_infer_request_bytes`, `verify_infer_request_signature` — anti-spoof
  pk→PeerId), `SignedComputeAdvertisement`/`SignedAgentAdvertisement`, planul de
  inferență (`infer_protocol.rs`: `InferRequest/Response/Progress/Failed`,
  `InferMessage`, `InferErrorCode`, `WorkerStatus`, `TaskPlacement`,
  `WorkerAnnouncement`). Graniță în general curată, dar crate-grab-bag: trei
  perechi aproape identice de "signed envelope".
- **registry** (829 linii, frunză). `ModelRegistry` (store canonic, scan
  recursiv care sare peste symlink-uri, interogări deterministe) +
  `CapabilityClaimRecord`. Path-safety e punctul forte (canonicalize +
  `starts_with`; `remove_model` respinge `..`/absolute).
  `scan_recursive` (registry/src/lib.rs:109-141) produce `relative_path`
  imbricate (`sub/model.gguf`).
- **p2p** (1232 + 359 + 550 + 675-test linii). Hub-ul planului: `P2PNode`
  actor, `RequestHandler`/`ChainedHandler`/`StaticFileServer`/`RegistryServer`,
  `FrameCodec`, `reputation.rs`, `transfer.rs` (download verificat, carantină,
  audit). Depinde de 5 crates — orchestrator acceptabil, dar transport,
  reputație, transfer, serving și handlere coabitează.
- **audit** (81 linii, frunză). Append-only JSON-lines, `record` (fsync per
  eveniment) + `record_best_effort`. Graniță curată.
- **tokens** (414 + 353 + 172 live + 276 moarte). `TokenStore` (BLAKE3-hashed,
  revoke/expiry/role/tier, atomic), `ConsumerKeyStore` (`dca_`),
  `plan_tier_changes`. **`src/tokenomics.rs` e mort: nedeclarat, referențiază
  `crate::{ContributionRecord, TrustRecord}` inexistente și `chrono` (nu e
  dependență)** — n-ar compila dacă ar fi wire-uit.

## 2. Abstractions & invariante

- **Verify-before-use**: aplicat în `transfer.rs`. BLAKE3 per chunk vs
  `chunk_hashes` (transfer.rs:405-417); gate final în `finalize_download`
  (transfer.rs:447-487): hash fișier complet == `model_id`, Merkle root ==
  `manifest.merkle_root`, apoi rename atomic. Ancoră content-addressed:
  `model_id` = hash fișier complet; `fetch_manifest` verifică
  `response.manifest.model_id == requested` (transfer.rs:309-315). Path-safety
  bună (`validate_artifact_component`, transfer.rs:347-370). **Complet pe calea
  de download; ocolibil upstream** (risc 1: calea announcement→auto-download
  alimentează manifeste arbitrare).
- **Doar eșecurile criptografice pedepsesc**: `record_failure` doar pe erori
  `verify_chunk` (transfer.rs:57, 146, 211); erorile de rețea nu ating scorurile
  (verificat în ambele căi de download). **DAR** carantina se declanșează și pe
  erori pure de rețea (`fetch_verified_or_quarantine` → `quarantine_staging`
  transfer.rs:236), încălcând doc-ul modulului, și strică resume-ul (risc 2).
- **Semnare canonică / determinism**: canonic = `serde_json::to_vec` în ordinea
  declarației, câmpul de semnătură scos (protocol/src/lib.rs:188-190, 219-223);
  fără câmpuri map, deci determinist. **DAR**: `sign_manifest` e chemat doar la
  emiterea anunțului (node-cli/main.rs:2711, 2869) și `verify_manifest_signature`
  are **zero apelanți în producție** — envelope-ul semnat e decorativ.
  `ManifestRequest.signature` (protocol/src/lib.rs:79-80) nu e niciodată setat
  sau verificat. Versionarea e inconsistentă: planul catalog o impune
  (p2p/src/lib.rs:172-174), `InferRequest` nu are câmp de versiune,
  `set_on_worker_announcement` e "not implemented" (distributed/src/worker.rs:433).
- **Secretele rămân locale**: identity 0600 atomic cu chmod (testat la
  244-257); token-uri stocate doar ca hash-uri (tokens/src/lib.rs:122-124,
  consumer.rs:101-103). Keypair-ul libp2p derivat din seed-ul nodului
  (p2p/src/lib.rs:472) leagă transport PeerId de identitate.
- **Completitudinea aplicării**: `verify_infer_request_signature` e aplicat
  corect pe calea worker live (distributed/src/lib.rs:639) cu replay guard +
  rate limit (distributed/src/lib.rs:606-626) — dar aplicarea trăiește în
  `distributed`, nu în acest plan; planul oferă doar primitive.

## 3. Integrare & cuplare

Graful: identity, manifest, registry, audit, tokens sunt frunze. protocol →
identity, manifest (partajează tipul `Manifest` direct — bun). p2p → audit,
identity, manifest, protocol, registry. Fără încălcări de strat ca "protocol
importing registry". Scurgerea moale: `RegistryServer` caută prin
`get_model(&manifest.file_name)` (p2p/src/lib.rs:281-284), unde `file_name` e de
fapt `relative_path` din registry (setat prin `scan_with_name`,
p2p/src/lib.rs:229-231) — dublu sens care deja diverge de semanticile basename
ale lui `scan()` (manifest/src/lib.rs:57-61). **Rupere de integrare**: modelele
din subdirectoare sunt *servite* cu acel file_name dar *respinse* de regula
single-component a downloader-ului (transfer.rs:360-369, test 533-537) —
modelele imbricate sunt nedescărcabile, descoperite doar la runtime.

## 4. Semnale de maturitate

- **Docs**: bune — module cu "de ce" și threat-model (p2p/src/lib.rs:1-8,
  transfer.rs:1-14, reputation.rs:1-7, consumer.rs:1-36); mai multe invariante
  au teste de regresie (cap control-plane p2p/src/lib.rs:1169-1186;
  path-traversal transfer.rs:521-538).
- **Erori**: anyhow+context la granițe; thiserror în manifest; pattern-uri
  best-effort pentru reputație, carantină, audit. Unele apeluri aruncă erorile
  de audit (`let _ = record(...)`, e.g. runtime/providers_api.rs:499) —
  consistent cu "never break the main flow", dar pierderile de audit sunt mute.
- **unwrap/expect în producție**: manifest/src/lib.rs:97-98 (`merkle_root`
  panichează pe hash non-hex/non-32B — API public); protocol/src/lib.rs:189, 222
  (`expect` în serializatoare canonice); identity/src/lib.rs:60
  (`try_into().unwrap()` după check de lungime); registry/src/lib.rs:350
  (`.expect` în `remove_model`); p2p/src/lib.rs:1023, 1035 (`unwrap_or_default`).
- **Cod mort**: `manifest/model_registry.rs`, `tokens/tokenomics.rs`
  (nedeclarate, necompilabile), `set_on_worker_announcement` stub
  (p2p/src/lib.rs:398-405), `NetworkConfig.max_connections` "informational"
  (p2p/src/lib.rs:53-55), `StaticFileServer` doar test, `#[allow(dead_code)]`
  pe `get_model`/`model_count` (registry/src/lib.rs:354-362).
- **Stabilitate API**: string-urile `InferErrorCode::code()` documentate ca
  contract cu test de stabilitate; câmpuri noi `#[serde(default)]`. Dar
  structurile planului de inferență **nu au** `deny_unknown_fields`
  (infer_protocol.rs:11-60) — strictețe inconsistentă față de planul catalog.

## 5. Mirosuri & riscuri concrete

1. **Injecție de modele neautentificate** (risc maxim). p2p/src/lib.rs:771-787
   livrează `ManifestAnnouncement` către `on_manifest` fără verificarea
   `signature`; share worker-ul (node-cli/main.rs:2729-2731, 2823-2886)
   auto-descarcă în `ShareMode::Auto` de la orice peer mDNS. Un peer LAN poate
   anunța `file_name: "legit.gguf"` + un `model_id` self-consistent (hash-ul
   blob-ului servit) și împinge octeți arbitrari în `models/`, pe care nodul îi
   re-anunță (main.rs:2866-2877) și îi poate încărca în llama-server.
   Verificarea semnăturii singură nu ar rezolva — lipsește ancora de încredere.
2. **Carantina strică resume-ul determinist** (risc maxim). `quarantine_staging`
   (transfer.rs:242-274) redenumește doar `<id>.part`, lasă `<id>.done`;
   `load_bitmap` (transfer.rs:489-494) are încredere în orice bitmap de
   lungime potrivită; `prepare_staging` (transfer.rs:420-435) recreează `.part`
   ca fișier sparse zero-filled. Retry după carantină: chunk-urile verificate
   sunt sărite, regiunile lor rămân zerouri, `finalize_download` eșuează
   hash-ul complet **pentru totdeauna** până la ștergerea manuală a bitmap-ului.
   Se declanșează și pe erori pure de rețea (transfer.rs:159-170 → 236).
3. **Codec-ul bufferează până la 96 MiB pe frame-uri de control** (risc maxim).
   `FrameCodec { max_frame_bytes: max(chunk, message) }` (p2p/src/lib.rs:477-479)
   se aplică pe ambele direcții (lib.rs:1071-1087); un peer poate împinge 96 MiB
   per frame de request și abia apoi se lovește de cap-ul de parsare de 1 MiB
   (handlerele folosesc constanta, nu `max_message_bytes` configurat — parametrul
   constructorului afectează doar cap-ul codec-ului). Amplificare de memorie;
   fără bound pe fluxuri concurente.
4. **`RegistryServer` re-hashează fiecare model la fiecare cerere** (perf).
   `manifests()` (p2p/src/lib.rs:224-241) rulează BLAKE3 complet per model per
   cerere de catalog/manifest/chunk — O(total model bytes) per chunk request.
   `ModelRegistry::load` (registry/src/lib.rs:61-67) are încredere în
   `root`/`canonical_path` din JSON fără re-canonicalizare; open-urile la
   serve-time folosesc path-ul stocat necontrolat (p2p/src/lib.rs:286-287) —
   symlink-swap/TOCTOU dacă models dir e scris de alt actor.
5. **Duplicare de envelope semnate** (risc de drift).
   `SignedComputeAdvertisement`/`SignedAgentAdvertisement` sunt envelope
   identice (protocol/src/lib.rs:268-350) cu 4 funcții sign/verify aproape
   identice (293-398); handler-ul le deosebește prin sniffing-ul payload-ului
   (distributed/src/p2p_handler.rs:151-177).
6. **Două concepte de PeerId**. `identity::PeerId` (blake3-hex) nefolosit în
   afara identity; CLI-ul printează ambele (node-cli/main.rs:2757-2759) — două
   string-uri diferite pentru aceeași cheie; trust records stochează peer ids
   ca `String` brut (discovery) — foot-gun de namespace.
7. **`chunk_size_mb` config e unwired și înșelător**: validat 1..=64
   (config/src/lib.rs:482), neconsumat nicăieri; planul hardcodează 4 MiB și
   respinge orice altă valoare (transfer.rs:320-326). Exemplele se contrazic
   (config default 16 vs node-cli example 4).
8. **`schema_version` scris dar niciodată verificat la load** în toate cele trei
   store-uri (reputation.rs:39,78; tokens/src/lib.rs:111,157; consumer.rs:87,141).
   Fișier corupt → restart silențios: la tokens fail-closed (toate token-urile
   refuzate), la reputație uită **silențios băn-urile**.
9. **`Tier` fără invariante la load**: `Tier(9)` parsează din JSON și
   `Tier::name` mapează `_ => "core"` (tokens/src/lib.rs:85-92); runtime-ul
   construiește `Tier(tier)` direct din config (runtime/api.rs:1173), ocolind
   singura validare a tipului.
10. **Magic numbers**: ponderea de reputație `2.0` duplicată (reputation.rs:132
    și 162); pragurile `0.1`/`10` în `can_accept_request`
    (infer_protocol.rs:291-296); timeouts 300s/60s/30s/5×500ms
    (p2p/src/lib.rs:35-37, 515-516, 553, 580); defaults temp 0.7/top_p 0.9/
    priority 128/30s deadline (infer_protocol.rs:62-85).
11. **Fără semantică de răspuns la eroare**: orice eșec de handler e un
    `continue` fără răspuns (p2p/src/lib.rs:842, 846) — requester-ul atârnă tot
    timeout-ul de 300s; fallthrough-ul `on_infer` la handler-ul generic la fel.
12. **Trust pe observed address**: `swarm.add_external_address(info.observed_addr)`
    (p2p/src/lib.rs:896-903) are încredere în adrese raportate de peer
    (foot-gun clasic libp2p); `external_addresses` crește fără cap.
13. **Check-ul de ban e doar la start** (transfer.rs:43-45); un peer banat
    mid-download continuă să servească.
14. **`set_len(manifest.file_size)`** folosește dimensiune controlată de
    atacator (transfer.rs:431); fără bound pe numărul de chunk-uri în
    `validate_manifest` (transfer.rs:319-338) — până la ~15k chunk-uri × 4 MiB
    ≈ 60 GiB revendicate per manifest de 1 MiB.
15. **`now_secs()` duplicat** de trei ori (reputation.rs:197-202,
    tokens/src/lib.rs:126-131, consumer.rs:110-115); tmp+sync+rename copiat în
    patru crates cu detalii diferite (doar identity face fsync pe directorul
    părinte + chmod).

## 6. Verdict (1–5)

| Crate | Scor | Justificare |
|---|---|---|
| identity | **4** | Secret handling atent (0600 atomic, chmod, teste), frunză curată; penalizat pentru `PeerId` nefolosit și unwrap-ul anti-convenție |
| manifest | **3** | Nucleu mic și corect, dar `merkle_root` panichează pe input public malformat, fișier legacy mort în `src/`, chunk size hardcodat contra unei opțiuni de config |
| protocol | **3** | Primitive bune de anti-spoof cu teste, dar verificarea manifest semnat e unwired în producție, envelope-urile sunt duplicate, strictețea/versionarea inconsistentă între planuri |
| p2p | **3** | Arhitectură actor reală, pipeline de transfer solid, e2e excelente — dar bug-ul carantină/resume, codec cu un singur cap, re-hash per cerere și calea de anunț neverificată blochează producția |
| registry | **4** | Cea mai bună disciplină de path-safety din plan, bine testată; penalizat pentru trust-ul pe `registry.json` la load și incompatibilitatea path-urilor imbricate cu transfer |
| audit | **3** | Simplu, corect, best-effort-safe; nume de evenimente libere, fără rotație, fsync-per-eveniment (ok pe LAN, nu la scală) |
| tokens | **4** | Hash-at-rest + atomic + role/tier/expiry testate; penalizat pentru `Tier`/`schema_version` nevalidate la load și `tokenomics.rs` mort |

**Plan overall: 3 — prototip credibil, nu production-hardened.**

## Top 5 riscuri pe termen lung (planul data)

1. **Injecție de modele neautentificate prin anunțuri** — semnăturile sunt
   create dar niciodată verificate; `ShareMode::Auto` instalează și re-servește
   GGUF controlat de atacator (→ llama-server). Necesită verificare de semnătură
   *plus* o ancoră de încredere (trusted set-ul scheduler-ului distribuit) pe
   calea auto-download.
2. **Carantină → bitmap stale → artefact permanent nedescărcabil**, plus
   carantină pe erori de rețea; fix: invalidează bitmap-ul `.done` la carantină
   și gate carantina pe eșecuri criptografice doar.
3. **Codec-ul bufferează 96 MiB per frame inbound înainte de cap-ul de 1 MiB**
   — cap-uri per-direcție + bound de flux inbound.
4. **Config de securitate unwired**: `chunk_size_mb` (16 default vs 4 MiB real),
   `max_connections` (informational), `RegistryServer` re-hash per cerere —
   suprafața de config și planul nu se acordă.
5. **Două cusături predispuse la drift**: (a) duplicarea
   `SignedComputeAdvertisement`/`SignedAgentAdvertisement` cu disambiguare prin
   sniffing; (b) dublul sens al lui `Manifest.file_name` (basename vs relative
   path) care deja rupe modelele imbricate end-to-end.

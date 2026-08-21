# DecentraAI VPS Node Profile

**Status:** design-only profile. No deployment is authorized by this document.

**Base:** `research/unified-selector-shadow@29984c8b35a86725dc8b32805185434be1761410`

**Main invariant:** `main@618e1c3` remains untouched. The VPS is a deployment profile of the existing Universal Node, not a second server architecture.

## 1. Purpose

The VPS Node is the first always-online public DecentraAI node. Its primary role is control-plane and network infrastructure:

- WAN bootstrap / rendezvous anchor;
- coordinator and scheduling host;
- node registry / discovery anchor;
- evidence and accounting persistence anchor where enabled;
- observability and operational endpoint;
- optional lightweight inference worker for small models.

The VPS is **not** expected to provide heavy LLM inference. Large models remain on capable worker hardware (for example, a home desktop/GPU node).

## 1a. Deployment target (concrete)

This profile is written against a concrete, verified target (the host provisioned for
the first official node), but the criteria below are the *selection contract* any
replacement host must satisfy — not a one-off description.

| Dimension | Required | Verified target |
|---|---|---|
| OS | Ubuntu Server 24.04 LTS (or current LTS), clean install | Ubuntu 24.04.4 LTS (`vmi3524028`) |
| CPU | ≥ 4 vCPU, x86-64 | 6× AMD EPYC 2.0 GHz |
| RAM | ≥ 8 GiB | 11 GiB |
| Disk | ≥ 40 GiB free, local/block (not ephemeral) | 193 GiB / `/dev/sda1` (2% used at provision) |
| GPU | **optional**; not required for the control-plane role | none (QEMU virtio VGA only) |
| Network | stable public IPv4; IPv6 strongly preferred if provider offers prefixes | public IPv4 (169.58.x.x, IBM SoftLayer) |
| Provider firewall | supported and mirrored to `ufw` | yes |

Resource sizing is deliberate: this host is a **control-plane / bootstrap / coordinator /
registry / evidence-anchor**, *not* an "AI server". Inference here is limited to at most a
small model running as an ordinary worker; the CPU/RAM headroom is for steady operation,
P2P message volume, ledger I/O and buffering, not for concurrent large-model generation.

**Resource selection criteria for any replacement host:**
- adequate for: P2P connection fan-in, registry/heartbeat volume, evidence/ledger JSON
  persistence, reverse-proxy buffering, one small local worker;
- NOT sized or priced for heavy GPU inference (keep that on home/LAN worker hardware);
- a GPU is only justified if the host will deliberately serve a model; otherwise it is
  cost that buys nothing for the control-plane role.

## 2. Threat-model change

LAN operation is trust-explicit and link-local. A VPS is internet-reachable and therefore must assume:

- unsolicited connections;
- credential probing;
- HTTP/API abuse;
- connection floods and queue pressure;
- malicious or malformed peer traffic;
- compromised or untrusted nodes attempting admission;
- disk exhaustion through logs/history;
- service restart/crash attempts;
- credential or private-key theft.

The design must reduce exposure without weakening the existing security invariants.

## 3. Public API boundary

The DecentraAI application API remains bound to **`127.0.0.1`** on the VPS.

Do **not** introduce or enable an `api.expose_public` configuration path merely to expose the service.

Public HTTPS traffic terminates at a reverse proxy (Caddy is the reference implementation):

```text
Internet
   |
   | TCP 443
   v
+-------------------+
| Caddy              |
| TLS / limits       |
| request filtering  |
+---------+---------+
          |
          | loopback
          v
+-------------------+
| DecentraAI Node    |
| 127.0.0.1:8080     |
+-------------------+
```

The reverse proxy is responsible for:

- automatic TLS certificates;
- HTTP to HTTPS redirect;
- connection/request limits;
- conservative request body limits;
- access logging;
- forwarding only required paths;
- preserving the Bearer authentication model already used by the node.

No direct public bind of the application API is permitted by the VPS profile.

## 4. Authentication and admission

Application authentication remains mandatory. The existing Bearer token mechanism is retained.

P2P admission is separate from HTTP authentication:

1. node discovers/bootstrap information;
2. node presents its identity and admission material;
3. trust is explicitly established;
4. only then may the peer participate in the trusted fabric.

A newly discovered WAN peer with reputation/trust zero must **not** receive implicit trusted-worker status.

The existing invite / admission flow remains the authority for onboarding. Do not replace it with IP allowlists as the primary identity mechanism.

Private identity material must remain readable only by the service account.

## 4a. Service exposure matrix

Three distinct network planes are never conflated. Each service belongs to exactly one
plane; a service's plane determines whether it may be reachable from the public Internet.

| Plane | Reachability | Services |
|---|---|---|
| **Public application** | Internet → `443/tcp` only, TLS-terminated at Caddy | OpenAI-compatible chat API, dashboard (authn) |
| **P2P** | Internet → selected libp2p listener port(s) only | peer dial/accept, bootstrap/rendezvous, remote inference traffic |
| **Administration / internal** | localhost + private admin network only (never public) | application API `127.0.0.1:8080`, SSH (separate, key-only), databases, metrics collectors, Prometheus/Grafana if any, emergency admin API |

Explicit **public / private / loopback** matrix:

| Service | Bind | Public | Private LAN/net | Loopback |
|---|---|---|---|---|
| DecentraAI application API | `127.0.0.1:8080` | ✗ | via SSH tunnel/Caddy only | ✓ |
| Caddy (TLS proxy) | `0.0.0.0:443` | ✓ | ✓ | ✓ |
| libp2p P2P listener | configured WAN port | ✓ (selected port) | ✓ | ✓ |
| SSH | provider-restricted | ✗ (key-only, source-restricted) | ✓ | ✓ |
| SQLite/Postgres | local | ✗ | ✗ | ✓ |
| Metrics (if used) | `127.0.0.1` | ✗ | admin net only | ✓ |
| Prometheus / Grafana | admin net | ✗ | admin net only | ✗ |
| llama-server / vLLM | `127.0.0.1` (executor) | ✗ | ✗ | ✓ |
| Emergency admin API | `127.0.0.1` | ✗ | ✗ | ✓ |

Rule: **anything that is not explicitly in the *public* column of this matrix is firewalled
closed by default.** Exposing a service means changing this matrix deliberately, not opening a
port ad hoc.

## 4b. HTTP vs P2P vs SSH — identity separation

Three different trust domains are involved and must never be interchangeable:

1. **HTTP application auth** (Bearer token) — authorizes *API callers* (a human/operator or a
   dashboard session). It says nothing about *peer* trust.
2. **WAN node identity** (Ed25519 keypair → PeerId + signed traffic) — authorizes *peers* to
   participate in the trusted fabric. It is independent of HTTP tokens.
3. **SSH admin** (key-only) — control of the host itself. Highest privilege.

A compromised token must not grant fabric peer trust. A compromised peer identity must not
grant HTTP admin. A dial on the P2P port must never touch the HTTP API surface (separate
processes/ports, cross-boundary validation at the application layer only).

## 5. Firewall and exposed ports

The host firewall should implement a default-deny inbound policy.

Required public surfaces are limited to:

- `443/tcp` — HTTPS reverse proxy;
- the explicitly selected DecentraAI/libp2p P2P listener port(s) required by the chosen transport configuration.

SSH is an administrative surface and should be restricted separately (prefer key-only authentication and source restriction where practical). It is not part of the public application surface.

Do not expose:

- PostgreSQL/SQLite databases;
- Redis;
- Prometheus;
- Grafana;
- internal application ports;
- debug endpoints;
- admin APIs not explicitly protected;
- model backends such as llama-server/vLLM directly to the Internet.

`ufw` is the reference host firewall; the VPS provider/network firewall should mirror the same minimal policy where available.

## 6. P2P connectivity and WAN bootstrap

LAN mDNS is not sufficient for WAN discovery. The VPS therefore acts as an always-online bootstrap/rendezvous anchor.

Home nodes should initiate outbound connectivity to the VPS. The architecture should not require inbound port forwarding at the home location merely to join the fabric.

The existing node identity and invite/bootstrap mechanism should carry the VPS bootstrap multiaddress and peer ID.

The VPS must not become a centralized inference dependency: if a worker is temporarily unavailable, the network should degrade according to existing admission/scheduling semantics rather than treating the VPS as a mandatory inference proxy.

## 6a. IPv6 and NAT / WAN treatment

**IPv4 / NAT:** home nodes behind CGNAT or NAT dial **out** to the VPS public IPv4. The VPS
must not require inbound port forwarding at the home site to admit a worker. The VPS's stable
public IPv4 + PeerId is the bootstrap target carried in the invite/join material. The P2P
listener advertises its real, reachable external address (a single public IPv4 is sufficient;
do NOT advertise RFC1918 or the internal NIC address as the dialable address).

**IPv6:** if the provider assigns a stable IPv6 prefix, enable it and advertise the reachable
global address in addition to IPv4 — it removes NAT hairpin/carrier-NAT latency and gives a
cleaner dial path for IPv6-capable peers. Rules:
- advertise only globally reachable addresses (no link-local `fe80::`, no ULA `fc00::/7`);
- the provider/cloud firewall must open the same P2P port over IPv6 as over IPv4;
- Caddy should listen `:443` for both stacks (Caddy handles dual-stack TLS);
- if the provider does not give IPv6, that is acceptable — the fabric is IPv4-first, IPv6-optional.

**Dial reliability:** prefer outbound dials from home → VPS for the steady state. Treat the VPS
as the rendezvous: it is contacted, it does not need to cold-dial home NAT'd workers. Keep the
P2P listener's messages bounded (existing size caps) — a public listener is a flood surface.

**Fixed P2P port (critical deployment requirement):** the VPS must use a **fixed, stable P2P port**
configured in the node's P2P listener address. A dynamic port (which libp2p chooses by default
when no explicit port is given) causes the VPS to become unreachable after every restart — home
workers' bootstrap addresses hard-code the old port and cannot discover the new one without
manual reconfiguration. The P2P address in `bootstrap_peers` must remain valid across restarts.

**Phase 4 deployment finding:** `private_swarm=true` does NOT block the P2P compute-advertisement
path — a trusted WAN peer's `ComputeAdvertisement` is accepted by the coordinator's scheduler
registry regardless of the `private_swarm` flag (the flag gates LAN discovery, not P2P message
exchanges). The only gate for scheduler eligibility is `self.trusted.contains(&adv.peer_id)`.
This means the VPS coordinator can accept advertisements from trusted remote workers without
disabling `private_swarm`.

## 6b. Threat model and failure modes (concrete)

Accepted threat model for the public node, beyond LAN:

| Threat | Concrete failure mode | Primary mitigation |
|---|---|---|
| Port/credential probing | scans on 443/P2P/22 | firewall default-deny; fail2ban on SSH; only required ports open |
| API abuse / token theft | hostile requests through Caddy | Bearer auth; Caddy TLS+limits; tier rate limits; rotation |
| Malformed/oversized requests | parse/memory pressure | request body caps; size caps; bounded queues |
| Connection floods | fd/memory exhaustion | connection limits at proxy; bounded accept; resource limits |
| Malicious/malformed peer | bad messages / crypto failures | per-chunk verify; signed messages; reputation (only crypto penalties); quarantine |
| Untrusted/compromised peer seeking admission | fraudulent worker/advertisement | explicit trust/invite flow; zero-trust default; reputation 0 |
| Disk exhaustion | logs/history/ledger fill disk | retention bounds; disk usage alert; bounded rings |
| Restart/crash attempts | availability loss | systemd restart w/ backoff; health probes; supervisor |
| Private-key/credential theft | identity takeover | mode-0600 keys, service account, encrypted backup, not in git |
| DoS on a single home worker | availability caveat | VPS is rendezvous only, not mandatory inference path |

Acceptable degradation: if the VPS is down, home nodes can still complete *already-trusted*
work among themselves over their existing links; admission/bootstrap of new peers waits for the
rendezvous. The VPS must never become a single point of authority for *execution* — routing and
inference decision-making stay distributed per the existing architecture.

## 7. Universal Node reuse

The VPS runs the existing `decentraai node` / Universal Node architecture.

Do not create a second daemon, separate public-server codebase, or VPS-only routing implementation.

The VPS profile changes deployment configuration and operational boundaries, not the core architecture.

Existing components remain responsible for their established roles:

- P2P / identity — node identity and peer connectivity;
- discovery / registry — worker and peer knowledge;
- distributed compute — planning and reservations;
- UnifiedSelector — selection logic;
- inference adapter — backend execution;
- dashboard/API — operator and application interface.

## 8. Scheduler and UnifiedSelector

After Phase 3 validation, UnifiedSelector may remain enabled in **shadow mode** for continued evidence collection.

For the first public VPS deployment:

- legacy planner remains authoritative unless a separately reviewed promotion decision is made;
- UnifiedSelector shadow execution remains observe-only;
- shadow errors fail closed;
- shadow mode remains disableable;
- routing/reservation must not depend on shadow success.

Promotion of UnifiedSelector from shadow to authoritative routing is explicitly out of scope for the initial VPS deployment.

## 9. Worker/model separation

The VPS should not expose inference backends directly.

Preferred topology:

```text
Public client
     |
    443
     v
 reverse proxy
     |
     v
 DecentraAI VPS
 coordinator / bootstrap
     |
     | secure P2P
     +--------------------+
     |                    |
 home GPU worker      home CPU worker
     |                    |
  large model          small model
```

If the VPS has sufficient resources, it may run a small local model as an ordinary worker. That capability must remain subordinate to the same node/worker abstractions used elsewhere.

## 10. Rate limiting and resource protection

The public boundary must enforce conservative limits before requests reach the application.

Required protections:

- connection limits at reverse proxy;
- request body/size caps;
- request rate limits;
- application tier limits already present in DecentraAI;
- queue depth limits;
- RAM reserve gate;
- inference timeout limits;
- bounded shadow-record storage;
- bounded logs/history/trace retention.

The objective is graceful degradation under load rather than unbounded memory or queue growth.

## 11. Process and OS hardening

Run DecentraAI as a dedicated non-root service account.

Prefer the existing systemd user-service model where appropriate; do not run the node as root.

The production unit should use systemd hardening appropriate to the actual filesystem/network requirements, including where compatible:

- `NoNewPrivileges=yes`;
- restricted filesystem access;
- private temporary directory;
- controlled device access;
- resource limits;
- automatic restart with bounded backoff;
- explicit environment/config paths;
- journald logging with retention controls.

Do not apply a hardening option that silently breaks required libp2p, model, or filesystem functionality; validate the final unit on the target VPS.

## 12. Fail2ban

Fail2ban may protect SSH and reverse-proxy authentication abuse where log patterns are reliable.

It is a secondary control, not the primary security boundary. Firewall policy, TLS, application authentication, admission control, and rate limiting remain mandatory.

## 13. Persistence and backup

The VPS is expected to become an always-online persistence anchor for operational evidence/accounting where those features are enabled.

Back up at minimum:

- node configuration;
- node identity/private key material using a secure backup mechanism;
- registry state;
- credit/token/evidence databases or files where enabled;
- relevant migration/version metadata.

Backups must not be publicly reachable from the web server.

Private keys require encrypted/off-host backup handling and must not be committed to Git.

A backup is considered valid only after restoration has been tested.

## 13a. Backup + restore: demonstrated procedure

Backup is a *procedure with a verification step*, not a config flag. The VPS is an
always-online persistence anchor, so this is tested on a schedule, not assumed.

**What is backed up (atomic, consistent snapshot):**

- `config.*` — node configuration;
- identity key material (`identity/key.pem`) — **encrypted, off-host only**, never in the
  same tarball as non-secret state, never in Git;
- registry state (`db/registry.json`, `db/registry.bak` etc.);
- evidence/credit/accounting files (`db/*.json`, `db/*.jsonl`) where enabled;
- migration/version metadata (the exact commit/hash and binary version in use).

**Procedure (reference commands; adapt to the actual data dir):**

```bash
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
# 1. freeze live writes by taking files while the node is healthy (copy, not tar -z from a
#    hot dir, to avoid torn writes); prefer the node's own atomic snapshot files.
nice tar -C "$DATA_DIR" -cf /var/backups/decentraai/state-${STAMP}.tar \
    config.yaml db/ tools/ 2>/dev/null
# 2. secret material separately, encrypted (age/gpg), off-host:
age -e -r "$RECIPIENT" identity/key.pem > /var/backups/decentraai-secret/key-${STAMP}.age
# 3. ship both to a DIFFERENT host/site (never only local to the VPS):
rsync -a /var/backups/decentraai/ "$BACKUP_HOST:/backups/decentraai/state/"
rsync -a /var/backups/decentraai-secret/ "$BACKUP_HOST:/backups/decentraai/secret/"
```

**Restoration verification (the only thing that makes a backup valid):**

On a disposable host (or in the staging profile), restore the tarball + decrypted key into a
fresh data dir, start a node with a *fresh* advertise/identity override, and assert:

- node starts and the registry/evidence/credits load (`load_execution_history`, `restore`
  paths report success);
- `db/` balances and idempotency sets round-trip (no double-credit on replay);
- a P2P dial to the production coordinator succeeds with the restored identity/PeerId.

Run this restoration drill at least once before going public and on a schedule (e.g. weekly)
thereafter. Log the drill result. A backup that has never been restored is considered untested
and NOT valid for release.

## 14. Upgrade and rollback

Production upgrades must be incremental and reversible.

Required pattern:

```text
current version
      |
 health snapshot
      |
 backup persistent state
      |
 install new binary
      |
 restart
      |
 health + P2P + API + inference checks
      |
      +---- PASS ---> retain
      |
      +---- FAIL ---> rollback previous binary
```

Keep the previous known-good binary available until the new deployment passes its verification window.

Use the existing `upgrade-node.sh` discipline rather than ad-hoc binary replacement.

## 14a. Automated verification gate (upgrade/rollback)

An upgrade is not complete until an automated gate PASSES *and* is recorded. The gate is the
same scripted sequence used at first provision, now run against the new binary:

1. **Pre-flight snapshot:** record current peer count, worker count, `/status` health, shadow
   metrics, disk, `git` HEAD, binary checksum.
2. **Backup persistent state** (§13a) — mandatory before installing.
3. **Install + restart** the new binary; wait for the service to reach `active` and pass its
   health probe.
4. **Automated verification checks** (each asserts a numeric/state threshold, not just "ran"):
   - API: `GET /status` returns `200` and `attached` true;
   - P2P: peer count ≥ pre-flight (bootstrap re-establishes); coordinator sees ≥ 1 verified peer;
   - routing/inference: one remote + one local inference round-trips successfully (the exact
     smoke path used on LAN);
   - registry/evidence: `load_*`/`restore` logs report success (no replay corruption);
   - observability: shadow metrics endpoint responds, UnifiedSelector still observe-only;
   - disk/MEM: usage within pre-flight ± threshold (no unbounded growth).
5. **PASS** → promote new binary, archive the previous binary with its checksum for rollback.
6. **FAIL** (any check) → **automatically** `systemctl restart` with the previous binary, re-run
   the gate; a second failure pages on-call. Log the outcome and the exact failing check.

The gate script must be idempotent and exit non-zero on any failing assertion so CI/CD can block.
Manual "I think it's fine" is not an upgrade result.

## 15. Observability

The public node must expose operational visibility without exposing internal management surfaces publicly.

Monitor at minimum:

- process/service uptime;
- API availability;
- P2P peer count and connectivity;
- bootstrap health;
- request rate;
- queue depth;
- rejection/rate-limit counts;
- inference latency;
- error rate;
- worker availability;
- shadow invocations/errors/agreements/diffs;
- shadow decision latency;
- disk usage;
- memory pressure;
- restart count.

Metrics collectors and dashboards should bind privately or be protected behind an authenticated administrative network. They must not be directly exposed on the public Internet.

## 16. Load and failure testing before public exposure

Before production exposure, run a controlled load test against the reverse proxy and application boundary.

The test must demonstrate:

- rate limits engage as configured;
- queue limits prevent unbounded growth;
- RAM reserve protection works;
- malformed/oversized requests are rejected;
- sustained load does not corrupt persistent state;
- shadow errors cannot affect authoritative routing;
- service recovery works after restart;
- rollback remains functional.

The load test is a release gate, not an optional benchmark.

**Concrete thresholds the gate asserts (tune to the actual host, but assert numbers):**

- **Rate limit engagement:** sustained request rate breaks the configured per-tier window and
  `429` (or the configured limit response) is returned; the node's request counter does not
  mean unbounded acceptance.
- **Queue bound:** under over-subscription the queue length stays ≤ configured cap; requests
  beyond it are rejected/shed, not buffered without limit; memory stays flat.
- **RAM reserve gate:** with a simulated low-RAM worker, admission refuses (the reserve is a
  hard floor, not a soft warning).
- **Malformed/oversized:** > cap body and truncated/malformed requests are rejected with a
  `4xx`; no panic, no crash, no state corruption.
- **Persistence integrity:** run the load, then verify `db/` globals + evidence/ledger still
  load and idempotency is intact (no duplicate credit after the churn).
- **Shadow isolation:** run load with shadow enabled; assert `errors==0`,
  `invocations>0`, and the authoritative routing result is byte-identical to shadow-off
  (shadow can not influence routing under load).
- **Recovery:** `systemctl restart` during/after load → service returns to healthy within the
  configured backoff; previous worker/peer links re-establish; verified peers survive.
- **Rollback:** an intentionally defective binary is detected by the gate and rolled back
  automatically with the previous binary, returning the node to a healthy state.

Pass = every assertion above. Any failure blocks public release.

## 17. Security release gates

The VPS Node is not production-ready until all of the following are true:

- [ ] API binds only to loopback.
- [ ] HTTPS reverse proxy is operational with valid TLS.
- [ ] Only required public ports are open.
- [ ] Bearer authentication is enabled and verified.
- [ ] WAN peer admission requires explicit trust/invite flow.
- [ ] Databases/metrics/admin ports are not publicly reachable.
- [ ] Node runs non-root.
- [ ] systemd hardening is validated.
- [ ] Rate limits and queue/resource gates pass load testing.
- [ ] Backups exist and restoration has been tested.
- [ ] Previous binary is retained for rollback.
- [ ] P2P bootstrap from a remote node succeeds.
- [ ] Remote inference succeeds through the normal DecentraAI routing path.
- [ ] UnifiedSelector remains observe-only unless separately promoted.
- [ ] No production deployment changes `main` without review.

## 18. Explicit non-goals

This profile does **not** authorize:

- direct public application binding;
- treating the VPS as an "AI server" for heavy inference (it is control-plane and an ordinary
  small-model worker at most);
- replacing the Universal Node with a new server architecture;
- exposing llama-server/vLLM directly to the Internet;
- automatic trust of unknown WAN peers;
- making UnifiedSelector authoritative (it stays **observe-only** even while the VPS is
  public — promotion is a separate reviewed decision);
- implementing worker-side KV cache identity;
- introducing crypto/token payments;
- opening databases, Redis, Prometheus, or Grafana publicly;
- production deployment itself.

## 19. Target topology

```text
                         PUBLIC INTERNET
                               |
                              443
                               |
                         +-----v-----+
                         |   Caddy   |
                         | TLS/limits|
                         +-----+-----+
                               |
                         127.0.0.1
                               |
                    +----------v-----------+
                    |  DecentraAI VPS Node  |
                    |                       |
                    | Universal Node        |
                    | Bootstrap / P2P       |
                    | Coordinator           |
                    | Registry              |
                    | Scheduler              |
                    | UnifiedSelector       |
                    | Evidence/Accounting   |
                    | Observability         |
                    +----------+------------+
                               |
                         secure P2P
                  +------------+------------+
                  |                         |
             Home Desktop              Home Laptop
             GPU / large model          CPU / small model
```

## 19a. First-worker bootstrap procedure (concrete)

The first node is the VPS bootstrap/coordinator. The first *controlled worker* is onboarded
with the invite/join flow — the authority for admission — never by auto-trust:

1. On the VPS, confirm identity is provisioned (`decentraai init` / node identity path) and
   the PeerId is known; confirm bootstrap P2P listener is up on the advertised public address.
2. On the home worker, generate/keep its own identity; obtain the VPS bootstrap multiaddr +
   PeerId from the operator (this is the **invite material**, not a raw IP allowlist).
3. From the worker, `decentraai join <bootstrap+peerid>` (or the equivalent existing invite
   command) → worker dials OUT to the VPS. No inbound port forwarding at home.
4. On the VPS coordinator, **explicitly trust** the worker's PeerId (the existing
   trust/admission primitive). Confirm the worker advertises its served model.
5. Verify end-to-end: from the VPS operator API, route a remote inference to the worker; the
   coordinator's selection trace records `selected == reserved == worker` (or the honest
   local-exclusion distinction), outcome `succeeded`.
6. Confirm the worker accrued real measured presence/contribution and reputation > 0 from a
   verified completion (never from a network-only observation).
7. Record the worker's identity + model in the registry/evidence anchor; it is now an
   operating peer. Only the coordinator's explicit trust keeps it admissible thereafter.

Do not skip step 4 (explicit trust). An untrusted WAN worker stays reputation-0 and is
rejected (`UntrustedWorker`) even if it dials correctly.

## 20. Deployment sequence after this profile

The profile is documentation only. The eventual deployment should proceed as a separate reviewed effort:

1. select VPS resources/provider (satisfying §1a selection criteria);
   2. provision a clean supported Linux LTS host;
   3. apply firewall and OS hardening (§5, §11, §12);
   4. install the existing DecentraAI node using the Universal Node path;
   5. configure loopback API + reverse proxy TLS (§3);
   6. configure VPS identity/bootstrap role (§6);
   7. on-board the first controlled worker via the invite/trust flow (§19a);
   8. verify admission, P2P, routing, inference and observability;
   9. run the load/failure gate with the numeric thresholds (§16);
   10. only then open public client access — and only after §17 security gates all pass.

Each step is its own reviewed, reversible stage with a recorded outcome. No step above
changes the current `main` branch or promotes UnifiedSelector to authoritative routing.
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
- replacing the Universal Node with a new server architecture;
- exposing llama-server/vLLM directly to the Internet;
- automatic trust of unknown WAN peers;
- making UnifiedSelector authoritative;
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

## 20. Deployment sequence after this profile

The profile is documentation only. The eventual deployment should proceed as a separate reviewed effort:

1. select VPS resources/provider;
2. provision a clean supported Linux LTS host;
3. apply firewall and OS hardening;
4. install the existing DecentraAI node using the Universal Node path;
5. configure loopback API + reverse proxy TLS;
6. configure VPS identity/bootstrap role;
7. connect one controlled remote worker;
8. verify admission, P2P, routing, inference and observability;
9. run load/failure gates;
10. only then open public client access.

No step above changes the current `main` branch or promotes UnifiedSelector to authoritative routing.
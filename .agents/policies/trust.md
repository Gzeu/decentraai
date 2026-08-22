# Policy: trust & security

## Threat model (check on every change)

malicious peer · forged announcement · replayed task · duplicate task ·
resource exhaustion · oversized message · malicious capability claim ·
forged contribution · peer impersonation · task-result tampering ·
credential exfiltration · prompt injection into any model in the loop ·
recursive agent loops.

## Standing requirements

1. **Identity binding**: transport-authenticated PeerId only; payload
   sender fields are attacker-controllable and never trusted.
2. **Bounded everything**: messages, plans, histories, leases. A runaway
   input must be rejected, not clamped into danger.
3. **deny_unknown_fields** on every wire schema.
4. **Replay/duplicate protection** where state mutates (nonces, idempotency
   keys, dedup sets).
5. **Lease expiry**: every reservation/lease expires even if RELEASE is
   lost.
6. **Evidence before credit**: verified success only, through existing
   ledger paths.
7. **Secrets stay local**: keys mode 0600; external API keys read from env
   AT CALL TIME; error paths pass through `redact_secrets`; nothing secret
   in logs, dashboard, audit or telemetry.
8. **No arbitrary execution from remote payloads**: remote data is data;
   code runs only from deterministic Rust.

## Auth vocabulary

- `dca_` — consumer key: quota-limited, rate-limited, non-admin.
- `dsk_` — legacy subscription token: NEVER admin UI.
- anything else — master CANDIDATE, proven only by probing a master-only
  endpoint (see dashboard classifier).

## AI-output rule

Model-generated decisions are untrusted input. The intelligence layer may
propose; deterministic Rust validates against real state and decides.

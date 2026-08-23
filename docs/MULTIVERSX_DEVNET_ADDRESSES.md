# MultiversX Devnet Addresses — VERIFIED ONLY

This file lists ONLY what is confirmed from official sources or live probes.
No address is guessed. Empty section = we found nothing verified yet.

## MX-8004 registry contracts (Identity / Validation / Reputation)

**UNVERIFIED — no devnet contract addresses found in the official
`multiversx/mx-agent-standard` specifications (S1/S2) nor in the skill.md
HTTP-wrapper reference.**

The HTTP wrapper (`POST /agents`, `GET /agents*`) abstracts the contracts;
its backend addresses are not published in these sources. To obtain them:
inspect the wrapper service's deployment config, or read an actual
registration transaction on the devnet explorer and extract the receiver
(Identity Registry), then follow cross-contract calls.

## Hosts probed live (2026-08-23, from this environment)

| Host | DNS resolves | HTTPS reachable | Note |
|---|---|---|---|
| `agents.multiversx.com` | ✅ | ✅ (serves skill.md) | official agent portal |
| `devnet-api.multiversx.com` | ✅ | ✅ (404 on unknown paths = server alive) | general devnet API |
| `devnet-mx8004-api.multiversx.com` | ❌ DNS failure here | untested | documented in S3; PARTIALLY VERIFIED at best |
| `devnet-explorer.multiversx.com` | not probed | — | referenced by S3 |

## Rule going forward

An address enters this file only with: source link + fetched date + where it
appears in that source. Everything else stays out.

## Address discovery attempts — 2026-08-23 (Track A, round 1)

| Method | Result |
|---|---|
| GitHub code search `org:multiversx mx8004` | found `mx-agent-standard/mx8004_technical_specs.md` (source spec, S1) — **no addresses** |
| `multiversx/mx-agent-kit` repo + submodules (`eliza`, `gateway`) | gateway = generic node proxy, not MX-8004; no registry addresses |
| Code search for `agentRegistry erd1` / `devnet address` in org repos | no hits |
| Live probe `devnet-mx8004-api.multiversx.com` | DNS unresolvable from this environment (webfetch transport also failed); other multiversx hosts resolve fine |
| Live probe `devnet-api.multiversx.com` | resolves and serves (host alive) |

Conclusion: contract addresses stay UNVERIFIED. Next legitimate sources to
try: official deployment artifacts/config of the wrapper service, a live
registration transaction on the devnet explorer (receiver = Identity
Registry), or direct contact with the MX agents team.

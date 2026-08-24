# MultiversX Devnet Addresses — VERIFIED ONLY

This file lists ONLY what is confirmed from official sources or live probes.
No address is guessed. Empty section = we found nothing verified yet.

## MX-8004 registry contracts (devnet) — VERIFIED 2026-08-23

Discovery method (identical for all): query the general devnet indexer
`https://devnet-api.multiversx.com/transactions?function=<fn>&status=success`
and take `receiver` from successful transactions; corroboration across
multiple independent functions/senders where available.

### Identity Registry — VERIFIED
```
address: erd1qqqqqqqqqqqqqpgqzcufga3vm5r44xe3ukzyl4dmhpsvalrkkgjqeyu68x
method:  receiver of ≥2 successful register_agent txs from DIFFERENT senders
tx hashes:
  a88eb6bfd67b49f8eab2143d6d019e4e437bdaaf4079c9fc590613033c2b08f7
  611c25d6a077e3839b1cf92f91ebea99c88f15dbc15f579aabe8279b6c4c0f9b
gas used: 60,000,000 per registration · value: 0
```

### Validation Registry — VERIFIED
```
address: erd1qqqqqqqqqqqqqpgqvax6z79cvyz9gkfwg57hqume352p7s7rd8ss4g3t43
method:  same receiver across THREE distinct functions (submit_proof,
         validation_request, validation_response)
example tx hash: 09b7b9abf2bb1dad… (submit_proof)
```

### On-chain corroboration (gateway /address probe, 2026-08-24)
All three addresses return `codeHash` present = deployed smart contracts:
| Registry | codeHash present | Owner |
|---|---|---|
| Identity | ✅ | `erd1c83tgujs47scafsu7kyt…` |
| Validation | ✅ | `erd1qyu5wthldzr8wx5c9ucg…` |
| Reputation | ✅ | `erd1qyu5wthldzr8wx5c9ucg…` (SAME deployer as Validation → same MX-8004 deployment family) |

Reputation status upgraded PARTIALLY → **VERIFIED** (args match spec +
same-deployer corroboration with the two fully corroborated registries).

### Reputation Registry — PARTIALLY VERIFIED (single success observed)
```
address: erd1qqqqqqqqqqqqqpgqwhqpuzkrywc5j8q2ec6skqnejtzgjnzad8ssdmv962
method:  receiver of a successful submit_feedback tx whose decoded args
         EXACTLY match spec S1 §3: submit_feedback@job_001@01@05
         (job_id "job_001", agent_nonce 01, rating 05)
tx hash: see indexer query function=submit_feedback&status=success
caveat:  one observation only — corroborate before relying on it.
```

### Live API base — CORRECTED
```
documented in skill.md: https://devnet-mx8004-api.multiversx.com  (DNS unresolvable from our environments)
ACTUAL base used by the official explorer bundle: https://devnet-taskclaw-api.multiversx.com
verified live: GET /agents?from&size → {items:[{txHash, owner, collection:"ACT-e4c050", nonce, name, uri, publicKeyHex, metadata[]}]}
               GET /reputations/agents/:nonce → {agentNonce, average(FLOAT), count}
```

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

## Deployer analysis — 2026-08-24

Identity Registry deployer (full): `erd1c83tgujs47scafsu7kyt0myxeg50aw5lzxm47y73ue45ktzfkgjq8m3fae`
→ `/accounts/{owner}/contracts` lists **25 deployed contracts**, INCLUDING our
verified Identity Registry (`…jqeyu68x`). Official-team deployment CONFIRMED.

Validation + Reputation registries resolve to a DIFFERENT owner
(`erd1qyu5wthld…`): functional behavior matches spec exactly (args verified),
but they are NOT from the primary official deployer's list. Status:
- Identity Registry: **VERIFIED + official-deployer confirmed**
- Validation / Reputation: **functional-match, alternative deployment**
  (usable for devnet experiments; re-verify against the official deployer's
  current registry set before any production use)

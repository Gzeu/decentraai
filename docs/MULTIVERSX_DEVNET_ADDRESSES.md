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

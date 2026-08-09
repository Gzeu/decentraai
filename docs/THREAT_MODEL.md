# Threat Model

## Trust assumptions
- Local disk, OS account, and private node keys are trusted only to the extent the host is trusted.
- Every network peer, DHT value, manifest, model, tokenizer, configuration import, and inference result is untrusted by default.
- Encryption in transit does not make a remote inference worker confidential.

## Threats and mandatory controls

| Threat | Mandatory control |
|---|---|
| Corrupted or poisoned chunks | Per-chunk BLAKE3, Merkle-root check, signed manifest, quarantine |
| Manifest spoofing | Canonical serialization, Ed25519 signatures, trust store |
| Replay and request forgery | Request ID, nonce, timestamps, signatures, expiry |
| Sybil and flood | Per-peer limits, connection caps, temporary bans, private-first swarm |
| DHT eclipse | Multiple bootstrap peers, bounded trust, independent peer observations |
| Resource exhaustion | CPU/RAM/VRAM/disk/network quotas, queue caps, timeouts, watchdogs |
| Malicious input parsing | Size caps, strict schemas, process isolation, no unsafe deserialization |
| Prompt leakage | No prompt/output logging by default, localhost default binding, private worker policy |
| Malicious update/plugin | Signed releases only; no peer-supplied executable code |

## Absolute prohibitions
- Never execute peer-supplied code.
- Never accept an artifact as verified before hash and manifest checks pass.
- Never commit private keys, API keys, tokens, model caches, or databases.
- Never bind the administrative API to a public interface by default.
- Never enable public remote inference without authentication and rate limits.

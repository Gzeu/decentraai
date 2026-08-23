# MultiversX Agent Identity — Devnet (verified-first phase)

Status: identity LINK + registration PREPARATION. No wallet created, no keys
in repo, no submission executed, no token. Branch `feat/mvx-agent-identity`.

## Verified API contract (official skill.md)

| Endpoint | Status | Notes |
|---|---|---|
| `GET /agents?from&size` | ✅ verified | list, page ≤1000 |
| `GET /agents/:nonce` | ✅ verified | single agent |
| `GET /reputations/agents/:nonce` | ✅ verified | `{agentNonce, average, count}` |
| `POST /agents` → `{nonce, txHash}` | ✅ verified | fields: name, uri, publicKey (0x-hex), metadata[], services[] |
| anchoring endpoint | ❌ NOT verified | preparation shape only (`anchoring_payload`) |
| A2A / x402 / MPP / OASF / escrow | ❌ NOT verified | design nothing against them |

## Identity lifecycle

```text
DecentraAI node boots
  └─ internal Identity = Ed25519 keypair + PeerId   (authoritative locally)
        │ public_key().to_bytes()  (32B)
        ▼
local_public_key_hex() → "0x…"                     (MX wire format)
        │ operator hosts manifest + funds devnet wallet
        ▼
POST /agents  (operator-executed; body validated offline by RegistrationBody)
        ▼
MxAgentRecord on devnet  ──verify_link()──▶ byte-equality with local key
```

`verify_link()` is the whole mapping: decoded registered key == local
32-byte public key. No third identifier exists.

## Key separation

- **Node/agent Ed25519 key** — signs DecentraAI receipts and the MX agent
  authentication. Lives in the node's data dir (0600), never in git/memory.
- **Funding wallet key** — operator-held secret manager. NEVER in this
  repository. Needed only at registration and for any future tx.
- Rotation: register a new MX record with the new public key; old soulbound
  record remains as history (non-transferable by design).

## Security model

- This crate never signs, never submits, never holds keys.
- `RegistrationBody::validate()` enforces hosting scheme (ipfs://|https://),
  key format, name bounds — malformed submissions are impossible to build
  through the typed path.
- Manifest protocols are a CLOSED set (`ACP/x402/UCP/MCP`) until standards
  pages are officially verified; unknown protocol names are rejected.
- Governance transition authority stays in DecentraAI's deterministic layer.

## Devnet setup (operator checklist)

1. Create devnet wallet via official tooling; fund from faucet.
2. Generate agent Ed25519 keypair (or reuse node identity public key).
3. Build manifest via `AgentManifest`, host on IPFS/HTTPS.
4. Submit `RegistrationBody::json()` to `POST /agents` yourself.
5. Verify: `MxDevnetClient.get_agent(nonce)` + `verify_link(local_key, record)`.

## What remains unverified / open

Anchoring endpoint · A2A · x402 · MPP · OASF · escrow contracts · mainnet
("coming soon"). Each blocks its own phase until independently verified.

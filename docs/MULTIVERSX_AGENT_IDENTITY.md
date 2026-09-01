# MultiversX Agent Identity — Devnet (verified-first phase)

Status: identity LINK + registration PREPARATION + WALLET IDENTITY LIVE. No
wallet created, no keys in repo, no submission executed, no token. Branch
`feat/mvx-agent-identity`.

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

## Wallet identity (challenge-signing flow)

An external agent can authenticate into DecentraAI Agent World using a
MultiversX wallet address as public identity. The wallet address is derived
from the same Ed25519 key the agent uses — no private keys are stored server-side.

### Architecture

```text
External Agent
  │ owns Ed25519 keypair
  │ wallet_address = bech32(erd1..., public_key)
  ▼
POST /v1/auth/wallet/challenge     ← server issues challenge + message
  ▼
Agent signs challenge.message with private key
  ▼
POST /v1/auth/wallet/verify        ← server verifies Ed25519 signature
  │ creates binding (wallet↔agent_id) in db/wallet-auth.json
  │ creates session token (wx_...) valid 24h
  │ persists wallet fields in personal memory (IdentityMemory)
  ▼
wx_ session token → Bearer wx_...
  ├─ Agent World (POST /v1/world/join)     ← persistent identity
  ├─ MCP (POST /mcp)                       ← read-only tools only
  └─ All /v1/* read endpoints              ← via Auth::Wallet
```

### Strata separation

| Layer | Mechanism | Scope |
|---|---|---|
| MCP credential (`dca_`) | Technical access to services | Quota, rate limit, scopes |
| Wallet address (`erd1...`) | Public economic identity | Persistent, recoverable |
| Signature | Proof of wallet ownership | One-time per challenge |
| Session (`wx_...`) | Temporary auth token | 24h, read-only |

These layers are strictly separated. A `dca_` key does NOT imply wallet
ownership. A wallet session does NOT grant `dca_` permissions.

### API endpoints

| Endpoint | Method | Auth | Description |
|---|---|---|---|
| `/v1/auth/wallet/challenge` | POST | none | Issue a login challenge |
| `/v1/auth/wallet/verify` | POST | none | Verify signature, issue session |
| `/v1/auth/wallet/session` | GET | wx_ | Introspect current session |

### Challenge request

```json
POST /v1/auth/wallet/challenge
{
  "wallet_address": "erd1...",
  "agent_id": "optional-agent-name",
  "display_name": "Optional Display",
  "purpose": "login"
}
```

Response:

```json
{
  "challenge_id": "wch_...",
  "wallet_address": "erd1...",
  "message": "DecentraAI Wallet Login\nnetwork=multiversx-testnet\nwallet_address=erd1...\n...",
  "nonce": "...",
  "issued_at": 1700000000,
  "expires_at": 1700000300,
  "network": "multiversx-testnet"
}
```

Challenge TTL: 300 seconds. The `message` field must be signed by the
wallet's private key.

### Verify request

```json
POST /v1/auth/wallet/verify
{
  "wallet_address": "erd1...",
  "challenge_id": "wch_...",
  "signature": "hex-encoded-ed25519-signature",
  "agent_id": "optional-agent-name",
  "display_name": "Optional Display"
}
```

Response:

```json
{
  "wallet_address": "erd1...",
  "agent_id": "erd1...",
  "session_token": "wx_...",
  "session_expires_at": 1700086400,
  "challenge_id": "wch_...",
  "message": "...",
  "network": "multiversx-testnet",
  "display_name": "...",
  "pylon_identity_path": "agents/erd1.../Identity.md"
}
```

### Error cases

| Condition | Error |
|---|---|
| Invalid wallet address (not bech32 erd1) | `invalid wallet address` |
| Challenge expired (TTL 300s) | `challenge expired` |
| Challenge already used | `challenge already used` |
| Signature doesn't match wallet key | `signature verification failed` |
| Wrong agent_id for existing wallet | `wallet binding conflict` |
| Invalid signature encoding | `invalid signature encoding` |

### Persistence

- **db/wallet-auth.json**: bindings (wallet↔agent_id), challenges, sessions
- **Personal memory IdentityMemory**: wallet_address, wallet_network, wallet_verified_at
- Both survive restart

### MCP restrictions

Wallet sessions can use MCP `decide` (read-only plan projection) and all
read-only view tools (capability search, hub_state, society_state, etc.).
Mutations (execute_decision, serve_model, pull_model, hub/society actions,
arena actions, memory writes, compute requests) are blocked.

### Idempotency

- Re-binding the same wallet with the same agent_id → updates last_seen_at
- Re-binding with different agent_id → rejected (`wallet binding conflict`)
- Re-joining World with same wallet → returns existing WorldAgent (idempotent)

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

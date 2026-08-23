# MultiversX Agent Integration — verified mapping + Devnet adapter

Status: RESEARCH + READ-ONLY ADAPTER. No mainnet, no token, no contracts,
no wallets created, no private keys. Branch `feat/multiversx-devnet-adapter`.

## Source verification discipline

Facts below marked ✅ were verified from the official
`https://agents.multiversx.com/skill.md` (fetched 2026-08-23).
Items marked ⚠️ were CLAIMED by a third-party summary and are NOT yet
verified against official pages — design nothing against them until checked:
A2A discovery · x402 payments · MPP voucher sessions · OASF skill taxonomy ·
escrow contract details · dedicated MCP server.

## What MX-8004 verifiably offers today (devnet)

| Fact | Source |
|---|---|
| API base `https://devnet-mx8004-api.multiversx.com` | ✅ skill.md |
| `GET /agents?from&size` list | ✅ |
| `GET /agents/:nonce` single agent | ✅ |
| `GET /reputations/agents/:nonce` → `{agentNonce, average, count}` | ✅ |
| Identity = soulbound NFT; agent Ed25519 key SEPARATE from wallet | ✅ |
| Registration `POST /agents` needs wallet + hosted (IPFS/HTTPS) manifest | ✅ |
| Mainnet: "Coming soon" | ✅ |

## Ownership split (do not duplicate)

| Concern | DecentraAI owns | MultiversX owns |
|---|---|---|
| Internal identity/RBAC | ✅ (identity crate, scopes) | external on-chain identity only |
| Economic ledger authority | ✅ CompensationLedger + CU v2 engine | mirrors/anchors only |
| Evidence authoring | ✅ EvidenceChain + SignedComputeReceipt | validation/anchoring surface |
| Reputation (deterministic) | ✅ internal reputation store | registry average/count as EXTERNAL reference |
| Settlement execution | adapter seam (`BlockchainAdapter`) | eventual tx inclusion |
| Capability taxonomy | ✅ hub CapabilityKind (26 kinds) | OASF mapping table, later |

Model: **internal identity + external MX-8004 identity** — linked via the
agent's registered Ed25519 public key, which is the SAME primitive our
receipts already use. No second internal identity system.

## Delivered now

`economy::multiversx_devnet`:
- `MxDevnetClient` over injectable `MxHttp` transport (offline tests):
  `list_agents(from,size)` · `get_agent(nonce)` · `reputation(nonce)`
  — lenient parsing (unknown fields → None, never invented).
- `MxDevnetSettlementAdapter` implements `BlockchainAdapter` in an explicit
  READ-ONLY posture: writes are refused with the reason (settlement needs
  wallet signing = Phase 7 `TransactionSigner`, keys live outside the repo).
- Deps added to workspace reqwest features: `blocking`.

## Integration boundaries & risks

1. Devnet resets/instability — treat every response as advisory cache.
2. API shape drift — lenient parsing degrades gracefully; add conformance
   test against live devnet before any write path lands.
3. Manifest hosting dependency (IPFS pinning) at registration time.
4. Key custody: registration binds a wallet we would need funded — that is
   an OPERATOR action with keys in a secret manager, never in-repo.
5. Unverified standards (⚠️ above): A2A/x402/MPP/OASF designs wait for
   official page verification.

## Future mainnet path (gated, in order)

1. Verify ⚠️ items officially.
2. Implement `TransactionSigner` backed by an operator-held secret manager;
   enable registration of ONE governor node on devnet.
3. Anchor `EconomicEvidence.evidence_hash` per epoch once an anchoring
   endpoint/contract exists; receipts stay the source of truth.
4. Mainnet only after devnet soak + operator sign-off + legal review of any
   token question (explicitly out of scope here).

# OPERATOR PROCEDURE — First DecentraAI Governor registration on MX-8004 Devnet

> Executed BY YOU (operator) with YOUR wallet. DecentraAI code prepares
> everything and verifies afterwards; it never holds keys or submits.

## 0. Generate the payload (offline, this repo)

```bash
AGENT_KEY=$(openssl genpkey -algorithm ED25519 2>/dev/null && \
  echo "see runbook note: agent key = separate Ed25519 keypair")

cargo run -p decentraai-economy --example register_prep -- \
  --name "DecentraGovernor" \
  --uri "ipfs://<your-manifest-hash>" \
  --key "0x<64-hex-chars-of-agent-public-key>" \
  --sender "erd1<your-devnet-wallet-address>" \
  --gas 30000000 | tee registration-prep.json
```

`data_field` from the JSON is your final transaction data.

## 1. Devnet wallet requirements
- Address `erd1…` + its secret key — generated with official tooling
  (`mxjs-wallet new` or devnet web wallet). Key NEVER enters this repo.
- This wallet becomes the NFT owner recorded by the registry.

## 2. Funding requirement
- Devnet EGLD from the official faucet (web wallet faucet or documented
  devnet faucet endpoint). Registration itself needs value=0 but gas is
  burned; keep ≥0.05 devnet-EGLD headroom.

## 3. register_agent payload
Produced above in `data_field`. Format (source S1/S2):
`register_agent@nameHex@uriHex@publicKeyHex`

## 4. Transaction fields you must provide
| Field | Where from |
|---|---|
| receiver | **`erd1qqqqqqqqqqqqqpgqzcufga3vm5r44xe3ukzyl4dmhpsvalrkkgjqeyu68x`** — VERIFIED Identity Registry (see MULTIVERSX_DEVNET_ADDRESSES.md; re-verify via step 8 anyway) |
| sender | your wallet address |
| value | `0` |
| data | `registration-prep.json → data_field` |
| nonce | current account nonce |
| gasLimit | start 30,000,000; raise if explorer reports insufficient |
| chainId | `D` |

## 5. Sign + submit manually
Use whichever official tool you prefer, e.g. sdk-wallet (TypeScript):
sign the transaction with YOUR wallet PEM/keystore, then
`POST https://devnet-gateway.multiversx.com/transaction/send`.
DecentraAI does not sign or submit.

## 6. Inspect the result
```bash
curl https://devnet-api.multiversx.com/transactions/<txHash>
```
Wait for `status: "success"`.

## 7. Extract receiver
From the confirmed tx JSON: `.receiver`.

## 8. Verify it is the MX-8004 Identity Registry
- tx logs contain an `agentRegistered` event (S1 §1.4);
- `GET https://devnet-mx8004-api.multiversx.com/agents/<nonce>` (or explorer)
  shows YOUR name/uri/publicKey;
- our `verify_link()` passes against that publicKey.

## 9. Record everything
tx hash · receiver · network (`devnet`) · explorer URL · source
("operator-executed registration") — paste into
`docs/MULTIVERSX_DEVNET_ADDRESSES.md`.

## 10. Update addresses doc
Add under Identity Registry with all five fields from step 9.
Validation/Reputation registries: repeat discovery by following
cross-contract interactions of a validation job, same recording rules.

## Final checklist

[ ] Devnet wallet created (key stays with operator)
[ ] Devnet funds claimed
[ ] Governor manifest hosted (IPFS/HTTPS)
[ ] Agent Ed25519 public key generated (separate from wallet key)
[ ] register_agent payload generated (this repo, offline)
[ ] Operator signs (own tooling)
[ ] Transaction submitted
[ ] Transaction confirmed (status success)
[ ] Receiver extracted from tx
[ ] Registry contract verified (agentRegistered + agent visible)
[ ] Address committed to docs/MULTIVERSX_DEVNET_ADDRESSES.md
[ ] Only then enable write path

No mainnet. No production funds. No token. No automatic signing.

# VPS OPERATIONS RUNBOOK — DecentraAI node

> Consolidated operator guide for the always-online VPS node. Deep-dive docs
> are linked, not duplicated: `VPS_NODE_PROFILE.md` (hardware/selection),
> `deploy/decentraai-vps.env.example` (timeout overrides),
> `NODE_UPGRADE.md` (upgrade flow), `docs/MULTIVERSX_AGENT_REGISTRATION_PROCEDURE.md` (first on-chain write).

## 1. Install / update

```bash
git pull --rebase origin main
cargo build --release -p decentraai-cli   # or scripts/install-app.sh
scripts/upgrade-remote-node.sh            # remote node flow (existing)
sudo systemctl restart decentraai-node && systemctl status decentraai-node
```

CI must be green on main before deploying (`gh run list --branch main`).

## 2. Data files to BACK UP (irreplaceable state)

| File | Contains | Loss impact |
|---|---|---|
| `data/db/agent_memory.sqlite` | collective memory scopes+entries+observations | knowledge loss |
| `data/db/model_intel.json` | colony governance stages | models reset to experimental |
| `data/identity*` | node Ed25519 identity ↔ PeerId | peer identity change |
| `data/runtime/api.token`, `db/tokens.json`, `db/consumer_keys.json` | credentials/tiers | access re-provisioning |
| `logs/` | audit trail | history loss |

## 3. Environment overrides (systemd drop-in)

Base template: `deploy/decentraai-vps.env.example`. New flags from the
Model Intelligence phases:

```ini
Environment=DECENTRAAI_MEMORY_PROPAGATE=1        # opt-in verified-knowledge push
Environment=DECENTRAAI_MEMORY_PROPAGATE_SECS=120 # cycle interval (min 10)
```

Everything else (timeouts, llama-server path) stays per the example file.

## 4. Services exposed by THIS node (operator endpoints)

All operator+ (master token): `/v1/memory/search|transition|index|
training-candidates|sync-to` · `/v1/models/intel|route|governance` ·
`/v1/bench/shadow` · dashboard views **Memory** & **Model Colony**.
Read-only public-ish: `/status`, `/v1/models`.

## 5. MultiversX Devnet (identity phase)

State: addresses VERIFIED (see `MULTIVERSX_DEVNET_ADDRESSES.md`);
registration is PREPARED but executed only by you:

```bash
# 1. host manifest (IPFS or HTTPS) built via AgentManifest::manifest_json()
# 2. generate payload (offline, keyless):
cargo run -p decentraai-economy --example register_prep -- \
  --name "DecentraGovernor" \
  --uri "ipfs://<manifest>" \
  --key "0x<agent ed25519 pubkey>" \
  --sender "erd1<your devnet wallet>" \
  --gas 30000000
# 3. sign+submit with YOUR wallet tooling → txHash back to me for verification
```

Receiver = Identity Registry (verified constant). Full runbook:
`MULTIVERSX_AGENT_REGISTRATION_PROCEDURE.md`. Wallet keys live in your
secret manager — never in repo/env/Obsidian.

## 6. Health checks after every deploy

```bash
curl -s localhost:$PORT/status | head -c 300          # model loaded?
curl -s localhost:$PORT/v1/memory -H "auth…" | jq '.attached'
journalctl --user -u decentraai-node -n 50             # errors?
bash scripts/production-audit.sh                       # existing audit script
```

## 7. Known gaps (honest)

- LAN two-node validation still pending real hardware round.
- Semantic indexing needs an embeddings backend on the VPS (small model ok).
- Announce-side `lan_discovery` gating = future p2p behaviour rework.
EOF
echo done
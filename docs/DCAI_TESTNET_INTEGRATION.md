# DCAI Testnet Integration

This branch is the integration seam for the next DecentraAI economic layer.

The World economy remains the source of truth for Cr.

## Issued token (2026-09-03, MultiversX testnet)

- Token identifier: **DCAI-51cb9b** (name "DecentraAI", ticker "DCAI", decimals 18)
- Initial supply: **zero** (no emission invented; Cr stays the World currency)
- Issuance tx: `bf876f93d717f3c6ee2bb2da6569ce60b8d646fbb2f2bdb77d3d0dd234b9dc70` (status success)
  - Explorer: https://testnet-explorer.multiversx.com/transactions/bf876f93d717f3c6ee2bb2da6569ce60b8d646fbb2f2bdb77d3d0dd234b9dc70
- Token id extracted from SCR `ESDTSetBurnRoleForAll@444341492d353163623962`
- Properties: canChangeOwner / canUpgrade / canAddSpecialRoles = true;
  canMint / canBurn / canFreeze / canWipe / canPause = false
- Implementation: `crates/economy/src/dcai_esdt.rs` (pure builder, no network/keys),
  CLI `decentraai dcai issue`, issuance cost 0.05 EGLD, gas 60_000_000,
  signed-envelope data field is base64.

## Live wiring (verified 2026-09-04 on VPS 169.58.213.145)

- `node.yaml` → `dcai: { token_identifier: DCAI-51cb9b, chain_id: "T" }`
- `/status` → `dcai: { configured: true, token_identifier: "DCAI-51cb9b", chain_id: "T" }`
- Root cause fixed in `cfc40cc`: `serve_start` path never called `attach_dcai`
  (only `node_start` did) — the VPS runtime path now attaches the seam.
- Binary on VPS SHA256 matches local `target/release/decentraai`
  (package `decentraai-cli`); deploy = scp, never git pull on VPS.
- Long-run observation gate still active: World tick is manual-only
  (`POST /v1/world/tick`), driven by `econ_observer.py` (nohup, 300s cycle).
  Post-unfreeze trend at tick 218: treasury minted 2660 / burned 130,
  61 quests completed, 168+ proofs with real testnet tx hashes.

## Direction

- keep Cr as the internal World currency;
- integrate an externally issued MultiversX testnet DCAI token through the existing `DcaiSection` configuration seam;
- connect DCAI only to already-defined economic flows such as stakes, provider bonds, and verified compute rewards;
- reuse M18 contracts, escrow, trust, EvidenceChain, Hub, and existing settlement infrastructure;
- do not invent tokenomics, emission rates, supply numbers, or a second economy;
- keep shadow mode when no token identifier is configured.

The implementation should proceed from the existing architecture and current economic evidence rather than introducing parallel systems.

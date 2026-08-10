# DecentraAI MVP Roadmap

## 1. Foundation (done)
- [x] Workspace, CI, and YAML config loader
- [x] `decentraai init` bootstrap
- [x] System probe + admission checks (`decentraai doctor`)
- [x] Model registry scan/list commands

## 2. Identity and Networking (done)
- [x] Ed25519 identity management with secure persistence
- [x] Message schema and canonical signing
- [x] Model manifest generation with Merkle root
- [x] libp2p transport with Noise + mDNS
- [x] Chunk transfer engine with per-chunk verification and resume
- [x] End-to-end test: two nodes exchange a real model

## 3. Runtime (done)
- [x] M4a: llama-server process manager crate (`decentraai-runtime`)
- [x] M4b: inference admission gate + `decentraai serve start`
- [x] M4c: OpenAI-compatible API endpoint with Bearer auth and idle unload
- [x] Fixed configurable API port + friendly root info page

## 4. Swarm Intelligence (done)
- [x] Reputation and peer scoring (M5a: bans, decay, persistence)
- [x] Manifest announcements + registry-backed serving (M5b)
- [x] Deterministic multi-provider scheduler (M5c: ranked waves + fallback)

## 5. Hardening (done)
- [x] Quarantine workflow for corrupted artifacts (metadata + reason)
- [x] RAM admission fix (reject below the configured reserve)
- [x] Security audit log (bans, admission rejections, verification failures)
- [x] M8: packaging — `scripts/install.sh` + `docs/deployment.md`
  (systemd unit, firewall, security checklist, troubleshooting)

## 6. Sharing and UX (done)
- [x] M7a: peer catalog + `decentraai pull` (share models with one command)
- [x] M7b: web dashboard on the API port
- [x] M7c: dashboard v2 — real inference metrics (tokens, tok/s, recent
  calls, uptime, RAM/GPU), self-poll fix (watching the page no longer
  inflates the counter or blocks idle unload)

## 7. Subscriptions: free, tiered by contribution (in progress)
- [x] P1: token registry (`db/tokens.json`, hashed) + `decentraai token`
  CLI + tiered auth in the proxy (per-tier model allowlist, sliding-window
  rate limit, usage counters, audits)
- [ ] P2: chat UI in the dashboard (model selector filtered by tier)
- [ ] P3: admin dashboard (create/revoke tokens, usage per token)
- [ ] P4: contribution-based tier suggestions from catalog + reputation
- [ ] P5: invites (`decentraai join <invite>`)

## 8. Operations and scale (in progress)
- [x] Q1: generation defaults (sampling + system prompt merged into
  requests), interactive model picker with memory-fit verdicts,
  dashboard lists every indexed model
- [x] Q2: fair FIFO queue for inference requests — one request at a
  time reaches the backend with full resources, 503/504 on full/timeout,
  Queue card on the dashboard shows serving + waiting live
- [ ] Q3: remote backend (`serve start --backend http://host:port`) —
  a weaker station keeps auth/tiers/queue while a stronger machine runs
  the model
- [ ] Q4: onboarding wizard (`decentraai setup`) writing a validated
  config on first run

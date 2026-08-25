# DecentraAI — Deployment & Operations

A production 3-node fabric: **VPS (orchestrator)** + **Desktop (worker)** + **Laptop (worker)**. One supervisor per node — never screen + systemd together (duplicate listeners on 32937 destabilise P2P, observed in practice).

## Topology

| Node | Hostname | Address | Role | Served model | Embeddings backend |
|------|----------|---------|------|--------------|--------------------|
| VPS | decentraai-vps | 169.58.213.145:32937 | Orchestrator + worker | Qwen3-1.7B | :7777 nomic |
| Desktop | i7 | 192.168.1.138:32937 | Worker | Qwen3-1.7B | :7777 nomic |
| Laptop | i5 | 192.168.1.132:32937 | Worker (+systemd) | qwen2.5-3b | :7777 nomic |

## Prerequisites
- Rust toolchain (edition compatible with workspace), git clone of `Gzeu/decentraai`.
- Models in `~/.decentraai/models/`: the chat GGUF (per config) + `nomic-embed-text-v1.5.Q4_K_M.gguf` (embeddings backend).
- llama.cpp built (`llama-server` in a common location or configured path).

## Build
```bash
cargo build --release -p decentraai-cli
# binary: target/release/decentraai
```

## Start a node (screen — recommended for servers)
```bash
cd ~/decentraai
screen -dmS dca-node bash -c "exec ./target/release/decentraai node --config ~/.decentraai/node.yaml > /tmp/dn.log 2>&1"
```
Restart on rebuild: kill the OLD supervisor, wait, start a NEW one AFTER the build finishes (a stale binary is a common silent-failure cause).

## Start a node (systemd user service — recommended for a single always-on machine, e.g. Laptop)
```bash
mkdir -p ~/.config/systemd/user
# deploy/decentraai-node.service (see example below)
systemctl --user daemon-reload
systemctl --user enable --now decentraai-node
sudo loginctl enable-linger $USER   # survive reboot without login
```
**Rule:** exactly ONE mechanism per node.

## Required config highlights (`~/.decentraai/node.yaml`)
```yaml
inference:
  allow_remote_inference: true          # this node accepts DFCP assignments
  embeddings_backend_url: "http://127.0.0.1:7777"
sharing:
  assist:
    enabled: true                       # worker side of Sharing is Caring
autonomous_assist:                      # optional M15 loop
  enabled: true
  tick_seconds: 5
  cooldown_seconds: 45
fabric_intelligence:
  enabled: true                         # unlocks /v1/governor/* and /v1/intel/*
```

## Embeddings backend (each node)
```bash
nohup ~/llama.cpp/build/bin/llama-server \
  --model ~/.decentraai/models/nomic-embed-text-v1.5.Q4_K_M.gguf \
  --host 127.0.0.1 --port 7777 --embedding > /tmp/embed.log 2>&1 &
```

## Health checks
```bash
curl -s http://127.0.0.1:8080/status | jq .model_loaded      # engine up
curl -s http://127.0.0.1:8080/fabric                          # live fabric dashboard
curl -s http://127.0.0.1:8080/flow                            # animated pipeline
curl -s http://127.0.0.1:8080/v1/peers -H "Authorization: Bearer $T"
curl -s http://127.0.0.1:7777/v1/embeddings -d '{"input":"x"}' -H 'Content-Type: application/json'
```

## BYOA setup (scoped agent access)
```bash
decentraai consumer-key create --account my-agent --quota-ceiling 5000 --scopes inference
curl -X POST http://127.0.0.1:8080/api/admin/quota/grant -H "Authorization: Bearer $MASTER" \
  -H "Content-Type: application/json" -d '{"account":"my-agent","amount":50000}'
# agent drives fabric:
curl -X POST http://127.0.0.1:8080/v1/governor/execute -H "Authorization: Bearer dca_…" \
  -H "Content-Type: application/json" -d '{"task_id":"j1","task_kind":"summarize","instruction":"…","content":"…"}'
```

## Troubleshooting
| Symptom | Cause | Fix |
|---|---|---|
| Peer connects then drops every ~2 min | idle timeout + ping failures | raise idle_connection_timeout; one node process per host |
| Two nodes on 32937 | duplicate supervisors (screen + systemd) | kill duplicates; keep ONE |
| `no spendable consumer quota` | account unfunded | `POST /api/admin/quota/grant` |
| Embeddings empty result | Qwen3 spent tokens on reasoning | larger max_tokens, or route reduce to a non-reasoning worker |
| Node dies silently after deploy | old binary running | restart supervisor AFTER build; check binary timestamp |
| Desktop DOWN after restart | dead screens accumulating | `screen -wipe`; start exactly one |

## Example systemd unit
```ini
[Unit]
Description=DecentraAI node
After=network-online.target

[Service]
Type=exec
ExecStart=/home/USER/decentraai/target/release/decentraai node --config /home/USER/.decentraai/node.yaml
WorkingDirectory=/home/USER/decentraai
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
```
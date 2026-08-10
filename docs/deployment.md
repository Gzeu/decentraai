# DecentraAI deployment guide

This guide takes a fresh Linux machine to a running DecentraAI node:
CLI installed, models indexed, swarm serving, inference with the
OpenAI-compatible API and the web dashboard.

## 1. Prerequisites

| Requirement | Why | Install hint |
|---|---|---|
| Rust 1.85+ | builds the node | `curl https://sh.rustup.rs -sSf \| sh` (the installer offers this) |
| git, cmake, a C++ compiler | builds llama.cpp | `sudo apt install git cmake build-essential` |
| GGUF model files | anything else needs them | e.g. `~/models/*.gguf` |
| NVIDIA driver + `nvidia-smi` | optional, GPU metrics and admission | distro packages |

## 2. Quick install

```bash
git clone https://github.com/Gzeu/decentraai && cd decentraai
bash scripts/install.sh              # add --no-llama to skip llama.cpp
export DECENTRAAI_LLAMA_SERVER=$HOME/llama.cpp/build/bin/llama-server
```

The script: verifies the Rust toolchain (installs rustup when missing),
`cargo install`s the `decentraai` binary into `~/.cargo/bin`, clones and
builds llama.cpp's `llama-server`, then runs `decentraai init` to create
the data directory and the Ed25519 identity.

## 3. Data directory layout

Everything lives under `node.data_dir` (default `~/.decentraai`):

```
~/.decentraai/
├── identity/key.pem      # Ed25519 node key, mode 0600 — never share
├── db/registry.json      # scanned local models
├── db/reputation.json    # peer scores and bans
├── logs/audit.jsonl      # security events (append-only)
├── models/               # downloaded artifacts land here
├── staging/              # in-progress downloads (.part + .done bitmap)
├── quarantine/           # corrupted artifacts + metadata JSON
└── runtime/api.token     # API Bearer token, mode 0600
```

Back up `identity/key.pem`; everything else is re-derivable.

## 4. Configuration

Start from `configs/node.example.yaml`, copy it, and validate:

```bash
decentraai config validate --file your-node.yaml
decentraai doctor --config your-node.yaml
```

The defaults are safe: inference `auto`, RAM/VRAM reserves enforced,
GPU temperature stop, Bearer auth on, API on loopback port 8080.

## 5. Ports and firewall

| Port | Exposure | Purpose |
|---|---|---|
| random TCP (printed by `swarm start`) | LAN | libp2p swarm traffic |
| 8080 (`inference.api_port`) | **loopback only** | OpenAI API + dashboard |

The API binds to `inference.bind_address`, which the config validation
forces to loopback. The dashboard deliberately exposes no secrets, but
every `/v1/*` endpoint requires the Bearer token. mDNS discovery needs
UDP 5353 allowed on the LAN.

## 6. Running as a service (systemd)

```ini
# /etc/systemd/system/decentraai-swarm@.service
[Unit]
Description=DecentraAI swarm node
After=network-online.target

[Service]
Type=simple
User=%i
Environment=DECENTRAAI_LLAMA_SERVER=/home/%i/llama.cpp/build/bin/llama-server
ExecStart=/home/%i/.cargo/bin/decentraai swarm start --config /home/%i/decentraai/configs/node.example.yaml
Restart=on-failure
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/home/%i/.decentraai

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable --now decentraai-swarm@youruser
journalctl -u decentraai-swarm@youruser -f
```

Run inference the same way with a second unit whose `ExecStart` is
`.../decentraai serve start --model <name>.gguf --config ...`. The node
shuts down cleanly on SIGINT/SIGTERM (Ctrl+C or `systemctl stop`).

## 7. Security checklist

- [ ] `identity/key.pem` and `runtime/api.token` are mode 0600 (default)
- [ ] API stays on loopback; use SSH port-forward for remote access:
      `ssh -L 8080:127.0.0.1:8080 user@node`
- [ ] Review `logs/audit.jsonl` and the dashboard's security events
      after incidents; check `quarantine/` before deleting anything
- [ ] Bans are temporary by design (`security.ban_duration_minutes`);
      inspect `db/reputation.json` before unbanning early (delete the file)
- [ ] Never commit the data dir; only the repository itself goes to Git

## 8. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| `llama-server not found on PATH` | set `DECENTRAAI_LLAMA_SERVER` or pass `--binary` |
| `inference admission rejected` | low free RAM / hot or missing GPU; see `decentraai doctor` |
| `peer is banned until ...` | it served corrupted chunks; wait out the ban or inspect quarantine metadata |
| pull lists an empty catalog | the peer's registry is empty: `decentraai registry scan` on that machine |
| dashboard shows `○ unloaded` | idle timeout fired; restart `decentraai serve start` |
| `bash: PORT: No such file or directory` | you kept the literal `<PORT>` placeholder in a multiaddr; copy the real address |

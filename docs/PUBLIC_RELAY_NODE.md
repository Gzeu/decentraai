# Public Relay & Bootstrap Node

How to stand up a small, always-on **public** DecentraAI node that lets fabric
members connect across subnets and over the internet.

## Why a public node is required

DecentraAI's transport (libp2p TCP + Noise + Yamux + mDNS) is LAN-first. For
two nodes on **different subnets** or **behind NAT / on the public internet**,
they need a rendezvous they can both reach. A public bootstrap node solves this:

- **Relay** — a node behind NAT connects to the public node; the public node
  relays `/p2p-circuit` traffic so two NAT'd nodes can exchange DecentraAI
  messages through it (even without port-forwarding).
- **DHT** — the public node seeds a Kademlia DHT; members discover each other
  by PeerId even when they don't know each other's current address.
- **Identify** — the public node tells each member its observed external
  address, so direct (hole-punched) dials can succeed where possible.

Public IPFS/libp2p bootstrap nodes were evaluated (e.g. `104.131.131.82:4001`)
and are **not** a drop-in: their transport (QUIC/WebTransport + IPFS-specific
Noise) does not complete the handshake with DecentraAI's plain TCP+Noise+Yamux.
The reliable path is **your own public DecentraAI node**.

## Requirements

- A small VPS (1 vCPU / 1 GiB RAM is plenty) with a public IPv4 address.
- Port **4001/tcp** reachable from the internet (open in the firewall).
- Optional: 8080/tcp for the dashboard (bind loopback only if you don't want it
  public).
- Docker, or a Rust toolchain to build.

## 1. Provision the VPS

```bash
# on the VPS
sudo apt update && sudo apt install -y docker.io docker-compose
# allow libp2p + optional dashboard
sudo ufw allow 4001/tcp
# (optional) sudo ufw allow 8080/tcp
```

## 2. Configure the node

```yaml
# /root/.decentraai/node.yaml  (create via: decentraai setup --no-llama, then edit)
network:
  private_swarm: true
  lan_discovery: false      # no LAN peers on a VPS
  dht_enabled: true         # run Kademlia DHT
  relay_enabled: true       # offer relay server + dcutr
  bootstrap_peers: []       # empty on the seed node itself
  max_connections: 512
  max_message_bytes: 1048576
inference:
  api_port: 8080
  bind_address: 127.0.0.1  # keep the API loopback-only (never public)
```

> The relay server is served by the node that has `relay_enabled: true` and a
> public address. On the VPS, port 4001 is the libp2p listen port (see below).

## 3. Run it

Using the existing container image:

```bash
# on the VPS, from a checkout of the repo
docker compose -f deploy/docker-compose.yml up -d
```

Or as a systemd service with the release binary:

```bash
cargo build --release --bin decentraai
cat > /etc/systemd/system/decentraai-relay.service <<'EOF'
[Unit]
Description=DecentraAI public relay/bootstrap node
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/usr/local/bin/decentraai node --config /root/.decentraai/node.yaml
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF
systemctl enable --now decentraai-relay
```

## 4. Advertise its bootstrap address to every fabric member

The VPS listens on `0.0.0.0:4001`. Get its PeerId:

```bash
# on the VPS
curl -s http://127.0.0.1:8080/status | grep -o '"peer_id":"[^"]*"'
# or from the node log: "P2P PeerId: 12D3KooW..."
```

Then on every member node, add to `~/.decentraai/node.yaml`:

```yaml
network:
  dht_enabled: true
  relay_enabled: true
  bootstrap_peers:
    - "/ip4/<VPS_IP>/tcp/4001/p2p/<VPS_PEERID>"
```

and restart the member node. Members will dial the public relay, exchange
external addresses via identify, discover each other over DHT, and route
through the relay when they cannot connect directly.

## 5. Verify

On any member node:

```bash
# after restart, the log should show a successful dial + connection:
journalctl -u decentraai-node | grep -E "dialing bootstrap|peer connected"
# and /v1/network should list the public peer among connected peers:
curl -s -H "Authorization: Bearer $(cat ~/.decentraai/runtime/api.token)" \
  http://127.0.0.1:8080/v1/network | python3 -m json.tool
```

## Security notes

- The **dashboard/API stays loopback-only** on every node (`bind_address:
  127.0.0.1` + `api_auth_required: true`). Only the libp2p port (4001) is public.
- The relay carries encrypted (Noise) /p2p-circuit traffic; peers authenticate
  by PeerId (identity). It does not decrypt member messages.
- Keep `private_swarm: true` and issue invites (trust) rather than opening the
  swarm to arbitrary peers.
- If you want the fabric to be private but still cross-subnet, put a **VPN**
  (Tailscale/WireGuard) between members instead of a public relay — that is
  simpler and fully private. The public relay is for open / internet reachability.

# Deployment: container (Docker) + systemd (native user service)

Two supported ways to run the universal DecentraAI node (LAN/P2P discovery +
model serving + dashboard, all one process):

1. **Native** — systemd user service (`decentraai-node.service`) or just
   `decentraai node`.
2. **Container** — `deploy/Dockerfile` + `deploy/docker-compose.yml`.

## Container (Docker)

```bash
docker compose -f deploy/docker-compose.yml config        # validate
docker compose -f deploy/docker-compose.yml up --build -d # build + run
open http://127.0.0.1:8080/
docker compose -f deploy/docker-compose.yml logs -f
docker compose -f deploy/docker-compose.yml down          # stop
docker compose -f deploy/docker-compose.yml down -v       # stop + drop volumes
```

What it does:

- Builds the workspace (`decentraai-cli` → the `decentraai` binary) with a
  multi-stage `rust:bookworm` → `debian:bookworm-slim` image.
- Runs one `decentraai node` (the universal process — every instance is both
  coordinator and worker; there is no separate worker/coordinator image).
- Binds the dashboard/API on `127.0.0.1:8080` only (never a public bind).
- Mounts `${DECENTRAAI_HOME:-$HOME/.decentraai}` read-write over
  `/root/.decentraai`, so identity (`identity/key.pem`), config (`node.yaml`)
  and `models/` persist and are reusable on the host. On a fresh mount the node
  auto-provisions identity + config on first start.
- Healthcheck hits the **real `/status`** endpoint (not a made-up `/ready`).

## Native (systemd user service)

See `decentraai-node.service` for install steps. `decentraai node` auto-provisions
identity + config if `~/.decentraai/node.yaml` is absent.

## Security checklist (applies to both)

- [x] API bound to loopback (config validation rejects public binds); the
      compose file maps `127.0.0.1:8080` only
- [x] no private key / bearer token baked into an image layer — mount state
      read-write from the host
- [x] no token on the command line — auth lives in `runtime/api.token` (0600)
- [x] health checks hit a real endpoint and leak no secrets
- [ ] image uses a pinned digest in production (currently `rust:bookworm` /
      `debian:bookworm-slim` tags)
- [ ] multi-node: run two `decentraai node` containers/instances on the same
      LAN and let mDNS discover each other automatically

## Honest note

`deploy/docker-compose.yml` and `deploy/Dockerfile` are current and match the
real CLI (`decentraai node --config`), the real `/status` endpoint, and the
universal-node architecture. The old `docker-compose.m10.yml` (which referenced
a nonexistent Dockerfile, the removed `decentraai worker` command, and a
`/ready` endpoint) was removed.
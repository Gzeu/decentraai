# M10 Deployment Guide

## Status

The M10 deployment files are a target topology and must be checked against the actual repository before use. A file referenced by Compose must exist, and a configuration key must be supported by the current Rust config loader.

Do not report deployment as working until these commands pass:

```bash
docker compose -f deploy/docker-compose.m10.yml config
cargo build --release --bin decentraai
```

## Local topology

- coordinator: control plane/API and P2P listener;
- worker: trusted inference node;
- llama: OpenAI-compatible llama-server backend.

The default CI path must use a deterministic HTTP backend fixture and must not download a large model. Real llama-server validation should be opt-in.

## Required preflight

1. Verify the CLI binary name with `cargo metadata`.
2. Verify whether `deploy/Dockerfile` exists; create it only after confirming the release binary/package.
3. Verify the actual node/worker subcommands and their arguments.
4. Verify the config loader's supported YAML structure.
5. Create coordinator and worker configs from supported keys only.
6. Mount identities and data directories with least privilege.
7. Never commit private keys, model files, bearer tokens or production endpoints.

## Environment contract

Use environment variables or secret mounts for:

- backend URL and optional backend token;
- node master/admin token;
- identity/key path;
- data directory;
- API/P2P bind addresses and ports;
- log level and redaction policy.

The example configuration is not proof that all keys are currently supported. The agent must update it after inspecting the actual config types and add a config parsing test.

## Health semantics

- `/health`: process is alive.
- `/ready`: identity, P2P and required backend are ready.
- model readiness: requested model is loaded and can answer a bounded probe.

A worker must not advertise itself as ready when only the process is alive.

## Security checklist

- [ ] no host-wide privileged container;
- [ ] no private key in image layer;
- [ ] no bearer token in command line;
- [ ] explicit network exposure for API/P2P only;
- [ ] backend is not publicly exposed by default;
- [ ] logs redact authorization and prompt content;
- [ ] image uses a pinned digest for production;
- [ ] health checks do not leak secrets.

## Operator test

After the agent implements the missing runtime pieces:

```bash
docker compose -f deploy/docker-compose.m10.yml config
docker compose -f deploy/docker-compose.m10.yml up --build -d
curl -fsS http://localhost:8080/health
curl -fsS http://localhost:8080/ready
# Pair/approve worker using the documented control-plane command.
# Submit one streamed request using the documented bearer token.
docker compose -f deploy/docker-compose.m10.yml down -v
```

The completion report must distinguish between `compose config` validity, service startup, backend readiness and successful distributed inference.

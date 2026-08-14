# DecentraAI universal node — container image.
#
# Builds the workspace and runs a single `decentraai node` (the universal
# process: LAN/P2P discovery + model serving + dashboard). Every instance is
# both coordinator and worker; there is no separate worker/coordinator image.
#
# Build:      docker compose -f deploy/docker-compose.yml up --build
# Run data:   mount a host directory with `~/.decentraai` (identity + config +
#             models) over /root/.decentraai, as the compose file does.

# ---- Builder ----
FROM rust:1.85-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# Build dependencies first for a warm layer.
RUN cargo fetch
RUN cargo build --release -p decentraai-cli

# ---- Runtime ----
FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/decentraai /usr/local/bin/decentraai

# Data/config/identity live under $HOME/.decentraai (matches the CLI default).
ENV HOME=/root
ENV DECENTRAAI_LOG=info
EXPOSE 8080

HEALTHCHECK --interval=10s --timeout=3s --retries=10 \
    CMD curl -fsS http://127.0.0.1:8080/status || exit 1

# The node auto-provisions identity + config on first start if node.yaml is
# absent; mount your ~/.decentraai over /root/.decentraai to persist it.
ENTRYPOINT ["decentraai", "node", "--config", "/root/.decentraai/node.yaml"]
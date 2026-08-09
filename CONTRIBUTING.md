# Contributing

## Local workflow

Install the Rust toolchain declared in `rust-toolchain.toml`, then run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p decentraai-cli -- config validate
cargo run -p decentraai-cli -- doctor
```

## Rules

- Keep commits focused and document protocol or security changes.
- Do not commit secrets, identities, model artifacts, caches, databases, or telemetry exports.
- Treat all peer-originated data as untrusted.
- Add regression tests for security bugs and data-corruption defects.
- Do not enable public DHT or remote inference without explicit architecture and threat-model updates.

# DecentraAI Inference Adapter

Provider boundary for OpenAI-compatible backends such as llama-server.

Status: WP-001 scaffold/integration-ready. Add this crate to the workspace, run fmt/check/test/clippy, add deterministic `/health` and `/v1/chat/completions` integration tests, then wire it into the real worker before `P2PNode::new()`.

Provider JSON must not leak into the P2P protocol or frontend. API keys, prompts and outputs must never be logged by this crate.

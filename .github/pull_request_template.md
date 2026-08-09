## Change summary

Describe the purpose and scope of this change.

## Validation
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] Documentation updated where relevant

## Security and privacy
- [ ] No secret, private key, token, model artifact, or local database is committed
- [ ] Untrusted network input has strict validation and size limits
- [ ] Resource, timeout, and error behavior are handled where applicable
- [ ] Prompt/output logging remains opt-in only

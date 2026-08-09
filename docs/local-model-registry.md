# Local model registry

The local model registry discovers model artifacts only from a caller-approved directory. It is designed for the M2 local-first workflow and does not enable remote inference or change the P2P protocol.

## Supported artifacts

The scanner recognizes files with these case-insensitive extensions: `.gguf` only. GGUF is the only format that decentraai-manifest can verify in v1, and other formats like `.pt`/`.pth` (pickle-based) conflict with the threat model's prohibition on unsafe deserialization.

## Safety rules

- The scan root must exist and be a directory.
- Every path is canonicalized before registration.
- Symbolic links are skipped; the scanner never traverses a symlinked directory or registers a symlinked file.
- A canonical file path must remain under the approved root.

## Registry behavior

Records are keyed by their path relative to the approved root, so scans are deterministic. Re-scanning an unchanged model updates the existing record rather than adding a duplicate. The stored metadata is the canonical path, byte size, and modification time.

## Integration status

The standalone scanner module is implemented in `crates/registry` (decentraai-registry crate). CLI command wiring is in `crates/node-cli/src/main.rs`. No remote model discovery or inference is included.

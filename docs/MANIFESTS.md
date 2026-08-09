# Verified GGUF manifests

The `decentraai-manifest` crate scans a local GGUF artifact only. It validates the `GGUF` magic bytes, reads the file in 4 MiB chunks, computes BLAKE3 hashes and a deterministic Merkle root, then can write a JSON manifest atomically.

Scanning does not load the model for inference, transfer it to a peer, seed it, or publish the manifest. Network publication is deferred to M3 after transfer protocol validation.

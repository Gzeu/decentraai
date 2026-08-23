# Cryptographic Evidence

Primitives are NOT invented here: Ed25519 via the existing audited
`signed_receipt` helpers (P13), BLAKE3 like every other fabric integrity
anchor.

## Chain

```text
execution facts
  → EconomicEvidence        (canonical compact-JSON bytes; µCU bound inside)
  → BLAKE3 hash             (settlement anchor)
  → Ed25519 signature       (over exact canonical bytes)
  → SignedEconomicEvidence
```

## Independent verification (5 steps, all mandatory)

1. envelope version matches;
2. Ed25519 signature validates over payload bytes;
3. BLAKE3(payload) == carried hash;
4. payload deserializes to EconomicEvidence;
5. recomputed CU under the claimed formula version == signed amount.

A correctly-signed receipt carrying a WRONG amount is still rejected
(`AmountMismatch`) — cryptography proves authorship, economics proves value.

## Separation of concerns

identity (verifying key) · authorization (caller's policy layer) ·
signature · evidence payload · economic accounting — five distinct layers;
no field or function conflates them.

Keys never enter this repository: signing takes caller-supplied key bytes;
production signers load from secret managers at call time.

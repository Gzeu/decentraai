# DCAI Testnet Integration

This branch is the integration seam for the next DecentraAI economic layer.

The World economy remains the source of truth for Cr. DCAI remains unissued until the long-run economic observation gate is satisfied.

## Direction

- keep Cr as the internal World currency;
- integrate an externally issued MultiversX testnet DCAI token through the existing `DcaiSection` configuration seam;
- connect DCAI only to already-defined economic flows such as stakes, provider bonds, and verified compute rewards;
- reuse M18 contracts, escrow, trust, EvidenceChain, Hub, and existing settlement infrastructure;
- do not invent tokenomics, emission rates, supply numbers, or a second economy;
- keep shadow mode when no token identifier is configured.

The implementation should proceed from the existing architecture and current economic evidence rather than introducing parallel systems.

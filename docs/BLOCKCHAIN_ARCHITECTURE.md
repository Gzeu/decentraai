# Blockchain Architecture (adapter interface only)

The core fabric runs with NO blockchain. Settlement is a replaceable trait:

```rust
trait BlockchainAdapter {
    fn name(&self) -> &'static str;
    fn submit_settlement(&self, record: &SettlementRecord) -> Result<SettlementReceipt, SettlementError>;
}
```

Today: `LocalTestAdapter` (deterministic sequential refs, fully offline).
Tomorrow: any chain/L2/database sink implementing the same trait.

## Future interfaces (declared, not implemented)

WalletIdentity · TransactionSigner · BalanceQuery · NetworkFeeQuote.
Real signers load keys from external secret managers AT CALL TIME; nothing
here stores, generates or transmits private keys.

## Explicit non-goals

No mainnet connection. No smart contracts deployed. No wallets auto-created.
No financial promises. The adapter must be replaceable without touching core
economics.

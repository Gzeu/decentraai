# DecentraAI Crypto Intelligence Domain Pack

## Status

**Domain Pack:** Crypto Intelligence  
**Current stage:** specification / benchmark preparation  
**Core fabric:** generic DecentraAI — unchanged by this domain pack  
**Primary principle:** AI proposes → deterministic systems validate → evidence is preserved → risk policy decides

This document is the technical contract for introducing crypto-market intelligence as a **domain capability** on top of DecentraAI. It is intentionally separate from the generic fabric so the same infrastructure can later support other domains.

---

## 1. Purpose

Crypto Intelligence lets DecentraAI combine:

- market data;
- technical indicators;
- time-series forecasting models;
- financial/crypto sentiment models;
- on-chain and flow signals;
- deterministic risk controls;
- specialized agents;
- distributed compute and memory;
- auditable evidence.

The goal is **decision support and controlled research**, not an assumption of profitability.

The domain must remain capability-first: models are workers/capabilities, not authorities.

### 1.1 Design inspiration from existing Gzeu repositories

The domain pack deliberately reuses patterns already explored in the user's other repositories, while keeping DecentraAI's core generic.

Relevant sources inspected:

| Repository | Pattern worth importing into the domain pack |
|---|---|
| `Gzeu/CryptoTraderPro` | live market dashboard, Binance WebSocket singleton, watchlists/alerts, portfolio P&L, SMA backtest, technical indicators, CryptoPanic/news integration |
| `Gzeu/crypto-mcp-assistant` | multi-timeframe analysis, RSI/MACD/Bollinger/EMA/SMA, sentiment + Fear & Greed, position sizing, stop-loss/take-profit, drawdown controls, paper trading, MCP integration |
| `Gzeu/profesorXtrader` | multi-source realtime feeds, statistical price aggregation, outlier detection, cross-chain monitoring, arbitrage opportunity scoring, latency/reliability metrics |
| `Gzeu/binance-crypto-dashboard` | spot/futures/margin portfolio state, signed Binance requests, caching/retry, margin/liquidation context, secure server-side key handling |
| `Gzeu/blockchain-intelligence-suite` | whale tracking, anomaly detection, DeFi/liquidity intelligence, market microstructure, portfolio optimization, smart-contract risk, cross-chain flow and security analytics |
| `Gzeu/mvx-onchain-proof` | provenance / proof-oriented thinking for on-chain evidence and verified claims |

These repositories are **reference implementations/pattern sources**, not trusted runtime authorities. Any capability imported into DecentraAI must pass the same validation, evidence, benchmark and security gates as native DecentraAI functionality.

---

## 2. Governing invariants

### 2.1 AI proposes; deterministic systems decide

Model output is **untrusted input**.

A model may propose:

- a direction;
- a forecast;
- a confidence estimate;
- a capability requirement;
- a task decomposition.

A model may never directly:

- place an exchange order;
- bypass risk policy;
- alter balances;
- alter trust/reputation;
- issue credentials;
- override resource reservations;
- fabricate market state.

### 2.2 Evidence before conclusion

Every material analysis must be reconstructable from timestamped inputs and recorded transformations.

### 2.3 No-trade is a valid answer

The domain treats insufficient evidence, conflicting signals, excessive latency and excessive risk as legitimate reasons to return `NO_TRADE` or `INSUFFICIENT_DATA`.

### 2.4 Time is part of the data

Every market input and derived signal must carry an observation timestamp. A signal without a reliable timestamp is not production-grade evidence.

### 2.5 No leakage in evaluation

Backtests and benchmarks must never use future information, future revisions, future candles, future news labels or future-derived normalization parameters.

### 2.6 Strategy is not truth

A named strategy is a **hypothesis template**, not a guarantee. Every strategy must be evaluated against deterministic baselines, transaction costs and regime changes.

---

## 3. Architecture

```text
                         DECENTRAAI FABRIC
                                |
                        Crypto Domain Pack
                                |
        +-----------------------+-----------------------+
        |                       |                       |
   MARKET AGENT            NEWS AGENT            ONCHAIN AGENT
        |                       |                       |
   price / TA             sentiment / events       flows / whales
        |                       |                       |
        +-----------------------+-----------------------+
                                |
                         SIGNAL FUSION LAYER
                                |
                  +-------------+-------------+
                  |                           |
           Regime / Data Quality       Evidence Graph
                  |                           |
                  +-------------+-------------+
                                |
                           RISK ANALYZER
                                |
                    Deterministic decision policy
                                |
        +----------------+----------------+----------------+
        |                |                |                |
 LONG_CANDIDATE     SHORT_CANDIDATE    NEUTRAL      NO_TRADE / INSUFFICIENT_DATA
```

A later controlled-execution stage may exist, but it remains outside the model layer and behind explicit policy.

---

## 4. Domain capabilities

The crypto pack should expose capabilities rather than hard-code individual models into the core fabric.

### Market capabilities

- `market_data`
- `technical_analysis`
- `time_series_forecasting`
- `volatility_estimation`
- `orderbook_analysis`
- `multi_source_price_aggregation`
- `outlier_detection`
- `market_regime_detection`

### Intelligence capabilities

- `crypto_sentiment`
- `fear_greed_analysis`
- `event_extraction`
- `onchain_analysis`
- `whale_flow_analysis`
- `depeg_detection`
- `liquidity_analysis`
- `signal_fusion`
- `anomaly_detection`

### Portfolio/risk capabilities

- `risk_analysis`
- `position_sizing`
- `portfolio_analysis`
- `exposure_check`
- `drawdown_monitoring`
- `liquidation_risk`
- `slippage_estimation`
- `fee_estimation`

### DeFi / cross-chain capabilities

- `defi_protocol_analysis`
- `tvl_analysis`
- `yield_analysis`
- `impermanent_loss_analysis`
- `cross_chain_flow`
- `bridge_risk`
- `arbitrage_detection`
- `gas_cost_analysis`

### Research capabilities

- `crypto_research`
- `backtesting`
- `walk_forward_evaluation`
- `benchmarking`
- `evidence_review`
- `strategy_diagnostics`

Capability names must remain aligned with the existing DecentraAI capability registry/taxonomy. The domain pack must not create a second capability vocabulary.

---

## 5. Candidate model families

These are **benchmark candidates**, not production recommendations.

### Technical / price forecasting

- `NeoQuasar/Kronos-small` — lightweight price/time-series candidate.
- `amazon/chronos-2` — compact time-series forecasting candidate.
- `Salesforce/moirai-2.0-R-small` — lightweight short-horizon forecasting candidate.

### Financial / crypto sentiment

- `ElKulako/cryptobert` — crypto-language sentiment candidate.
- `kk08/CryptoBERT` — alternative crypto sentiment candidate.
- `ProsusAI/finbert` — financial-news sentiment candidate.

### Model selection rule

Download counts, parameter count and model popularity are **discovery signals only**. A model is accepted for a DecentraAI node only after measuring:

- memory footprint;
- load time;
- CPU/GPU utilization;
- latency;
- throughput;
- quality on the intended crypto task;
- calibration;
- robustness to stale/noisy inputs;
- license compatibility.

The actual node resource envelope is authoritative.

---

## 6. Data contract

### 6.1 Market snapshot

Every analysis run begins from a versioned `MarketSnapshot` concept containing:

```text
snapshot_id
observed_at
assets
prices
ohlcv
orderbook_summary
funding/open_interest (when available)
source_versions
latency/freshness
```

A snapshot must be immutable once used for evaluation. Re-fetching live data must create a new snapshot rather than mutate the prior one.

### 6.2 Signal contract

All crypto signals should normalize to a common structure before fusion.

Conceptual schema:

```json
{
  "signal_id": "...",
  "asset": "BTCUSDT",
  "observed_at": "2026-08-23T00:00:00Z",
  "source": "market|news|onchain|model|indicator",
  "kind": "sentiment|forecast|breakout|flow|trend|risk|arbitrage",
  "direction": "bullish|bearish|neutral",
  "score": 0.0,
  "confidence": 0.0,
  "horizon": "5m|1h|4h|1d|1w",
  "freshness_ms": 0,
  "evidence_ref": "...",
  "model": "...",
  "model_version": "..."
}
```

Implementation may use a Rust-native schema rather than this JSON representation. The JSON is a contract illustration, not an instruction to expose unvalidated arbitrary JSON.

### 6.3 Data quality state

Each source/snapshot should be classified as one of:

- `HEALTHY`
- `STALE`
- `PARTIAL`
- `DEGRADED`
- `INVALID`

A strategy may define a maximum acceptable quality state. `INVALID` must never be consumed as production evidence.

---

## 7. Data normalization and provenance

Raw external sources must not be passed blindly to models.

The ingestion layer should:

1. normalize timestamps to UTC;
2. deduplicate records;
3. preserve source identifiers;
4. normalize asset symbols;
5. detect stale data;
6. preserve original observation time;
7. classify source type;
8. attach provenance/evidence references;
9. record transport latency;
10. retain enough metadata to replay the feature construction path.

### Social/news feeds

Do not assume `CryptoBERT` understands an unprocessed Telegram/Twitter stream. Normalize posts/messages first, remove duplicates and bot-like spam where possible, then aggregate into time-bucketed features.

### Market data

At minimum retain:

- OHLCV;
- volume;
- spread/order-book features where available;
- volatility;
- funding/open interest where available;
- source timestamp;
- receive timestamp.

### On-chain

Keep source, chain, wallet/entity classification method and observation timestamp. A whale label should never be treated as ground truth without provenance.

---

## 8. Deterministic analytics layer

The model layer should be complemented by deterministic analytics already represented across the user's existing trading projects.

Recommended baseline indicators:

- RSI;
- MACD;
- SMA/EMA;
- Bollinger Bands;
- support/resistance;
- volume and volume anomalies;
- ATR / volatility;
- trend strength;
- order-book imbalance;
- funding/open-interest deltas;
- spread and liquidity metrics.

### Why this matters

A model prediction should be a **signal**, not the entire analysis. A simple baseline can sometimes outperform a complex model after fees and slippage. Therefore every model signal should be benchmarkable against deterministic baselines.

---

## 9. Multi-source price intelligence

The `profesorXtrader` pattern of combining Binance/CoinGecko/MultiversX feeds suggests a reusable capability: **multi-source price aggregation**.

Recommended flow:

```text
Source A ──┐
Source B ──┼──> normalize -> outlier filter -> aggregate -> confidence
Source C ──┘
```

Useful fields:

- source count;
- median price;
- VWAP where applicable;
- inter-source spread;
- outlier count;
- source reliability;
- latency by source.

A price with one source and high spread should carry less confidence than a converged multi-source price.

Do not use an aggregated price if the source disagreement exceeds the configured market/asset tolerance.

---

## 10. Strategy compositions

### 10.1 Social Momentum

```text
Crypto sentiment
      +
price forecast
      +
volume / breakout confirmation
      +
volatility regime
      +
risk checks
      -> directional analysis
```

Key failure modes:

- social spam/bots;
- duplicated stories;
- sentiment lag;
- false breakouts;
- low-liquidity moves.

### 10.2 Macro & Trend

```text
financial sentiment
      +
time-series forecast
      +
macro/event signals
      +
volatility regime
      -> multi-day trend analysis
```

Key failure modes:

- event surprise;
- regime change;
- forecast uncertainty ignored;
- correlated signals counted as independent evidence.

### 10.3 Scalping / Whale Alert

```text
short-horizon forecast
      +
crypto sentiment
      +
order-book / volume
      +
on-chain activity
      +
strict latency/risk gates
      -> short-horizon signal
```

This is the most latency-sensitive strategy and should be the **last** to approach controlled execution.

### 10.4 Multi-timeframe confirmation

The `crypto-mcp-assistant` pattern of 1m/5m/15m/1h/4h/1d analysis should become a reusable strategy primitive:

```text
micro timeframe
    ↓
short timeframe
    ↓
trend timeframe
    ↓
higher-timeframe regime
```

A lower-timeframe signal should not automatically override a higher-timeframe conflict. The strategy definition must specify which horizons are confirming vs vetoing.

### 10.5 Price + sentiment consensus

Use explicit disagreement handling:

```text
price bullish + sentiment bullish  -> stronger candidate
price bullish + sentiment neutral  -> ordinary candidate
price bullish + sentiment bearish  -> reduce confidence / NO_TRADE
```

Do not double-count multiple models trained on the same news source.

### 10.6 Cross-market / cross-chain arbitrage research

Inspired by the cross-chain monitoring patterns in `profesorXtrader`, model an opportunity as:

```text
buy venue
sell venue
raw spread
fees
estimated gas/bridge cost
slippage
latency
execution risk
net edge
```

A raw price difference is **not** an arbitrage opportunity until all costs and timing constraints are satisfied.

### 10.7 Liquidity-aware momentum

Combine price movement with:

- volume;
- spread;
- order-book depth;
- estimated market impact;
- funding/open interest.

A strong move on thin liquidity should be classified differently from the same move on deep liquidity.

### 10.8 Regime-aware strategy selection

Possible regimes:

- `TREND_UP`
- `TREND_DOWN`
- `RANGE`
- `HIGH_VOLATILITY`
- `LOW_LIQUIDITY`
- `EVENT_DRIVEN`
- `UNCERTAIN`

The active strategy must declare which regimes it expects. The system should prefer `NO_TRADE` when the current regime is outside its validated operating envelope.

---

## 11. Signal fusion

Signals must not be naively averaged.

The fusion layer should account for:

- signal freshness;
- model confidence;
- source reliability;
- correlation between signals;
- missing data;
- regime compatibility;
- disagreement;
- market liquidity;
- latency.

Conceptual decision:

```text
candidate = fuse(signals)

if data_invalid:
    INSUFFICIENT_DATA
elif risk_fail:
    NO_TRADE
elif stale_or_latency_too_high:
    NO_TRADE
elif insufficient_evidence:
    INSUFFICIENT_DATA
elif conflicting_signals:
    NEUTRAL / NO_TRADE
else:
    LONG_CANDIDATE or SHORT_CANDIDATE
```

The exact fusion formula must be deterministic, versioned and benchmarked.

---

## 12. Agent organization

Recommended logical agents:

- `crypto-governor` — decomposes and reviews crypto workflows.
- `market-agent` — technical and time-series analysis.
- `news-agent` — sentiment and event extraction.
- `onchain-agent` — on-chain and flow analytics.
- `risk-agent` — risk/exposure analysis.
- `portfolio-agent` — portfolio-level synthesis.
- `crypto-qa` — adversarial review for leakage, false positives and overfitting.
- `researcher` — source-grounded research and model evaluation.
- `data-quality-agent` — freshness, provenance and feed integrity.
- `regime-agent` — market-state classification.
- `model-evaluator` — benchmark and calibration diagnostics.
- `backtest-agent` — reproducible historical evaluation.

All agents follow the existing Agent Operating System contract:

**identity → role → skills → scopes → memory scope → approval gates → evidence**.

Agent role is independent from model identity. A role may switch models/providers while preserving its operating contract.

---

## 13. Agent debate / adversarial review

For higher-value analyses, allow independent agents to produce separate views before fusion:

```text
Market Agent ----\
News Agent -------+--> Debate/Review --> Risk Agent --> deterministic policy
Onchain Agent ----/
```

The review stage should explicitly surface:

- disagreement;
- missing evidence;
- contradictory timeframes;
- stale data;
- overfitting concerns;
- assumptions that are not observed facts.

The debate output is evidence for a decision, not authority over the deterministic policy layer.

---

## 14. Evidence-first output

Every analysis should preserve:

```text
request_id
market_snapshot_id
market_snapshot_timestamp
source_ids
model_ids + versions
model runtime
input feature summary
signal outputs
indicator outputs
risk checks
final classification
confidence
reason codes
evidence references
```

The final response must distinguish:

- **observed fact** — directly observed input;
- **model estimate** — generated by a model;
- **derived signal** — deterministic transformation;
- **hypothesis** — unverified interpretation;
- **decision classification** — policy output;
- **next check** — what additional evidence would reduce uncertainty.

---

## 15. Decision classes

Preferred output classes:

- `LONG_CANDIDATE`
- `SHORT_CANDIDATE`
- `NEUTRAL`
- `NO_TRADE`
- `INSUFFICIENT_DATA`

Optional diagnostic tags:

- `STALE_DATA`
- `HIGH_LATENCY`
- `MODEL_DISAGREEMENT`
- `HIGH_VOLATILITY`
- `LIQUIDITY_RISK`
- `EVENT_RISK`
- `MISSING_MARKET_DATA`
- `UNVERIFIED_ONCHAIN_SIGNAL`
- `SOURCE_DISAGREEMENT`
- `CORRELATED_EVIDENCE`
- `OUTLIER_FILTERED`

These tags explain **why** a signal was rejected without turning the model into an authority.

---

## 16. Portfolio and risk intelligence

The domain should separate **signal generation** from **portfolio decisioning**.

Portfolio-aware inputs should include:

- current exposure;
- unrealized P&L;
- realized P&L;
- available balance;
- margin usage;
- leverage;
- liquidation distance;
- correlated asset exposure;
- open orders;
- drawdown state.

The `binance-crypto-dashboard` pattern of monitoring spot/futures/margin state should be treated as a portfolio-observation reference. Any future execution integration must remain server-side and credential-scoped.

Position sizing should be deterministic and based on explicit risk parameters, not model confidence alone.

---

## 17. Risk boundary

This domain pack is for analysis and controlled execution research.

Before any live order pathway exists, it must have deterministic controls for at least:

- maximum position size;
- maximum portfolio exposure;
- leverage limits;
- per-trade loss limits;
- daily loss limits;
- stop-loss policy;
- cooldowns;
- stale-data rejection;
- market-liquidity checks;
- exchange/API availability;
- duplicate-order protection;
- explicit user/agent authorization.

No LLM may:

- invent balances;
- invent prices/fills;
- override risk policy;
- disable safety limits;
- create credentials;
- mutate trust/reputation;
- execute arbitrary exchange actions.

---

## 18. Backtesting contract

Backtesting is a **research system**, not proof of profitability.

Every backtest run must record:

```text
strategy_version
code_commit
model_versions
dataset/version
train_window
validation_window
test_window
fees
slippage assumptions
latency assumptions
position sizing rules
risk rules
random seeds where applicable
```

### Anti-leakage requirements

- no future candles in features;
- no future news labels;
- no future normalization statistics;
- no post-event revisions unless the test explicitly models them;
- signal timestamp must precede simulated decision time;
- execution price must respect simulated latency/slippage.

### Evaluation modes

- single split baseline;
- walk-forward evaluation;
- regime-separated evaluation;
- stress period evaluation;
- ablation (remove one signal family at a time);
- model-vs-baseline comparison;
- single-node-vs-collective comparison.

### Metrics

At minimum:

- return;
- volatility;
- max drawdown;
- Sharpe/Sortino where meaningful;
- hit rate;
- average win/loss;
- turnover;
- fees/slippage impact;
- exposure;
- tail loss;
- calibration of confidence.

Compare against deterministic baselines such as buy-and-hold or simple technical rules. A model only wins if it beats the baseline under the **same** data, costs and evaluation window.

---

## 19. Distributed execution model

Crypto workloads are capability-oriented rather than model-oriented.

Example:

```text
VPS
  - lightweight sentiment
  - financial sentiment
  - on-chain preprocessing
  - data adapters

Laptop
  - short-horizon forecasting

Desktop
  - larger forecasting model
  - portfolio/risk analysis
```

A single logical request may be decomposed across workers through:

**Fabric Intelligence → deterministic planner → DFCP → Sharing is Caring**.

The requester must not need to know which physical node owns the capability.

### Local-vs-distributed rule

Distributed execution is preferred only when measured/estimated benefit exceeds coordination cost.

Consider:

```text
benefit =
    compute_gain
  + capability_gain
  + cache/locality_gain
  + data_availability_gain

cost =
    network_latency
  + serialization
  + transfer
  + coordination
  + failure_risk
```

If `benefit <= cost`, stay local.

---

## 20. Memory and research journal

Crypto research is well suited to evidence-linked memory.

Potential memory objects:

```text
market-regime
asset-thesis
strategy-experiment
signal-failure
backtest-result
news-event
onchain-event
risk-lesson
model-evaluation
```

These belong in Agent OS / Obsidian memory with:

- source links;
- timestamps;
- confidence;
- agent ownership;
- evidence references;
- obsolete/version status.

Shared knowledge must pass through the existing Memory Keeper rules. Private agent memory must remain isolated.

A strategy memory entry must never silently become a production rule. Promotion from experiment/lesson to validated strategy requires a documented benchmark and approval path.

---

## 21. Credential boundary

Exchange credentials/API keys must never be stored in Obsidian memory, model prompts or signal records.

Use the existing secret-by-reference pattern:

```text
provider: exchange
credential_ref: BINANCE_API_KEY
```

The actual secret lives in the protected runtime environment.

Any future exchange integration must use scoped credentials with the minimum permissions required for the operation.

---

## 22. Repository-derived strategy backlog

These are candidates to extract from the user's existing projects and redesign as generic DecentraAI capabilities.

### Tier A — high-value foundations

1. **Multi-source price aggregator** — from `profesorXtrader`: normalize Binance/CoinGecko/chain feeds, measure spread/outliers, produce confidence.
2. **Multi-timeframe analyzer** — from `crypto-mcp-assistant`: reconcile 1m/5m/15m/1h/4h/1d signals under an explicit hierarchy.
3. **Technical baseline suite** — RSI/MACD/Bollinger/SMA/EMA/support-resistance/volume.
4. **Portfolio risk observer** — from `binance-crypto-dashboard`: spot/futures/margin state, exposure, margin/liquidation context.
5. **Paper-trading mode** — from `crypto-mcp-assistant`: safe research path before any controlled execution.
6. **Backtest/equity curve** — from `CryptoTraderPro`: deterministic baseline strategy with explicit fees/slippage.

### Tier B — intelligence expansion

7. **Whale flow engine** — from `blockchain-intelligence-suite`: large-flow detection with provenance rather than raw labels.
8. **Anomaly detection** — statistical + ML outlier detection.
9. **Liquidity and market-impact analysis** — spread, order-book depth, estimated slippage.
10. **DeFi intelligence** — TVL, pool depth, yield, impermanent loss, protocol risk.
11. **Cross-chain arbitrage research** — raw spread minus fees/gas/slippage/latency/bridge risk.
12. **Event intelligence** — macro/news/event timeline linked to market snapshots.

### Tier C — advanced intelligence

13. **Regime-aware strategy router**.
14. **Signal correlation / evidence independence analysis**.
15. **Agent debate and adversarial review**.
16. **Walk-forward model champion/challenger evaluation**.
17. **Collective crypto research across the DecentraAI fabric**.
18. **Evidence-linked strategy memory**.

### Priority rule

Do not implement these as one monolithic trading engine. Each becomes a capability/agent task that can be benchmarked, routed and independently trusted.

---

## 23. Benchmark matrix

Each candidate capability should be scored on:

| Dimension | Measurement |
|---|---|
| Quality | task-specific accuracy / ranking / calibration |
| Freshness | observation-to-decision age |
| Latency | p50/p95/p99 |
| Resource cost | CPU/RAM/VRAM |
| Stability | error/reconnect/retry rate |
| Coverage | assets/timeframes/chains |
| Provenance | % outputs with complete evidence |
| Robustness | performance under stale/noisy data |
| Cost | external/API/compute cost |
| Fabric benefit | improvement vs single-node |

A capability cannot become the default merely because it is the most accurate offline. It must satisfy the actual production constraints of the node/fabric.

---

## 24. Roadmap

### Crypto-1 — Domain pack specification

**Status: documented.**

This document, agent roles, capability vocabulary, evidence contract and safety boundaries.

### Crypto-2 — Data adapters

Normalize market/news/on-chain sources into versioned, timestamped inputs.

### Crypto-3 — Model benchmark

Benchmark small CPU-friendly models on the actual DecentraAI nodes. Select per-capability winners by measured quality and resource cost.

### Crypto-4 — Signal and fusion layer

Implement normalized signal schemas, freshness checks, provenance and deterministic fusion.

### Crypto-5 — Crypto Agent Team

Instantiate market/news/on-chain/risk/portfolio/QA agents under Agent OS and connect their memory scopes.

### Crypto-6 — Collective crypto analysis

Run crypto workflows across multiple nodes through Fabric Intelligence, PlacementEngine and DFCP/Sharing is Caring.

### Crypto-7 — Backtest lab

Compare deterministic baselines, model strategies, ablations and collective strategies with no leakage and realistic costs.

### Crypto-8 — Controlled execution research

Only after evidence, risk, credential, paper-trading and authorization gates are proven. Start read-only/testnet before any live trading pathway.

### Crypto-9 — Agent Gateway / MCP

Expose selected crypto capabilities to authorized external agents through scoped DecentraAI identity, MCP and OpenAI-compatible interfaces.

---

## 25. Non-goals

- Turning the DecentraAI core into a crypto-only system.
- Assuming a small model is automatically profitable.
- Treating social sentiment as ground truth.
- Treating model forecasts as guaranteed predictions.
- Building a crypto marketplace before the analysis layer is validated.
- Allowing model output to bypass deterministic policy.
- Treating repository-derived strategies as proven trading systems without fresh validation.

---

## 26. Definition of Done

Crypto Intelligence v1 is not complete because models run.

It is complete when:

1. data sources are timestamped and provenance-aware;
2. market snapshots are replayable;
3. deterministic indicators exist as baselines;
4. at least one small model is benchmarked per required capability;
5. signal fusion is deterministic and versioned;
6. stale/noisy/conflicting data can produce `NO_TRADE`;
7. evidence links every material conclusion to inputs;
8. single-node and collective modes are measured honestly;
9. agent memory distinguishes experiment from validated strategy;
10. no credential is exposed to memory/model output;
11. paper trading/backtesting exists before live execution;
12. any execution path remains behind explicit policy and authorization.

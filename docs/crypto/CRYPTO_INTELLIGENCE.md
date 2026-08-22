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
- license compatibility;
- reproducibility at a pinned revision.

The actual node resource envelope is authoritative.

---

## 6. Model Intelligence Registry

This is the model-level operational reference. It distinguishes **what a model is intended to do** from **what DecentraAI has actually measured**.

The registry below is a benchmark starting point, not a claim that every model is currently installed or validated on our nodes.

### 6.1 `NeoQuasar/Kronos-small`

**Role:** lightweight price/time-series forecasting candidate.  
**Input family:** historical numerical market sequences.  
**Output family:** future price/time-series estimate.  
**Best DecentraAI capability:** `time_series_forecasting`.  
**Best placement:** CPU-friendly worker where low memory footprint matters.  
**Good use:** short-horizon forecast as one independent signal.  
**Do not assume:** forecast confidence equals probability of profit.  
**Required validation:** walk-forward accuracy, calibration, regime-specific performance, RAM, load time, p50/p95 latency.

### 6.2 `amazon/chronos-2`

**Role:** general time-series forecasting candidate.  
**Input family:** numerical time series with temporal context.  
**Output family:** forecast/distribution over future values.  
**Best DecentraAI capability:** `time_series_forecasting`.  
**Best placement:** stronger worker when additional compute is justified.  
**Good use:** medium-horizon forecasting and comparison against other forecasting families.  
**Main risk:** crypto non-stationarity and regime shifts can invalidate apparently good historical calibration.  
**Required validation:** directional accuracy, interval calibration, regime split, resource cost, comparison against simple baselines.

### 6.3 `Salesforce/moirai-2.0-R-small`

**Role:** compact forecasting candidate.  
**Input family:** numerical time-series context.  
**Output family:** future forecast estimate.  
**Best DecentraAI capability:** `time_series_forecasting`.  
**Best placement:** CPU/lightweight forecasting worker.  
**Good use:** independent forecast family for ensemble disagreement.  
**Main risk:** model diversity may be illusory if models respond to the same market features in similar ways.  
**Required validation:** disagreement matrix, regime-conditioned accuracy, latency and calibration.

### 6.4 `ElKulako/cryptobert`

**Role:** crypto-domain language/sentiment candidate.  
**Input family:** normalized crypto text such as news/posts.  
**Output family:** sentiment classes/scores.  
**Best DecentraAI capability:** `crypto_sentiment`.  
**Best placement:** lightweight CPU worker.  
**Good use:** sentiment signal after deduplication, source normalization and temporal aggregation.  
**Main risk:** social spam, bots, sarcasm, duplicated articles and sentiment/price disconnect.  
**Required validation:** precision/recall by source, time stability, confidence calibration, robustness to duplicated/spam content.

### 6.5 `kk08/CryptoBERT`

**Role:** alternative crypto-language sentiment candidate.  
**Input family:** crypto-related text.  
**Output family:** sentiment score/class.  
**Best DecentraAI capability:** `crypto_sentiment`.  
**Good use:** independent comparison with other sentiment models.  
**Main risk:** correlated errors with other crypto-trained sentiment models.  
**Required validation:** model agreement/correlation matrix and source-specific accuracy.

### 6.6 `ProsusAI/finbert`

**Role:** financial-news sentiment candidate.  
**Input family:** finance/economics/news text.  
**Output family:** positive/neutral/negative sentiment.  
**Best DecentraAI capability:** `crypto_sentiment` or `event_extraction`.  
**Good use:** macro/regulatory/financial news context that may be less dependent on crypto slang.  
**Main risk:** financial sentiment does not directly encode crypto price direction.  
**Required validation:** event relevance, temporal stability, asset-specific read-through.

### 6.7 Embedding models

**Role:** semantic retrieval, memory and research context rather than price prediction.  
**Input family:** normalized research/news/strategy/evidence text.  
**Output family:** embeddings.  
**Best DecentraAI capability:** embedding/RAG capability.  
**Best placement:** small model on VPS or stronger model on GPU worker depending on retrieval workload.  
**Main risk:** semantic similarity is not causal relevance.  
**Required validation:** retrieval recall/precision, latency, memory and multilingual quality.

### 6.8 Lightweight OCR

**Role:** document intelligence for reports, screenshots and announcements.  
**Input:** image/PDF/document.  
**Output:** text/structured extraction.  
**Best placement:** VPS capability worker.  
**Main risk:** extraction errors can corrupt downstream market intelligence.  
**Required validation:** character/field accuracy and confidence plus source-document linkage.

### 6.9 Lightweight ASR

**Role:** speech/audio transcription.  
**Input:** audio.  
**Output:** timestamped transcript segments.  
**Best placement:** CPU VPS worker.  
**Main risk:** transcription errors and timing errors.  
**Required validation:** word error rate on relevant speech, latency and CPU footprint.

### Model registry rule

Every installed model must eventually advertise through the same worker capability mechanism:

```text
model_id
model_revision
capability
runtime
quantization
artifact_size
resource_cost
measured_latency
measured_throughput
quality_status
license_status
availability
```

The model registry is **descriptive**; the live worker advertisement remains authoritative for current availability.

---

## 7. Model lifecycle and trust

```text
DISCOVERED
   ↓
CANDIDATE
   ↓
BENCHMARKED
   ↓
VALIDATED
   ↓
ENABLED
   ↓
MONITORED
   ├── DEGRADED
   ├── SUSPENDED
   └── RETIRED
```

A model becomes production-eligible only after:

- pinned revision;
- reproducible load;
- capability compatibility;
- resource envelope check;
- evaluation dataset/version;
- quality gate;
- calibration gate where probabilities are used;
- evidence/output contract compliance;
- license/security review.

---

## 8. Model-output contract

Every model invocation that participates in trading intelligence should be wrapped in a deterministic result envelope.

```json
{
  "model_id": "...",
  "model_revision": "...",
  "capability": "time_series_forecasting|crypto_sentiment|...",
  "input_snapshot_id": "...",
  "observed_at": "...",
  "generated_at": "...",
  "horizon": "5m|1h|4h|1d|1w",
  "output_type": "forecast|classification|embedding|extraction",
  "value": {},
  "confidence": 0.0,
  "calibration_version": "...",
  "evidence_refs": [],
  "runtime": {
    "worker_id": "...",
    "latency_ms": 0
  }
}
```

Raw model output remains **untrusted**. The wrapper validates bounds, metadata, timestamps and evidence references before the signal layer can consume it.

---

## 9. Signal contract

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

### Signal validation

Reject signals when:

- timestamp is missing or implausible;
- score/confidence is outside configured bounds;
- direction is unknown;
- asset is not recognized;
- model metadata is missing where required;
- evidence reference cannot be resolved;
- freshness exceeds the strategy's maximum age.

---

## 10. Data normalization and provenance

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
10. retain enough metadata to replay feature construction.

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

## 11. Deterministic analytics layer

The model layer is complemented by deterministic analytics already represented across the user's trading projects.

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

A model prediction is a **signal**, not the entire analysis. A simple baseline can sometimes outperform a complex model after fees and slippage. Every model signal therefore remains benchmarkable against deterministic baselines.

---

## 12. Multi-source price intelligence

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

Do not use an aggregated price if source disagreement exceeds configured market/asset tolerance.

---

## 13. Repository-derived strategy library

These are strategy **patterns** discovered in Gzeu-owned projects. They are research templates, not guaranteed profitable strategies.

### 13.1 `CryptoTraderPro` patterns

Relevant patterns include live Binance WebSocket market data, watchlists, threshold alerts, portfolio P&L, compare views, news integration and an SMA crossover backtest.

DecentraAI should extract:

- event-driven market monitoring;
- deterministic alert conditions;
- simple technical baselines;
- reproducible backtest reference implementations;
- portfolio-aware context.

### 13.2 `crypto-mcp-assistant` patterns

Relevant patterns include multi-timeframe analysis, RSI/MACD/Bollinger/EMA/SMA, support/resistance, volume, sentiment, position sizing, stop-loss/take-profit, drawdown protection and paper trading.

DecentraAI should extract:

```text
1m / 5m / 15m / 1h / 4h / 1d
          ↓
per-timeframe signals
          ↓
consistency / conflict analysis
          ↓
regime-aware fusion
```

Higher-timeframe structure may define the regime while lower timeframes provide timing context.

### 13.3 `profesorXtrader` patterns

Relevant patterns include multi-source realtime feeds, price aggregation, outlier detection, confidence scoring, cross-chain price monitoring and arbitrage opportunity analysis.

DecentraAI should extract:

- source agreement;
- statistical outlier rejection;
- cross-source latency/reliability;
- net-spread calculation;
- cross-chain opportunity detection.

### 13.4 `binance-crypto-dashboard` patterns

Relevant patterns include spot/futures/margin account state, unrealized P&L, leverage, liquidation price, margin state, signed requests and retry/caching.

DecentraAI should extract:

- account-aware risk context;
- liquidation-distance checks;
- exposure controls;
- stale-account rejection;
- server-side credential handling.

### 13.5 `blockchain-intelligence-suite` patterns

Relevant patterns include whale tracking, anomaly detection, DeFi/liquidity intelligence, market microstructure, MEV/arbitrage, cross-chain flow, fraud/rug-pull detection, contract risk and portfolio optimization.

DecentraAI should extract:

- whale-flow context;
- anomaly detection;
- liquidity/TVL context;
- DeFi risk;
- smart-contract risk;
- cross-chain flows;
- address/asset risk scoring.

These remain separate evidence streams. A smart-contract risk score must not silently become a price forecast.

### 13.6 `mvx-onchain-proof` pattern

Proof/provenance concepts are especially useful for DecentraAI evidence: a claim about an on-chain event should carry the source, observation time, verification state and evidence reference rather than merely a generated narrative.

---

## 14. Advanced strategy families

The domain should support multiple strategy families instead of a single monolithic "AI trading strategy".

### 14.1 Trend-following ensemble

```text
higher-timeframe trend
        +
EMA/SMA structure
        +
MACD momentum
        +
volume confirmation
        +
forecast model
        → trend candidate
```

### 14.2 Mean-reversion regime strategy

```text
volatility regime
      +
Bollinger distance
      +
RSI extremes
      +
liquidity/spread
      +
trend filter
      → mean-reversion candidate or NO_TRADE
```

### 14.3 Social momentum

```text
news/social burst
      +
crypto sentiment
      +
source diversity
      +
volume confirmation
      +
breakout confirmation
      → directional candidate
```

### 14.4 Multi-timeframe confirmation

```text
1d regime
  ↓
4h structure
  ↓
1h momentum
  ↓
15m setup
  ↓
5m execution context
```

Conflicting timeframes must reduce confidence or produce `NEUTRAL/NO_TRADE` according to deterministic policy.

### 14.5 Whale / on-chain flow strategy

```text
large transfer
      +
entity/provenance confidence
      +
exchange inflow/outflow context
      +
market response
      +
liquidity
      → flow signal
```

Whale movement alone is never a buy/sell decision.

### 14.6 Cross-exchange price dislocation

```text
source A price
source B price
source C price
      ↓
outlier filtering
      ↓
net spread
      ↓
fees + latency + liquidity
      ↓
arbitrage candidate
```

Only net executable edge qualifies; gross price difference is insufficient.

### 14.7 Portfolio-aware signal gating

A bullish asset signal may still produce `NO_TRADE` when:

- portfolio exposure is already concentrated;
- leverage is too high;
- liquidation distance is unsafe;
- daily drawdown limit is reached;
- account data is stale;
- the position would violate configured constraints.

### 14.8 Event-driven / news shock

```text
event detected
   ↓
source verification
   ↓
relevance / novelty
   ↓
sentiment + market reaction
   ↓
volatility expansion check
   ↓
event-risk decision
```

Avoid acting on the first unverified headline.

### 14.9 Regime-switching strategy

Possible regimes:

- `TREND_UP`
- `TREND_DOWN`
- `RANGE`
- `HIGH_VOLATILITY`
- `LOW_LIQUIDITY`
- `EVENT_DRIVEN`
- `UNCERTAIN`

The active strategy must declare which regimes it expects. Prefer `NO_TRADE` when the current regime is outside its validated operating envelope.

---

## 15. Signal fusion

Signals must not be naively averaged.

The fusion layer should account for:

- signal freshness;
- model confidence;
- source reliability;
- correlation between signals;
- missing data;
- regime compatibility;
- disagreement;
- portfolio context;
- transaction/coordination cost.

Conceptual decision:

```text
candidate = fuse(signals)

if risk_fail:
    NO_TRADE
elif stale_data:
    INSUFFICIENT_DATA
elif insufficient_evidence:
    INSUFFICIENT_DATA
elif conflicting_signals:
    NEUTRAL / NO_TRADE
else:
    LONG_CANDIDATE or SHORT_CANDIDATE
```

The exact fusion formula must be deterministic, versioned and benchmarked.

### Signal independence rule

If multiple models consume the same source or nearly identical features, do not count them as independent evidence without correlation analysis.

---

## 16. Distributed execution model

Crypto workloads are capability-oriented rather than model-oriented.

Example:

```text
VPS
  - lightweight sentiment
  - financial sentiment
  - on-chain preprocessing
  - OCR / STT / data adapters

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

```text
benefit =
    compute_gain
  + capability_gain
  + cache/locality_gain
  + independent-evidence_gain

cost =
    network_latency
  + serialization
  + transfer
  + coordination
  + failure_risk
```

If `benefit <= cost`, stay local.

---

## 17. Agent organization

Recommended logical agents:

- `crypto-governor` — decomposes and reviews crypto workflows;
- `market-agent` — technical and time-series analysis;
- `news-agent` — sentiment and event extraction;
- `onchain-agent` — on-chain and flow analytics;
- `regime-agent` — market-regime detection and strategy-family selection;
- `risk-agent` — risk/exposure analysis;
- `portfolio-agent` — portfolio-level synthesis;
- `data-quality-agent` — freshness, provenance and source integrity;
- `model-evaluator` — model benchmark and calibration analysis;
- `backtest-agent` — reproducible backtests and statistical diagnostics;
- `crypto-qa` — adversarial review for leakage, false positives and overfitting;
- `researcher` — source-grounded research and model evaluation.

All agents follow the existing Agent Operating System contract:

**identity → role → skills → scopes → memory scope → approval gates → evidence**.

Agent role is independent from model identity. A role may switch models/providers while preserving its operating contract.

---

## 18. Evidence-first output

Every analysis should preserve:

```text
request_id
market_snapshot_id
market_snapshot_timestamp
source_ids
model_ids + revisions
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

## 19. Decision classes

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
- `PORTFOLIO_LIMIT`
- `ACCOUNT_DATA_STALE`

These tags explain **why** a signal was rejected without turning the model into an authority.

---

## 20. Risk boundary

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

## 21. Backtesting contract

Backtesting is a **research system**, not proof of profitability.

Every backtest run must record:

```text
strategy_version
code_commit
model_revisions
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
- confidence calibration.

Compare against deterministic baselines such as buy-and-hold and simple technical rules. A model only wins if it beats the baseline under the **same** data, costs and evaluation window.

---

## 22. Benchmark: single vs collective

The most important DecentraAI experiment is:

```text
SINGLE NODE
    vs
COLLECTIVE FABRIC
```

Hold constant:

- task;
- data snapshot;
- strategy;
- model revisions;
- decision policy;
- evaluation window.

Measure:

- quality;
- p50/p95 latency;
- throughput;
- CPU/RAM/GPU utilization;
- network transfer;
- coordination overhead;
- failure rate;
- compute contribution;
- evidence completeness.

The collective path is successful only when measured quality/performance or capability coverage improves enough to justify coordination overhead.

---

## 23. Distributed memory for crypto

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

---

## 24. Data and credential boundary

Exchange credentials/API keys must never be stored in Obsidian memory, model prompts or signal records.

Use the existing secret-by-reference pattern:

```text
provider: exchange
credential_ref: BINANCE_API_KEY
```

The actual secret lives in the protected runtime environment.

Any future exchange integration must use scoped credentials with the minimum permissions required for the operation.

---

## 25. Roadmap

### Crypto-1 — Domain pack specification

**Status: documented.**

This document, model intelligence registry, agent roles, capability vocabulary, evidence contract and safety boundaries.

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

Compare deterministic baselines, single-model strategies and collective strategies without implying live profitability.

### Crypto-8 — Controlled execution research

Only after evidence, risk and authorization layers are proven. Start with simulation/testnet/paper execution before any real-money pathway.

### Crypto-9 — Agent Gateway / MCP

Expose approved crypto capabilities to external agents through scoped API/MCP access, preserving deterministic policy and credential boundaries.

---

## 26. Non-goals

- Turning the DecentraAI core into a crypto-only system.
- Assuming a small model is automatically profitable.
- Treating social sentiment as ground truth.
- Treating model forecasts as guaranteed predictions.
- Counting correlated models as independent evidence.
- Building live execution before deterministic policy and research gates exist.
- Copying strategy claims from other projects without reproducing their assumptions.
- Hard-coding exchange/provider-specific logic into the generic fabric.

---

## 27. Definition of Done for Crypto Intelligence v1

Crypto Intelligence v1 is not complete when models can produce predictions.

It is complete when:

1. data is timestamped, normalized and provenance-aware;
2. candidate models are benchmarked on actual DecentraAI nodes;
3. model outputs are wrapped as untrusted evidence-bearing results;
4. signals are deterministic, versioned and freshness-checked;
5. multiple signal families can disagree safely;
6. portfolio/risk context can veto a market signal;
7. backtests are leakage-safe and reproducible;
8. single-node and collective paths are benchmarked honestly;
9. agent roles and memory boundaries are enforced;
10. evidence can reconstruct why a classification was produced;
11. `NO_TRADE` and `INSUFFICIENT_DATA` are routine, valid outcomes;
12. any future execution path remains behind deterministic policy and explicit authorization.

The target is not a model that predicts the market perfectly.

The target is a **distributed, evidence-driven crypto intelligence system** that becomes more capable when additional agents, models, data sources and compute nodes join the fabric.

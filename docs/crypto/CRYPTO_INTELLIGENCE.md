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
- `anomaly_detection`
- `depeg_detection`
- `liquidity_analysis`
- `signal_fusion`
- `cross_source_consensus`

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
- `model_evaluation`

Capability names must remain aligned with the existing DecentraAI capability registry/taxonomy. The domain pack must not create a second capability vocabulary.

---

## 5. Model intelligence registry

This section records what each candidate is useful for, what it can consume, what it can produce and what must be validated before enabling it.

**Important:** model cards, repository revision, tokenizer/input contract, license, quantization and actual node benchmarks are authoritative at enablement time. The descriptions below are planning metadata, not guarantees.

### 5.1 `NeoQuasar/Kronos-small`

**Role:** lightweight price/time-series forecasting candidate.  
**Best fit:** short-horizon forecasting experiments on CPU-constrained nodes.  
**Input concept:** historical market sequences / time-series context.  
**Output concept:** future trajectory / forecast estimate.  
**Best DecentraAI capability:** `time_series_forecasting`.  
**Strength:** very small specialist footprint versus general LLMs.  
**Risks:** regime dependence, calibration uncertainty and sensitivity to feature construction.  
**Benchmark:** 5m/15m/1h/4h horizons, walk-forward validation, p50/p95 latency, RAM and CPU utilization.

### 5.2 `amazon/chronos-2`

**Role:** compact general time-series forecasting candidate.  
**Best fit:** richer medium-horizon forecasting where extra compute is justified.  
**Input concept:** historical time series and temporal context.  
**Output concept:** forecast distribution/trajectory.  
**Best capability:** `time_series_forecasting`.  
**Strength:** broader temporal modeling than a tiny point predictor.  
**Risks:** non-stationary crypto regimes and overconfidence during structural breaks.  
**Benchmark:** interval coverage, directional accuracy, calibration, latency and resource cost versus Kronos/Moirai.

### 5.3 `Salesforce/moirai-2.0-R-small`

**Role:** compact time-series forecasting candidate.  
**Best fit:** short-horizon experiments and model-diversity checks.  
**Input:** numerical time series.  
**Output:** future forecast estimates.  
**Best capability:** `time_series_forecasting`.  
**Strength:** useful as an independent forecast family.  
**Risks:** model diversity does not automatically mean independent information.  
**Benchmark:** disagreement matrix versus Chronos/Kronos, regime-specific accuracy and calibration.

### 5.4 `ElKulako/cryptobert`

**Role:** crypto-domain language/sentiment candidate.  
**Best fit:** normalized crypto news/social text.  
**Input:** post/news/message text.  
**Output:** sentiment class/score.  
**Best capability:** `crypto_sentiment`.  
**Strength:** crypto-oriented vocabulary.  
**Risks:** spam, sarcasm, bot amplification, duplicated narratives and domain drift.  
**Benchmark:** precision/recall by source type, temporal stability and calibration.

### 5.5 `kk08/CryptoBERT`

**Role:** alternative crypto-language sentiment candidate.  
**Best fit:** independent sentiment signal and disagreement analysis.  
**Input:** crypto text.  
**Output:** sentiment classification/score.  
**Best capability:** `crypto_sentiment`.  
**Strength:** model diversity.  
**Risks:** source correlation and bias.  
**Benchmark:** agreement/correlation matrix against other sentiment models and downstream incremental value.

### 5.6 `ProsusAI/finbert`

**Role:** financial-news sentiment candidate.  
**Best fit:** macro, market, regulatory and finance-heavy news.  
**Input:** financial/news text.  
**Output:** positive/neutral/negative sentiment.  
**Best capability:** `crypto_sentiment` or `event_extraction`.  
**Strength:** complementary finance-domain language.  
**Risks:** financial sentiment is not a direct crypto direction signal. Relevance must be established separately.

### 5.7 Compact embedding models

**Role:** semantic retrieval and evidence memory rather than direct prediction.  
**Best fit:** VPS/CPU embedding worker plus richer embeddings on GPU-capable nodes.  
**Input:** research notes, news, strategy records, market events and prior evidence.  
**Output:** vectors.  
**Best capability:** `crypto_research` / existing generic embedding capability.  
**Strength:** enables semantic memory and evidence retrieval.  
**Risk:** similarity is not proof of causal relevance.  
**Benchmark:** Recall@K, MRR/nDCG where a labeled retrieval set exists, latency, RAM and index size.

### 5.8 Compact reranking models

**Role:** improve relevance after embedding retrieval.  
**Best fit:** evidence-heavy research and news retrieval.  
**Input:** query + retrieved candidate text.  
**Output:** relevance score/ranking.  
**Best capability:** `evidence_review` / generic reranking capability.  
**Risk:** added latency and compute.  
**Benchmark:** retrieval uplift versus no reranker and end-to-end latency.

### 5.9 OCR/document models

**Role:** convert PDFs, reports, screenshots and announcements into analyzable text/evidence.  
**Best fit:** VPS capability worker.  
**Input:** documents/images.  
**Output:** text/structured extraction.  
**Best capability:** generic `ocr`.  
**Risk:** extraction errors can become false market evidence. Preserve source document references and extraction confidence.

### 5.10 Compact ASR models

**Role:** speech/audio → timestamped text for calls, videos and market commentary.  
**Best fit:** CPU VPS worker using a small/quantized runtime.  
**Input:** audio.  
**Output:** timestamped transcript.  
**Best capability:** generic `stt` plus crypto event extraction.  
**Risk:** transcription and speaker-attribution errors.

### 5.11 Vision-capable small models

**Role:** interpret chart screenshots, scanned research documents and visual market reports.  
**Best fit:** GPU-capable desktop; optionally a very small CPU/GPU model on VPS if measured viable.  
**Input:** image/chart/document page.  
**Output:** structured visual description or extracted fields.  
**Best capability:** generic vision/document-understanding capability.  
**Risk:** chart hallucination, inaccurate OCR/axis interpretation and visual reasoning errors.  
**Rule:** vision-derived signals must remain evidence-qualified and should not override structured market feeds.

### 5.12 Small anomaly/classification models

**Role:** classify unusual market/on-chain/news patterns.  
**Best fit:** low-cost always-on worker.  
**Input:** engineered feature vector or normalized event.  
**Output:** class / anomaly score.  
**Best capability:** `anomaly_detection` or `event_extraction`.  
**Risk:** class imbalance, false positives and concept drift.  
**Benchmark:** precision at alert budget, false-positive rate, drift stability.

---

## 6. Model selection and capability placement

Popularity, download counts and parameter size are discovery signals only.

A model/capability candidate is enabled only after measuring:

- artifact size;
- resident RAM;
- peak RAM;
- load time;
- cold-start latency;
- warm p50/p95 latency;
- throughput;
- concurrency stability;
- CPU/GPU utilization;
- quality on the intended task;
- calibration;
- robustness to stale/noisy inputs;
- license compatibility;
- pinned revision reproducibility.

### 6.1 Capability → model → node matrix

The registry should eventually expose a deterministic matrix like:

```text
Capability                Preferred model family       Preferred node
--------------------------------------------------------------------------
time_series_forecasting   Kronos / Moirai / Chronos   Desktop / Laptop
crypto_sentiment          CryptoBERT / FinBERT        VPS / Desktop
embedding                 compact embedding            VPS / Desktop
reranking                 compact reranker             Desktop / VPS
ocr                       lightweight OCR              VPS
stt                       compact ASR                  VPS
vision                    small VLM                    Desktop
anomaly_detection         lightweight classifier      VPS
portfolio_risk            deterministic engine        Desktop/VPS
onchain_analysis          deterministic + small AI     VPS/Desktop
```

This is a **planning matrix**, not a fixed placement rule. The live Fabric Planner remains authoritative.

### 6.2 Fallback ladder

Every capability should have a degraded path where practical:

```text
BEST MODEL
   ↓ unavailable / overloaded / unhealthy
SECOND MODEL
   ↓
DETERMINISTIC BASELINE
   ↓
INSUFFICIENT_DATA / NO_TRADE
```

A missing model must not silently become a missing capability if a validated fallback exists.

### 6.3 Model lifecycle

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
   ↓
DEGRADED / SUSPENDED / RETIRED
```

---

## 7. Model-output contract

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
  "output_type": "forecast|classification|embedding|extraction|anomaly",
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

### 7.1 Model agreement is not independence

If multiple models consume the same source or highly similar features, treat their outputs as correlated evidence unless measurement demonstrates otherwise.

### 7.2 Confidence is not probability by default

`confidence=0.9` must not be interpreted as a calibrated 90% chance of correctness until calibration is measured and versioned.

---

## 8. Signal contract

All crypto signals should normalize to a common structure before fusion.

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
- freshness exceeds the strategy maximum age;
- underlying data quality is `INVALID`.

---

## 9. Data normalization and provenance

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

Do not assume crypto sentiment models understand an unprocessed Telegram/X/news stream. Normalize posts/messages first, remove duplicates and obvious bot-like spam where practical, then aggregate into time buckets.

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

## 10. Deterministic analytics layer

Recommended baselines:

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

A model prediction is a **signal**, not the entire analysis. Every model signal should be benchmarkable against deterministic baselines.

---

## 11. Multi-source price intelligence

The `profesorXtrader` pattern suggests a reusable capability: **multi-source price aggregation**.

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

Do not use the aggregate when source disagreement exceeds configured asset/market tolerance.

---

## 12. Strategy compositions

### 12.1 Social Momentum

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

### 12.2 Macro & Trend

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

### 12.3 Scalping / Whale Alert

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

This is the most latency-sensitive strategy and should be the last to approach controlled execution.

### 12.4 Multi-timeframe confirmation

Reusable horizons: `1m / 5m / 15m / 1h / 4h / 1d`.

Higher-timeframe structure may define regime while lower timeframes provide timing. The policy must define which horizons confirm and which can veto.

### 12.5 Price + sentiment consensus

```text
price bullish + sentiment bullish  -> stronger candidate
price bullish + sentiment neutral  -> ordinary candidate
price bullish + sentiment bearish  -> reduce confidence / NO_TRADE
```

Do not double-count correlated sentiment models.

### 12.6 Cross-market / cross-chain arbitrage research

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

Only net executable edge qualifies; gross spread is insufficient.

### 12.7 Liquidity-aware momentum

Combine movement with volume, spread, order-book depth, market-impact estimate and funding/open interest.

### 12.8 Regime-aware strategy selection

Possible regimes:

- `TREND_UP`
- `TREND_DOWN`
- `RANGE`
- `HIGH_VOLATILITY`
- `LOW_LIQUIDITY`
- `EVENT_DRIVEN`
- `UNCERTAIN`

If the current regime is outside a strategy's validated operating envelope, prefer `NO_TRADE`.

---

## 13. Signal fusion

Signals must not be naively averaged.

The fusion layer should account for:

- signal freshness;
- model confidence;
- source reliability;
- model/source correlation;
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

---

## 14. Distributed execution model

Crypto workloads are capability-oriented rather than model-oriented.

Example:

```text
VPS
  - lightweight sentiment
  - embeddings/reranking
  - OCR/STT
  - on-chain preprocessing
  - data adapters

Laptop
  - short-horizon forecasting
  - lightweight research workers

Desktop
  - larger forecasting model
  - vision analysis
  - portfolio/risk analysis
```

A single logical request may be decomposed through:

**Fabric Intelligence → deterministic planner → DFCP → Sharing is Caring**.

### Local-vs-distributed rule

Prefer distributed execution only when measured/estimated benefit exceeds coordination cost.

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

## 15. Agent organization

Recommended logical agents:

- `crypto-governor` — decomposes and reviews workflows.
- `market-agent` — technical/time-series analysis.
- `news-agent` — sentiment and event extraction.
- `onchain-agent` — on-chain and flow analytics.
- `regime-agent` — regime detection and strategy applicability.
- `risk-agent` — risk/exposure analysis.
- `portfolio-agent` — portfolio synthesis.
- `data-quality-agent` — freshness/provenance/source integrity.
- `model-evaluator` — benchmark/calibration analysis.
- `backtest-agent` — reproducible backtests/statistics.
- `crypto-qa` — adversarial review for leakage, false positives and overfitting.
- `researcher` — source-grounded research and model evaluation.

All agents follow the existing Agent Operating System contract:

**identity → role → skills → scopes → memory scope → approval gates → evidence**.

---

## 16. Evidence-first output

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

- observed fact;
- model estimate;
- derived signal;
- hypothesis;
- decision classification;
- next check.

---

## 17. Decision classes

Preferred output classes:

- `LONG_CANDIDATE`
- `SHORT_CANDIDATE`
- `NEUTRAL`
- `NO_TRADE`
- `INSUFFICIENT_DATA`

Diagnostic tags:

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

---

## 18. Risk boundary

Before any live order pathway exists, deterministic controls must cover:

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

No LLM may invent balances, prices, fills, override risk policy, disable limits, create credentials, mutate trust or execute arbitrary exchange actions.

---

## 19. Backtesting contract

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
- signal timestamp precedes simulated decision time;
- execution price respects simulated latency/slippage.

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

Compare against deterministic baselines under the same data, costs and window.

---

## 20. Benchmark: single vs collective

Hold constant:

- task;
- data snapshot;
- strategy;
- model versions;
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

The collective path succeeds only when measured quality/performance or capability coverage improves enough to justify coordination overhead.

---

## 21. Distributed memory for crypto

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

Shared knowledge must pass through the existing Memory Keeper rules. Private agent memory remains isolated.

---

## 22. Data and credential boundary

Exchange credentials/API keys must never be stored in Obsidian memory, prompts or signal records.

Use secret-by-reference:

```text
provider: exchange
credential_ref: BINANCE_API_KEY
```

Future exchange integrations must use minimum-permission scoped credentials.

---

## 23. Roadmap

### Crypto-1 — Domain pack specification

**Status: documented.** Model registry, capability vocabulary, agent roles, evidence contract and safety boundaries.

### Crypto-2 — Data adapters

Normalize market/news/on-chain sources into versioned, timestamped inputs.

### Crypto-3 — Model benchmark

Benchmark small CPU-friendly and GPU-assisted candidates on actual DecentraAI nodes. Select per-capability winners by quality/resource cost.

### Crypto-4 — Signal and fusion layer

Implement normalized signal schemas, freshness, provenance and deterministic fusion.

### Crypto-5 — Crypto Agent Team

Instantiate the domain agents under Agent OS and connect memory scopes.

### Crypto-6 — Collective crypto analysis

Run crypto workflows across multiple nodes through Fabric Intelligence, PlacementEngine and DFCP/Sharing is Caring.

### Crypto-7 — Backtest lab

Compare deterministic baselines, single-model and collective strategies without implying live profitability.

### Crypto-8 — Controlled execution research

Only after evidence/risk/authorization gates are proven. Start with simulation, paper trading and testnet.

### Crypto-9 — Agent Gateway / MCP

Expose approved crypto capabilities to external agents through scoped API/MCP access.

### Crypto-10 — Adaptive model routing

Select the best validated model/worker per asset, horizon, regime, latency budget and resource state.

### Crypto-11 — Collective market intelligence

Fuse independent agents and worker capabilities while preserving provenance, correlation awareness and contribution/evidence accounting.

---

## 24. Non-goals

- Turning the DecentraAI core into a crypto-only system.
- Assuming a small model is automatically profitable.
- Treating social sentiment as ground truth.
- Treating model forecasts as guaranteed predictions.
- Counting correlated models as independent evidence.
- Building live execution before deterministic policy and research gates exist.
- Copying strategy claims from other projects without reproducing assumptions.
- Hard-coding exchange/provider-specific logic into the generic fabric.

---

## 25. Definition of Done for Crypto Intelligence v1

Crypto Intelligence v1 is complete when:

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
11. `NO_TRADE` and `INSUFFICIENT_DATA` are routine valid outcomes;
12. future execution remains behind deterministic policy and explicit authorization.

The target is not a model that predicts the market perfectly.

The target is a **distributed, evidence-driven crypto intelligence system** that becomes more capable when additional agents, models, data sources and compute nodes join the fabric.

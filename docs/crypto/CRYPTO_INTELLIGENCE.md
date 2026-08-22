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
- auditable evidence;
- reproducible research and backtests.

The goal is **decision support and controlled research**, not an assumption of profitability.

The domain must remain capability-first: models are workers/capabilities, not authorities.

### 1.1 Design objective

Crypto is the first serious domain used to prove that the generic fabric can coordinate:

```text
DATA
  ↓
CAPABILITIES
  ↓
SPECIALIZED AGENTS
  ↓
SIGNAL FUSION
  ↓
RISK POLICY
  ↓
EVIDENCE
  ↓
DECISION SUPPORT
```

A successful implementation must improve **capability coverage, evidence quality, latency, or research quality**, not merely add more models.

---

## 2. Governing invariants

### 2.1 AI proposes; deterministic systems decide

Model output is **untrusted input**.

A model may propose:

- a direction;
- a forecast;
- a confidence estimate;
- a capability requirement;
- a task decomposition;
- a candidate strategy;
- a candidate execution plan.

A model may never directly:

- place an exchange order;
- bypass risk policy;
- alter balances;
- alter trust/reputation;
- issue credentials;
- override resource reservations;
- fabricate market state;
- invent fills;
- turn an unverified signal into a verified fact.

### 2.2 Evidence before conclusion

Every material analysis must be reconstructable from timestamped inputs and recorded transformations.

### 2.3 No-trade is a valid answer

The domain treats insufficient evidence, conflicting signals, excessive latency and excessive risk as legitimate reasons to return `NO_TRADE` or `INSUFFICIENT_DATA`.

### 2.4 Time is part of the data

Every market input and derived signal must carry an observation timestamp. A signal without a reliable timestamp is not production-grade evidence.

### 2.5 No leakage in evaluation

Backtests and benchmarks must never use future information, future revisions, future candles, future news labels or future-derived normalization parameters.

### 2.6 Abstention is a capability

A healthy crypto intelligence system must be able to say:

```text
NO_TRADE
INSUFFICIENT_DATA
STALE_DATA
HIGH_LATENCY
CONFLICTING_SIGNALS
RISK_REJECTED
```

without being pressured to produce a directional answer.

---

## 3. Architecture

```text
                         DECENTRAAI FABRIC
                                |
                        Crypto Domain Pack
                                |
          +---------------------+----------------------+
          |                     |                      |
     MARKET AGENT          NEWS AGENT           ONCHAIN AGENT
          |                     |                      |
      price / TA          sentiment / events       flows / whales
          |                     |                      |
          +---------------------+----------------------+
                                |
                        SIGNAL NORMALIZATION
                                |
                       SIGNAL FUSION LAYER
                                |
                      Evidence + timestamps
                                |
                    REGIME / QUALITY ANALYZER
                                |
                          RISK ANALYZER
                                |
                     Deterministic policy
                                |
        +------------+-----------+------------+----------------+
        |            |                        |                |
 LONG_CANDIDATE  SHORT_CANDIDATE           NEUTRAL          NO_TRADE
                                |
                         INS UFFICIENT_DATA
```

A later controlled-execution stage may exist, but it remains outside the model layer and behind explicit policy.

### 3.1 Domain lifecycle

Every crypto request should move through:

```text
REQUEST
  ↓
SNAPSHOT
  ↓
DATA QUALITY
  ↓
CAPABILITY PLAN
  ↓
AGENT DECOMPOSITION
  ↓
SIGNALS
  ↓
FUSION
  ↓
RISK
  ↓
EVIDENCE
  ↓
CLASSIFICATION
```

The orchestration layer must never silently skip a required stage.

---

## 4. Domain capabilities

The crypto pack should expose capabilities rather than hard-code individual models into the core fabric.

### Market capabilities

- `market_data`
- `technical_analysis`
- `time_series_forecasting`
- `volatility_estimation`
- `orderbook_analysis`
- `liquidity_analysis`
- `market_regime_detection`

### Intelligence capabilities

- `crypto_sentiment`
- `financial_sentiment`
- `event_extraction`
- `onchain_analysis`
- `whale_flow_analysis`
- `flow_anomaly_detection`
- `signal_fusion`

### Portfolio/risk capabilities

- `risk_analysis`
- `position_sizing`
- `portfolio_analysis`
- `exposure_check`
- `drawdown_analysis`
- `scenario_analysis`

### Research capabilities

- `crypto_research`
- `backtesting`
- `benchmarking`
- `evidence_review`
- `strategy_validation`
- `data_quality_review`

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
- reproducibility with the chosen runtime.

The actual node resource envelope is authoritative.

### 5.1 Model registry requirements

Every selected model should be recorded with:

```text
model_id
model_version
runtime
quantization
artifact_size
hardware_profile
task/capability
benchmark_commit
quality_score
latency_p50
latency_p95
throughput
license
status
```

A model without a reproducible benchmark record remains a candidate.

---

## 6. Data contract

The crypto domain must have a canonical snapshot boundary.

A **market snapshot** is a versioned, immutable set of inputs used by one analysis request or benchmark run.

Conceptual structure:

```json
{
  "snapshot_id": "...",
  "asset_set": ["BTCUSDT", "ETHUSDT"],
  "observed_at": "2026-08-23T00:00:00Z",
  "market_source_ids": ["..."],
  "news_source_ids": ["..."],
  "onchain_source_ids": ["..."],
  "normalization_version": "...",
  "timezone": "UTC"
}
```

The snapshot ID becomes the root reference for evidence and replay.

---

## 7. Signal contract

All crypto signals should normalize to a common structure before fusion.

Conceptual schema:

```json
{
  "signal_id": "...",
  "snapshot_id": "...",
  "asset": "BTCUSDT",
  "observed_at": "2026-08-23T00:00:00Z",
  "source": "market|news|onchain|model|indicator",
  "kind": "sentiment|forecast|breakout|flow|trend|risk|regime",
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

### 7.1 Signal validation

Reject signals when:

- timestamp is missing or implausible;
- score/confidence is outside configured bounds;
- direction is unknown;
- asset is not recognized;
- model metadata is missing where required;
- evidence reference cannot be resolved;
- freshness exceeds the strategy's maximum age;
- source provenance is incomplete;
- the signal was generated after the decision cutoff.

---

## 8. Data normalization and provenance

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
9. version transformations;
10. record data-quality results.

### Social/news feeds

Do not assume `CryptoBERT` understands an unprocessed Telegram/Twitter stream.

Normalize posts/messages first:

```text
raw item
  ↓
deduplicate
  ↓
language/source classification
  ↓
spam/bot filtering
  ↓
timestamp alignment
  ↓
entity/asset extraction
  ↓
sentiment/event inference
  ↓
time-bucket aggregation
```

### Market data

At minimum retain:

- OHLCV;
- volume;
- spread/order-book features where available;
- volatility;
- funding/open interest where available;
- source timestamp;
- sequence/order identifier where the source provides one.

### On-chain

Keep source, chain, wallet/entity classification method and observation timestamp. A whale label should never be treated as ground truth without provenance.

### Data quality states

Every input stream should be classifiable as:

```text
HEALTHY
STALE
PARTIAL
DEGRADED
INVALID
```

The planner may refuse to use `INVALID` data and should expose the reason.

---

## 9. Feature and indicator layer

Deterministic features must remain separate from model outputs.

Examples:

- returns;
- realized volatility;
- ATR;
- RSI;
- MACD;
- moving averages;
- momentum;
- volume anomalies;
- spread;
- order-book imbalance;
- funding rates;
- open interest;
- liquidation intensity;
- cross-asset correlation;
- market breadth where available.

Each derived feature needs:

```text
feature_name
formula_version
input_snapshot_id
computed_at
parameters
```

This makes the feature layer reproducible.

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
- low-liquidity moves;
- coordinated manipulation.

### 10.2 Macro & Trend

```text
financial sentiment
      +
time-series forecast
      +
macro/event signals
      +
volatility regime
      +
cross-asset context
      -> multi-day trend analysis
```

Key failure modes:

- event surprise;
- regime change;
- forecast uncertainty ignored;
- correlated signals counted as independent evidence;
- macro information arriving after market repricing.

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

### 10.4 Portfolio / Regime Strategy

A fourth strategy should be evaluated:

```text
market regime
      +
portfolio exposure
      +
asset correlation
      +
volatility
      +
drawdown
      ->
reduce / maintain / rebalance candidate
```

This is intentionally slower and risk-centric.

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
- calibration;
- historical failure modes.

Conceptual decision:

```text
candidate = fuse(signals)

if data_quality_fail:
    INSUFFICIENT_DATA
elif risk_fail:
    NO_TRADE
elif conflicting_signals:
    NEUTRAL / NO_TRADE
elif expected_cost > expected_benefit:
    NO_TRADE
else:
    LONG_CANDIDATE or SHORT_CANDIDATE
```

The exact fusion formula must be deterministic, versioned and benchmarked.

### 11.1 Correlated evidence

Two models consuming the same underlying news should not count as two independent confirmations.

Evidence should carry a `source_family` or equivalent correlation grouping.

---

## 12. Regime detection

A crypto intelligence system should explicitly recognize regime changes.

Candidate regimes:

```text
TREND_BULL
TREND_BEAR
RANGE
HIGH_VOLATILITY
LOW_VOLATILITY
LIQUIDITY_STRESS
EVENT_DRIVEN
UNKNOWN
```

Regime changes should reduce confidence in models validated only on a different regime.

---

## 13. Distributed execution model

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

**Fabric Intelligence → deterministic planner → DFCP → Sharing is Caring**

The requester must not need to know which physical node owns the capability.

### Local-vs-distributed rule

Distributed execution is preferred only when measured/estimated benefit exceeds coordination cost.

Consider:

```text
benefit =
    compute_gain
  + capability_gain
  + cache/locality_gain
  + parallelism_gain

cost =
    network_latency
  + serialization
  + transfer
  + coordination
  + failure_risk
```

If `benefit <= cost`, stay local.

### 13.1 Autonomous pressure integration

Once M15 is available:

```text
pressure
  ↓
Fabric Intelligence proposal
  ↓
deterministic planner
  ↓
DFCP
  ↓
capability worker
```

The crypto domain must never bypass the general pressure/planner architecture.

---

## 14. Agent organization

Recommended logical agents:

- `crypto-governor` — decomposes and reviews crypto workflows.
- `market-agent` — technical and time-series analysis.
- `news-agent` — sentiment and event extraction.
- `onchain-agent` — on-chain and flow analytics.
- `risk-agent` — risk/exposure analysis.
- `portfolio-agent` — portfolio-level synthesis.
- `crypto-qa` — adversarial review for leakage, false positives and overfitting.
- `researcher` — source-grounded research and model evaluation.

Optional specialists:

- `data-quality-agent` — freshness/provenance/data integrity.
- `backtest-agent` — reproducible backtest runs and statistical diagnostics.
- `regime-agent` — regime classification and transition monitoring.
- `model-evaluator` — resource/quality benchmarking and calibration.
- `execution-policy-agent` — proposes execution parameters but never bypasses deterministic policy.

All agents follow the existing Agent Operating System contract:

**identity → role → skills → scopes → memory scope → approval gates → evidence**

Agent role is independent from model identity. A role may switch models/providers while preserving its operating contract.

---

## 15. Evidence-first output

Every analysis should preserve:

```text
request_id
snapshot_id
market_snapshot_timestamp
source_ids
model_ids + versions
model runtime
input feature summary
signal outputs
indicator outputs
regime
risk checks
final classification
confidence
reason codes
evidence references
decision policy version
```

The final response must distinguish:

- **observed fact** — directly observed input;
- **model estimate** — generated by a model;
- **derived signal** — deterministic transformation;
- **hypothesis** — unverified interpretation;
- **decision classification** — policy output;
- **next check** — what additional evidence would reduce uncertainty.

No answer should hide uncertainty behind a single confidence number.

---

## 16. Decision classes

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
- `REGIME_UNCERTAIN`
- `DATA_DEGRADED`

These tags explain **why** a signal was rejected without turning the model into an authority.

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
- explicit user/agent authorization;
- maximum order frequency;
- kill switch / trading halt;
- circuit breakers for abnormal market conditions.

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
feature_version
signal_fusion_version
risk_policy_version
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
- execution price must respect simulated latency/slippage;
- model selection must not use the held-out test window;
- hyperparameters must be frozen before final evaluation.

### Metrics

At minimum:

- return;
- volatility;
- maximum drawdown;
- Sharpe/Sortino where meaningful;
- hit rate;
- average win/loss;
- turnover;
- fees/slippage impact;
- exposure;
- tail loss;
- calibration of confidence;
- trade count;
- time-in-market.

Compare against deterministic baselines such as buy-and-hold or simple technical rules.

A model only wins if it beats the baseline under the **same** data, costs and evaluation window.

---

## 19. Walk-forward evaluation

For any strategy intended beyond experimentation, prefer:

```text
train → validate → test
       ↓
roll forward
       ↓
train → validate → test
       ↓
repeat
```

Aggregate out-of-sample results across windows.

Do not choose the best window and report only that result.

---

## 20. Benchmark: single vs collective

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
- evidence completeness;
- capability coverage.

The collective path is successful only when measured quality/performance or capability coverage improves enough to justify coordination overhead.

---

## 21. Distributed memory for crypto

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
model-benchmark
data-quality-incident
```

These belong in Agent OS / Obsidian memory with:

- source links;
- timestamps;
- confidence;
- agent ownership;
- evidence references;
- obsolete/version status.

Shared knowledge must pass through the existing Memory Keeper rules. Private agent memory must remain isolated.

### 21.1 Memory should distinguish

```text
FACT
OBSERVATION
MODEL_OUTPUT
LESSON
HYPOTHESIS
DECISION
FAILED_EXPERIMENT
```

A failed strategy must remain retrievable so the colony does not rediscover the same mistake.

---

## 22. Data and credential boundary

Exchange credentials/API keys must never be stored in Obsidian memory, model prompts or signal records.

Use the existing secret-by-reference pattern:

```text
provider: exchange
credential_ref: BINANCE_API_KEY
```

The actual secret lives in the protected runtime environment.

Any future exchange integration must use scoped credentials with the minimum permissions required for the operation.

Recommended first integration posture:

```text
market-data read-only
news read-only
on-chain read-only
account read-only
```

Order permissions should remain disabled until the controlled execution milestone is explicitly approved.

---

## 23. Research quality gates

Before a model or strategy can move from candidate to validated:

```text
DATA QUALITY
   ↓
REPRODUCIBILITY
   ↓
OUT-OF-SAMPLE PERFORMANCE
   ↓
CALIBRATION
   ↓
ROBUSTNESS
   ↓
RESOURCE COST
   ↓
COLLECTIVE BENEFIT
```

A strong backtest with poor reproducibility is not production-ready.

A high-performing model that requires excessive resources should not automatically win.

A smaller model that delivers better quality/cost may be preferred.

---

## 24. Crypto capability lifecycle

Every capability should move through:

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
DEPRECATED
```

Each transition should be recorded in evidence and model/strategy documentation.

---

## 25. Roadmap

### Crypto-1 — Domain pack specification

**Status: documented.**

This document, agent roles, capability vocabulary, evidence contract and safety boundaries.

### Crypto-2 — Data adapters

Normalize market/news/on-chain sources into versioned, timestamped inputs.

### Crypto-3 — Model benchmark

Benchmark small CPU-friendly models on the actual DecentraAI nodes. Select per-capability winners by measured quality and resource cost.

### Crypto-4 — Signal and fusion layer

Implement normalized signal schemas, freshness checks, provenance, regime handling and deterministic fusion.

### Crypto-5 — Crypto Agent Team

Instantiate market/news/on-chain/risk/portfolio/QA agents under Agent OS and connect their memory scopes.

### Crypto-6 — Collective crypto analysis

Run crypto workflows across multiple nodes through Fabric Intelligence, PlacementEngine and DFCP/Sharing is Caring.

### Crypto-7 — Walk-forward backtest lab

Reproducible backtests with strict train/validation/test separation, fees, slippage, latency and baseline comparison.

### Crypto-8 — Controlled execution research

Only after the evidence, risk and authorization layers are proven.

### Crypto-9 — Agent-facing Crypto Gateway

Expose validated crypto capabilities through the existing OpenAI-compatible API and MCP gateway with scoped credentials and quotas.

---

## 26. Non-goals

- Turning the DecentraAI core into a crypto-only system.
- Assuming a small model is automatically profitable.
- Treating social sentiment as ground truth.
- Treating model forecasts as guaranteed predictions.
- Building a crypto marketplace before the analysis layer is validated.
- Allowing a model to become the execution authority.
- Reporting backtests without explicit costs, latency and leakage controls.
- Mixing private agent memory with shared collective knowledge without policy.

---

## 27. Definition of done

Crypto Intelligence v1 is not complete because models load.

It is complete when one logical crypto analysis can:

1. create a reproducible market snapshot;
2. discover required capabilities across the fabric;
3. select locally or distribute according to measured benefit;
4. obtain normalized, timestamped signals;
5. preserve provenance and evidence;
6. fuse signals deterministically;
7. apply deterministic risk gates;
8. produce an explainable classification;
9. store useful lessons in agent memory;
10. replay the analysis from the recorded evidence;
11. compare single-node vs collective execution honestly.

The final objective is not “a trading bot”.

It is:

> **A verifiable Crypto Intelligence domain running on a cooperative AI fabric.**

The same architecture should remain reusable for future domains such as document intelligence, security intelligence, research intelligence, industrial intelligence and other specialized agent workloads.

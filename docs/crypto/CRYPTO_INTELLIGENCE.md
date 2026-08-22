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
                       Evidence + timestamps
                                |
                           RISK ANALYZER
                                |
                    Deterministic decision policy
                                |
        +----------------+----------------+----------------+
        |                |                |
 LONG_CANDIDATE     SHORT_CANDIDATE    NEUTRAL
        |                |                |
        +----------------+----------------+
                         |
                   NO_TRADE / INSUFFICIENT_DATA
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

### Intelligence capabilities

- `crypto_sentiment`
- `event_extraction`
- `onchain_analysis`
- `whale_flow_analysis`
- `regime_detection`
- `signal_fusion`

### Portfolio/risk capabilities

- `risk_analysis`
- `position_sizing`
- `portfolio_analysis`
- `exposure_check`

### Research capabilities

- `crypto_research`
- `backtesting`
- `benchmarking`
- `evidence_review`

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

## 6. Signal contract

All crypto signals should normalize to a common structure before fusion.

Conceptual schema:

```json
{
  "signal_id": "...",
  "asset": "BTCUSDT",
  "observed_at": "2026-08-23T00:00:00Z",
  "source": "market|news|onchain|model|indicator",
  "kind": "sentiment|forecast|breakout|flow|trend|risk",
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

## 7. Data normalization

Raw external sources must not be passed blindly to models.

The ingestion layer should:

1. normalize timestamps to UTC;
2. deduplicate records;
3. preserve source identifiers;
4. normalize asset symbols;
5. detect stale data;
6. preserve original observation time;
7. classify source type;
8. attach provenance/evidence references.

### Social/news feeds

Do not assume `CryptoBERT` understands an unprocessed Telegram/Twitter stream. Normalize posts/messages first and aggregate them into time-bucketed features.

### Market data

At minimum retain:

- OHLCV;
- volume;
- spread/order-book features where available;
- volatility;
- funding/open interest where available;
- source timestamp.

### On-chain

Keep source, chain, wallet/entity classification method and observation timestamp. A whale label should never be treated as ground truth without provenance.

---

## 8. Strategy compositions

### 8.1 Social Momentum

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

### 8.2 Macro & Trend

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

### 8.3 Scalping / Whale Alert

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

---

## 9. Signal fusion

Signals must not be naively averaged.

The fusion layer should account for:

- signal freshness;
- model confidence;
- source reliability;
- correlation between signals;
- missing data;
- regime compatibility;
- disagreement.

Conceptual decision:

```text
candidate = fuse(signals)

if risk_fail:
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

## 10. Distributed execution model

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

cost =
    network_latency
  + serialization
  + transfer
  + coordination
  + failure_risk
```

If `benefit <= cost`, stay local.

---

## 11. Agent organization

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
- `execution-policy-agent` — proposes execution parameters but never bypasses deterministic policy.

All agents follow the existing Agent Operating System contract:

**identity → role → skills → scopes → memory scope → approval gates → evidence**.

Agent role is independent from model identity. A role may switch models/providers while preserving its operating contract.

---

## 12. Evidence-first output

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

## 13. Decision classes

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

These tags explain **why** a signal was rejected without turning the model into an authority.

---

## 14. Risk boundary

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

## 15. Backtesting contract

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

## 16. Benchmark: single vs collective

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
- evidence completeness.

The collective path is successful only when measured quality/performance or capability coverage improves enough to justify coordination overhead.

---

## 17. Distributed memory for crypto

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

## 18. Data and credential boundary

Exchange credentials/API keys must never be stored in Obsidian memory, model prompts or signal records.

Use the existing secret-by-reference pattern:

```text
provider: exchange
credential_ref: BINANCE_API_KEY
```

The actual secret lives in the protected runtime environment.

Any future exchange integration must use scoped credentials with the minimum permissions required for the operation.

---

## 19. Roadmap

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

### Crypto-7 — Backtest Lab

Reproducible single-vs-collective backtesting with leakage protection and deterministic baselines.

### Crypto-8 — Controlled execution research

Only after evidence, risk, authorization and exchange integration are independently verified. No assumption of profitability.

---

## 20. Non-goals

- Turning the DecentraAI core into a crypto-only system.
- Treating a small model as automatically profitable.
- Treating social sentiment as ground truth.
- Treating forecasts as guaranteed predictions.
- Allowing LLMs to execute exchange actions directly.
- Using backtest results as proof of future profitability.
- Storing exchange secrets in agent memory.
- Building a crypto marketplace before the analysis/evidence layer is validated.

---

## 21. Definition of done for Crypto Intelligence v1

Crypto Intelligence v1 is not complete because models load successfully.

It is complete when:

1. data is timestamped and provenance-aware;
2. candidate models are benchmarked on real DecentraAI hardware;
3. normalized signals are validated before fusion;
4. leakage-safe backtests are reproducible;
5. single-node and collective execution are compared honestly;
6. agent roles and memory scopes are enforced;
7. every conclusion is evidence-linked;
8. `NO_TRADE` and `INSUFFICIENT_DATA` work as first-class outcomes;
9. remote capabilities use existing DFCP/Sharing is Caring infrastructure;
10. no model can bypass deterministic risk/execution policy.

**Target architecture:**

```text
Data → Normalize → Evidence → Specialist Agents → Signal Fusion
                                              ↓
                                      Risk / Policy Engine
                                              ↓
                               LONG / SHORT / NEUTRAL / NO-TRADE
                                              ↓
                                Optional controlled execution
```

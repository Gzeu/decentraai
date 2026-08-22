# DecentraAI Crypto Intelligence Domain Pack

## Purpose

Crypto Intelligence is a domain pack built on top of the generic DecentraAI fabric. It must not modify the core fabric into a trading-specific system.

The goal is to let DecentraAI combine small, specialized market models, market data, sentiment, on-chain signals, technical indicators, risk controls and distributed compute into one auditable crypto-analysis workflow.

## Core principle

> AI proposes. Deterministic systems validate. Evidence is required. Risk controls decide whether an action is permitted.

Models must never directly place trades, bypass risk limits, alter balances, or override deterministic execution policy.

## Architecture

```text
                    DECENTRAAI FABRIC
                           |
                    Crypto Domain Pack
                           |
          +----------------+----------------+
          |                |                |
      MARKET AGENT     NEWS AGENT      ONCHAIN AGENT
          |                |                |
      Forecasting       Sentiment       Flow/Whale
          |                |                |
          +----------------+----------------+
                           |
                     Evidence Layer
                           |
                       Risk Agent
                           |
                    Deterministic Policy
                           |
                 LONG / SHORT / NEUTRAL /
                       NO-TRADE
```

## Candidate model families

These are candidates for benchmark and integration, not unconditional production recommendations.

### Technical / price forecasting

- `NeoQuasar/Kronos-small` — lightweight time-series/price-oriented candidate.
- `amazon/chronos-2` — compact time-series forecasting candidate.
- `Salesforce/moirai-2.0-R-small` — lightweight forecasting candidate for short windows.

### Financial / crypto sentiment

- `ElKulako/cryptobert` — crypto-language sentiment candidate.
- `kk08/CryptoBERT` — alternative crypto sentiment candidate.
- `ProsusAI/finbert` — financial news sentiment candidate.

### Supporting analytics

The model layer is only one signal source. Production analysis should combine model output with deterministic market data and indicators such as:

- OHLCV
- volume
- volatility
- RSI
- MACD
- moving averages
- order-book measurements
- funding/open-interest data when available
- on-chain flow metrics when available
- event/macro calendar

## Recommended strategy compositions

### Social Momentum

```text
Crypto sentiment
      +
price/forecast model
      +
volume / breakout confirmation
      +
risk checks
      -> directional analysis
```

Do not assume a sentiment model directly understands raw Telegram/Twitter streams. Normalize, deduplicate, timestamp and score source data before inference.

### Macro & Trend

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

### Scalping / Whale Alert

```text
short-horizon forecast
      +
crypto sentiment
      +
order-book / volume
      +
on-chain activity
      +
strict latency + risk gates
      -> short-horizon signal
```

This strategy is the most sensitive to data latency, stale data and execution costs.

## Distributed execution model

Crypto workloads are intentionally capability-oriented rather than model-oriented.

Example:

```text
VPS
  - sentiment model
  - financial sentiment
  - on-chain preprocessing

Laptop
  - short-horizon forecasting

Desktop
  - larger forecasting model
  - portfolio/risk analysis
```

A crypto task may use several workers through the existing Fabric Intelligence, deterministic planner and DFCP/Sharing-is-Caring mechanisms.

The user should experience one logical analysis request even when the workload is distributed.

## Agent organization

Recommended logical agents:

- `crypto-governor` — decomposes and reviews crypto-analysis workflows.
- `market-agent` — technical and time-series analysis.
- `news-agent` — financial/crypto sentiment and event extraction.
- `onchain-agent` — on-chain and flow analytics.
- `risk-agent` — risk and exposure analysis.
- `portfolio-agent` — portfolio-level synthesis.
- `crypto-qa` — adversarial review of signal quality, leakage and false positives.
- `researcher` — source-grounded research and model evaluation.

Each agent follows the existing Agent Operating System contract, RBAC model, memory boundaries and approval rules.

## Evidence-first output

Every analysis should preserve:

```text
request_id
market snapshot timestamp
models + versions
input signal summary
model outputs
indicator outputs
risk checks
final classification
confidence
reason codes
```

The final response should distinguish clearly between:

- observed facts
- model estimates
- derived signals
- hypotheses
- recommended next checks

## Decision classes

The crypto domain should prefer:

- `LONG_CANDIDATE`
- `SHORT_CANDIDATE`
- `NEUTRAL`
- `NO_TRADE`
- `INSUFFICIENT_DATA`

`NO_TRADE` and `INSUFFICIENT_DATA` are first-class valid outcomes.

## Safety boundary

This domain pack is for analysis and controlled execution research. Any live order execution must remain behind explicit deterministic policy, authentication, quota/risk controls and auditable execution paths.

Never allow an LLM to:

- invent account balances
- invent prices
- invent fills
- bypass risk limits
- disable stop-loss or exposure constraints
- create credentials
- mutate trust/reputation
- directly execute arbitrary exchange actions without validated policy

## Benchmark plan

Before production use, benchmark candidate combinations on the actual DecentraAI nodes.

Capture at minimum:

- model load time
- RAM footprint
- CPU utilization
- inference latency p50/p95
- throughput
- concurrency behavior
- signal quality
- calibration/confidence
- false-positive rate
- sensitivity to stale data
- network transfer cost
- local vs distributed latency

Compare:

```text
single-node analysis
vs
collective fabric analysis
```

The benchmark must remain honest: distributed execution is only better when the measured result improves enough to justify network and coordination overhead.

## Roadmap

### Crypto-1 — Domain pack documentation

Current milestone: this document and agent-role definition.

### Crypto-2 — Data adapters

Normalize market, news, sentiment and on-chain inputs.

### Crypto-3 — Model benchmark

Benchmark small CPU-friendly candidates and select per-capability winners.

### Crypto-4 — Crypto Fabric Agents

Instantiate market/news/on-chain/risk/portfolio agents under Agent OS.

### Crypto-5 — Collective crypto analysis

Run the domain workflow across multiple DecentraAI workers through DFCP and Sharing is Caring.

### Crypto-6 — Backtest lab

Compare deterministic baselines, single-model strategies and collective strategies without implying live profitability.

### Crypto-7 — Controlled execution

Only after the evidence, risk and authorization layers are proven.

## Non-goals

- Turning the DecentraAI core into a crypto-only system.
- Assuming a small model is automatically profitable.
- Treating social sentiment as ground truth.
- Treating model forecasts as guaranteed predictions.
- Building a crypto marketplace before the analysis layer is validated.

# Auto-Tuning Planner for DecentraAI

Status: MIXED (VERIFIED for metrics and cost models in vLLM, SGLang, LMCache, decentralized and energy-aware scheduling papers; INFERRED for DecentraAI’s planner design).[cite:118][cite:127][cite:131][cite:112][cite:139][cite:122][cite:126][cite:129]

This document explores performance learning and auto-tuning for DecentraAI’s execution planner.

## 1. Evidence of Learned Scheduling (VERIFIED)

- vLLM and NVIDIA AIPerf expose per-request metrics, including speculative decoding acceptance rates.[cite:118][cite:127]
- LMCache and prefix caching stacks compute cache affinity scores and use them in routing decisions.[cite:112][cite:139]
- Decentralized inference and energy-aware scheduling papers demonstrate that lightweight, adaptive scheduling strategies significantly improve throughput and energy efficiency under dynamic conditions.[cite:122][cite:126][cite:129]

These systems show that schedulers:

- Consume metrics.
- Update internal models.
- Adjust scheduling decisions over time.

## 2. DecentraAI Planner: From Static to Learned (INFERRED)

Currently, DecentraAI’s planner uses static scoring functions based on WorkerFacts, NetworkGraph, and ModelFacts.[cite:98]

Auto-tuning adds a feedback loop:

- Request.
- Candidate execution strategies.
- Predicted cost and benefit.
- Execution.
- Measurement.
- Model update.

## 3. Cost Model Design (INFERRED)

Use simple, explainable cost models initially:

- Per strategy `S`, maintain:
  - `ttft_estimate`.
  - `throughput_estimate` (tokens/s).
  - `latency_estimates` (p50, p99).
  - `error_rate`.
  - `spec_acceptance_estimate` (if speculative).[cite:127][cite:131]

Update via EWMA:

- New estimate = `alpha * measured_value + (1 - alpha) * old_estimate`.

Features for prediction:

- Context length.
- Model size and type.
- Hardware characteristics.
- Network metrics.
- KV state (prefix coverage, cache hits).

## 4. Provenance and Transparency (INFERRED)

All predictions should carry provenance:

- MEASURED: directly from metrics.
- ESTIMATED: derived via cost models, with confidence scores.
- INFERRED: heuristics or static rules.

Operators should be able to introspect why a strategy was chosen.

## 5. Advanced Techniques (EXPERIMENTAL)

When simple cost models fail, consider:

- Contextual bandits for strategy selection, treating each strategy as an arm and features as context.[cite:126]
- Bayesian optimization over strategy parameters (e.g., number of speculative tokens, KV transfer thresholds).[cite:122]

These should be carefully controlled and logged to maintain transparency.

## 6. Recommendations (GO / EXPERIMENT / WAIT)

- **GO NOW**:
  - Implement metric collection per execution (TTFT, throughput, acceptance rates, cache hits, network transfer times).
  - Add EWMA-based cost models to predict per-strategy performance.

- **EXPERIMENT FIRST**:
  - Explore contextual bandits in a simulated environment to test robustness.

- **WAIT**:
  - Complex ML-based schedulers until simpler models are demonstrably insufficient.

An auto-tuning planner will enable DecentraAI to move beyond static heuristics and continually optimize execution strategies based on real workloads.

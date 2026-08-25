# DecentraAI — Benchmark Report

Measured on the live 3-node fabric: **VPS** (decentraai-vps, Qwen3-1.7B), **Desktop** (i7, Qwen3-1.7B), **Laptop** (i5, qwen2.5-3b). Every number below was produced by a node, not estimated.

## Distributed embeddings

| Scale | Nodes | Executed | Failed | Wall (pool) | Serial baseline | Speedup | Throughput |
|-------|-------|----------|--------|-------------|-----------------|---------|------------|
| 1,000 | 3 | 1000/1000 | 0 | 31.3 s | 1,179,655 ms | **37.8×** | 32 emb/s |
| 100,000 | 3 | 100000/100000 | 0 | 3,165 s (53 min) | 132,997 s (~37 h) | **42.1×** | 31.6 emb/s |

Batch size: 500/chunk, ~24 vectors per DFCP round-trip, streaming (no RAM blow-up), deterministic `emb_{i}` ids, retry on failed batches (0 retries needed at both scales).

## Chat batch

| Scale | Speedup | Notes |
|-------|---------|-------|
| 12 prompts, 3 workers | **4.3×** | valid outputs on all nodes |

## Distributed inference (map/reduce)

| Workload | Result |
|----------|--------|
| 6,109-char document summary | 2.26× speedup, single coherent result |
| Shard failure recovery | FAILED → REPLAN → COMPLETED → reduce VALID |

Map-reduce splits a single logical workload into shards, maps across workers, reduces into one answer. llama-server cannot split one forward pass across nodes (intra-machine `--split-mode` only) — map-reduce/context-split is the honest distributed-inference primitive.

## Model Colony evidence (Model Intelligence corpus, VPS)

| Model | Accuracy | Latency | Role |
|-------|----------|---------|------|
| Phi-4-mini-instruct | 0.33 | 803 ms | non-reasoner reducer candidate |
| Gemma-3-1B-it | 0.33 | 578 ms | summarization/classification |
| Qwen3-1.7B | 0.25 | 4,624 ms | reasoner (empty-output risk on reduce) |

Model Colony selects by capability + RAM fit + measured evidence (non-reasoners preferred for reduce).

## Economy / attribution

| Event | Value |
|-------|-------|
| Desktop worker balance (verified contributions) | 16,872 credits |
| Consumer quota settlement (BYOA run) | available 50,000 → 49,999, consumed 0 → 1 |

Every remote contribution is measured and credited only on verified completion; credit is fail-closed on the signed evidence.

## Notes / limitations
- Qwen3-1.7B spends generation budget on hidden reasoning; with small caps the visible reduce content can be empty — detected and reported honestly (incomplete, never fabricated).
- CPU-only fabric; tensor parallelism is out of scope (needs accelerators + fast interconnects).
- Replan requires an alternative worker; with a single node the honest answer is LOCAL or incomplete.
# Speculative Distributed Inference for DecentraAI

Status: MIXED (VERIFIED for vLLM, SGLang, TensorRT-LLM, and NVIDIA DFlash speculative decoding; INFERRED for cross-worker speculative collaboration in DecentraAI).[cite:118][cite:120][cite:121][cite:123][cite:124][cite:131][cite:128][cite:130]

This document investigates how speculative decoding can enable heterogeneous DecentraAI nodes to cooperate on one user task without tensor/model splitting.

## 1. Speculative Decoding Basics (VERIFIED)

- A small **draft** model proposes multiple candidate tokens per step.
- The large **target** model verifies all candidates in one forward pass.
- If candidates match the target distribution, the system accepts the longest valid prefix, yielding multiple tokens for nearly the cost of one verification.[cite:121][cite:131]

Methods include:

- Draft models (smaller versions of the target).
- EAGLE-3/P‑EAGLE heads attached to the target model’s hidden states.
- Multi-token prediction (MTP).
- N‑gram and suffix-based drafting without a model.[cite:118][cite:125][cite:131]

Frameworks:

- vLLM: `--speculative-config` and `--speculative-model`.[cite:118][cite:132]
- SGLang: `--speculative-algorithm` (EAGLE3, DFlash, ngram).[cite:119][cite:121]
- TensorRT-LLM: speculative decoding modules and EAGLE heads.[cite:123][cite:131]

## 2. Heterogeneous Speculative Decoding (VERIFIED/EXPERIMENTAL)

Recent research and engineering work explores heterogeneous drafting/verification:

- Dovetail: CPU/GPU heterogeneous speculative decoding, with draft on GPU and target on CPU, optimizing throughput via careful balance of draft size and verification cost.[cite:128]
- AHASD: mobile NPU/PIM heterogeneous speculative decoding, using asynchronous queues to coordinate drafting and verification across devices.[cite:130]
- NVIDIA DFlash: block-diffusion drafter integrated with vLLM and SGLang, generating entire blocks of candidate tokens in one GPU pass, verified by the target model.[cite:120]

These show that the draft and target can reside on different devices and architectures, and asynchronous coordination can leverage heterogeneous strengths.

## 3. DecentraAI Heterogeneous Draft/Verify Strategy (INFERRED)

Define `ExecutionStrategy::SpeculativeDraftVerify`:

- **Draft worker(s)**:
  - Run small draft models (1–3 B LLM or EAGLE heads) on weaker hardware (laptop GPU, CPU-only nodes).
  - Generate candidate tokens and, when required, hidden-state features.
- **Verify worker**:
  - Run the large target model on strong hardware (desktop GPU).
  - Verify candidate tokens and accept prefixes.

### 3.1 Data Flow

- Draft worker produces:
  - Candidate tokens (sequence of IDs) per step.
  - Optional additional data (hidden states, logits) for methods like DFlash and EAGLE.[cite:120][cite:123]
- DecentraAI transports candidate tokens (and optionally hidden states) from draft worker to verify worker.
- Verify worker runs batched verification passes and feeds accepted tokens back into the overall decode stream.

Network payload is significantly smaller than full KV caches, especially when only tokens are transported.

### 3.2 Acceptance Metrics and Planner Integration

- vLLM and NVIDIA’s AIPerf expose per-request speculative decoding statistics, including draft acceptance rates.[cite:118][cite:127]
- DecentraAI can ingest these metrics into HistoricalExecutionFacts to estimate net benefit of speculative strategies per request type.

Planner should enable speculative strategies only when:

- Draft acceptance rate is high enough (e.g., >0.6) to justify overhead.[cite:121][cite:124]
- Network latency and bandwidth between draft and verify workers are acceptable.

## 4. Multi-Draft Approaches (EXPERIMENTAL)

Speculative decoding can employ multiple drafts:

- Multiple small draft models proposing different token sequences.
- Draft heads operating with tree attention to explore branched continuations (SGLang’s tree-based speculation).[cite:121]

DecentraAI could, INFERRED:

- Assign different draft models to different workers (e.g., CPU node runs n‑gram draft; laptop GPU runs small LLM draft).
- Aggregate candidates and feed them to a single target model for verification.

This raises complexity and network traffic; likely EXPERIMENTAL in DecentraAI until acceptance and throughput gains justify it.

## 5. Opportunity for "Weak" Workers (INFERRED)

Speculative decoding enables weak workers to contribute positive value without joining the tensor-parallel path:

- CPU-only nodes can host n‑gram or small LLM draft models.
- Laptop GPUs can host EAGLE-3 or draft LLMs.
- The heavy target model remains on the strongest GPU.

Planner chooses whether to involve these workers based on measured acceptance rate and network cost.

## 6. Recommendations (GO / EXPERIMENT / WAIT)

- **GO NOW**:
  - Add `ExecutionStrategy::SpeculativeDraftVerify` to the planner abstraction.
  - Extend WorkerFacts to label speculative capabilities (draft-model support, spec-decode engine support).

- **EXPERIMENT FIRST**:
  - Integrate speculative decoding on a single node (vLLM or SGLang) and measure acceptance rates, throughput gains, and latency.[cite:118][cite:121][cite:131]
  - Prototype heterogeneous draft/verify across Desktop + Laptop using vLLM or SGLang, transporting token sequences or hidden states over the network.

- **WAIT**:
  - Multi-draft speculative strategies across many workers; likely too complex until single-draft heterogeneous strategies are well understood.

Speculative distributed inference is a promising way for DecentraAI to harness heterogeneous nodes in one user task with lower synchronization costs than full tensor-parallel model splitting.

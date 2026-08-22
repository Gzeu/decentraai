# DecentraAI Crypto Model Training

## Status

**Stage:** design locked / implementation preparation  
**Domain:** Crypto Intelligence  
**Training principle:** benchmark first → train adapter → evaluate → promote only if the candidate wins  
**Primary training method:** SFT with LoRA/QLoRA  
**Initial target:** small causal instruction model, starting with Qwen2.5-1.5B-Instruct  

This document defines the first-party training pipeline for a DecentraAI-owned crypto-specialized model. It is intentionally separate from the generic fabric and is connected to the existing Agent OS, Obsidian Memory, Model Registry, Fabric Intelligence and evidence system.

---

## 1. Why this exists

DecentraAI should eventually own models/adapters that encode the project's accumulated crypto-analysis knowledge without changing the generic fabric into a crypto-only system.

The first target is **not** a model that directly trades.

The first target is a model that reliably performs:

- market-context interpretation;
- regime classification;
- technical-signal interpretation;
- news/sentiment interpretation;
- on-chain-context interpretation;
- risk explanation;
- structured evidence-aware analysis;
- explicit `NO_TRADE` / `INSUFFICIENT_DATA` behavior.

The model remains a **capability worker**. It does not become a policy authority.

---

## 2. Governing invariants

### 2.1 AI proposes; deterministic systems decide

The trained model may produce an analysis or classification proposal.

It must never directly:

- place exchange orders;
- bypass risk controls;
- issue credentials;
- mutate trust/reputation;
- alter Fabric reservations;
- fabricate market state;
- promote itself to production.

### 2.2 Baseline before training

A base model must be evaluated on a frozen benchmark **before** training. No trained model is accepted without demonstrating its change relative to that baseline.

### 2.3 Temporal evaluation

Crypto data is time-dependent. Train/validation/test partitions must preserve chronology. Random row splitting is not acceptable for the primary evaluation.

### 2.4 No future leakage

Training and evaluation must not use future candles, future labels, future-derived normalization values, future revised headlines, or post-decision information.

### 2.5 Reproducibility

Every experiment must record:

- repository commit;
- dataset version/hash;
- base model identifier;
- base model revision;
- adapter configuration;
- library versions;
- seed(s);
- training configuration;
- node/accelerator;
- resulting artifact hash;
- evaluation report.

---

## 3. Initial model choice

### 3.1 First baseline: `Qwen/Qwen2.5-1.5B-Instruct`

This is the initial reference model because it is small enough for rapid local iteration while remaining an instruction-tuned causal LM suitable for structured conversational training. Its current Hugging Face model card documents 1.54B parameters, 32,768-token context and Apache-2.0 licensing. The exact model revision used by DecentraAI must be pinned in the Model Registry at download time. citeturn688428search9turn688428search5

This is a **starting baseline, not a permanent model decision**. The Model Registry may later compare Qwen3, SmolLM, Llama/Phi or another small model family using the same benchmark.

### 3.2 Model selection gate

Before replacing the baseline, compare candidate models using:

- structured-output validity;
- crypto task accuracy;
- calibration;
- refusal/no-trade behavior;
- hallucination rate;
- inference latency;
- resident/peak RAM;
- VRAM;
- load time;
- context handling;
- license compatibility;
- reproducibility.

Popularity/download count is not a selection criterion by itself.

---

## 4. Target training architecture

```text
                    DECENTRAAI CRYPTO DOMAIN
                              │
                       MarketSnapshot
                              │
                   deterministic feature layer
                              │
                     training example builder
                              │
                    temporal dataset registry
                              │
                ┌─────────────┴─────────────┐
                │                           │
             baseline                    training
                │                           │
         frozen benchmark             QLoRA / SFT
                │                           │
                └─────────────┬─────────────┘
                              ▼
                         evaluation
                              │
                     promote / reject
                              │
                        Model Registry
                              │
                    Fabric capability worker
```

The Desktop/GPU node should perform the main training workload when capable. VPS/Laptop can prepare data, validate examples, run evaluation and serve lightweight inference. The Fabric may distribute preprocessing/evaluation tasks, but the first training run should remain simple and reproducible on one training host.

---

## 5. Artifact layout

Recommended repository/data layout:

```text
training/crypto/
├── base-models/
├── datasets/
│   ├── raw/
│   ├── normalized/
│   ├── train/
│   ├── validation/
│   └── test/
├── experiments/
│   ├── baseline-v0/
│   └── qlora-v0.1/
├── adapters/
├── evaluation/
└── manifests/
```

Large datasets and model weights should not be committed to Git. Git stores manifests, code, schemas, hashes and experiment metadata.

---

## 6. Download and pin the base model

Use the Hugging Face CLI to download the selected model into the local model cache/artifact directory. Hugging Face documents `hf download` as the standard model download path. The downloaded revision must be pinned in a manifest instead of relying on a moving `main` reference. citeturn688428search9

Conceptual first download:

```bash
hf auth login
hf download Qwen/Qwen2.5-1.5B-Instruct \
  --local-dir training/crypto/base-models/qwen2.5-1.5b-instruct
```

At download time record:

```yaml
model_id: Qwen/Qwen2.5-1.5B-Instruct
revision: <git-revision-or-commit>
license: apache-2.0
downloaded_at: <timestamp>
artifact_hash: <sha256-or-manifest-hash>
```

The exact revision is authoritative for every experiment.

---

## 7. Environment isolation

Training must use a dedicated Python environment and a pinned dependency file.

Minimum stack:

```text
Python
PyTorch
Transformers
Datasets
TRL
PEFT
Accelerate
bitsandbytes (for QLoRA)
```

Hugging Face TRL supports PEFT directly and documents LoRA and QLoRA integration. QLoRA uses a quantized frozen base plus trainable LoRA adapters and is designed to reduce training memory. citeturn235632search0turn688428search3

Initial environment concept:

```bash
python -m venv .venv-crypto-train
source .venv-crypto-train/bin/activate

pip install -U torch transformers datasets accelerate
pip install -U "trl[peft]" bitsandbytes
```

The resulting environment must be exported/locked for reproducibility.

---

## 8. Dataset design

### 8.1 Training objective

The model should learn **analysis behavior**, not merely memorize labels.

Each example should contain enough context to reconstruct the decision:

```text
MarketSnapshot
+ deterministic indicators
+ source-grounded news/sentiment
+ on-chain context when available
+ current regime
+ known constraints
→ structured assistant analysis
```

### 8.2 Preferred SFT format

TRL supports conversational datasets with a `messages` structure and automatically applies the model chat template. For instruction tuning, use conversational examples with explicit `user` and `assistant` turns. citeturn688428search0turn688428search10

Example:

```json
{
  "messages": [
    {
      "role": "user",
      "content": "Analyze BTCUSDT on 4h using the supplied market snapshot."
    },
    {
      "role": "assistant",
      "content": "{\"classification\":\"LONG_CANDIDATE\",\"confidence\":0.68,\"risk\":\"medium\",\"reasons\":[\"...\"],\"invalidating_conditions\":[\"...\"]}"
    }
  ]
}
```

For an assistant-behavior dataset, prefer assistant-only loss where the model/chat template supports it. TRL documents `assistant_only_loss=True`; for supported families such as Qwen, TRL can patch the training template automatically. citeturn688428search1turn688428search3

### 8.3 Example classes

The dataset should deliberately contain:

- bullish candidates;
- bearish candidates;
- neutral cases;
- `NO_TRADE` cases;
- `INSUFFICIENT_DATA` cases;
- conflicting model signals;
- stale-data cases;
- high-volatility cases;
- low-liquidity cases;
- portfolio/risk-veto cases;
- failed historical predictions;
- regime transitions.

This prevents the model from learning "always predict a direction".

---

## 9. Dataset construction pipeline

```text
raw market/news/on-chain data
          ↓
source validation
          ↓
MarketSnapshot creation
          ↓
feature/indicator calculation
          ↓
label generation using only allowed future outcome window
          ↓
quality filtering
          ↓
temporal split
          ↓
SFT examples
          ↓
dataset manifest + hash
```

### Important label rule

Labels may use future outcome information **only to construct the training target for an example at time T**. Features and input context for that same example must contain only information available at or before T.

---

## 10. Temporal train/validation/test split

Example:

```text
OLDER DATA                                      NEWER DATA
│────────────────────────────────────────────────────────│
│                    TRAIN │ VALIDATION │      TEST      │
```

The exact dates will be determined by the dataset available at the time of the run.

Primary evaluation must be chronological. Randomized splits may be used only as supplementary diagnostics and must never replace the temporal test.

---

## 11. Synthetic vs real examples

### Real examples

Preferred source:

- timestamped market snapshots;
- actual technical features;
- real historical news/events;
- verified on-chain observations;
- recorded strategy outcomes.

### Synthetic examples

Synthetic examples may improve coverage for rare combinations, but must be explicitly tagged:

```yaml
source_type: synthetic
synthetic_generator: <version>
```

Synthetic examples must never silently masquerade as historical market evidence.

Recommended initial ratio: keep the majority of benchmark-critical evaluation examples real/historical and separately track synthetic data influence.

---

## 12. Baseline benchmark

Before training the adapter, create a frozen benchmark set.

Recommended first benchmark categories:

1. market context interpretation;
2. regime classification;
3. technical reasoning;
4. sentiment interpretation;
5. on-chain interpretation;
6. risk analysis;
7. conflict handling;
8. stale-data handling;
9. `NO_TRADE` quality;
10. structured JSON/schema compliance.

Record:

```text
baseline_model
model_revision
benchmark_version
dataset_hash
prompt_version
metrics
latency
RAM/VRAM
```

Never modify the benchmark casually after seeing model results. Changes require a new benchmark version.

---

## 13. First training method: SFT + LoRA/QLoRA

The first training pass should be supervised fine-tuning, not reinforcement learning.

TRL's `SFTTrainer` supports conversational or prompt-completion datasets and integrates directly with PEFT. LoRA trains only adapter parameters while keeping the base model frozen; QLoRA combines LoRA with 4-bit quantization for lower memory usage. citeturn235632search0turn688428search3

Conceptual configuration:

```python
from peft import LoraConfig
from trl import SFTConfig, SFTTrainer

peft_config = LoraConfig(
    r=32,
    lora_alpha=64,
    lora_dropout=0.05,
    bias="none",
    task_type="CAUSAL_LM",
    target_modules=["q_proj", "k_proj", "v_proj", "o_proj"],
)

training_args = SFTConfig(
    output_dir="training/crypto/adapters/crypto-v0.1",
    learning_rate=1e-4,
    num_train_epochs=1,
    per_device_train_batch_size=1,
    gradient_accumulation_steps=8,
    assistant_only_loss=True,
    logging_steps=10,
    eval_strategy="steps",
)
```

For QLoRA, use a `BitsAndBytesConfig` with 4-bit loading and combine it with the PEFT configuration. Hugging Face recommends NF4 for 4-bit quantization and documents double quantization as an additional memory-saving option. citeturn235632search0

The exact hyperparameters above are **starting defaults**, not guaranteed optimal settings. They must be recorded and changed only through controlled experiments.

---

## 14. Why not full fine-tuning first

Full fine-tuning:

- requires much more memory;
- is more expensive;
- is slower to iterate;
- produces larger artifacts;
- makes baseline comparison harder;
- is unnecessary for the first domain-adaptation phase.

The first objective is a high-quality domain adapter. Base-model replacement is a later decision.

---

## 15. First run strategy — minimize experiments

Do not launch dozens of hyperparameter runs.

Use a **small controlled sweep**:

### Run A — reference

Base model, no adapter.

### Run B — conservative LoRA

Default LoRA rank/alpha, one epoch, low learning rate range.

### Run C — QLoRA

Same dataset/seed/schedule family, 4-bit base.

Choose the best candidate only if it improves the frozen benchmark without unacceptable regressions in:

- hallucination;
- schema validity;
- no-trade behavior;
- latency;
- memory;
- calibration.

Once a candidate wins, freeze the configuration as the new reference.

---

## 16. Evaluation gate

A trained adapter is promoted only if it beats baseline on the agreed benchmark or provides a clearly documented capability improvement.

Required report:

```text
model/base revision
adapter revision
training dataset version
benchmark version
quality metrics
schema-validity rate
NO_TRADE precision/recall
calibration
hallucination/error rate
p50/p95 latency
peak RAM
VRAM
throughput
known regressions
```

A model with higher directional accuracy but materially worse safety/schema behavior does **not** automatically win.

---

## 17. Artifact and registry promotion

A successful adapter gets a registry entry:

```yaml
model_id: decentraai-crypto-0.1
base_model: Qwen/Qwen2.5-1.5B-Instruct
base_revision: <pinned>
adapter_type: lora
training_method: sft
adapter_revision: <commit-or-hash>
dataset_version: crypto-dataset-v0.1
benchmark_version: crypto-bench-v0.1
status: validated
capabilities:
  - crypto_market_analysis
  - crypto_regime_analysis
  - crypto_risk_explanation
preferred_node: desktop
fallback:
  - base-model
```

The trained artifact is still not an execution authority.

---

## 18. Serving the trained model

The adapter can be served using the same OpenAI-compatible serving layer already used by DecentraAI, after validation.

The Model Registry should expose:

```text
model_id
version
capabilities
node
health
benchmark_summary
adapter/base relation
```

Fabric Intelligence can then treat the trained model as a capability provider and route work to it based on live node capacity and policy.

---

## 19. Obsidian experiment memory

Every training run should write a typed experiment memory object:

```yaml
type: experiment
status: completed
agent: model-evaluator
base_model: Qwen/Qwen2.5-1.5B-Instruct
dataset: crypto-dataset-v0.1
benchmark: crypto-bench-v0.1
result: pass|fail|inconclusive
```

Store:

- hypothesis;
- configuration;
- result;
- observed regressions;
- lesson;
- artifact reference;
- next recommendation.

Secrets and raw credentials never enter the vault.

---

## 20. Training feedback loop

```text
market data
    ↓
analysis
    ↓
actual outcome
    ↓
evaluation
    ↓
error / lesson
    ↓
curated training example
    ↓
dataset version
    ↓
next adapter training
    ↓
benchmark
```

This is a **curated learning loop**, not uncontrolled self-training.

The model cannot silently generate and approve its own labels.

---

## 21. Human and deterministic gates

Before promotion:

1. dataset manifest is reproducible;
2. benchmark is frozen/versioned;
3. training artifact is hashable;
4. security checks pass;
5. evaluation report exists;
6. deterministic promotion policy passes;
7. human approval is recorded for first-party production promotion.

Later, a trusted automated promotion policy may be introduced, but it must remain deterministic and auditable.

---

## 22. Hardware strategy for current DecentraAI fabric

### Desktop

Primary training node.

Use for:

- model training;
- adapter evaluation;
- heavier inference;
- benchmark generation.

### Laptop

Use for:

- dataset preparation;
- feature generation;
- lightweight evaluation;
- reproducibility check;
- CPU fallback.

### VPS

Use for:

- dataset orchestration;
- experiment registry;
- validation;
- lightweight model serving;
- memory/Obsidian coordination;
- agent control plane.

Do not move large training tensors across the fabric unless a benchmark demonstrates that distributed training is worthwhile. The first training run is intentionally single-training-node for simplicity and reproducibility.

---

## 23. Distributed training — later, not first

Distributed training becomes relevant only when:

- the base model exceeds a single-node training envelope;
- dataset processing becomes a bottleneck;
- multiple GPUs provide a measurable benefit;
- communication overhead is acceptable.

At that point evaluate Accelerate/FSDP/DeepSpeed or another appropriate distributed trainer based on actual hardware.

This should remain separate from normal DecentraAI inference-time Sharing is Caring. Training synchronization has different communication and consistency requirements.

---

## 24. Required implementation files

When training implementation begins, prefer a dedicated tree such as:

```text
training/crypto/
├── README.md
├── requirements.lock
├── configs/
│   ├── baseline.yaml
│   ├── lora-v0.1.yaml
│   └── qlora-v0.1.yaml
├── data/
│   ├── schema.json
│   ├── build_dataset.py
│   └── validate_dataset.py
├── train/
│   └── sft.py
├── eval/
│   ├── benchmark.py
│   ├── metrics.py
│   └── report.py
├── registry/
│   └── model-manifest.yaml
└── experiments/
    └── README.md
```

Implementation should remain isolated from the Rust fabric until the model has passed baseline/validation gates.

---

## 25. First milestone: Crypto Model v0.1

Definition of done:

- base model downloaded and revision pinned;
- environment locked;
- dataset schema defined;
- `crypto-dataset-v0.1` generated and hashed;
- chronological train/validation/test split validated;
- frozen baseline benchmark completed;
- one controlled LoRA/QLoRA training run completed;
- adapter artifact stored;
- benchmark compared against baseline;
- regression report written;
- model registry entry created;
- Obsidian experiment memory recorded;
- no secrets in datasets/artifacts/memory;
- only then integrate into Fabric as a candidate capability.

The success criterion is **measured improvement with reproducibility**, not simply successful training.

---

## 26. Official references

The implementation must follow the current official Hugging Face documentation for the exact versions pinned in the environment:

- Qwen2.5-1.5B-Instruct model card and license: https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct citeturn688428search9
- TRL SFTTrainer: https://huggingface.co/docs/trl/sft_trainer citeturn688428search3
- TRL dataset formats: https://huggingface.co/docs/trl/dataset_formats citeturn688428search0
- TRL chat templates: https://huggingface.co/docs/trl/chat_templates citeturn688428search1
- TRL + PEFT/QLoRA: https://huggingface.co/docs/trl/peft_integration citeturn235632search0

The URLs above are references only. Runtime behavior is determined by the pinned package/model revisions used by the experiment.

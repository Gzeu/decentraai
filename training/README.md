# Model Training Lab

Pipeline for creating DecentraAI-Governor-v0.1 by adapting Qwen3.5-4B with LoRA/QLoRA.

## Quick start (smoke test, ~100 examples)

```bash
# 1. Build dataset from repo
python3 training/datasets/builders/build_corpus.py --output training/datasets/smoke_v0.jsonl --max-examples 100

# 2. Validate
python3 training/datasets/validate.py training/datasets/smoke_v0.jsonl

# 3. Train (needs GPU or Colab — NOT the VPS)
# See training/configs/qwen35_4b_lora_smoke.yaml
```

## Structure

```
training/
├── configs/           Training YAML configs (LoRA rank, LR, epochs)
├── datasets/
│   ├── builders/      Corpus extraction + example generation scripts
│   └── validate.py    Schema + quality checks
├── evaluation/        Base vs adapted comparison prompts
└── registry/          Model version records (JSON with evidence)
```

## Key rules

1. **Never train GGUF** — use HF Transformers checkpoint, export GGUF after.
2. **Redact secrets** before any data enters the dataset.
3. **Quality > quantity**: 100 verified examples beat 100k noisy ones.
4. **Negative examples matter**: teach the model what it must NOT do.
5. **VPS = orchestration only** — training runs on Desktop/GPU/Colab.

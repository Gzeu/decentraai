#!/usr/bin/env python3
"""Smoke training run: Qwen3.5-4B + LoRA on DecentraAI corpus.

Runs the FULL pipeline: load base → load dataset → SFT train → save adapter.
Designed for CPU-only (slow but proves the pipeline works end-to-end).

Usage:
  source training/venv/bin/activate  # or pip install deps
  python3 training/configs/run_smoke.py
"""

import json
import os
import sys
from pathlib import Path

# Ensure user site-packages are visible
import site
site.addsitedir(str(Path.home() / ".local" / "lib" / "python3.14" / "site-packages"))

DATASET = Path(__file__).parent.parent / "datasets" / "corpus_v0.jsonl"
OUTPUT = Path(__file__).parent.parent / "artifacts" / "smoke_output"
MODEL_NAME = "Qwen/Qwen3-0.6B"  # Small enough for CPU smoke test


def main():
    print("=== DecentraAI Training Lab — Smoke Test ===")
    print(f"dataset: {DATASET}")
    print(f"output: {OUTPUT}")
    print(f"model: {MODEL_NAME}")

    # Load dataset
    examples = []
    with open(DATASET) as f:
        for line in f:
            ex = json.loads(line)
            examples.append(ex["messages"])
    print(f"loaded {len(examples)} training examples")

    if not examples:
        sys.exit("no examples — aborting")

    try:
        from transformers import AutoModelForCausalLM, AutoTokenizer, TrainingArguments
        from trl import SFTTrainer
        from peft import LoraConfig, get_peft_model
    except ImportError as e:
        print(f"missing dependency: {e}")
        print("install with:")
        print("  pip3 install --user --break-system-packages transformers datasets peft trl torch")
        sys.exit(1)

    # Load tokenizer and model — use Qwen3-0.6B for CPU smoke test (small)
    smoke_model = "Qwen/Qwen3-0.6B"
    print(f"loading tokenizer: {smoke_model}")
    tokenizer = AutoTokenizer.from_pretrained(smoke_model, trust_remote_code=True)
    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token

    print(f"loading model: {smoke_model}")
    model = AutoModelForCausalLM.from_pretrained(
        smoke_model,
        torch_dtype="float32",  # CPU needs fp32
        device_map="cpu",
        trust_remote_code=True,
    )

    # Configure LoRA
    from peft import LoraConfig, TaskType
    lora_config = LoraConfig(
        task_type=TaskType.CAUSAL_LM,
        r=8,
        lora_alpha=16,
        lora_dropout=0.05,
        target_modules=["q_proj", "k_proj", "v_proj"],
    )
    model = get_peft_model(model, lora_config)
    model.print_trainable_parameters()

    # Prepare dataset for TRL
    from datasets import Dataset
    formatted = []
    for messages in examples[:20]:  # Limit for CPU smoke test
        formatted.append({"messages": messages})
    ds = Dataset.from_list(formatted)

    # Training arguments — minimal for CPU
    output_dir = str(OUTPUT)
    os.makedirs(output_dir, exist_ok=True)

    training_args = TrainingArguments(
        output_dir=output_dir,
        num_train_epochs=1,
        per_device_train_batch_size=1,
        gradient_accumulation_steps=1,
        learning_rate=5e-5,
        logging_steps=1,
        save_strategy="no",
        report_to="none",
        remove_unused_columns=False,
        dataloader_pin_memory=False,
    )

    # Train
    trainer = SFTTrainer(
        model=model,
        args=training_args,
        train_dataset=ds,
        processing_class=tokenizer,
    )

    print("\n=== TRAINING ===")
    trainer.train()
    print("=== TRAINING COMPLETE ===")

    # Save adapter
    adapter_path = os.path.join(output_dir, "adapter")
    model.save_pretrained(adapter_path)
    tokenizer.save_pretrained(adapter_path)
    print(f"\nadapter saved to: {adapter_path}")

    # Quick evaluation: generate one response
    print("\n=== QUICK EVAL ===")
    test_prompt = "What is Sharing is Caring in DecentraAI?"
    inputs = tokenizer(test_prompt, return_tensors="pt")
    outputs = model.generate(**inputs, max_new_tokens=50)
    response = tokenizer.decode(outputs[0], skip_special_tokens=True)
    print(f"test prompt: {test_prompt}")
    print(f"response: {response[:200]}")

    print("\n=== SMOKE TEST COMPLETE ===")


if __name__ == "__main__":
    main()

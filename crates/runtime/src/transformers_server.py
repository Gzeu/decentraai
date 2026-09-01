#!/usr/bin/env python3
"""DecentraAI Transformers Inference Server — OpenAI-compatible backend.

External subprocess (never FFI): the node spawns this process and drives it
through the existing InferenceBackend adapter. The server exposes an
OpenAI-compatible `/v1/*` surface so the same backend adapter used for
llama-server can drive Transformers models with zero runtime changes.

Design rules:
  - The engine is ALWAYS an external process (never FFI / pyo3 / cffi).
  - Secrets/prompts are never logged; only security-relevant events land in audit.
  - Models load lazily on first request (avoids blocking node startup).
  - Single-device CPU by default; GPU auto-detect if CUDA is available.

Supported model types:
  - Sentence-transformers: sentence-transformers/all-MiniLM-L6-v2, etc.
    → exposes /v1/embeddings (OpenAI-compatible)
  - Causal LM (AutoModelForCausalLM): Qwen, LLaMA, Mistral, etc.
    → exposes /v1/chat/completions, /v1/completions

Endpoints:
  GET  /health              -> 200 "ok" (used by the node's health probe)
  GET  /v1/models           -> OpenAI-compatible model list
  POST /v1/embeddings       -> OpenAI-compatible embeddings (sentence-transformers)
  POST /v1/chat/completions -> chat completion (causal LM)
  POST /v1/completions      -> text completion (causal LM)

Usage:
  python transformers_server.py --port <n> --model <model-id> [--device cpu|cuda|auto]
"""

import argparse
import json
import math
import os
import sys
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, os.environ.get("PYTHONPATH", ""))

# ── Global state ────────────────────────────────────────────────────────────

_MODEL_ID = None
_DEVICE = "cpu"
_MODEL = None
_TOKENIZER = None
_MODEL_TYPE = None  # "sentence-transformer" or "causal-lm"
_MODEL_LOAD_TIME = None
_EMBEDDING_DIM = None


def _detect_model_type(model_id):
    """Detect whether the model is a sentence-transformer or causal LM.
    
    Checks the model's config.json for sentence-transformers metadata.
    """
    try:
        from transformers import AutoConfig
        config = AutoConfig.from_pretrained(model_id, trust_remote_code=True)
        # sentence-transformers models have sentence_transformers in their config
        if hasattr(config, "sentence_transformers_config"):
            return "sentence-transformer"
        # Also check by trying to detect known embedding architectures
        # Some models use a different attribute name
        cfg_dict = config.to_dict() if hasattr(config, "to_dict") else {}
        if "sentence_transformers" in cfg_dict:
            return "sentence-transformer"
        # Check model_type hints
        model_type = getattr(config, "model_type", "")
        if model_type in ("bert", "roberta", "distilbert", "sentence-bert"):
            return "sentence-transformer"
    except Exception:
        pass
    
    # Default: try sentence-transformers first, then causal-lm
    return "causal-lm"


def _load_model():
    """Lazy-load the model on first request."""
    global _MODEL, _TOKENIZER, _MODEL_TYPE, _MODEL_LOAD_TIME, _EMBEDDING_DIM
    if _MODEL is not None:
        return

    import torch

    device = _DEVICE
    if device == "auto":
        device = "cuda" if torch.cuda.is_available() else "cpu"

    print(f"[transformers_server] detecting model type for {_MODEL_ID}...", flush=True)
    _MODEL_TYPE = _detect_model_type(_MODEL_ID)
    print(f"[transformers_server] model type: {_MODEL_TYPE}", flush=True)

    t0 = time.time()

    if _MODEL_TYPE == "sentence-transformer":
        from sentence_transformers import SentenceTransformer
        print(f"[transformers_server] loading SentenceTransformer {_MODEL_ID} on {device}...", flush=True)
        _MODEL = SentenceTransformer(_MODEL_ID, device=device)
        _EMBEDDING_DIM = _MODEL.get_sentence_embedding_dimension()
        print(f"[transformers_server] embedding dimension: {_EMBEDDING_DIM}", flush=True)
    else:
        from transformers import AutoModelForCausalLM, AutoTokenizer
        print(f"[transformers_server] loading CausalLM {_MODEL_ID} on {device}...", flush=True)
        _TOKENIZER = AutoTokenizer.from_pretrained(_MODEL_ID, trust_remote_code=True)
        _MODEL = AutoModelForCausalLM.from_pretrained(
            _MODEL_ID,
            torch_dtype=torch.float16 if device == "cuda" else torch.float32,
            device_map=device if device == "cuda" else None,
            trust_remote_code=True,
        )
        if device == "cpu":
            _MODEL = _MODEL.to("cpu")
        _MODEL.eval()

    _MODEL_LOAD_TIME = time.time() - t0
    print(f"[transformers_server] model loaded in {_MODEL_LOAD_TIME:.1f}s", flush=True)


def _embed(texts):
    """Run embedding inference using the loaded sentence-transformer model."""
    _load_model()
    embeddings = _MODEL.encode(texts, convert_to_numpy=True)
    return embeddings.tolist()


def _chat_completion(messages, max_tokens, temperature, top_p):
    """Run chat completion using the loaded model."""
    _load_model()

    import torch

    # Apply chat template if the tokenizer supports it
    if hasattr(_TOKENIZER, "apply_chat_template"):
        prompt = _TOKENIZER.apply_chat_template(
            messages, tokenize=False, add_generation_prompt=True
        )
    else:
        # Fallback: concatenate messages
        parts = []
        for msg in messages:
            role = msg.get("role", "user")
            content = msg.get("content", "")
            parts.append(f"<|{role}|>\n{content}")
        parts.append("<|assistant|>\n")
        prompt = "\n".join(parts)

    inputs = _TOKENIZER(prompt, return_tensors="pt")
    if _MODEL.device.type != "cpu":
        inputs = {k: v.to(_MODEL.device) for k, v in inputs.items()}

    input_len = inputs["input_ids"].shape[1]

    with torch.no_grad():
        outputs = _MODEL.generate(
            **inputs,
            max_new_tokens=max_tokens,
            temperature=max(temperature, 0.01),
            top_p=top_p,
            do_sample=temperature > 0,
            pad_token_id=_TOKENIZER.eos_token_id,
        )

    generated = outputs[0][input_len:]
    text = _TOKENIZER.decode(generated, skip_special_tokens=True)
    tokens_used = outputs.shape[1]

    finish_reason = "stop"
    if outputs.shape[1] - input_len >= max_tokens:
        finish_reason = "length"

    return text, tokens_used, finish_reason


# ── HTTP Handler ────────────────────────────────────────────────────────────

class Handler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        # Suppress default access logs (prompts must not be logged).
        pass

    def _send_json(self, status, data):
        body = json.dumps(data).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health":
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.end_headers()
            self.wfile.write(b"ok")
        elif self.path == "/v1/models":
            data = {
                "object": "list",
                "data": [
                    {
                        "id": _MODEL_ID or "local-model",
                        "object": "model",
                        "created": int(_MODEL_LOAD_TIME or time.time()),
                        "owned_by": "decentraai-transformers",
                        "permission": [],
                    }
                ],
            }
            self._send_json(200, data)
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        if self.path == "/v1/embeddings":
            self._handle_embeddings()
        elif self.path == "/v1/chat/completions":
            self._handle_chat_completions()
        elif self.path == "/v1/completions":
            self._handle_completions()
        else:
            self.send_response(404)
            self.end_headers()

    def _handle_embeddings(self):
        try:
            length = int(self.headers.get("Content-Length", 0))
            body = self.rfile.read(length)
            req = json.loads(body)
        except (json.JSONDecodeError, ValueError):
            self._send_json(400, {"error": "invalid JSON"})
            return

        input_data = req.get("input", "")
        model = req.get("model", _MODEL_ID or "local-model")

        # Normalize input to list of strings
        if isinstance(input_data, str):
            texts = [input_data]
        elif isinstance(input_data, list):
            texts = input_data
        else:
            self._send_json(400, {"error": "input must be a string or array of strings"})
            return

        if not texts:
            self._send_json(400, {"error": "input required"})
            return

        # Check model type — only sentence-transformers support embeddings
        _load_model()
        if _MODEL_TYPE != "sentence-transformer":
            self._send_json(400, {
                "error": f"model '{_MODEL_ID}' is not an embedding model "
                         f"(detected as {_MODEL_TYPE}). Use a sentence-transformer model."
            })
            return

        try:
            t0 = time.time()
            embeddings = _embed(texts)
            duration_ms = (time.time() - t0) * 1000
        except Exception as e:
            self._send_json(500, {"error": str(e)})
            return

        data = []
        for i, emb in enumerate(embeddings):
            data.append({
                "object": "embedding",
                "embedding": emb,
                "index": i,
            })

        result = {
            "object": "list",
            "data": data,
            "model": model,
            "usage": {
                "prompt_tokens": sum(len(t.split()) for t in texts),
                "total_tokens": sum(len(t.split()) for t in texts),
            },
        }
        self._send_json(200, result)

    def _handle_chat_completions(self):
        try:
            length = int(self.headers.get("Content-Length", 0))
            body = self.rfile.read(length)
            req = json.loads(body)
        except (json.JSONDecodeError, ValueError):
            self._send_json(400, {"error": "invalid JSON"})
            return

        messages = req.get("messages", [])
        max_tokens = min(req.get("max_tokens", 512), 4096)
        temperature = req.get("temperature", 0.7)
        top_p = req.get("top_p", 0.9)
        stream = req.get("stream", False)
        model = req.get("model", _MODEL_ID or "local-model")

        if not messages:
            self._send_json(400, {"error": "messages required"})
            return

        # Non-streaming only for now (streaming can be added later)
        if stream:
            self._send_json(400, {"error": "streaming not yet supported by Transformers backend"})
            return

        _load_model()
        if _MODEL_TYPE == "sentence-transformer":
            self._send_json(400, {
                "error": f"model '{_MODEL_ID}' is an embedding model, not a chat model. "
                         "Use /v1/embeddings instead."
            })
            return

        try:
            text, tokens_used, finish_reason = _chat_completion(
                messages, max_tokens, temperature, top_p
            )
        except Exception as e:
            self._send_json(500, {"error": str(e)})
            return

        completion_id = f"chatcmpl-{uuid.uuid4().hex[:12]}"
        data = {
            "id": completion_id,
            "object": "chat.completion",
            "created": int(time.time()),
            "model": model,
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": text},
                    "finish_reason": finish_reason,
                }
            ],
            "usage": {
                "prompt_tokens": 0,
                "completion_tokens": tokens_used,
                "total_tokens": tokens_used,
            },
        }
        self._send_json(200, data)

    def _handle_completions(self):
        """Legacy /v1/completions — wraps messages into a single prompt."""
        try:
            length = int(self.headers.get("Content-Length", 0))
            body = self.rfile.read(length)
            req = json.loads(body)
        except (json.JSONDecodeError, ValueError):
            self._send_json(400, {"error": "invalid JSON"})
            return

        prompt = req.get("prompt", "")
        max_tokens = min(req.get("max_tokens", 512), 4096)
        temperature = req.get("temperature", 0.7)
        top_p = req.get("top_p", 0.9)
        model = req.get("model", _MODEL_ID or "local-model")

        if not prompt:
            self._send_json(400, {"error": "prompt required"})
            return

        messages = [{"role": "user", "content": prompt}]
        try:
            text, tokens_used, finish_reason = _chat_completion(
                messages, max_tokens, temperature, top_p
            )
        except Exception as e:
            self._send_json(500, {"error": str(e)})
            return

        completion_id = f"cmpl-{uuid.uuid4().hex[:12]}"
        data = {
            "id": completion_id,
            "object": "text_completion",
            "created": int(time.time()),
            "model": model,
            "choices": [
                {
                    "text": text,
                    "index": 0,
                    "finish_reason": finish_reason,
                }
            ],
            "usage": {
                "prompt_tokens": 0,
                "completion_tokens": tokens_used,
                "total_tokens": tokens_used,
            },
        }
        self._send_json(200, data)


def main():
    global _MODEL_ID, _DEVICE

    parser = argparse.ArgumentParser(description="DecentraAI Transformers Inference Server")
    parser.add_argument("--port", type=int, required=True, help="Port to listen on")
    parser.add_argument("--host", default="127.0.0.1", help="Host to bind (default: loopback)")
    parser.add_argument("--model", required=True, help="HuggingFace model ID or local path")
    parser.add_argument("--device", default="cpu", choices=["cpu", "cuda", "auto"],
                        help="Device to run on (default: cpu)")
    args = parser.parse_args()

    _MODEL_ID = args.model
    _DEVICE = args.device

    server = ThreadingHTTPServer((args.host, args.port), Handler)
    print(f"[transformers_server] listening on {args.host}:{args.port}", flush=True)
    print(f"[transformers_server] model: {_MODEL_ID}", flush=True)
    print(f"[transformers_server] device: {_DEVICE}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\n[transformers_server] shutting down", flush=True)
        server.server_close()


if __name__ == "__main__":
    main()

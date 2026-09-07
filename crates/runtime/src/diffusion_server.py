#!/usr/bin/env python3
"""DecentraAI Diffusion server — Stable Diffusion text-to-image generation.

External subprocess (never FFI): the node spawns this process and proxies
results through the authenticated `/v1/diffusion` API. Prompts are accepted
as JSON in the request body; generated images are returned as base64 PNG.
Prompts and outputs are never logged.

Uses HuggingFace `diffusers` library with a Stable Diffusion model.
Models are downloaded on first run into `<data_dir>/tools/diffusion/models/`.
Runs on CPU by default; GPU (CUDA) is used automatically when available.

Endpoints:
  GET  /health            -> 200 "ok" (used by the node's health probe)
  POST /v1/diffusion/t2i  -> {"image_b64": "...", "seed": N}
                             (body: {"prompt": "...", "negative_prompt": "...",
                              "width": 512, "height": 512, "steps": 20,
                              "guidance_scale": 7.5, "seed": -1})
  GET  /v1/diffusion/models -> {"models": [...]} (list available models)
"""

import argparse
import base64
import io
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, os.environ.get("PYTHONPATH", ""))

# Lazy imports so /health works even if the dependency is missing.
_pipe = None
_model_id = None


def get_pipeline(model_id: str):
    global _pipe, _model_id
    if _pipe is not None and _model_id == model_id:
        return _pipe
    import torch
    from diffusers import StableDiffusionPipeline, DPMSolverMultistepScheduler

    device = "cuda" if torch.cuda.is_available() else "cpu"
    dtype = torch.float16 if device == "cuda" else torch.float32

    print(f"[diffusion] loading model {model_id} on {device} ({dtype})", flush=True)
    pipe = StableDiffusionPipeline.from_pretrained(
        model_id,
        torch_dtype=dtype,
        safety_checker=None,
        requires_safety_checker=False,
    )
    pipe.scheduler = DPMSolverMultistepScheduler.from_config(pipe.scheduler.config)
    pipe = pipe.to(device)

    # Memory optimizations
    if device == "cpu":
        pipe.enable_attention_slicing()

    _pipe = pipe
    _model_id = model_id
    print(f"[diffusion] model loaded on {device}", flush=True)
    return _pipe


def run_t2i(
    prompt: str,
    negative_prompt: str,
    width: int,
    height: int,
    steps: int,
    guidance_scale: float,
    seed: int,
    model_id: str,
):
    import torch

    pipe = get_pipeline(model_id)

    generator = None
    if seed >= 0:
        generator = torch.Generator(device=pipe.device).manual_seed(seed)
    else:
        seed = torch.randint(0, 2**31, (1,)).item()
        generator = torch.Generator(device=pipe.device).manual_seed(seed)

    result = pipe(
        prompt=prompt,
        negative_prompt=negative_prompt or None,
        width=width,
        height=height,
        num_inference_steps=steps,
        guidance_scale=guidance_scale,
        generator=generator,
    )

    image = result.images[0]
    buf = io.BytesIO()
    image.save(buf, format="PNG")
    image_b64 = base64.b64encode(buf.getvalue()).decode("utf-8")
    return {"image_b64": image_b64, "seed": seed}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        pass  # keep stdout clean

    def _send(self, code, payload: dict, content_type="application/json"):
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health":
            self._send(200, {"status": "ok"})
        elif self.path == "/v1/diffusion/models":
            self._send(200, {"models": [_model_id] if _model_id else []})
        else:
            self._send(404, {"error": "not found"})

    def do_POST(self):
        if self.path != "/v1/diffusion/t2i":
            self._send(404, {"error": "not found"})
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            body = json.loads(self.rfile.read(length).decode("utf-8"))
            prompt = body.get("prompt", "")
            if not prompt:
                self._send(400, {"error": "prompt is required"})
                return
            result = run_t2i(
                prompt=prompt,
                negative_prompt=body.get("negative_prompt", ""),
                width=min(body.get("width", 512), 1024),
                height=min(body.get("height", 512), 1024),
                steps=min(body.get("steps", 20), 50),
                guidance_scale=body.get("guidance_scale", 7.5),
                seed=body.get("seed", -1),
                model_id=body.get("model", _model_id or "stable-diffusion-v1-5/stable-diffusion-v1-5"),
            )
            self._send(200, result)
        except Exception as exc:  # noqa: BLE001 — surface upstream error safely
            self._send(500, {"error": f"diffusion failed: {exc}"})


def main():
    parser = argparse.ArgumentParser(description="DecentraAI Diffusion server")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument(
        "--model",
        default="stable-diffusion-v1-5/stable-diffusion-v1-5",
        help="HuggingFace model ID for Stable Diffusion",
    )
    args = parser.parse_args()

    global _model_id
    _model_id = args.model

    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"[diffusion] listening on 127.0.0.1:{args.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()

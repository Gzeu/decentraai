#!/usr/bin/env python3
"""DecentraAI HF Skills server — small transformers pipelines as tools.

External subprocess (never FFI): the node spawns this process and proxies
results through the authenticated `/v1/skills/<id>` API. Each skill maps to a
small CPU-friendly HuggingFace model (sentence-transformers / opus-mt /
distilbart). Prompts and outputs are never logged.

Skills provided (all tiny models, CPU int8):
  sentiment      -> "positive"/"negative"/"neutral" + score
                   (distilbert-base-uncased-finetuned-sst-2-english)
  ner            -> named entities with labels (dslim/bert-base-NER)
  summarize      -> extractive/abstractive summary (sshleifer/distilbart-cnn-12-6)
  translate_ro_en-> Romanian -> English  (Helsinki-NLP/opus-mt-ro-en)
  translate_en_ro-> English -> Romanian  (Helsinki-NLP/opus-mt-en-ro)

Models download on first use into the HF cache pointed at
`<data_dir>/tools/skills/models` by the setup script.

Endpoints:
  GET  /health             -> 200 "ok" (used by the node's health probe)
  GET  /v1/skills          -> {"skills": ["sentiment", ...], "loaded": {...}}
  POST /v1/skills/<id>     -> skill-specific JSON
                             (body: {"text": "..."})
"""

import argparse
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, os.environ.get("PYTHONPATH", ""))

# Lazy pipelines: /health and /v1/skills work even if no model is loaded yet.
_PIPELINES = {}
_MODEL_NAMES = {
    "sentiment": "distilbert-base-uncased-finetuned-sst-2-english",
    "ner": "dslim/bert-base-NER",
    "summarize": "sshleifer/distilbart-cnn-12-6",
    "translate_ro_en": "Helsinki-NLP/opus-mt-ro-en",
    "translate_en_ro": "Helsinki-NLP/opus-mt-en-ro",
}
_SKILLS = list(_MODEL_NAMES.keys())


def _get_pipeline(skill: str):
    if skill not in _MODEL_NAMES:
        raise ValueError(f"unknown skill '{skill}' (available: {_SKILLS})")
    if skill not in _PIPELINES:
        from transformers import pipeline

        if skill.startswith("translate"):
            # opus-mt models are translation pipelines with explicit src/tgt.
            src, tgt = ("ro", "en") if skill == "translate_ro_en" else ("en", "ro")
            _PIPELINES[skill] = pipeline(
                "translation",
                model=_MODEL_NAMES[skill],
                tokenizer=_MODEL_NAMES[skill],
                src_lang=src,
                tgt_lang=tgt,
                device=-1,  # CPU
            )
        else:
            task = {
                "sentiment": "sentiment-analysis",
                "ner": "token-classification",
                "summarize": "summarization",
            }[skill]
            _PIPELINES[skill] = pipeline(task, model=_MODEL_NAMES[skill], device=-1)
    return _PIPELINES[skill]


def run_skill(skill: str, text: str):
    pipe = _get_pipeline(skill)
    if skill == "sentiment":
        out = pipe(text)[0]
        return {"label": out["label"], "score": round(out["score"], 4)}
    if skill == "ner":
        ents = pipe(text)
        return {
            "entities": [
                {"text": e["word"], "label": e["entity"], "score": round(e["score"], 4)}
                for e in ents
            ]
        }
    if skill == "summarize":
        out = pipe(text, max_length=150, min_length=30, do_sample=False)[0]
        return {"summary": out["summary_text"]}
    if skill == "translate_ro_en":
        out = pipe(text)[0]
        return {"translation": out["translation_text"]}
    if skill == "translate_en_ro":
        out = pipe(text)[0]
        return {"translation": out["translation_text"]}
    raise ValueError(f"unhandled skill '{skill}'")


class Handler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        pass  # keep stdout clean

    def _send(self, code, payload, content_type="application/json"):
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/health":
            self._send(200, {"status": "ok"})
        elif self.path == "/v1/skills":
            self._send(200, {"skills": _SKILLS, "loaded": sorted(_PIPELINES.keys())})
        else:
            self._send(404, {"error": "not found"})

    def do_POST(self):
        if not self.path.startswith("/v1/skills/"):
            self._send(404, {"error": "not found"})
            return
        skill = self.path[len("/v1/skills/"):].split("/")[0]
        try:
            length = int(self.headers.get("Content-Length", "0"))
            body = json.loads(self.rfile.read(length).decode("utf-8"))
            text = body.get("text", "")
            if not text or not text.strip():
                self._send(400, {"error": "text is required"})
                return
            if len(text) > 32_000:
                self._send(400, {"error": "text exceeds 32000 chars"})
                return
            result = run_skill(skill, text)
            self._send(200, result)
        except ValueError as exc:
            self._send(400, {"error": str(exc)})
        except Exception as exc:  # noqa: BLE001 — surface upstream error safely
            self._send(500, {"error": f"{skill} failed: {exc}"})


def main():
    parser = argparse.ArgumentParser(description="DecentraAI HF Skills server")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--skills", default=",".join(_SKILLS),
                        help="comma-separated skills to enable")
    args = parser.parse_args()

    enabled = [s.strip() for s in args.skills.split(",") if s.strip()]
    for s in enabled:
        if s not in _MODEL_NAMES:
            print(f"error: unknown skill '{s}'", file=sys.stderr)
            sys.exit(2)
    print(f"enabled skills: {enabled}", flush=True)

    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.serve_forever()


if __name__ == "__main__":
    main()
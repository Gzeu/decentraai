#!/usr/bin/env python3
"""DecentraAI OCR server — RapidOCR (ONNX Runtime) text extraction.

External subprocess (never FFI): the node spawns this process and proxies
results through the authenticated `/v1/ocr` API. Images are accepted as
base64 in the JSON body; the extracted text is returned as-is. Prompts and
outputs are never logged.

RapidOCR is a pure-Python + onnxruntime pipeline (PP-OCRv4) that runs well
on CPU and needs no GPU. Models are bundled in the wheel; no separate
download. Installed into `<data_dir>/tools/ocr/venv` by the setup script.

Endpoints:
  GET  /health  -> 200 "ok" (used by the node's health probe)
  POST /v1/ocr  -> {"text": "...", "boxes": [...], "lines": [...]}
                   (body: {"image_b64": "...", "lang": "en"})
"""

import argparse
import base64
import io
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, os.environ.get("PYTHONPATH", ""))

# Lazy import so /health works even if the dependency is missing.
_ocr = None


def get_ocr():
    global _ocr
    if _ocr is None:
        from rapidocr_onnxruntime import RapidOCR

        _ocr = RapidOCR()
    return _ocr


def run_ocr(image_bytes: bytes, lang: str):
    engine = get_ocr()
    import numpy as np
    from PIL import Image

    img = Image.open(io.BytesIO(image_bytes)).convert("RGB")
    arr = np.asarray(img)
    result, _ = engine(arr)
    if not result:
        return {"text": "", "lines": [], "boxes": []}
    lines = []
    boxes = []
    for item in result:
        box = item[0] if isinstance(item, (list, tuple)) else []
        text = item[1] if isinstance(item, (list, tuple)) and len(item) > 1 else ""
        lines.append(text)
        boxes.append(box)
    return {"text": "\n".join(lines), "lines": lines, "boxes": boxes}


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
        else:
            self._send(404, {"error": "not found"})

    def do_POST(self):
        if self.path != "/v1/ocr":
            self._send(404, {"error": "not found"})
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            body = json.loads(self.rfile.read(length).decode("utf-8"))
            image_b64 = body.get("image_b64", "")
            lang = body.get("lang", "en")
            if not image_b64:
                self._send(400, {"error": "image_b64 is required"})
                return
            image_bytes = base64.b64decode(image_b64)
            result = run_ocr(image_bytes, lang)
            self._send(200, result)
        except Exception as exc:  # noqa: BLE001 — surface upstream error safely
            self._send(500, {"error": f"ocr failed: {exc}"})


def main():
    parser = argparse.ArgumentParser(description="DecentraAI OCR server")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--lang", default="en")
    args = parser.parse_args()

    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.serve_forever()


if __name__ == "__main__":
    main()
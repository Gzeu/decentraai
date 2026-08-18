#!/usr/bin/env python3
"""DecentraAI TTS server — Piper VITS voice synthesis (Romanian-capable).

External subprocess (never FFI): the node spawns this process and proxies
audio through the authenticated `/v1/tts` API. Prompts and outputs are
never logged. Models live in the node data dir under `tts/models/piper-ro/`.

Piper (VITS + espeak-ng, embedded in the wheel) supports Romanian natively:
`ro_RO-raluca-high` (female, WER 2.2%), `ro_RO-lili-high`, `ro_RO-mihai-medium`
(male). Non-autoregressive — no hallucinations, reliable on CPU.

Endpoints:
  GET  /health   -> 200 "ok" (used by the node's health probe)
  POST /v1/tts   -> audio/wav (body: {"text": "...", "speed": 1.0})
"""

import argparse
import io
import json
import os
import sys
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# Import the venv's installed packages regardless of the interpreter the
# node used to launch us (the node passes PYTHONPATH=/venv site-packages).
sys.path.insert(0, os.environ.get("PYTHONPATH", ""))

from piper import PiperVoice, SynthesisConfig  # noqa: E402

VOICE = None


class TtsHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def do_GET(self):
        if self.path == "/health":
            body = b"ok"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()

    def do_POST(self):
        if self.path != "/v1/tts":
            self.send_response(404)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        length = int(self.headers.get("Content-Length", 0))
        try:
            payload = json.loads(self.rfile.read(length) or b"{}")
        except (ValueError, UnicodeDecodeError):
            self._error(400, "invalid JSON body")
            return
        text = str(payload.get("text", "")).strip()
        if not text:
            self._error(400, "text is required")
            return
        if len(text) > 4096:
            self._error(400, "text exceeds 4096 chars")
            return
        # speed > 1 = faster -> shorter phoneme length scale.
        speed = float(payload.get("speed", 1.0) or 1.0)
        length_scale = 1.0 / max(0.5, min(speed, 2.0))
        try:
            buf = io.BytesIO()
            with wave.open(buf, "wb") as w:
                VOICE.synthesize_wav(
                    text,
                    w,
                    syn_config=SynthesisConfig(length_scale=length_scale),
                )
        except Exception as exc:  # phonemizer/synthesis failures
            self._error(400, f"TTS synthesis failed: {exc}")
            return
        audio = buf.getvalue()
        self.send_response(200)
        self.send_header("Content-Type", "audio/wav")
        self.send_header("Content-Length", str(len(audio)))
        self.send_header("X-TTS-Voice", args_voice)
        self.end_headers()
        self.wfile.write(audio)

    def _error(self, code, message):
        body = json.dumps({"error": {"message": message}}).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


args_voice = "ro_RO-raluca-high"


def main():
    global VOICE, args_voice
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", required=True)
    parser.add_argument("--config", required=True)
    parser.add_argument("--port", type=int, default=8731)
    parser.add_argument("--voice", default="ro_RO-raluca-high")
    args = parser.parse_args()

    args_voice = args.voice
    model_path = args.model or os.environ.get("PIPER_MODEL")
    config_path = args.config or os.environ.get("PIPER_CONFIG")
    if not model_path or not config_path:
        sys.stderr.write("TTS: --model and --config are required\n")
        sys.exit(2)
    if not os.path.exists(model_path) or not os.path.exists(config_path):
        sys.stderr.write(f"TTS: missing model/config at {model_path} / {config_path}\n")
        sys.exit(2)

    VOICE = PiperVoice.load(model_path, config_path=config_path)
    # Warm up so the first user request does not pay the load latency.
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        VOICE.synthesize_wav("Fabricul DecentraAI este online.", w)
    server = ThreadingHTTPServer(("127.0.0.1", args.port), TtsHandler)
    sys.stderr.write(f"TTS ready on 127.0.0.1:{args.port} voice={args_voice}\n")
    sys.stderr.flush()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
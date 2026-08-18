#!/usr/bin/env python3
"""DecentraAI TTS server — Kokoro-82M ONNX voice synthesis.

External subprocess (never FFI): the node spawns this process and proxies
audio through the authenticated `/v1/tts` API. Prompts and outputs are
never logged. The model/voices live in the node data dir under `tts/`.

Endpoints:
  GET  /health   -> 200 "ok" (used by the node's health probe)
  POST /v1/tts   -> audio/wav (body: {"text": "...", "voice": "...", "speed": 1.0})
"""

import argparse
import base64
import io
import json
import os
import sys
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# Import the venv's installed packages regardless of the interpreter the
# node used to launch us (the node passes PYTHONPATH=/venv site-packages).
sys.path.insert(0, os.environ.get("PYTHONPATH", ""))

from kokoro_onnx import Kokoro  # noqa: E402
import numpy as np  # noqa: E402
import soundfile  # noqa: E402  (ensures libsndfile is available for wave io)

MODEL = None
VOICES = None
VOICE_DEFAULT = "af_heart"


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
        voice = str(payload.get("voice", VOICE_DEFAULT) or VOICE_DEFAULT)
        speed = float(payload.get("speed", 1.0) or 1.0)
        try:
            samples, sr = MODEL.create(text, voice=voice, speed=speed)
        except Exception as exc:  # kokoro raises for unknown voices etc.
            self._error(400, f"TTS synthesis failed: {exc}")
            return
        buf = io.BytesIO()
        # Kokoro returns float32 mono at 24 kHz; encode as 16-bit PCM WAV.
        pcm = (np.clip(samples, -1.0, 1.0) * 32767).astype(np.int16)
        with wave.open(buf, "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(int(sr))
            w.writeframes(pcm.tobytes())
        audio = buf.getvalue()
        self.send_response(200)
        self.send_header("Content-Type", "audio/wav")
        self.send_header("Content-Length", str(len(audio)))
        self.send_header("X-TTS-Voice", voice)
        self.send_header("X-TTS-Sample-Rate", str(sr))
        self.end_headers()
        self.wfile.write(audio)

    def _error(self, code, message):
        body = json.dumps({"error": {"message": message}}).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main():
    global MODEL, VOICES, VOICE_DEFAULT
    parser = argparse.ArgumentParser()
    parser.add_argument("--model", default=None)
    parser.add_argument("--voices", default=None)
    parser.add_argument("--port", type=int, default=8731)
    parser.add_argument("--voice", default="af_heart")
    args = parser.parse_args()

    VOICE_DEFAULT = args.voice
    model_path = args.model or os.environ.get("KOKORO_MODEL")
    voices_path = args.voices or os.environ.get("KOKORO_VOICES")
    if not model_path or not voices_path:
        sys.stderr.write("TTS: --model and --voices are required\n")
        sys.exit(2)
    if not os.path.exists(model_path) or not os.path.exists(voices_path):
        sys.stderr.write(f"TTS: missing model/voices at {model_path} / {voices_path}\n")
        sys.exit(2)

    MODEL = Kokoro(model_path, voices_path)
    # Warm up so the first user request does not pay the load latency.
    MODEL.create("DecentraAI voice online.", voice=VOICE_DEFAULT, speed=1.0)
    server = ThreadingHTTPServer(("127.0.0.1", args.port), TtsHandler)
    sys.stderr.write(f"TTS ready on 127.0.0.1:{args.port}\n")
    sys.stderr.flush()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
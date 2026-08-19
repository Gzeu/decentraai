#!/usr/bin/env python3
"""DecentraAI STT server — faster-whisper (CTranslate2) speech-to-text.

External subprocess (never FFI): the node spawns this process and proxies
transcriptions through the authenticated `/v1/stt` API. Audio is accepted as
base64 WAV/MP3/OGG in the JSON body; the transcript is returned as-is.
Prompts and outputs are never logged.

faster-whisper uses CTranslate2 and runs on CPU (float32). The tiny/base
models download on first use into the HF cache (or into
`<data_dir>/tools/stt/models` when HF_HOME is pointed there by the setup
script). Romanian is supported (`lang: ro`).

Endpoints:
  GET  /health  -> 200 "ok" (used by the node's health probe)
  POST /v1/stt  -> {"text": "...", "language": "...", "duration_s": 1.23}
                   (body: {"audio_b64": "...", "lang": null})
"""

import argparse
import base64
import io
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, os.environ.get("PYTHONPATH", ""))

_model = None
_model_name = None


def get_model(name: str):
    global _model, _model_name
    if _model is None or _model_name != name:
        from faster_whisper import WhisperModel

        _model = WhisperModel(name, device="cpu", compute_type="int8")
        _model_name = name
    return _model


def run_stt(audio_bytes: bytes, lang: str | None, model: str):
    import numpy as np
    import wave

    # Accept WAV directly; other formats are decoded by faster-whisper's
    # bundled ffmpeg via the file-like buffer.
    try:
        with wave.open(io.BytesIO(audio_bytes), "rb") as wav:
            sr = wav.getframerate()
            frames = wav.readframes(wav.getnframes())
            audio = np.frombuffer(frames, dtype=np.int16).astype(np.float32) / 32768.0
            duration_s = len(audio) / sr
    except Exception:
        # Not a WAV — hand the raw bytes to faster-whisper (it shells out to
        # ffmpeg if needed; the venv should have ffmpeg installed).
        engine = get_model(model)
        segments, info = engine.transcribe(
            io.BytesIO(audio_bytes), language=lang, beam_size=5
        )
        text = "".join(seg.text for seg in segments)
        return {"text": text.strip(), "language": info.language, "duration_s": info.duration}

    engine = get_model(model)
    segments, info = engine.transcribe(audio, language=lang, beam_size=5)
    text = "".join(seg.text for seg in segments)
    return {
        "text": text.strip(),
        "language": info.language,
        "duration_s": info.duration if info.duration else duration_s,
    }


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
        if self.path != "/v1/stt":
            self._send(404, {"error": "not found"})
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            body = json.loads(self.rfile.read(length).decode("utf-8"))
            audio_b64 = body.get("audio_b64", "")
            lang = body.get("lang")
            model = body.get("model", "base")
            if not audio_b64:
                self._send(400, {"error": "audio_b64 is required"})
                return
            audio_bytes = base64.b64decode(audio_b64)
            result = run_stt(audio_bytes, lang, model)
            self._send(200, result)
        except Exception as exc:  # noqa: BLE001 — surface upstream error safely
            self._send(500, {"error": f"stt failed: {exc}"})


def main():
    parser = argparse.ArgumentParser(description="DecentraAI STT server")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--model", default="base")
    args = parser.parse_args()

    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.serve_forever()


if __name__ == "__main__":
    main()
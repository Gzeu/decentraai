#!/usr/bin/env python3
"""Trace Collection Phase — real-workload driver.

Drives REAL inference through the node's OpenAI-compatible API so the live
planner produces real SelectionTraces under varied conditions:
  - local vs remote placement (model served here vs on the peer),
  - KV locality (multi-turn continuations sharing a session_id),
  - streaming and non-streaming,
  - multiple models.
Observational only: standard chat calls, no routing/config changes.

stdlib-only.
"""
import argparse
import json
import sys
import time
import urllib.request


def chat(url: str, token: str, body: dict, timeout: float = 120.0):
    req = urllib.request.Request(
        f"{url}/v1/chat/completions",
        data=json.dumps(body).encode("utf-8"),
        headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read().decode("utf-8"))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--url", required=True)
    ap.add_argument("--token-file", required=True)
    ap.add_argument("--models", required=True, help="comma-separated model file names")
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--session-turns", type=int, default=3)
    ap.add_argument("--max-tokens", type=int, default=24)
    args = ap.parse_args()

    with open(args.token_file, encoding="utf-8") as f:
        token = f.read().strip()
    models = [m.strip() for m in args.models.split(",") if m.strip()]
    ok = fail = 0

    def run(tag: str, body: dict):
        nonlocal ok, fail
        try:
            t0 = time.time()
            resp = chat(args.url, token, body)
            dt = time.time() - t0
            content = ""
            if resp.get("choices"):
                content = resp["choices"][0].get("message", {}).get("content", "")
            print(f"[{tag}] {dt:.1f}s ok={bool(content)} usage={resp.get('usage')}", flush=True)
            ok += 1
        except Exception as e:  # noqa: BLE001 — workload driver must keep going
            print(f"[{tag}] FAIL: {e}", flush=True)
            fail += 1

    for r in range(args.rounds):
        for mi, model in enumerate(models):
            # 1) plain non-streaming (cold)
            run(f"r{r}-m{mi}-plain", {
                "model": model,
                "messages": [{"role": "user", "content": "Reply with the word READY."}],
                "max_tokens": args.max_tokens,
                "stream": False,
            })
            # 2) streaming
            run(f"r{r}-m{mi}-stream", {
                "model": model,
                "messages": [{"role": "user", "content": "Count from 1 to 5."}],
                "max_tokens": args.max_tokens + 16,
                "stream": True,
            })
            # 3) KV locality: one session, multiple turns -> continuation affinity
            sid = f"trace-collect-r{r}-m{mi}"
            for turn in range(args.session_turns):
                run(f"r{r}-m{mi}-sess{turn}", {
                    "model": model,
                    "session_id": sid,
                    "messages": [
                        {"role": "user",
                         "content": f"Turn {turn}: remember the number {turn + 7}. Reply briefly."}
                    ],
                    "max_tokens": args.max_tokens,
                    "stream": False,
                })
        time.sleep(1.0)

    print(f"done: ok={ok} fail={fail}", flush=True)
    return 0 if fail == 0 else 1


if __name__ == "__main__":
    sys.exit(main())

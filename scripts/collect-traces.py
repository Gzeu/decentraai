#!/usr/bin/env python3
"""Trace Collection Phase — persistent SelectionTrace / GoldenCase collector.

Observational tooling ONLY: polls the node's read-only API surfaces and
appends to local JSONL files. Never mutates node state, never touches routing.

Two corpora:
  1. REAL TRACE CORPUS (--out): every new SelectionTrace from GET /v1/execution
     (the live legacy planner's decisions on real workloads), deduped on
     (request_id, attempt).
  2. GOLDEN CASE CORPUS (--golden-out): GoldenCase snapshots from
     GET /v1/golden-capture (research/trace-collection build) — request +
     WorkerFacts + golden trace, replayable through UnifiedSelector offline.

stdlib-only; no third-party dependencies.
"""
import argparse
import json
import sys
import time
import urllib.request


def http_json(url: str, token: str, timeout: float = 10.0):
    req = urllib.request.Request(url, headers={"Authorization": f"Bearer {token}"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.loads(r.read().decode("utf-8"))


def append_line(path: str, obj) -> None:
    with open(path, "a", encoding="utf-8") as f:
        f.write(json.dumps(obj, separators=(",", ":")) + "\n")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--url", required=True, help="node API base, e.g. http://127.0.0.1:8080")
    ap.add_argument("--token-file", required=True, help="path to runtime/api.token (mode 0600)")
    ap.add_argument("--out", required=True, help="JSONL path for the real SelectionTrace corpus")
    ap.add_argument("--label", default="node", help="short label recorded in each line")
    ap.add_argument("--duration", type=int, default=0, help="seconds to run (0 = forever)")
    ap.add_argument("--interval", type=float, default=2.0, help="poll interval seconds")
    # Phase B (optional): golden-capture polling for the replayable corpus.
    ap.add_argument("--golden-out", default="", help="JSONL path for GoldenCase corpus")
    ap.add_argument("--golden-model", default="", help="model file_name/hash to capture")
    ap.add_argument("--golden-interval", type=float, default=30.0)
    args = ap.parse_args()

    with open(args.token_file, encoding="utf-8") as f:
        token = f.read().strip()

    seen = set()
    lines_written = 0
    last_golden = 0.0
    deadline = time.time() + args.duration if args.duration > 0 else None
    print(f"collecting from {args.url} -> {args.out} (label={args.label})", flush=True)

    while True:
        try:
            data = http_json(f"{args.url}/v1/execution", token)
            for t in data.get("selection_traces", []):
                key = (t.get("request_id"), t.get("attempt"))
                if key in seen or not t.get("request_id"):
                    continue
                seen.add(key)
                t["_collected_from"] = args.label
                t["_collected_at"] = int(time.time())
                append_line(args.out, t)
                lines_written += 1
        except Exception as e:  # noqa: BLE001 — collector must survive node restarts
            print(f"warn: execution poll failed: {e}", flush=True)

        if args.golden_out and args.golden_model:
            if time.time() - last_golden >= args.golden_interval:
                last_golden = time.time()
                try:
                    url = (
                        f"{args.url}/v1/golden-capture?model_hash="
                        f"{urllib.parse.quote(args.golden_model)}"
                        f"&request_id=gc-{args.label}-{int(time.time())}"
                    )
                    gc = http_json(url, token)
                    gc["_collected_from"] = args.label
                    append_line(args.golden_out, gc)
                    lines_written += 1
                except Exception as e:  # noqa: BLE001
                    print(f"warn: golden capture failed: {e}", flush=True)

        print(f"[{args.label}] corpus lines: {lines_written}", flush=True)
        if deadline and time.time() >= deadline:
            break
        time.sleep(args.interval)
    return 0


if __name__ == "__main__":
    import urllib.parse  # noqa: F401 — used via urllib.parse.quote above

    sys.exit(main())

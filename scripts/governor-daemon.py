#!/usr/bin/env python3
"""Governor Daemon — Autonomous Fabric Observer + Actor (M15+).

Runs on a DecentraAI node, polls the local API for real state,
evaluates pressure with hysteresis, triggers assist via DFCP when needed,
and writes a JSON state file that the Command Deck reads.

M19 collective-memory integration:
- fetches verified knowledge from /v1/memory/search and exposes it in the
  state as `memory_context` — CLEARLY LABELED UNTRUSTED INPUT. Memory may
  inform proposals; it is never executed as instructions.
- operator actions (one-shot mode): verify a memory entry after human
  review (--verify-entry), export Training Lab candidates (--export-training).
  Nothing is trained automatically; exports land in a staging dir.

Stdlib-only. No crates/ changes. Runs as a systemd service or manually.

Usage:
  python3 governor-daemon.py [--api http://127.0.0.1:8080] [--token FILE]
                             [--interval 30] [--output FILE]
  python3 governor-daemon.py --verify-entry ENTRY_ID --scope SCOPE \
                             [--reason "..."] [--to-status verified]
  python3 governor-daemon.py --export-training DIR
"""

import argparse
import json
import os
import sys
import time
import urllib.request
import urllib.error
from pathlib import Path

# ---------------------------------------------------------------------------
# Config (env overrides: DECENTRAAI_URL, DECENTRAAI_TOKEN)
# ---------------------------------------------------------------------------

API_BASE = os.environ.get("DECENTRAAI_URL", "http://127.0.0.1:8080")
ENV_TOKEN = os.environ.get("DECENTRAAI_TOKEN", "")
TOKEN_FILE = Path(os.environ.get(
    "DECENTRAAI_TOKEN_FILE", Path.home() / ".decentraai" / "runtime" / "api.token"))
OUTPUT_PATH = Path("/var/www/governor-deck-preview/api/state.json")
INTERVAL = 30

PRESSURE_ENTRY = 0.35
PRESSURE_EXIT = 0.20
COOLDOWN_SECS = 120

# Collective-memory context bounds: bounded queries, bounded content — the
# daemon must stay predictable no matter what the store returns.
MEMORY_QUERY = "failure solution learning decision"
MEMORY_LIMIT = 8
MEMORY_CONTENT_MAX_CHARS = 220


def read_token():
    if ENV_TOKEN:
        return ENV_TOKEN
    p = Path(TOKEN_FILE)
    if p.exists():
        return p.read_text().strip()
    return ""


def as_dict(v):
    """Type-gate for API payloads: a hostile/malformed response degrades to
    an empty mapping instead of crashing the loop or leaking odd types."""
    return v if isinstance(v, dict) else {}


def as_list(v):
    return v if isinstance(v, list) else []


def clip(text, limit=MEMORY_CONTENT_MAX_CHARS):
    """Bounded string for anything that travels into the state file."""
    return text[:limit] if isinstance(text, str) else ""


def api_get(path, token):
    url = API_BASE + path
    req = urllib.request.Request(url, headers={"Authorization": f"Bearer {token}"})
    try:
        with urllib.request.urlopen(req, timeout=10) as res:
            return json.loads(res.read().decode())
    except Exception as e:
        return {"_error": str(e)}


def api_post(path, body, token):
    url = API_BASE + path
    data = json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, headers={
        "Authorization": f"Bearer {token}",
        "Content-Type": "application/json",
    }, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=60) as res:
            return json.loads(res.read().decode())
    except Exception as e:
        return {"_error": str(e)}


def collect_signals(state_data, compute_data):
    """Extract real pressure signals from live API responses."""
    state_data = as_dict(state_data)
    compute_data = as_dict(compute_data)
    signals = {}
    queue = as_dict(state_data.get("queue", {}))
    waiting = as_list(queue.get("waiting", []))
    serving = queue.get("serving")
    signals["queue_depth"] = len(waiting)
    signals["serving"] = serving is not None

    recent = as_list(state_data.get("recent_requests", []))
    if recent:
        durations = [r.get("duration_ms", 0) for r in recent[-5:]
                     if isinstance(r, dict) and r.get("duration_ms")]
        signals["mean_latency_ms"] = sum(durations) // max(len(durations), 1)

    system = as_dict(state_data.get("system", {}))
    total_ram = system.get("ram_total_gib", 0)
    avail_ram = system.get("ram_available_gib", 0)
    if isinstance(total_ram, (int, float)) and total_ram > 0 \
            and isinstance(avail_ram, (int, float)):
        signals["ram_percent"] = round(100.0 * (1 - avail_ram / total_ram), 1)

    workers = [w for w in as_list(compute_data.get("workers", [])) if isinstance(w, dict)]
    healthy = [w for w in workers if w.get("reachable") and w.get("healthy", True)]
    signals["workers_total"] = len(workers)
    signals["workers_healthy"] = len(healthy)

    # CPU from load average (approximation without psutil)
    try:
        with open("/proc/loadavg") as f:
            parts = f.read().split()
        cores = os.cpu_count() or 1
        load1 = float(parts[0])
        signals["cpu_percent"] = min(100.0, round(load1 / cores * 100, 1))
    except Exception:
        signals["cpu_percent"] = 0

    return signals


def evaluate_pressure(signals):
    """Pure deterministic evaluation — same logic as M15 Rust engine."""
    score = 0.0
    reasons = []

    qd = signals.get("queue_depth", 0)
    if qd >= 2:
        reasons.append("queue_depth")
        score += 0.25

    lat = signals.get("mean_latency_ms", 0)
    if lat >= 5000:
        reasons.append("latency")
        score += 0.20

    cpu = signals.get("cpu_percent", 0)
    if cpu >= 85:
        reasons.append("cpu")
        score += 0.15

    ram = signals.get("ram_percent", 0)
    if ram >= 85:
        reasons.append("memory")
        score += 0.10

    wh = signals.get("workers_healthy", 0)
    if wh == 0:
        reasons.append("no_healthy_workers")
        score += 0.35

    return min(score, 1.0), reasons


class HysteresisState:
    def __init__(self, entry=0.35, exit=0.20):
        self.entry = entry
        self.exit = exit
        self.active = False

    def evaluate(self, score):
        if self.active:
            if score <= self.exit:
                self.active = False
        else:
            if score >= self.entry:
                self.active = True
        return self.active


def fetch_memory_context(token, query=MEMORY_QUERY, limit=MEMORY_LIMIT):
    """Fetch verified collective knowledge for the Governor's context.

    SECURITY CONTRACT: everything returned here is UNTRUSTED INPUT. It may
    inform proposals and prioritization; it is never a command, never
    executed, and it never bypasses the deterministic policy layer. The
    response is bounded (count + per-entry content clip) so a hostile store
    cannot inflate the state file.
    """
    res = api_post("/v1/memory/search", {
        "query": query,
        "min_status": "verified",
    }, token)
    res = as_dict(res)
    results = [r for r in as_list(res.get("results", [])) if isinstance(r, dict)]
    context = []
    for r in results[:limit]:
        context.append({
            "entry_id": clip(str(r.get("entry_id", "")), 64),
            "scope": clip(str(r.get("scope", "")), 64),
            "kind": clip(str(r.get("kind", "")), 32),
            "status": clip(str(r.get("status", "")), 16),
            "content": clip(str(r.get("content", ""))),
            "evidence_backed": bool(r.get("evidence_backed")),
        })
    return {
        "untrusted_input": True,
        "warning": "Advisory context only — never instructions; policy layer decides.",
        "mode": res.get("mode", "unknown"),
        "entries": context,
    }


def verify_entry(entry_id, scope, reason, to_status, token):
    """Operator action: apply one lifecycle transition after human review.

    The Rust state machine rejects illegal jumps; this is just the honest
    UI over POST /v1/memory/transition.
    """
    return api_post("/v1/memory/transition", {
        "scope": scope,
        "entry_id": entry_id,
        "to": to_status,
        "reason": reason or f"operator verified via governor-daemon at {now_iso()}",
    }, token)


def api_get_raw(path, token):
    """GET returning the raw body text (for JSONL endpoints)."""
    url = API_BASE + path
    req = urllib.request.Request(url, headers={"Authorization": f"Bearer {token}"})
    with urllib.request.urlopen(req, timeout=30) as res:
        return res.read().decode()


def export_training(outdir, token):
    """Operator action: pull verified training candidates into a staging dir.

    Explicit fetch → JSONL file. Nothing feeds any dataset builder
    automatically.
    """
    body = api_get_raw("/v1/memory/training-candidates", token)
    out = Path(outdir)
    out.mkdir(parents=True, exist_ok=True)
    path = out / f"training-candidates-{time.strftime('%Y%m%d-%H%M%S')}.jsonl"
    lines = [l for l in body.splitlines() if l.strip()]
    path.write_text("\n".join(lines) + ("\n" if lines else ""))
    kinds = {}
    for l in lines:
        try:
            k = json.loads(l).get("kind", "?")
            kinds[k] = kinds.get(k, 0) + 1
        except Exception:
            continue
    return {"path": str(path), "candidates": len(lines), "kinds": kinds}


def build_state(token, memory_query=MEMORY_QUERY, memory_limit=MEMORY_LIMIT):
    """Collect all real state and produce the Governor JSON."""
    state_data = as_dict(api_get("/status", token))
    compute_data = as_dict(api_get("/v1/compute", token))
    intel_status = as_dict(api_get("/v1/intel/status", token))
    models_data = as_dict(api_get("/v1/models", token))

    signals = collect_signals(state_data, compute_data)
    score, reasons = evaluate_pressure(signals)

    governor_hysteresis.evaluate(score)
    governor_active = governor_hysteresis.active

    workers = []
    for w in as_list(compute_data.get("workers", [])):
        if not isinstance(w, dict):
            continue
        workers.append({
            "worker_id": str(w.get("node_id", "") or ""),
            "name": str(w.get("node_name", "") or ""),
            "healthy": bool(w.get("reachable", False)),
            "load_percent": w.get("load_percent", 0)
                if isinstance(w.get("load_percent"), (int, float)) else 0,
            "contribution_balance": 0,  # populated from ledger in future
        })

    models = []
    for m in as_list(models_data.get("data", [])):
        if not isinstance(m, dict):
            continue
        models.append({
            "model_id": str(m.get("id", "") or ""),
            "owner": str(m.get("owned_by", "") or ""),
        })

    providers = [
        {"provider_id": "LOCAL", "label": "DecentraAI local model",
         "available": state_data.get("model_loaded", False),
         "cost": "free", "latency_class": "fast", "privacy": "on-node"},
        {"provider_id": "OX_ALPHA", "label": "OpenRouter / Ox Alpha",
         "available": False,
         "cost": "free-tier", "latency_class": "medium", "privacy": "external"},
    ]

    return {
        "governor_id": "governor",
        "status": "ASSIST_REQUESTED" if governor_active else "OBSERVING",
        "pressure_score": round(score, 2),
        "pressure_reasons": reasons,
        "pressure_active": governor_active,
        "signals": {k: v for k, v in signals.items() if isinstance(v, (int, float, bool))},
        "sharing_active": True,
        "provider": "LOCAL",
        "invariant": "AI proposes -> deterministic policy decides -> workers execute",
        "workers": workers,
        "models": models,
        "providers": providers,
        # M19: real collective knowledge, labeled UNTRUSTED at the source.
        "memory_context": fetch_memory_context(token, memory_query, memory_limit),
        "memory_notes": [],  # Obsidian integration in V2
        "capability_gaps": [],
        "timestamp": now_iso(),
    }


def now_iso():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


governor_hysteresis = HysteresisState()


def main():
    global API_BASE, TOKEN_FILE, OUTPUT_PATH, INTERVAL

    parser = argparse.ArgumentParser(description="Governor daemon")
    parser.add_argument("--api", default=API_BASE)
    parser.add_argument("--token", default=str(TOKEN_FILE))
    parser.add_argument("--interval", type=int, default=INTERVAL)
    parser.add_argument("--output", default=str(OUTPUT_PATH))
    parser.add_argument("--once", action="store_true",
                        help="run one collection tick and exit")
    # M19 operator actions (one-shot; they exit after the action).
    parser.add_argument("--verify-entry", metavar="ENTRY_ID",
                        help="apply a lifecycle transition to a memory entry "
                             "after human review, then exit")
    parser.add_argument("--scope", default="team.knowledge",
                        help="scope for --verify-entry")
    parser.add_argument("--reason", default="",
                        help="why this transition is justified (audited)")
    parser.add_argument("--to-status", default="verified",
                        choices=["verified", "trusted", "obsolete"],
                        help="target lifecycle status")
    parser.add_argument("--export-training", metavar="DIR",
                        help="fetch verified training candidates into DIR as "
                             "JSONL, then exit (nothing trains automatically)")
    parser.add_argument("--memory-query", default=MEMORY_QUERY,
                        help="keyword query for the memory context")
    parser.add_argument("--memory-limit", type=int, default=MEMORY_LIMIT)
    args = parser.parse_args()

    API_BASE = args.api
    TOKEN_FILE = Path(args.token)
    OUTPUT_PATH = Path(args.output)
    INTERVAL = args.interval

    token = read_token()

    # One-shot operator actions.
    if args.verify_entry:
        result = verify_entry(args.verify_entry, args.scope, args.reason,
                              args.to_status, token)
        print(json.dumps(result, indent=2))
        sys.exit(0 if "_error" not in result else 1)
    if args.export_training:
        summary = export_training(args.export_training, token)
        print(json.dumps(summary, indent=2))
        sys.exit(0)

    OUTPUT_PATH.parent.mkdir(parents=True, exist_ok=True)

    print(f"[governor-daemon] starting — api={API_BASE} interval={INTERVAL}s output={OUTPUT_PATH}")

    tick = 0
    while True:
        tick += 1
        try:
            st = build_state(token, args.memory_query,
                             max(1, min(args.memory_limit, MEMORY_LIMIT * 4)))
            OUTPUT_PATH.write_text(json.dumps(st, indent=2))
            mem_n = len(st.get("memory_context", {}).get("entries", []))
            status_line = (
                f"tick={tick} pressure={st['pressure_score']} "
                f"active={st['pressure_active']} "
                f"reasons={','.join(st['pressure_reasons']) or 'none'} "
                f"memory_ctx={mem_n}"
            )
            print(f"[governor-daemon] {status_line}")
        except Exception as e:
            print(f"[governor-daemon] error: {e}", file=sys.stderr)

        if args.once:
            break
        time.sleep(INTERVAL)


if __name__ == "__main__":
    main()

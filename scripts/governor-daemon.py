#!/usr/bin/env python3
"""Governor Daemon — Autonomous Fabric Observer + Actor (M15+).

Runs on a DecentraAI node, polls the local API for real state,
evaluates pressure with hysteresis, triggers assist via DFCP when needed,
and writes a JSON state file that the Command Deck reads.

Stdlib-only. No crates/ changes. Runs as a systemd service or manually.

Usage:
  python3 governor-daemon.py [--api http://127.0.0.1:8080] [--token FILE]
                             [--interval 30] [--output /var/www/governor-deck-preview/api/state.json]
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
# Config
# ---------------------------------------------------------------------------

API_BASE = "http://127.0.0.1:8080"
TOKEN_FILE = Path.home() / ".decentraai" / "runtime" / "api.token"
OUTPUT_PATH = Path("/var/www/governor-deck-preview/api/state.json")
INTERVAL = 30

PRESSURE_ENTRY = 0.35
PRESSURE_EXIT = 0.20
COOLDOWN_SECS = 120


def read_token():
    p = Path(TOKEN_FILE)
    if p.exists():
        return p.read_text().strip()
    return ""


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
    signals = {}
    queue = state_data.get("queue", {})
    waiting = queue.get("waiting", [])
    serving = queue.get("serving")
    signals["queue_depth"] = len(waiting)
    signals["serving"] = serving is not None

    recent = state_data.get("recent_requests", [])
    if recent:
        durations = [r.get("duration_ms", 0) for r in recent[-5:] if r.get("duration_ms")]
        signals["mean_latency_ms"] = sum(durations) // max(len(durations), 1)

    system = state_data.get("system", {})
    total_ram = system.get("ram_total_gib", 0)
    avail_ram = system.get("ram_available_gib", 0)
    if total_ram > 0:
        signals["ram_percent"] = round(100.0 * (1 - avail_ram / total_ram), 1)

    workers = compute_data.get("workers", [])
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


def build_state(token):
    """Collect all real state and produce the Governor JSON."""
    state_data = api_get("/status", token)
    compute_data = api_get("/v1/compute", token)
    intel_status = api_get("/v1/intel/status", token)
    models_data = api_get("/v1/models", token)

    signals = collect_signals(state_data, compute_data)
    score, reasons = evaluate_pressure(signals)

    governor_hysteresis.evaluate(score)
    governor_active = governor_hysteresis.active

    workers = []
    for w in compute_data.get("workers", []):
        workers.append({
            "worker_id": w.get("node_id", ""),
            "name": w.get("node_name", ""),
            "healthy": w.get("reachable", False),
            "load_percent": w.get("load_percent", 0),
            "contribution_balance": 0,  # populated from ledger in future
        })

    models = []
    for m in models_data.get("data", []):
        models.append({"model_id": m.get("id", ""), "owner": m.get("owned_by", "")})

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
    args = parser.parse_args()

    API_BASE = args.api
    TOKEN_FILE = Path(args.token)
    OUTPUT_PATH = Path(args.output)
    INTERVAL = args.interval

    token = read_token()
    OUTPUT_PATH.parent.mkdir(parents=True, exist_ok=True)

    print(f"[governor-daemon] starting — api={API_BASE} interval={INTERVAL}s output={OUTPUT_PATH}")

    tick = 0
    while True:
        tick += 1
        try:
            st = build_state(token)
            OUTPUT_PATH.write_text(json.dumps(st, indent=2))
            status_line = (
                f"tick={tick} pressure={st['pressure_score']} "
                f"active={st['pressure_active']} "
                f"reasons={','.join(st['pressure_reasons']) or 'none'}"
            )
            print(f"[governor-daemon] {status_line}")
        except Exception as e:
            print(f"[governor-daemon] error: {e}", file=sys.stderr)

        time.sleep(INTERVAL)


if __name__ == "__main__":
    main()

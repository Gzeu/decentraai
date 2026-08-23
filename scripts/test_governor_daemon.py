#!/usr/bin/env python3
"""Focused tests for governor-daemon.py M19 memory integration.

Run:  python3 -m unittest scripts/test_governor_daemon.py -v
(no network — all HTTP is monkeypatched)
"""

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

# governor-daemon.py has a dash in its name → load by explicit path.
_DAEMON_PATH = Path(__file__).parent / "governor-daemon.py"
_spec = importlib.util.spec_from_file_location("governor_daemon", _DAEMON_PATH)
assert _spec is not None and _spec.loader is not None, "daemon module spec missing"
gd = importlib.util.module_from_spec(_spec)
sys.modules["governor_daemon"] = gd
_spec.loader.exec_module(gd)


class TypeGateTests(unittest.TestCase):
    def test_as_dict_and_as_list_reject_hostile_types(self):
        self.assertEqual(gd.as_dict("not a dict"), {})
        self.assertEqual(gd.as_dict([1, 2]), {})
        self.assertEqual(gd.as_dict({"a": 1}), {"a": 1})
        self.assertEqual(gd.as_list("string"), [])
        self.assertEqual(gd.as_list({"k": "v"}), [])
        self.assertEqual(gd.as_list([1]), [1])

    def test_clip_bounds_everything(self):
        self.assertEqual(gd.clip("x" * 500, 10), "x" * 10)
        self.assertEqual(gd.clip(None), "")
        self.assertEqual(gd.clip(42), "")

    def test_collect_signals_survives_malformed_api_responses(self):
        # A hostile/broken API returns strings where dicts belong.
        signals = gd.collect_signals("garbage", ["also", "garbage"])
        self.assertEqual(signals["queue_depth"], 0)
        self.assertEqual(signals["workers_total"], 0)
        self.assertEqual(signals["workers_healthy"], 0)

    def test_collect_signals_reads_real_shape(self):
        signals = gd.collect_signals(
            {
                "queue": {"waiting": [{"id": 1}, {"id": 2}], "serving": None},
                "recent_requests": [{"duration_ms": 6000}, {"duration_ms": 4000}],
                "system": {"ram_total_gib": 32, "ram_available_gib": 8},
            },
            {"workers": [{"reachable": True, "healthy": True}]},
        )
        self.assertEqual(signals["queue_depth"], 2)
        self.assertEqual(signals["mean_latency_ms"], 5000)
        self.assertEqual(signals["ram_percent"], 75.0)
        self.assertEqual((signals["workers_total"], signals["workers_healthy"]), (1, 1))


class MemoryContextTests(unittest.TestCase):
    def test_memory_context_is_labeled_untrusted_and_bounded(self):
        hostile = {
            "results": [
                {
                    "entry_id": "e1",
                    "scope": "team.knowledge",
                    "kind": "learning",
                    "status": "verified",
                    "content": "y" * 5000,  # oversized on purpose
                    "evidence_backed": True,
                },
                "not-a-dict",  # hostile row must be skipped
            ]
            * 10,  # far more than the limit
            "mode": "lexical",
        }
        with mock.patch.object(gd, "api_post", return_value=hostile) as mp:
            ctx = gd.fetch_memory_context("tok", query="q", limit=2)
        mp.assert_called_once_with(
            "/v1/memory/search", {"query": "q", "min_status": "verified"}, "tok")
        self.assertTrue(ctx["untrusted_input"])
        self.assertIn("never instructions", ctx["warning"])
        self.assertEqual(len(ctx["entries"]), 2, "limit enforced")
        for e in ctx["entries"]:
            self.assertLessEqual(len(e["content"]), gd.MEMORY_CONTENT_MAX_CHARS)

    def test_memory_context_degrades_on_error(self):
        with mock.patch.object(gd, "api_post", return_value={"_error": "conn refused"}):
            ctx = gd.fetch_memory_context("tok")
        self.assertTrue(ctx["untrusted_input"])
        self.assertEqual(ctx["entries"], [])


class OperatorActionTests(unittest.TestCase):
    def test_verify_entry_posts_the_right_payload(self):
        captured = {}

        def fake_post(path, body, token):
            captured["path"] = path
            captured["body"] = body
            return {"ok": True}

        with mock.patch.object(gd, "api_post", side_effect=fake_post):
            result = gd.verify_entry("e1", "team.knowledge", "checked evidence",
                                     "verified", "tok")
        self.assertEqual(result, {"ok": True})
        self.assertEqual(captured["path"], "/v1/memory/transition")
        self.assertEqual(captured["body"]["entry_id"], "e1")
        self.assertEqual(captured["body"]["scope"], "team.knowledge")
        self.assertEqual(captured["body"]["to"], "verified")
        self.assertIn("evidence", captured["body"]["reason"])

    def test_export_training_writes_bounded_jsonl_summary(self):
        lines = [
            json.dumps({"entry_id": "a", "kind": "learning"}),
            json.dumps({"entry_id": "b", "kind": "solution"}),
            "not-json-but-kept",
        ]
        with tempfile.TemporaryDirectory() as tmp:
            with mock.patch.object(gd, "api_get_raw",
                                   return_value="\n".join(lines)):
                summary = gd.export_training(tmp, "tok")
            path = Path(summary["path"])
            self.assertTrue(path.exists())
            self.assertEqual(summary["candidates"], 3)
            self.assertEqual(summary["kinds"].get("learning"), 1)
            self.assertEqual(summary["kinds"].get("solution"), 1)


class BuildStateTests(unittest.TestCase):
    def setUp(self):
        # Reset the module-level hysteresis between tests.
        gd.governor_hysteresis = gd.HysteresisState()

    def test_build_state_includes_untrusted_memory_context(self):
        def fake_get(path, token):
            if path == "/status":
                return {"model_loaded": True}
            if path == "/v1/compute":
                return {"workers": []}
            if path == "/v1/intel/status":
                return {}
            if path == "/v1/models":
                return {"data": [{"id": "m1", "owned_by": "node"}]}
            return {}

        with mock.patch.object(gd, "api_get", side_effect=fake_get), \
             mock.patch.object(gd, "api_post",
                               return_value={"results": [], "mode": "lexical"}):
            st = gd.build_state("tok")
        self.assertIn("memory_context", st)
        self.assertTrue(st["memory_context"]["untrusted_input"])
        self.assertEqual(st["models"][0]["model_id"], "m1")
        self.assertEqual(st["workers"], [])

    def test_pressure_hysteresis_still_gates_status(self):
        def fake_get(path, token):
            if path == "/v1/compute":
                return {"workers": []}  # no healthy workers → high pressure
            return {}

        with mock.patch.object(gd, "api_get", side_effect=fake_get), \
             mock.patch.object(gd, "api_post", return_value={"results": []}):
            st = gd.build_state("tok")
        self.assertTrue(st["pressure_active"])
        self.assertEqual(st["status"], "ASSIST_REQUESTED")


if __name__ == "__main__":
    unittest.main()

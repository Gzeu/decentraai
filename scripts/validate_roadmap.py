#!/usr/bin/env python3
"""Validate that the roadmap execution tracker contains steps 1..345 exactly once."""
from __future__ import annotations

import re
import sys
from pathlib import Path

EXPECTED_MAX = 345
TRACKER = Path(__file__).resolve().parents[1] / "docs" / "ROADMAP_345_EXECUTION_TRACKER.md"
STEP_PATTERN = re.compile(r"^\s*(\d+)\.\s+", re.MULTILINE)


def main() -> int:
    if not TRACKER.is_file():
        print(f"ERROR: tracker not found: {TRACKER}", file=sys.stderr)
        return 2

    numbers = [int(value) for value in STEP_PATTERN.findall(TRACKER.read_text(encoding="utf-8"))]
    counts: dict[int, int] = {}
    for number in numbers:
        counts[number] = counts.get(number, 0) + 1

    duplicates = sorted(number for number, count in counts.items() if count > 1)
    unexpected = sorted(number for number in counts if number < 1 or number > EXPECTED_MAX)
    missing = sorted(set(range(1, EXPECTED_MAX + 1)) - set(counts))

    if duplicates or unexpected or missing:
        if missing:
            print(f"Missing step IDs: {missing}")
        if duplicates:
            print(f"Duplicate step IDs: {duplicates}")
        if unexpected:
            print(f"Unexpected step IDs: {unexpected}")
        return 1

    print(f"Roadmap tracker valid: {EXPECTED_MAX} unique steps found")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

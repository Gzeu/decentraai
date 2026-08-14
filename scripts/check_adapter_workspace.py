#!/usr/bin/env python3
"""Report whether the inference adapter is included in the Cargo workspace."""
from __future__ import annotations

import json
import subprocess
import sys

PACKAGE = "decentraai-inference-adapter"


def main() -> int:
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        print(result.stderr, file=sys.stderr)
        return result.returncode

    metadata = json.loads(result.stdout)
    packages = {package["name"] for package in metadata.get("packages", [])}
    workspace_members = set(metadata.get("workspace_members", []))
    package_ids = {package["id"] for package in metadata.get("packages", []) if package["name"] == PACKAGE}

    if PACKAGE not in packages:
        print(f"MISSING: {PACKAGE} is not visible to Cargo metadata")
        return 1
    if not package_ids.intersection(workspace_members):
        print(f"MISSING: {PACKAGE} exists but is not a workspace member")
        return 1

    print(f"OK: {PACKAGE} is a Cargo workspace member")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

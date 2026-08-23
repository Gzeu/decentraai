#!/usr/bin/env python3
"""Validate a training JSONL dataset: schema, dedup, redaction, balance."""

import json
import sys
from collections import Counter
from pathlib import Path

REQUIRED_ROLES = {"system", "user", "assistant"}
SECRET_PATTERNS = [
    "sk-", "dca_", "dsk_", "Bearer ",
]


def validate(path):
    errors = []
    warnings = []
    examples = []
    seen_hashes = set()
    domains = Counter()
    negatives = 0

    for i, line in enumerate(open(path)):
        try:
            ex = json.loads(line)
        except json.JSONDecodeError as e:
            errors.append(f"line {i+1}: invalid JSON: {e}")
            continue
        msgs = ex.get("messages", [])
        if not msgs:
            errors.append(f"line {i+1}: empty messages")
            continue
        roles = [m.get("role") for m in msgs]
        if not REQUIRED_ROLES.issubset(set(roles)):
            errors.append(f"line {i+1}: missing required role (need system+user+assistant)")
        content = json.dumps(msgs)
        for pattern in SECRET_PATTERNS:
            if pattern in content and "[REDACTED]" not in content.split(pattern)[1][:20]:
                errors.append(f"line {i+1}: potential secret leak: '{pattern}...'")
        h = hash(content)
        if h in seen_hashes:
            warnings.append(f"line {i+1}: duplicate example")
        seen_hashes.add(h)
        domain = ex.get("domain", "unknown")
        domains[domain] += 1
        if ex.get("negative"):
            negatives += 1
        examples.append(ex)

    print(f"examples: {len(examples)}")
    print(f"errors: {len(errors)}")
    print(f"warnings: {len(warnings)}")
    print(f"domains: {dict(domains)}")
    print(f"negative examples: {negatives}/{len(examples)} ({100*negatives/max(len(examples),1):.0f}%)")

    for e in errors[:10]:
        print(f"ERROR: {e}")
    for w in warnings[:5]:
        print(f"WARN: {w}")

    return len(errors) == 0


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("usage: validate.py <dataset.jsonl>")
        sys.exit(1)
    ok = validate(sys.argv[1])
    sys.exit(0 if ok else 1)

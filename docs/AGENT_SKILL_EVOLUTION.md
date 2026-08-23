# DecentraAI Agent Skill Evolution System

## Purpose

This document defines the optional, governed mechanism by which the DecentraAI Governor can discover capability gaps, propose new skills or improve existing skills, test them, benchmark them, and promote only evidence-backed changes.

This is an extension of the Agent Operating System. It does not replace `AGENTS.md`, `.agents/policies/`, deterministic Rust policy, or human approval boundaries.

> **Evolution means verified capability improvement, not unrestricted self-modification.**

## Governing rules

1. AI may identify gaps and propose solutions.
2. Skills are versioned artifacts with explicit permissions and dependencies.
3. New or changed skills are sandbox-tested before activation.
4. A single speculative observation is not sufficient reason to rewrite a skill.
5. Skills never grant authority beyond their declared scopes.
6. Existing skills should be improved before creating duplicates.
7. External code must be reviewed and sandboxed before production use.
8. Core governance, authentication, trust, credential issuance, and safety rules are not self-modifiable by a skill.
9. Promotion requires evidence from tests/benchmarks and follows project policy.

## Skill lifecycle

```text
DISCOVERED
   ↓
DRAFT
   ↓
SANDBOX
   ↓
BENCHMARKED
   ↓
VALIDATED
   ↓
ACTIVE
   ↓
DEPRECATED
   ↓
ARCHIVED
```

A skill may move backward when regression or security issues are found.

## Skill registry

Every managed skill should have machine-readable metadata alongside `SKILL.md` where practical.

Example:

```yaml
id: fabric-diagnostics
version: 1.3.0
status: validated

capabilities:
  - diagnose_worker
  - analyze_pressure

required_tools:
  - github
  - decentraai_mcp

permissions:
  read_fabric: true
  execute_compute: false
  modify_code: false
  deploy: false
  credentials: false

risk_level: low

memory_scope: governor

dependencies:
  - fabric
  - dfcp

tests:
  - worker_offline
  - high_cpu
  - high_queue
```

The exact registry schema may evolve, but identity, version, status, permissions, dependencies and verification must remain discoverable.

## Capability Gap Detector

The Governor should periodically ask:

- What failed repeatedly?
- What required manual intervention?
- What task is consistently too slow?
- What capability is missing?
- What workflow repeats unnecessarily?
- What error keeps recurring?
- What external tool/model/protocol could materially improve the workflow?

A detected gap should become a structured proposal, for example:

```text
CAPABILITY GAP #42

Problem:
Model benchmarking requires repeated manual setup.

Frequency:
17 occurrences

Impact:
HIGH

Existing skills checked:
model-registry, testing, fabric

Proposed skill:
model-benchmark-orchestrator

Dependencies:
model-registry
fabric-metrics

Expected benefit:
Automated reproducible benchmark runs and comparable reports.
```

The detector should prefer improving an existing skill when it already covers most of the required behavior.

## Skill construction

A new or updated skill should contain:

```text
SKILL.md
references/
examples/
tests/
metadata (where used)
```

`SKILL.md` should define:

- trigger;
- objective;
- prerequisites;
- workflow;
- required tools;
- required permissions;
- safety constraints;
- verification;
- failure modes;
- references;
- rollback/deactivation notes where applicable.

## Skill compiler pattern

The Governor may turn a verified recurring problem into a candidate skill:

```text
problem
  +
research
  +
existing knowledge
  ↓
SKILL PROPOSAL
  ↓
SKILL.md + tests + metadata
  ↓
sandbox
  ↓
benchmark
  ↓
validation / approval gate
  ↓
ACTIVE
```

The Governor must not silently activate arbitrary generated instructions as trusted operational behavior.

## Skill dependencies

Skills may depend on other skills, but dependencies must be explicit.

Example:

```text
crypto-analysis
   ├── market-data
   ├── regime-detection
   ├── sentiment
   ├── risk-analysis
   └── evidence
```

Dependency changes should trigger relevant regression tests.

Avoid circular dependencies unless a runtime mechanism explicitly supports them.

## Skill testing

Every meaningful skill should have repeatable tests for:

- expected success path;
- missing capability;
- malformed input;
- unavailable worker/tool;
- permission denial;
- stale data where relevant;
- failure recovery;
- security-sensitive edge cases.

Example:

```text
.agents/skills/autonomous-pressure/
├── SKILL.md
├── references/
├── examples/
└── tests/
    ├── high_cpu.md
    ├── low_cpu.md
    ├── hysteresis.md
    └── no_worker.md
```

## Skill benchmarking

Track, where applicable:

```text
success_rate
failure_rate
latency
resource_usage
tool_calls
human_rejections
regressions
security_findings
```

Example:

```text
security-audit v1.4

success:          96.8%
false_positive:    3.1%
median_latency:    4.2s
regressions:       0
critical_findings: 0
```

A new version should not replace a validated version merely because it is newer. It should demonstrate a meaningful improvement or address an identified defect.

## A/B and canary evaluation

For skills with measurable output, evaluation may compare:

```text
skill v1.4 → N representative tasks
skill v1.5 → N equivalent representative tasks
```

Compare quality, failure rate, latency, cost/resource use and safety findings.

Promotion should favor the version with the better verified outcome, not necessarily the more feature-rich version.

## Skill memory

Each skill should have durable knowledge about why it exists:

```text
skill
 ↓
why created
 ↓
problems solved
 ↓
versions
 ↓
failures
 ↓
lessons
```

Use the Agent OS / Obsidian memory system for experiment history and lessons. Do not store credentials or raw secrets.

Useful links can include:

- `[[experiment-...]]`
- `[[lesson-...]]`
- `[[decision-...]]`
- `[[incident-...]]`

## External skill discovery

The Governor may research:

- GitHub projects;
- Hugging Face models;
- official documentation;
- MCP servers;
- libraries and protocols;
- benchmarks and papers.

Flow:

```text
DISCOVER
  ↓
RESEARCH
  ↓
SECURITY REVIEW
  ↓
SANDBOX
  ↓
BENCHMARK
  ↓
PROPOSE
  ↓
APPROVE / REJECT
```

Never install arbitrary external code directly into production because an agent recommends it.

## Permissions

A skill must request only the authority it needs.

Example:

```yaml
permissions:
  read_fabric: true
  execute_compute: false
  modify_code: true
  deploy: false
  credentials: false
```

Permission changes should be treated as a policy-sensitive change, especially for:

- credentials;
- authentication;
- trust/reputation;
- production deployment;
- exchange writes;
- destructive infrastructure actions.

## Governor self-improvement loop

```text
             GOVERNOR
                 ↓
          OBSERVE / WORK
                 ↓
          FAILURE / GAP
                 ↓
             RESEARCH
                 ↓
          SKILL PROPOSAL
                 ↓
             SANDBOX
                 ↓
            BENCHMARK
                 ↓
         POLICY / REVIEW GATE
                 ↓
             SKILL vNext
                 ↓
              MEMORY
                 ↘
              future work
```

The Governor should optimize for **maximum reliable capability under explicit governance**, not maximum autonomy and not maximum number of skills.

## When not to evolve a skill

Do not create or modify a skill when:

- the issue happened only once and has no durable lesson;
- an existing skill already solves the problem;
- the proposed skill duplicates another capability;
- the improvement has no measurable or defensible benefit;
- the implementation introduces disproportionate complexity;
- security or permission boundaries cannot be demonstrated safely.

## Integration with Governor

The Governor should include this policy in its periodic review loop.

At each deep review it should produce:

```text
CAPABILITY GAPS
- ...

REPEATED FAILURES
- ...

SKILL CANDIDATES
- ...

SKILL REGRESSIONS
- ...

SKILL PROMOTIONS
- ...

SKILLS THAT SHOULD NOT CHANGE
- ...
```

A candidate skill becomes real only after the normal branch/test/review/promotion workflow.

## Definition of done

The skill-evolution system is useful when the Governor can:

1. detect a repeated capability gap;
2. find and reuse an existing skill where possible;
3. research alternatives;
4. propose or draft a new skill;
5. declare dependencies and permissions;
6. create reproducible tests;
7. run it in a safe environment;
8. benchmark it against the previous workflow;
9. preserve the experiment and lesson in memory;
10. open a PR for review when promotion is warranted;
11. roll back or deprecate a skill when evidence shows regression.

This system is intentionally **governed self-improvement**, not unrestricted self-modifying code.

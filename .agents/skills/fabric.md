# Skill: fabric — how to reason about the DecentraAI fabric

## The mental model

```text
AI proposes → deterministic Rust decides → workers execute
```

An LLM may produce plans, parameters and recommendations. It can never
select a peer, mutate trust, issue credentials, alter hashes/reputation/
configuration, or bypass reservations.

## Capability-first

The decision unit is the CAPABILITY (`decentraai_hub::capability::CapabilityKind`,
26 kinds, snake_case wire form), never a model name:

```text
CapabilityKind  +  worker advertisements  +  runtime state
      = what the fabric can actually do right now
```

## Where things live

| Concern | Crate / file |
|---|---|
| Taxonomy | `crates/hub/src/capability.rs` (`ALL_NAMES` pinned by test) |
| Deterministic planner | `crates/fabric` (ExecutionPlan, plan_and_reserve) |
| Intelligence layer | `crates/fabric-intelligence` (TaskPlan, policy, providers) |
| Compute sharing domain | `crates/compute` (assist.rs, credits, reservations) |
| Runtime wiring | `crates/distributed`, `crates/runtime/src/api.rs` |

## Rules

1. Find the existing primitive before adding anything new.
2. Pure decisions are separated from I/O so tests drive them with
   synthetic input.
3. Model output is untrusted input — closed schemas, bounded sizes,
   taxonomy validation at parse time.
4. Backend URLs are resolved LIVE per request (engine ports are ephemeral).

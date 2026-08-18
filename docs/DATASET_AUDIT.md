# P8 Dataset/Skill layer — Architecture Audit

> Status: audit complete (Pylon, 2026-08-18, commit `41f894c`). Research +
> design first; the smallest coherent fix follows. DecentraAI remains an
> execution fabric / collective-intelligence infrastructure — NOT a training
> framework. Datasets/skills are artifacts that feed the capability system.

## Verified direction

```
Hardware → Models → Tools → Datasets → Capabilities → Talents → Agent Powers
   → Agent Execution → DecentraAI Fabric
```

## Current state

`crates/agents/src/dataset.rs` (P8 dataset layer) adds `DatasetDescriptor`,
`DatasetKind`, `SkillDescriptor`, `SkillRegistry`, `CapabilityBuild`,
`build_agent_capabilities()`. Pure / no I/O, consistent with the crate.
`decentraai agent skill` demonstrates it (read-only).

## Answers to the audit questions

### A. Dataset semantics
A `DatasetDescriptor` is best read as **evidence**: it declares which
capabilities it *develops* (trains/tunes/curates/benchmarks), with a source,
kind, size, quality and provenance. `DatasetKind`
(`Training | FineTune | KnowledgeBase | Benchmarks`) is a reasonable starting
enum and is extensible (a plain enum, not a fixed-wire constraint). It is not
(and should not be) a full training-manifest model — that belongs to external
tooling; DecentraAI consumes the *evidence* (the capability claim), not the
training run.

### B. Skill semantics — SEMANTIC INCONSISTENCY (confirmed)
`SkillDescriptor` has `dataset_id`, `requires_model`, `develops`,
`prerequisites`, `resource_mb`. **There is NO validation that `skill.develops`
is a subset of `dataset.develops`.** Worse: `build_agent_capabilities` unlocks
**only `skill.develops`** and stamps it with the **dataset's provenance**
(dataset.rs:339-345), then `CapabilityBuild::all` merges Inferred+Verified →
Verified (dataset.rs:308-311).

**Consequence — a provenance-laundering hole:** a `Verified` dataset can lend
`Verified` to capabilities its dataset never develops, via a skill that
declares them. This violates "dataset existence ≠ capability proof" and
"no arbitrary capability invention".

The semantic distinction should be made explicit and enforced:
- **Dataset = evidence.** `dataset.develops` is the set of capabilities the
  dataset *actually* develops (the only legitimate source of a new claim).
- **Skill = application gate.** It declares *when* a dataset may be applied to
  a model (`requires_model`, `prerequisites`, `resource_mb`) — it should NOT
  invent capabilities of its own.

### C. Provenance
Preserved well from dataset → `CapabilityBuild`/`CapabilityClaim` → wire
`AgentAdvertisement` → matcher. The matcher genuinely enforces provenance
(`EvidenceLevel::Verified` requires a Verified claim; Inferred is surfaced as
`InsufficientProvenance`). **Two breaks:**
1. **Talent tree ignores provenance.** `TalentNode.provenance_required` and
   `confidence` are declared as honesty gates but **never read** by
   `can_unlock`/`resolve_path`/`available_capabilities`.
2. **No runtime wiring.** `build_agent_capabilities` runs only in the CLI
   demo; live nodes build agents from `default_local_agents` (all Inferred) or
   hub `classify` — never from datasets/skills. The full chain
   dataset → talent → execution does not exist in production yet.

### D. Quality
`DatasetDescriptor.quality` and `TalentNode.confidence` are **inert** — stored,
clamped, serialized, but never consumed by any decision (matcher, reputation,
planner, scheduler, orchestrator). (Economy offer quality and compute/verification
confidence are separate, independently-consumed fields.)
Where it should enter later, without coupling unrelated domains: as a **weight**
in capability *confidence* and a **gate/weight** in executor *selection* — not
as a hard filter, and only for capabilities claimed via datasets/skills.

### E. HuggingFace boundary
- **GitHub** = source code / schemas / deterministic logic.
- **HF Dataset repos** = versioned datasets (this is where real datasets live).
- **HF Model repos** = distributable model artifacts (already used via
  `decentraai model pull`).
- **HF Bucket (`hf://buckets/Snakeeu/DecentraAi`)** = mutable large artifacts /
  checkpoints / experiments / staging — NOT a runtime dependency.
- **DecentraAI local registry** = verified runtime artifacts.

The bucket should NOT become a runtime dependency. It is an operator/experiment
workspace.

### F. Artifact verification (datasets)
DecentraAI has strong model verification (SHA-256 pinned before download).
A dataset eventually needs: source repo + revision, license, size, hash,
dataset version, processing version, provenance, evaluation evidence, and the
capability it supports. This is metadata on `DatasetDescriptor` (extend later) —
a dataset must be **verified before its claims are usable**, mirroring the
model trust boundary.

### G. Dataset → capability evidence
**Dataset existence alone MUST NOT prove a capability.** The minimum evidence
model is:
```
Dataset → processing → training/evaluation → benchmark evidence → CapabilityClaim
```
A claim becomes usable only when it has an evidence path. For now, that means:
`dataset.develops` is the *only* source of dataset-derived claims (fix B), and
`provenance` reflects how the claim was obtained (Verified only from
trustworthy evaluation, Inferred otherwise).

### H. HF bucket namespace (design only — do not create yet)
```
models/         datasets/       experiments/    checkpoints/
adapters/       benchmarks/     capabilities/
```
`models/`, `datasets/`, `benchmarks/` should eventually be **versioned HF
repos**; the bucket holds mutable experiments/checkpoints/adapters/staging.

### I. Qwen first real test
`qwen2.5-coder-7b-instruct-q4_k_m` is the first real model. The first real
experiment should prove the complete path:
`model → dataset → skill → capability → TalentTree → agent power → real
execution`. Today the path is proven only as data structures + CLI demo; the
runtime wiring (I.1 below) is the missing piece.

### J. Embeddings / RAG
`nomic-embed-text-v1.5.Q4_K_M` fits as:
`dataset → indexed knowledge → embeddings → retrieval capability → agent
capability → execution`. Do not couple the embedding model to the Talent Tree
until the capability model supports the distinction cleanly (Retrieval is
already a `CapabilityKind`; RAG → Retrieval per the talent-tree mapping doc).

### K. Future self-improvement
`selfopt.rs` runs on abstract fabric dimensions, not capability evidence.
Future feedback should be: `execution result → evaluation → reputation →
capability evidence → TalentTree evolution` — with a **hard gate**: evidence
must be earned from real verified execution, never self-granted.

## Integrity / trust invariants (preserved)
Power ≠ Permission · dataset existence ≠ capability proof · skill existence ≠
capability proof · no dishonest provenance upgrade · no arbitrary capability
invention · no secrets leave the node · no prompts/outputs in audit · artifacts
verified before use · deterministic registries · pure logic separated from I/O.

## What is correct
- Pure, serde-serializable, deterministic layer in `crates/agents`.
- Provenance carried on the wire and enforced by the matcher.
- Honest Inferred-by-default; Verified only from explicit evidence.
- Clean separation: datasets/skills are not runtime dependencies of the engine.

## What is missing
1. **Invariant: skill.develops ⊆ dataset.develops** (integrity fix).
2. Runtime wiring: build agents from dataset/skill evidence (not just CLI demo).
3. Talent tree consuming provenance/confidence (currently inert fields).
4. Verified dataset import/verification (source+revision+hash) — later.
5. RAG path via `nomic-embed` — later.

## Recommended next milestone (smallest coherent, unambiguous)
**Fix the provenance-laundering hole (B).** Enforce the invariant that a
skill can only unlock capabilities its dataset actually develops, and make the
dataset's `develops` the single source of evidence for new claims. This is
small, unambiguous, and required before any real dataset is imported.

Files that change: `crates/agents/src/dataset.rs` (validation in `add_skill` +
use `dataset.develops` in `build_agent_capabilities`), `lib.rs` (new error
variant), tests in `dataset.rs`, `node-cli` (align `agent skill` demo).

Tests required: skill.develops outside dataset.develops is rejected; a skill
cannot lend a dataset's Verified provenance to a capability the dataset does
not develop; build still unlocks dataset-developed capabilities; registry
round-trip unchanged.

Gates: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`,
`cargo test --workspace`. Update ROADMAP/todo only for completed work.

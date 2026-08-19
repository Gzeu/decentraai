// Placeholder: ExecutionStrategy implementation per P1 roadmap
// NOTE: This file documents the intended ExecutionStrategy abstraction.
// Actual Rust code changes to crates/fabric will be implemented in follow-up commits
// once full source contents are accessible in this environment.

# DecentraAI ExecutionStrategy P1 Implementation Plan

START_SHA: 48e039b2825c75f2f209a4c9fc7aa9f77e4a61ae

Planned changes (not yet applied):
- Add enum StrategyKind { SingleWorker, BatchFanOut, SpeculativeDraftVerify(Experimental), DisaggregatedPrefillDecode(Experimental), CacheAwareRoute(Experimental), CollaborativeModel(Experimental) } in crates/fabric/src/plan.rs.
- Introduce struct ExecutionStrategy { kind: StrategyKind, rationale: StrategyRationale, provenance: EvidenceProvenance }.
- Wire ExecutionPlanner to produce ExecutionStrategy instances for existing SingleWorker and BatchFanOut paths only.
- Implement CAN_RUN based on existing get_worker_capability/aggregate_can_i_run.
- Implement conservative CAN_COLLABORATE that returns true only for BatchFanOut.
- Extend ExecutionDecision to carry strategy kind, CAN_RUN/CAN_COLLABORATE snapshots, and provenance flags (MEASURED/ESTIMATED/INFERRED/EXPERIMENTAL/UNKNOWN).

Due to limited access to raw Rust source via this interface, actual code edits and tests must be performed locally in the repo following this plan.

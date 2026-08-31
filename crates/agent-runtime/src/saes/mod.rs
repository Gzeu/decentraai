//! SAES — Structured Agent Evolution System
//!
//! Builds the autonomous cycle: identity → goals → observe → decide → act
//! → evidence → outcome → memory → reputation → learning → changed behaviour.
//!
//! This module is pure, decision-only. It extends the existing `AgentRuntime`
//! foundation without breaking the v1 API. Each sub-module is independently
//! testable with synthetic inputs.
//!
//! SAES 0.3 adds `persistence` — SQLite-backed stores that ensure agent
//! experience survives process restarts. Enable with the `persistence`
//! feature flag.
//!
//! SAES 0.4 adds `collective` — collective goal coordination enabling
//! multiple agents to work on shared objectives with progress propagation.
//!
//! SAES 0.5 adds `pressure` — a deterministic pressure trigger that turns
//! "the agent cannot continue alone" into an explicit collaboration signal
//! (Phase 1 of Pressure Trigger → Placement Fairness → Agent Gateway).

pub mod adaptation;
pub mod goals;
pub mod learning;
pub mod outcomes;
pub mod placement;
pub mod pressure;

#[cfg(feature = "persistence")]
pub mod persistence;

pub mod collective;
#[cfg(test)]
pub mod collective_tests;

#[cfg(feature = "persistence")]
pub mod collective_persistence;

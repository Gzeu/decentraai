//! SAES 0.2 — Structured Agent Evolution System
//!
//! Builds the autonomous cycle: identity → goals → observe → decide → act
//! → evidence → outcome → memory → reputation → learning → changed behaviour.
//!
//! This module is pure, decision-only. It extends the existing `AgentRuntime`
//! foundation without breaking the v1 API. Each sub-module is independently
//! testable with synthetic inputs.

pub mod adaptation;
pub mod goals;
pub mod learning;
pub mod outcomes;

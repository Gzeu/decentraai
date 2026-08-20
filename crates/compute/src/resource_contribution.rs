//! Unified resource contribution model (P14 Phase A–B).
//!
//! This module turns the raw evidence produced by verified compute execution
//! into a structured, provenance-preserving contribution record. It does not
//! invent data: every field is either measured by the runtime, derived from
//! measured values through deterministic formulas, or explicitly marked as
//! unknown.
//!
//! The new layer remains additive: existing [`CompensationLedger`] and
//! [`QuotaLedger`] keep working exactly as before; the types here provide a
//! richer, receipt-backed view that future dashboards and planners can consume.
//!
//! # Provenance rules
//!
//! - `Measured`  – the value came directly from a probe, timer, counter, or
//!   signed receipt (e.g. execution duration, tokens used).
//! - `Derived`   – computed from measured values using a documented, versioned
//!   policy (e.g. `ram_bytes_seconds` from sampled RAM × duration).
//! - `Estimated` – an intentional upper/lower bound when measurement is
//!   impossible; must be labeled as such and never silently treated as exact.
//! - `Unknown`   – the value is genuinely absent. `Unknown` is never converted
//!   into a fake number.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How a value in a resource contribution was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    Measured,
    Derived,
    Estimated,
    #[default]
    Unknown,
}

/// A single resource dimension with its value, unit, and provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceDimension {
    pub name: String,
    pub value: f64,
    pub unit: String,
    pub provenance: Provenance,
}

impl ResourceDimension {
    pub fn new(name: impl Into<String>, value: f64, unit: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value,
            unit: unit.into(),
            provenance: Provenance::Measured,
        }
    }

    pub fn with_provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = provenance;
        self
    }
}

/// A unified, evidence-backed record of the resources a node contributed during
/// one verified execution. Not every workload populates every field.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceContribution {
    /// Stable execution id (idempotency key).
    pub execution_id: String,
    /// Worker node (peer id) that performed the work.
    pub worker_node: String,
    /// Capability / workload kind exercised.
    pub capability: String,
    /// Model identity (file hash or canonical name) when known.
    pub model: Option<String>,
    /// Verdict of the verified execution.
    pub success: bool,

    /// CPU time contributed (seconds, measured or derived).
    pub cpu_time_seconds: Option<ResourceDimension>,
    /// RAM × seconds (byte-seconds).
    pub ram_bytes_seconds: Option<ResourceDimension>,
    /// GPU time contributed (seconds).
    pub gpu_time_seconds: Option<ResourceDimension>,
    /// VRAM × seconds (byte-seconds).
    pub vram_bytes_seconds: Option<ResourceDimension>,
    /// Tokens processed/generated.
    pub tokens_processed: Option<ResourceDimension>,
    /// Wall-clock execution duration.
    pub execution_duration_ms: Option<ResourceDimension>,
    /// Model-specific work units (engine-dependent, e.g. prefills).
    pub model_work_units: Option<ResourceDimension>,
    /// Network bytes transferred for this execution.
    pub network_bytes: Option<ResourceDimension>,
    /// Network time spent routing/transferring the workload.
    pub network_time_ms: Option<ResourceDimension>,
    /// Duration the resource reservation was held.
    pub reservation_duration_ms: Option<ResourceDimension>,

    /// Receipt id that attests to this execution.
    pub receipt_id: Option<String>,
    /// Hash of the verified output (BLAKE3 hex) when present.
    pub output_hash: Option<String>,
    /// Free-form dimensions for engine-specific metrics.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, ResourceDimension>,
}

impl ResourceContribution {
    /// Returns the total measured/derived resource "mass" of this record,
    /// weighted by a simple policy: 1 token ≈ 1 unit, 1 ms duration ≈ 1 unit,
    /// 1 byte-second ≈ 1 / (1024³) unit. The exact number is arbitrary; it
    /// exists only so the dashboard can sort/compare contributions without
    /// pretending one dimension dominates another.
    pub fn weighted_mass(&self) -> f64 {
        let mut mass = 0.0;
        if let Some(d) = &self.tokens_processed {
            mass += d.value;
        }
        if let Some(d) = &self.execution_duration_ms {
            mass += d.value;
        }
        if let Some(d) = &self.cpu_time_seconds {
            mass += d.value * 1000.0;
        }
        if let Some(d) = &self.ram_bytes_seconds {
            mass += d.value / (1024.0 * 1024.0 * 1024.0);
        }
        if let Some(d) = &self.gpu_time_seconds {
            mass += d.value * 1000.0;
        }
        if let Some(d) = &self.vram_bytes_seconds {
            mass += d.value / (1024.0 * 1024.0 * 1024.0);
        }
        mass
    }
}

/// A builder helper for runtime code that records resource contributions.
#[derive(Debug, Default)]
pub struct ResourceContributionBuilder {
    execution_id: String,
    worker_node: String,
    capability: String,
    model: Option<String>,
    success: bool,
    dimensions: Vec<ResourceDimension>,
    receipt_id: Option<String>,
    output_hash: Option<String>,
    extra: BTreeMap<String, ResourceDimension>,
}

impl ResourceContributionBuilder {
    pub fn new(execution_id: impl Into<String>, worker_node: impl Into<String>) -> Self {
        Self {
            execution_id: execution_id.into(),
            worker_node: worker_node.into(),
            ..Default::default()
        }
    }

    pub fn capability(mut self, capability: impl Into<String>) -> Self {
        self.capability = capability.into();
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn success(mut self, success: bool) -> Self {
        self.success = success;
        self
    }

    pub fn dimension(mut self, d: ResourceDimension) -> Self {
        self.dimensions.push(d);
        self
    }

    pub fn receipt_id(mut self, id: impl Into<String>) -> Self {
        self.receipt_id = Some(id.into());
        self
    }

    pub fn output_hash(mut self, hash: impl Into<String>) -> Self {
        self.output_hash = Some(hash.into());
        self
    }

    pub fn extra(mut self, key: impl Into<String>, d: ResourceDimension) -> Self {
        self.extra.insert(key.into(), d);
        self
    }

    pub fn build(self) -> ResourceContribution {
        let mut rc = ResourceContribution {
            execution_id: self.execution_id,
            worker_node: self.worker_node,
            capability: self.capability,
            model: self.model,
            success: self.success,
            cpu_time_seconds: None,
            ram_bytes_seconds: None,
            gpu_time_seconds: None,
            vram_bytes_seconds: None,
            tokens_processed: None,
            execution_duration_ms: None,
            model_work_units: None,
            network_bytes: None,
            network_time_ms: None,
            reservation_duration_ms: None,
            receipt_id: self.receipt_id,
            output_hash: self.output_hash,
            extra: self.extra,
        };
        for d in self.dimensions {
            match d.name.as_str() {
                "cpu_time_seconds" => rc.cpu_time_seconds = Some(d),
                "ram_bytes_seconds" => rc.ram_bytes_seconds = Some(d),
                "gpu_time_seconds" => rc.gpu_time_seconds = Some(d),
                "vram_bytes_seconds" => rc.vram_bytes_seconds = Some(d),
                "tokens_processed" => rc.tokens_processed = Some(d),
                "execution_duration_ms" => rc.execution_duration_ms = Some(d),
                "model_work_units" => rc.model_work_units = Some(d),
                "network_bytes" => rc.network_bytes = Some(d),
                "network_time_ms" => rc.network_time_ms = Some(d),
                "reservation_duration_ms" => rc.reservation_duration_ms = Some(d),
                _ => {
                    rc.extra.insert(d.name.clone(), d);
                }
            }
        }
        rc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_places_dimensions() {
        let rc = ResourceContributionBuilder::new("e1", "peer-a")
            .capability("inference")
            .model("llama.gguf")
            .success(true)
            .dimension(ResourceDimension::new("tokens_processed", 42.0, "tokens"))
            .dimension(ResourceDimension::new("execution_duration_ms", 120.0, "ms"))
            .build();
        assert_eq!(rc.execution_id, "e1");
        assert_eq!(rc.worker_node, "peer-a");
        assert!(rc.success);
        assert_eq!(rc.tokens_processed.as_ref().unwrap().value, 42.0);
        assert_eq!(rc.execution_duration_ms.as_ref().unwrap().value, 120.0);
    }

    #[test]
    fn unknown_fields_remain_none() {
        let rc = ResourceContributionBuilder::new("e2", "peer-b")
            .capability("inference")
            .build();
        assert!(rc.cpu_time_seconds.is_none());
        assert!(rc.gpu_time_seconds.is_none());
    }

    #[test]
    fn weighted_mass_is_non_negative() {
        let rc = ResourceContributionBuilder::new("e3", "peer-c")
            .capability("inference")
            .dimension(ResourceDimension::new("tokens_processed", 100.0, "tokens"))
            .dimension(ResourceDimension::new("execution_duration_ms", 500.0, "ms"))
            .build();
        assert!(rc.weighted_mass() > 0.0);
    }
}

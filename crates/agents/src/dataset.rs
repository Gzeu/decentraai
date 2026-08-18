//! Dataset & Skill layer — the mechanism that lets the Talent Tree evolve.
//!
//! The architecture chain (agreed 2026-08-17) is:
//!
//! ```text
//! Hardware → Models → Tools → Datasets → Capabilities → Talents → Agent Power
//! ```
//!
//! The P8 [`TalentTree`] is the *static* capability graph (prerequisites +
//! resource estimates). This module is what feeds it: a **dataset** develops
//! capabilities, and a **skill** binds a dataset to a model that has a base
//! capability, thereby *unlocking additional capabilities* for that model. An
//! agent built on a model + the skills applied to it therefore has a richer
//! capability set — and that set is what the Talent Tree's
//! `available_capabilities` consumes.
//!
//! Honesty rules (same provenance discipline as the fabric):
//! - A dataset claims the capabilities it develops with a
//!   [`decentraai_hub::capability::Provenance`]. A `Verified` capability is
//!   only claimed for datasets with trustworthy provenance; inferred
//!   otherwise — never claimed stronger than real.
//! - A skill only unlocks a capability when its prerequisites are met and the
//!   target model has the required base capability. It never invents a
//!   capability the underlying model cannot express.
//!
//! Pure (no I/O, no async) — same pattern as the rest of `crates/agents`.

use std::collections::{BTreeMap, BTreeSet};

use decentraai_hub::capability::{CapabilityClaim, CapabilityKind, Provenance};
use serde::{Deserialize, Serialize};

/// What a dataset develops/trains for. Extensible — not a fixed list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetKind {
    /// General-purpose training data.
    Training,
    /// Fine-tuning data for a specific capability.
    FineTune,
    /// A curated knowledge base (documents, examples, code).
    KnowledgeBase,
    /// Evaluation / benchmark data used to measure a capability.
    Benchmarks,
}

/// A dataset that develops one or more capabilities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetDescriptor {
    /// Unique id (e.g. "code_finetune_2024").
    pub id: String,
    /// Human name.
    pub name: String,
    /// What the dataset develops (e.g. Coding, ToolCalling).
    pub develops: Vec<CapabilityKind>,
    /// Where it came from (a HuggingFace ref, URL, or local path).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// Kind of dataset.
    pub kind: DatasetKind,
    /// Estimated size in bytes.
    pub size_bytes: u64,
    /// Quality in 0..=1 (clamped).
    pub quality: f32,
    /// Provenance of the capability claims.
    pub provenance: Provenance,
    /// License / usage terms (best-effort string).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub license: String,
    /// Creation time (unix ms).
    pub created_at_ms: u64,
}

impl DatasetDescriptor {
    /// A dataset that develops the given capabilities.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        develops: Vec<CapabilityKind>,
        kind: DatasetKind,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            develops,
            source: String::new(),
            kind,
            size_bytes: 0,
            quality: 1.0,
            provenance: Provenance::Inferred,
            license: String::new(),
            created_at_ms: 0,
        }
    }

    /// Sets the source reference (HF path / URL).
    pub fn from(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// Sets size and quality; quality is clamped to 0..=1.
    pub fn sized(mut self, size_bytes: u64, quality: f32) -> Self {
        self.size_bytes = size_bytes;
        self.quality = quality.clamp(0.0, 1.0);
        self
    }

    /// Sets the provenance of the capability claims.
    pub fn with_provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = provenance;
        self
    }

    /// Sets the license string.
    pub fn with_license(mut self, license: impl Into<String>) -> Self {
        self.license = license.into();
        self
    }

    /// Sets the creation timestamp.
    pub fn created_at(mut self, created_at_ms: u64) -> Self {
        self.created_at_ms = created_at_ms;
        self
    }

    /// Whether the dataset claims it develops a capability.
    pub fn develops(&self, capability: CapabilityKind) -> bool {
        self.develops.contains(&capability)
    }
}

/// A skill: a dataset applied to a model with a base capability, unlocking
/// additional capabilities (which feed the Talent Tree).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillDescriptor {
    /// Unique skill id.
    pub id: String,
    /// Human name.
    pub name: String,
    /// The dataset this skill is built on.
    pub dataset_id: String,
    /// The base capability the target model must have (e.g. Coding to apply a
    /// code-finetune skill). `None` = applicable to any model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_model: Option<CapabilityKind>,
    /// Capabilities this skill unlocks (beyond what the dataset develops; the
    /// model must be able to express them).
    pub develops: Vec<CapabilityKind>,
    /// Prerequisites (other capabilities the model must already have).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prerequisites: Vec<CapabilityKind>,
    /// Estimated additional resource footprint (MiB) this skill needs.
    pub resource_mb: u64,
}

impl SkillDescriptor {
    /// A skill that applies `dataset` to a model with `requires_model`.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        dataset_id: impl Into<String>,
        requires_model: Option<CapabilityKind>,
        develops: Vec<CapabilityKind>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            dataset_id: dataset_id.into(),
            requires_model,
            develops,
            prerequisites: Vec::new(),
            resource_mb: 0,
        }
    }

    /// Sets prerequisites (capabilities the model must already have).
    pub fn with_prerequisites(mut self, prereqs: Vec<CapabilityKind>) -> Self {
        self.prerequisites = prereqs;
        self
    }

    /// Sets the estimated additional resource footprint (MiB).
    pub fn with_resource(mut self, resource_mb: u64) -> Self {
        self.resource_mb = resource_mb;
        self
    }

    /// Whether the skill is applicable to a model that has `model_caps` and
    /// the required base capability + all prerequisites.
    pub fn applicable_to(
        &self,
        model_caps: &[CapabilityKind],
        dataset: Option<&DatasetDescriptor>,
    ) -> bool {
        // The dataset must exist and develop at least one capability.
        if let Some(d) = dataset {
            if d.develops.is_empty() {
                return false;
            }
        }
        // Model must have the base capability, if required.
        if let Some(req) = self.requires_model {
            if !model_caps.contains(&req) {
                return false;
            }
        }
        // All prerequisites must be present.
        self.prerequisites.iter().all(|p| model_caps.contains(p))
    }
}

/// Registry errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DatasetError {
    #[error("dataset '{id}' is already registered")]
    DuplicateDataset { id: String },
    #[error("skill '{id}' is already registered")]
    DuplicateSkill { id: String },
    #[error("dataset '{dataset_id}' referenced by skill '{skill_id}' is not registered")]
    UnknownDataset {
        skill_id: String,
        dataset_id: String,
    },
    #[error(
        "skill '{skill_id}' declares capability {capability:?} which its dataset does not develop"
    )]
    SkillDevelopsNotInDataset {
        skill_id: String,
        capability: CapabilityKind,
    },
}

/// Deterministic registries of datasets and skills.
#[derive(Debug, Clone, Default)]
pub struct SkillRegistry {
    datasets: BTreeMap<String, DatasetDescriptor>,
    skills: BTreeMap<String, SkillDescriptor>,
}

impl SkillRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a dataset.
    pub fn add_dataset(&mut self, dataset: DatasetDescriptor) -> Result<(), DatasetError> {
        let id = dataset.id.clone();
        if self.datasets.contains_key(&id) {
            return Err(DatasetError::DuplicateDataset { id });
        }
        self.datasets.insert(id, dataset);
        Ok(())
    }

    /// Registers a skill, requiring its dataset to be known and that the
    /// skill only unlocks capabilities its dataset actually develops.
    ///
    /// Integrity invariant (audit B): a dataset is *evidence* — it declares
    /// the capabilities it develops. A skill is an *application gate*; it must
    /// not invent capabilities the dataset does not support, otherwise a
    /// `Verified` dataset could lend `Verified` to capabilities it never
    /// developed (provenance laundering).
    pub fn add_skill(&mut self, skill: SkillDescriptor) -> Result<(), DatasetError> {
        if self.skills.contains_key(&skill.id) {
            return Err(DatasetError::DuplicateSkill {
                id: skill.id.clone(),
            });
        }
        let dataset = self
            .datasets
            .get(&skill.dataset_id)
            .ok_or(DatasetError::UnknownDataset {
                skill_id: skill.id.clone(),
                dataset_id: skill.dataset_id.clone(),
            })?;
        for cap in &skill.develops {
            if !dataset.develops.contains(cap) {
                return Err(DatasetError::SkillDevelopsNotInDataset {
                    skill_id: skill.id.clone(),
                    capability: *cap,
                });
            }
        }
        self.skills.insert(skill.id.clone(), skill);
        Ok(())
    }

    /// Looks up a dataset.
    pub fn dataset(&self, id: &str) -> Option<&DatasetDescriptor> {
        self.datasets.get(id)
    }

    /// Looks up a skill.
    pub fn skill(&self, id: &str) -> Option<&SkillDescriptor> {
        self.skills.get(id)
    }

    /// All datasets, sorted by id.
    pub fn datasets(&self) -> Vec<&DatasetDescriptor> {
        self.datasets.values().collect()
    }

    /// All skills, sorted by id.
    pub fn skills(&self) -> Vec<&SkillDescriptor> {
        self.skills.values().collect()
    }

    /// The skills applicable to a model with the given base capabilities.
    pub fn applicable_skills(&self, model_caps: &[CapabilityKind]) -> Vec<&SkillDescriptor> {
        self.skills
            .values()
            .filter(|s| s.applicable_to(model_caps, self.dataset(&s.dataset_id)))
            .collect()
    }
}

/// Result of building an agent's capability set from a model + its skills.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityBuild {
    /// The model's base capabilities.
    pub base: Vec<CapabilityClaim>,
    /// Capabilities unlocked by applying skills (with provenance).
    pub unlocked: Vec<CapabilityClaim>,
}

impl CapabilityBuild {
    /// All capabilities (base + unlocked), deduplicated, sorted by capability.
    pub fn all(&self) -> Vec<CapabilityClaim> {
        let mut map: BTreeMap<CapabilityKind, Provenance> = BTreeMap::new();
        for c in self.base.iter().chain(self.unlocked.iter()) {
            // Strongest provenance wins (Verified over Inferred).
            let entry = map.entry(c.capability).or_insert(c.provenance);
            if c.provenance == Provenance::Verified {
                *entry = Provenance::Verified;
            }
        }
        map.into_iter()
            .map(|(capability, provenance)| CapabilityClaim {
                capability,
                provenance,
            })
            .collect()
    }
}

/// Builds an agent's capability set from a model's base capabilities plus the
/// skills that apply to it. This is the seam that lets the Talent Tree evolve:
/// the returned capabilities are the ones `TalentTree::available_capabilities`
/// should consider as "already had".
pub fn build_agent_capabilities(
    model_caps: Vec<CapabilityClaim>,
    registry: &SkillRegistry,
) -> CapabilityBuild {
    let base_kinds: Vec<CapabilityKind> = model_caps.iter().map(|c| c.capability).collect();
    let applicable = registry.applicable_skills(&base_kinds);

    let mut unlocked = Vec::new();
    let mut seen: BTreeSet<CapabilityKind> = base_kinds.iter().copied().collect();
    for skill in applicable {
        let dataset = registry.dataset(&skill.dataset_id);
        // Capabilities come from the DATASET's develops (evidence) with the
        // dataset's provenance — never from skill.develops (which is validated
        // to be a subset, but the dataset is the authoritative evidence
        // source). This prevents a skill from lending a dataset's Verified
        // provenance to capabilities the dataset does not actually develop.
        let Some(dataset) = dataset else { continue };
        let prov = dataset.provenance;
        for cap in dataset.develops.iter().copied() {
            if seen.insert(cap) {
                unlocked.push(CapabilityClaim {
                    capability: cap,
                    provenance: prov,
                });
            }
        }
    }
    CapabilityBuild {
        base: model_caps,
        unlocked,
    }
}

/// Builds the demonstration dataset/skill registry (the P8 dataset demo).
///
/// Single source of truth for the seeded example so the CLI (`agent skill`)
/// and the runtime (`/v1/skills` view) render the exact same data — never
/// duplicated frontend constants. Clearly labelled demonstration data, not
/// production evidence: it exists to show the dataset → skill → capability
/// mechanism, and its capability claims are marked with their provenance.
pub fn demo_skill_registry() -> SkillRegistry {
    let mut registry = SkillRegistry::new();
    let dataset = DatasetDescriptor::new(
        "code_finetune_2024",
        "Code fine-tune 2024",
        vec![CapabilityKind::Coding, CapabilityKind::ToolCalling],
        DatasetKind::FineTune,
    )
    .from("hf:example/code-finetune")
    .sized(10 * 1024 * 1024 * 1024, 0.9)
    .with_provenance(Provenance::Verified)
    .with_license("MIT");
    // A registered dataset is not silently inserted — the demo is deterministic.
    let _ = registry.add_dataset(dataset);
    let _ = registry.add_skill(
        SkillDescriptor::new(
            "code-agent",
            "Code agent",
            "code_finetune_2024",
            Some(CapabilityKind::Coding),
            vec![CapabilityKind::Coding, CapabilityKind::ToolCalling],
        )
        .with_prerequisites(vec![CapabilityKind::Reasoning])
        .with_resource(1024),
    );
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coding_dataset() -> DatasetDescriptor {
        DatasetDescriptor::new(
            "code_finetune_2024",
            "Code fine-tune 2024",
            vec![CapabilityKind::Coding, CapabilityKind::ToolCalling],
            DatasetKind::FineTune,
        )
        .from("hf:some/code-finetune")
        .sized(10 * 1024 * 1024 * 1024, 0.9)
        .with_provenance(Provenance::Verified)
        .with_license("MIT")
    }

    #[test]
    fn dataset_round_trips_and_clamps_quality() {
        let d = coding_dataset();
        let json = serde_json::to_string(&d).unwrap();
        let back: DatasetDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
        assert_eq!(d.quality, 0.9);
        assert!(d.develops(CapabilityKind::Coding));
        assert!(!d.develops(CapabilityKind::Ocr));
    }

    #[test]
    fn dataset_clamps_quality_to_unit_range() {
        let d = coding_dataset().sized(100, 1.7);
        assert_eq!(d.quality, 1.0);
        let d2 = coding_dataset().sized(100, -0.5);
        assert_eq!(d2.quality, 0.0);
    }

    #[test]
    fn skill_requires_model_and_prerequisites() {
        let skill = SkillDescriptor::new(
            "code-agent",
            "Code agent",
            "code_finetune_2024",
            Some(CapabilityKind::Coding),
            vec![CapabilityKind::Agents],
        );
        // Model has Coding but not a prerequisite — still applicable here
        // (no prerequisites set). Check base-capability gating.
        assert!(skill.applicable_to(&[CapabilityKind::Coding], Some(&coding_dataset())));
        assert!(!skill.applicable_to(&[CapabilityKind::Chat], Some(&coding_dataset())));
    }

    #[test]
    fn skill_registry_requires_known_dataset() {
        let mut reg = SkillRegistry::new();
        let skill = SkillDescriptor::new("s", "S", "missing-dataset", None, vec![]);
        assert!(matches!(
            reg.add_skill(skill),
            Err(DatasetError::UnknownDataset { .. })
        ));
        reg.add_dataset(coding_dataset()).unwrap();
        let skill2 = SkillDescriptor::new("s", "S", "code_finetune_2024", None, vec![]);
        assert!(reg.add_skill(skill2).is_ok());
        assert!(matches!(
            reg.add_dataset(coding_dataset()),
            Err(DatasetError::DuplicateDataset { .. })
        ));
    }

    #[test]
    fn build_agent_capabilities_unlocks_skills() {
        let mut reg = SkillRegistry::new();
        reg.add_dataset(coding_dataset()).unwrap();
        reg.add_skill(
            SkillDescriptor::new(
                "code-agent",
                "Code agent",
                "code_finetune_2024",
                Some(CapabilityKind::Coding),
                vec![CapabilityKind::ToolCalling],
            )
            .with_prerequisites(vec![CapabilityKind::Reasoning]),
        )
        .unwrap();

        // Model with Coding + Reasoning → unlocks ToolCalling.
        let build = build_agent_capabilities(
            vec![
                CapabilityClaim {
                    capability: CapabilityKind::Coding,
                    provenance: Provenance::Inferred,
                },
                CapabilityClaim {
                    capability: CapabilityKind::Reasoning,
                    provenance: Provenance::Inferred,
                },
            ],
            &reg,
        );
        let all = build.all();
        assert!(
            all.iter()
                .any(|c| c.capability == CapabilityKind::ToolCalling)
        );
        assert!(all.iter().any(|c| c.capability == CapabilityKind::Coding));
    }

    #[test]
    fn build_agent_capabilities_skips_unsatisfied_prerequisites() {
        let mut reg = SkillRegistry::new();
        reg.add_dataset(coding_dataset()).unwrap();
        reg.add_skill(
            SkillDescriptor::new(
                "code-agent",
                "Code agent",
                "code_finetune_2024",
                Some(CapabilityKind::Coding),
                vec![CapabilityKind::ToolCalling],
            )
            .with_prerequisites(vec![CapabilityKind::Reasoning]),
        )
        .unwrap();

        // Model has Coding but NOT Reasoning → skill not applied.
        let build = build_agent_capabilities(
            vec![CapabilityClaim {
                capability: CapabilityKind::Coding,
                provenance: Provenance::Inferred,
            }],
            &reg,
        );
        assert!(
            !build
                .all()
                .iter()
                .any(|c| c.capability == CapabilityKind::ToolCalling)
        );
        assert!(build.unlocked.is_empty());
    }

    #[test]
    fn all_dedups_and_prefers_verified_provenance() {
        let build = CapabilityBuild {
            base: vec![CapabilityClaim {
                capability: CapabilityKind::Coding,
                provenance: Provenance::Inferred,
            }],
            unlocked: vec![CapabilityClaim {
                capability: CapabilityKind::Coding,
                provenance: Provenance::Verified,
            }],
        };
        let all = build.all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].provenance, Provenance::Verified);
    }

    #[test]
    fn registry_lists_sorted() {
        let mut reg = SkillRegistry::new();
        reg.add_dataset(DatasetDescriptor::new(
            "b",
            "B",
            vec![CapabilityKind::Chat],
            DatasetKind::Training,
        ))
        .unwrap();
        reg.add_dataset(DatasetDescriptor::new(
            "a",
            "A",
            vec![CapabilityKind::Coding],
            DatasetKind::Training,
        ))
        .unwrap();
        let ids: Vec<&str> = reg.datasets().iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn skill_cannot_declare_capabilities_outside_its_dataset() {
        // Integrity invariant (audit B): a skill must not unlock capabilities
        // its dataset does not develop — otherwise a Verified dataset could
        // lend Verified provenance to capabilities it never trained for.
        let mut reg = SkillRegistry::new();
        reg.add_dataset(
            DatasetDescriptor::new(
                "code_finetune_2024",
                "Code fine-tune",
                vec![CapabilityKind::Coding],
                DatasetKind::FineTune,
            )
            .with_provenance(Provenance::Verified),
        )
        .unwrap();

        // A skill claiming Ocr (not developed by the code dataset) is rejected.
        let bad = SkillDescriptor::new(
            "bad-skill",
            "Bad",
            "code_finetune_2024",
            Some(CapabilityKind::Coding),
            vec![CapabilityKind::Ocr],
        );
        assert!(matches!(
            reg.add_skill(bad),
            Err(DatasetError::SkillDevelopsNotInDataset {
                capability: CapabilityKind::Ocr,
                ..
            })
        ));

        // A skill claiming only capabilities the dataset develops is accepted.
        let good = SkillDescriptor::new(
            "good-skill",
            "Good",
            "code_finetune_2024",
            Some(CapabilityKind::Coding),
            vec![CapabilityKind::Coding],
        );
        assert!(reg.add_skill(good).is_ok());
    }

    #[test]
    fn build_unlocks_dataset_develops_not_skill_develops() {
        // The unlocked capabilities must come from the dataset (evidence),
        // never from a skill's own declaration.
        let mut reg = SkillRegistry::new();
        reg.add_dataset(
            DatasetDescriptor::new(
                "code_finetune_2024",
                "Code fine-tune",
                vec![CapabilityKind::Coding, CapabilityKind::ToolCalling],
                DatasetKind::FineTune,
            )
            .with_provenance(Provenance::Verified),
        )
        .unwrap();
        reg.add_skill(SkillDescriptor::new(
            "code-agent",
            "Code agent",
            "code_finetune_2024",
            Some(CapabilityKind::Coding),
            vec![CapabilityKind::Coding], // skill only re-declares Coding
        ))
        .unwrap();

        // Model has only Coding + Reasoning (no ToolCalling yet).
        let build = build_agent_capabilities(
            vec![
                CapabilityClaim {
                    capability: CapabilityKind::Coding,
                    provenance: Provenance::Inferred,
                },
                CapabilityClaim {
                    capability: CapabilityKind::Reasoning,
                    provenance: Provenance::Inferred,
                },
            ],
            &reg,
        );
        // ToolCalling is unlocked because the DATASET develops it (evidence),
        // even though the skill only re-declared Coding.
        let all = build.all();
        assert!(
            all.iter()
                .any(|c| c.capability == CapabilityKind::ToolCalling)
        );
        assert!(all.iter().any(|c| c.capability == CapabilityKind::Coding));
        // Unlocked ToolCalling carries the dataset's Verified provenance.
        let tc = all
            .iter()
            .find(|c| c.capability == CapabilityKind::ToolCalling)
            .unwrap();
        assert_eq!(tc.provenance, Provenance::Verified);
    }
}

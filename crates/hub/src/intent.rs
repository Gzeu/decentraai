//! Deterministic intent → capability resolution (Intent Planner, Phase L).
//!
//! The goal: turn "I need OCR and summarization" into a concrete set of
//! [`CapabilityKind`]s — deterministically and explainably, with no opaque AI
//! scoring. This is the FIRST step of the Intent Planner: it maps a
//! natural-language intent to the capabilities it *probably* requires, and the
//! caller then verifies actual model support against persisted claims using
//! the existing [`crate::requirements`] matcher.
//!
//! Honesty rule (§49): intent → capability is a **keyword heuristic**, so the
//! result is inherently **INFERRED** at this layer. This module NEVER claims a
//! model can do anything — it only proposes which capabilities the intent
//! points at. Whether a model genuinely supports a capability is verified
//! separately, against real persisted claims, via `match_requirements`. An
//! intent with no recognized keyword yields an empty list (honest UNKNOWN), it
//! is never guessed.
//!
//! The module is pure: no I/O, no async, uses only `std` and the existing
//! capability types.

use crate::capability::CapabilityKind;
use crate::requirements::{CapabilityRequirement, EvidenceLevel};

/// A conservative keyword → capability lexicon. Ordered: earlier entries win
/// on first match, and the input is lowercased/trimmed before scanning, so the
/// order determines precedence for overlapping phrases (e.g. "text to image"
/// is checked before the generic "image").
const LEXICON: &[(&[&str], CapabilityKind)] = &[
    (
        &["vision", "image understanding", "analyze image", "image"],
        CapabilityKind::Vision,
    ),
    (
        &["ocr", "extract text", "read text in image"],
        CapabilityKind::Ocr,
    ),
    (
        &["summarize", "summary", "summarization"],
        CapabilityKind::Summarization,
    ),
    (&["coding", "code"], CapabilityKind::Coding),
    (&["translation", "translate"], CapabilityKind::Translation),
    (&["chat"], CapabilityKind::Chat),
    (&["embeddings", "embed"], CapabilityKind::Embeddings),
    (&["tool", "function call"], CapabilityKind::ToolCalling),
    (
        &["speech to text", "transcribe", "transcription"],
        CapabilityKind::SpeechToText,
    ),
    (
        &["image generation", "text to image"],
        CapabilityKind::ImageGeneration,
    ),
    (
        &["classification", "classify"],
        CapabilityKind::Classification,
    ),
    // --- appended conservative entries (relative order kept stable) ---
    (
        &["retrieval", "search", "find relevant"],
        CapabilityKind::Retrieval,
    ),
    (&["rerank", "reranking"], CapabilityKind::Reranking),
    (
        &[
            "reason",
            "reasoning",
            "think step by step",
            "chain of thought",
        ],
        CapabilityKind::Reasoning,
    ),
    (
        &[
            "structured output",
            "json output",
            "output json",
            "json mode",
        ],
        CapabilityKind::StructuredOutput,
    ),
    (&["agent", "agents", "autonomous"], CapabilityKind::Agents),
    (
        &["text to speech", "tts", "speech synthesis"],
        CapabilityKind::TextToSpeech,
    ),
    (&["audio", "speech"], CapabilityKind::Audio),
    (&["multimodal"], CapabilityKind::Multimodal),
    (
        &[
            "document understanding",
            "understand document",
            "understand",
            "pdf understanding",
        ],
        CapabilityKind::DocumentUnderstanding,
    ),
    (&["video", "video understanding"], CapabilityKind::Video),
];

/// Normalize an intent for keyword scanning: lowercase + trim.
fn normalize(text: &str) -> String {
    text.trim().to_lowercase()
}

/// Map a natural-language intent to the capability kinds it points at.
///
/// Conservative and deterministic: each keyword is looked up as a substring of
/// the normalized input; matches are deduplicated while preserving lexicon
/// order. Returns an empty vec for unknown / empty input (honest UNKNOWN).
///
/// The result is INFERRED — see the module docs.
pub fn capabilities_for_intent(text: &str) -> Vec<CapabilityKind> {
    let normalized = normalize(text);
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (keywords, capability) in LEXICON {
        if keywords.iter().any(|k| normalized.contains(k)) {
            out.push(*capability);
        }
    }
    out
}

/// A short human-readable label for a capability kind, for presentation only
/// (chips/badges/tooltips). Thin wrapper over [`CapabilityKind::label`].
///
/// Presentation-only: the [`CapabilityKind`] itself is the machine value
/// resolved from an intent; the label is never used for matching, ranking, or
/// any decision logic.
pub fn capability_label(capability: CapabilityKind) -> &'static str {
    capability.label()
}

/// Map an intent to its capabilities together with their presentation labels.
///
/// Convenience over [`capabilities_for_intent`] + [`capability_label`]; the
/// capability kind is the machine value, the label is display-only.
pub fn intent_capabilities_with_labels(text: &str) -> Vec<(CapabilityKind, &'static str)> {
    capabilities_for_intent(text)
        .into_iter()
        .map(|capability| (capability, capability_label(capability)))
        .collect()
}

/// The same intent mapping, expressed as [`CapabilityRequirement`]s so callers
/// can feed it to [`crate::requirements::match_requirements`].
///
/// `evidence` sets the strength required for every capability. This still
/// describes the *intent* (INFERRED), not a model claim.
pub fn intent_requirements(text: &str, evidence: EvidenceLevel) -> Vec<CapabilityRequirement> {
    capabilities_for_intent(text)
        .into_iter()
        .map(|capability| CapabilityRequirement {
            capability,
            evidence,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_and_summarization_intent() {
        let caps = capabilities_for_intent("I need OCR and summarization");
        assert_eq!(
            caps,
            vec![CapabilityKind::Ocr, CapabilityKind::Summarization]
        );
    }

    #[test]
    fn summarize_this_meeting() {
        assert_eq!(
            capabilities_for_intent("summarize this meeting"),
            vec![CapabilityKind::Summarization]
        );
    }

    #[test]
    fn single_unknown_word_is_empty() {
        assert!(capabilities_for_intent("falafel").is_empty());
    }

    #[test]
    fn images_and_text_intent_maps_vision_then_ocr() {
        let caps = capabilities_for_intent("analyze these images and extract text");
        assert_eq!(caps, vec![CapabilityKind::Vision, CapabilityKind::Ocr]);
    }

    #[test]
    fn dedup_preserves_order() {
        assert_eq!(
            capabilities_for_intent("ocr and OCR"),
            vec![CapabilityKind::Ocr]
        );
    }

    #[test]
    fn empty_string_is_empty() {
        assert!(capabilities_for_intent("").is_empty());
        assert!(capabilities_for_intent("   ").is_empty());
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(
            capabilities_for_intent("I NEED OCR AND SUMMARIZATION"),
            vec![CapabilityKind::Ocr, CapabilityKind::Summarization]
        );
    }

    #[test]
    fn search_intent_maps_to_retrieval() {
        assert!(
            capabilities_for_intent("search for relevant documents")
                .contains(&CapabilityKind::Retrieval)
        );
    }

    #[test]
    fn rerank_intent_maps_to_reranking() {
        assert!(
            capabilities_for_intent("rerank these results").contains(&CapabilityKind::Reranking)
        );
    }

    #[test]
    fn think_step_by_step_maps_to_reasoning() {
        assert!(capabilities_for_intent("think step by step").contains(&CapabilityKind::Reasoning));
    }

    #[test]
    fn output_json_maps_to_structured_output() {
        assert!(capabilities_for_intent("output json").contains(&CapabilityKind::StructuredOutput));
    }

    #[test]
    fn autonomous_agent_maps_to_agents() {
        assert!(capabilities_for_intent("autonomous agent").contains(&CapabilityKind::Agents));
    }

    #[test]
    fn text_to_speech_maps_to_tts() {
        assert!(capabilities_for_intent("text to speech").contains(&CapabilityKind::TextToSpeech));
    }

    #[test]
    fn multimodal_maps_to_multimodal() {
        assert!(
            capabilities_for_intent("multimodal understanding")
                .contains(&CapabilityKind::Multimodal)
        );
    }

    #[test]
    fn understand_document_maps_to_document_understanding() {
        assert!(
            capabilities_for_intent("understand this document")
                .contains(&CapabilityKind::DocumentUnderstanding)
        );
    }

    #[test]
    fn same_capability_multiple_phrases_dedups() {
        // "audio" and "speech" both match the Audio capability → it appears once.
        let caps = capabilities_for_intent("audio and speech");
        assert_eq!(
            caps.iter().filter(|c| **c == CapabilityKind::Audio).count(),
            1
        );
    }

    #[test]
    fn labels_are_presentational() {
        assert!(!capability_label(CapabilityKind::Retrieval).is_empty());
        assert_eq!(capability_label(CapabilityKind::Ocr), "OCR");
    }

    #[test]
    fn labels_pair_up_with_capabilities() {
        let pairs = intent_capabilities_with_labels("summarize this meeting");
        assert_eq!(
            pairs,
            vec![(CapabilityKind::Summarization, "summarization")]
        );
    }

    #[test]
    fn intent_requirements_carry_given_evidence() {
        let reqs = intent_requirements("summarize and translate", EvidenceLevel::Verified);
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].capability, CapabilityKind::Summarization);
        assert_eq!(reqs[0].evidence, EvidenceLevel::Verified);
        assert_eq!(reqs[1].capability, CapabilityKind::Translation);
        assert_eq!(reqs[1].evidence, EvidenceLevel::Verified);

        let any = intent_requirements("chat", EvidenceLevel::Any);
        assert_eq!(any[0].capability, CapabilityKind::Chat);
        assert_eq!(any[0].evidence, EvidenceLevel::Any);
    }
}

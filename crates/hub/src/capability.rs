//! Capability taxonomy for DecentraAI models (Issue #26 §4–§6, §31).
//!
//! The Hub is a *capability marketplace*, not a raw HuggingFace frontend:
//! users should be able to go from "WHAT DO I WANT TO DO?" to compatible
//! models. This module classifies a model from real Hub metadata
//! (pipeline tag, tags, id) into semantic capabilities and tasks.
//!
//! Honesty rules (§4, §49):
//! - VERIFIED claims come from metadata the Hub states explicitly (pipeline
//!   tag, `tools` tag).
//! - INFERRED claims come from name/tag heuristics (e.g. a repo named
//!   `codestral` is inferred coding-capable). They are always labeled.
//! - We NEVER claim a capability merely because it would be convenient.
//!
//! The taxonomy is pure: no I/O, no async. The Hub search / model detail
//! paths serialize it for the admin API.

use serde::{Deserialize, Serialize};

/// A capability category a model may genuinely have.
///
/// Wire-compatible: derives `Deserialize` so capability claims can travel
/// inside agent advertisements and be parsed by other nodes (the collective
/// fabric uses this taxonomy as the *semantic* half of its capability model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Chat,
    TextGeneration,
    Reasoning,
    Coding,
    Agents,
    ToolCalling,
    FunctionCalling,
    StructuredOutput,
    Vision,
    Multimodal,
    Ocr,
    DocumentUnderstanding,
    Embeddings,
    Reranking,
    Retrieval,
    SpeechToText,
    TextToSpeech,
    Audio,
    Translation,
    Summarization,
    Classification,
    ImageGeneration,
    Video,
    Local,
    SmallModels,
    Experimental,
}

impl CapabilityKind {
    /// Every capability in the taxonomy, in its snake_case wire form.
    ///
    /// The Fabric Intelligence layer hands this list to the planning model as
    /// the ONLY allowed vocabulary, so a hallucinated capability name can be
    /// rejected deterministically before any fabric validation runs. Kept
    /// here (not in the intelligence crate) so the taxonomy has ONE source
    /// of truth: adding a variant to the enum without extending this list
    /// fails the `all_names_covers_every_variant` test.
    pub const ALL_NAMES: &[&'static str] = &[
        "chat",
        "text_generation",
        "reasoning",
        "coding",
        "agents",
        "tool_calling",
        "function_calling",
        "structured_output",
        "vision",
        "multimodal",
        "ocr",
        "document_understanding",
        "embeddings",
        "reranking",
        "retrieval",
        "speech_to_text",
        "text_to_speech",
        "audio",
        "translation",
        "summarization",
        "classification",
        "image_generation",
        "video",
        "local",
        "small_models",
        "experimental",
    ];

    /// Short human label for chips/badges.
    pub fn label(&self) -> &'static str {
        match self {
            CapabilityKind::Chat => "chat",
            CapabilityKind::TextGeneration => "text generation",
            CapabilityKind::Reasoning => "reasoning",
            CapabilityKind::Coding => "coding",
            CapabilityKind::Agents => "agents",
            CapabilityKind::ToolCalling => "tool calling",
            CapabilityKind::FunctionCalling => "function calling",
            CapabilityKind::StructuredOutput => "structured output",
            CapabilityKind::Vision => "vision",
            CapabilityKind::Multimodal => "multimodal",
            CapabilityKind::Ocr => "OCR",
            CapabilityKind::DocumentUnderstanding => "document understanding",
            CapabilityKind::Embeddings => "embeddings",
            CapabilityKind::Reranking => "reranking",
            CapabilityKind::Retrieval => "retrieval",
            CapabilityKind::SpeechToText => "speech-to-text",
            CapabilityKind::TextToSpeech => "text-to-speech",
            CapabilityKind::Audio => "audio",
            CapabilityKind::Translation => "translation",
            CapabilityKind::Summarization => "summarization",
            CapabilityKind::Classification => "classification",
            CapabilityKind::ImageGeneration => "image generation",
            CapabilityKind::Video => "video",
            CapabilityKind::Local => "local/edge",
            CapabilityKind::SmallModels => "small models",
            CapabilityKind::Experimental => "experimental",
        }
    }
}

impl std::str::FromStr for CapabilityKind {
    type Err = ();
    /// Parse a capability from its snake_case serialized form (e.g. `ocr`,
    /// `text_generation`, `tool_calling`). Unknown strings error.
    fn from_str(s: &str) -> Result<Self, Self::Err> {        Ok(match s {
            "chat" => CapabilityKind::Chat,
            "text_generation" => CapabilityKind::TextGeneration,
            "reasoning" => CapabilityKind::Reasoning,
            "coding" => CapabilityKind::Coding,
            "agents" => CapabilityKind::Agents,
            "tool_calling" => CapabilityKind::ToolCalling,
            "function_calling" => CapabilityKind::FunctionCalling,
            "structured_output" => CapabilityKind::StructuredOutput,
            "vision" => CapabilityKind::Vision,
            "multimodal" => CapabilityKind::Multimodal,
            "ocr" => CapabilityKind::Ocr,
            "document_understanding" => CapabilityKind::DocumentUnderstanding,
            "embeddings" => CapabilityKind::Embeddings,
            "reranking" => CapabilityKind::Reranking,
            "retrieval" => CapabilityKind::Retrieval,
            "speech_to_text" => CapabilityKind::SpeechToText,
            "text_to_speech" => CapabilityKind::TextToSpeech,
            "audio" => CapabilityKind::Audio,
            "translation" => CapabilityKind::Translation,
            "summarization" => CapabilityKind::Summarization,
            "classification" => CapabilityKind::Classification,
            "image_generation" => CapabilityKind::ImageGeneration,
            "video" => CapabilityKind::Video,
            "local" => CapabilityKind::Local,
            "small_models" => CapabilityKind::SmallModels,
            "experimental" => CapabilityKind::Experimental,
            _ => return Err(()),
        })
    }
}

/// How a capability claim was obtained. Never hidden from the user (§31).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Stated explicitly by the source metadata (e.g. Hub pipeline tag).
    Verified,
    /// Derived by a deterministic heuristic (e.g. repo id contains "code").
    Inferred,
}

/// One capability claim with its provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityClaim {
    pub capability: CapabilityKind,
    pub provenance: Provenance,
}

/// The full capability view of a model: claims plus the task-level filters
/// that are supported by the claimed capabilities (§5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCapabilities {
    pub claims: Vec<CapabilityClaim>,
    /// Task filters exposed for this model, grouped by capability. Derived
    /// deterministically from the claims — a task is only present when its
    /// capability is.
    pub tasks: Vec<TaskEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEntry {
    pub capability: CapabilityKind,
    pub task: String,
}

/// Deterministic task list per capability (§5). Only capabilities we can
/// back with real metadata expose tasks.
pub fn tasks_for(capability: CapabilityKind) -> &'static [&'static str] {
    match capability {
        CapabilityKind::Coding => &[
            "completion",
            "generation",
            "explanation",
            "review",
            "debugging",
            "repository understanding",
        ],
        CapabilityKind::Vision => &[
            "image understanding",
            "visual question answering",
            "OCR",
            "document understanding",
            "image classification",
        ],
        CapabilityKind::Agents | CapabilityKind::ToolCalling => &[
            "tool calling",
            "function calling",
            "structured output",
            "planning",
        ],
        CapabilityKind::Embeddings => &["semantic search", "retrieval", "clustering", "similarity"],
        CapabilityKind::SpeechToText => &[
            "transcription",
            "speech recognition",
            "translation",
            "speech synthesis",
        ],
        CapabilityKind::Summarization => &[
            "abstractive summarization",
            "extractive summarization",
            "meeting notes",
        ],
        CapabilityKind::Translation => &["machine translation", "document translation"],
        _ => &[],
    }
}

/// Classify a model from real Hub metadata.
///
/// `pipeline_tag` is the coarse category the Hub reports (VERIFIED when it
/// maps to a capability). `tags` are the Hub's raw tag list (we read
/// `tools`/`vision`/etc. as VERIFIED, plus `gguf` family markers).
/// `id` is the repo id (`org/name`) used only for conservative name
/// heuristics labeled INFERRED.
pub fn classify(pipeline_tag: Option<&str>, tags: &[String], id: &str) -> ModelCapabilities {
    let mut claims: Vec<CapabilityClaim> = Vec::new();

    // --- VERIFIED from the Hub pipeline tag ---
    match pipeline_tag {
        Some("text-generation") | Some("text2text-generation") => {
            // The Hub states the model *generates text* — that is VERIFIED.
            // Whether it is a *chat* model (multi-turn conversational
            // template) is NOT stated by this pipeline tag, so Chat is only
            // INFERRED. Honesty rule (§49): never claim a capability the
            // metadata does not state explicitly.
            claims.push(claim(CapabilityKind::TextGeneration, Provenance::Verified));
            claims.push(claim(CapabilityKind::Chat, Provenance::Inferred));
        }
        Some("image-text-to-text") | Some("image-to-text") | Some("visual-question-answering") => {
            claims.push(claim(CapabilityKind::Vision, Provenance::Verified));
            claims.push(claim(CapabilityKind::Multimodal, Provenance::Verified));
        }
        Some("automatic-speech-recognition") => {
            claims.push(claim(CapabilityKind::SpeechToText, Provenance::Verified));
            claims.push(claim(CapabilityKind::Audio, Provenance::Verified));
        }
        Some("text-to-audio") | Some("text-to-speech") => {
            claims.push(claim(CapabilityKind::TextToSpeech, Provenance::Verified));
            claims.push(claim(CapabilityKind::Audio, Provenance::Verified));
        }
        Some("translation") => {
            claims.push(claim(CapabilityKind::Translation, Provenance::Verified));
        }
        Some("summarization") => {
            claims.push(claim(CapabilityKind::Summarization, Provenance::Verified));
        }
        Some("text-classification")
        | Some("token-classification")
        | Some("zero-shot-classification")
        | Some("question-answering") => {
            claims.push(claim(CapabilityKind::Classification, Provenance::Verified));
        }
        Some("feature-extraction") | Some("sentence-similarity") => {
            claims.push(claim(CapabilityKind::Embeddings, Provenance::Verified));
        }
        Some("text-to-image") => {
            claims.push(claim(CapabilityKind::ImageGeneration, Provenance::Verified));
        }
        Some("text-to-video") => {
            claims.push(claim(CapabilityKind::Video, Provenance::Verified));
        }
        Some(_) | None => {}
    }

    // --- VERIFIED from Hub tags the model itself declares ---
    let has = |needle: &str| tags.iter().any(|t| t.eq_ignore_ascii_case(needle));
    if has("tools") || has("tool-calling") || has("function-calling") {
        // The tag states tool/function calling — that is VERIFIED. "Agents"
        // implies a broader autonomous capability beyond the tag, so it is
        // only INFERRED (honesty rule §49: never invent).
        claims.push(claim(CapabilityKind::ToolCalling, Provenance::Verified));
        claims.push(claim(CapabilityKind::FunctionCalling, Provenance::Verified));
        claims.push(claim(CapabilityKind::Agents, Provenance::Inferred));
    }
    if has("structured-output") || has("json-mode") || has("json") {
        claims.push(claim(
            CapabilityKind::StructuredOutput,
            Provenance::Verified,
        ));
    }
    if has("vision") {
        claims.push(claim(CapabilityKind::Vision, Provenance::Verified));
        claims.push(claim(CapabilityKind::Multimodal, Provenance::Verified));
    }
    if has("image-to-text") {
        claims.push(claim(CapabilityKind::Ocr, Provenance::Inferred));
    }
    if has("ocr") || has("document-ai") {
        claims.push(claim(CapabilityKind::Ocr, Provenance::Verified));
        claims.push(claim(
            CapabilityKind::DocumentUnderstanding,
            Provenance::Verified,
        ));
    }
    if has("embeddings") || has("sentence-transformers") || has("feature-extraction") {
        claims.push(claim(CapabilityKind::Embeddings, Provenance::Verified));
        claims.push(claim(CapabilityKind::Retrieval, Provenance::Inferred));
    }
    if has("reranker") || has("reranking") {
        claims.push(claim(CapabilityKind::Reranking, Provenance::Verified));
    }
    if has("audio") || has("speech") {
        claims.push(claim(CapabilityKind::Audio, Provenance::Verified));
    }

    // --- INFERRED from conservative name heuristics ---
    let id_lower = id.to_lowercase();
    let name_has = |needle: &str| id_lower.contains(needle);

    // BUGFIX (audit finding): a bare `name_has("code")` substring check
    // false-positives on "encoder", "decoder", "codec" and "vocoder" — "code"
    // is a literal substring of all four (e.g. "en-C-O-D-E-r"). Any
    // embedding/vision/audio model with one of those extremely common words in
    // its name was silently mislabeled `Coding: Inferred`, directly
    // contradicting this module's own "never claim a capability merely because
    // it would be convenient" rule. Exclude the codec/encoder/decoder family
    // explicitly before applying the broad "code" check, and detect the real
    // codellama/code-llama family the old heuristic actually missed.
    let is_codec_family =
        name_has("encoder") || name_has("decoder") || name_has("codec") || name_has("vocoder");
    if !is_codec_family
        && (name_has("codestral")
            || name_has("code")
            || name_has("coder")
            || name_has("starcoder")
            || name_has("codellama")
            || name_has("code-llama"))
    {
        push_unique(&mut claims, CapabilityKind::Coding, Provenance::Inferred);
    }
    if name_has("reason") || name_has("think") || name_has("deepseek-r1") || name_has("qwq") {
        push_unique(&mut claims, CapabilityKind::Reasoning, Provenance::Inferred);
    }
    if name_has("llava") || name_has("vision") || name_has("qwen2-vl") || name_has("qwen2.5-vl") {
        push_unique(&mut claims, CapabilityKind::Vision, Provenance::Inferred);
        push_unique(
            &mut claims,
            CapabilityKind::Multimodal,
            Provenance::Inferred,
        );
    }
    if name_has("embed") || name_has("e5-") || name_has("bge-") || name_has("gte-") {
        push_unique(
            &mut claims,
            CapabilityKind::Embeddings,
            Provenance::Inferred,
        );
        push_unique(&mut claims, CapabilityKind::Retrieval, Provenance::Inferred);
    }
    if name_has("rerank") {
        push_unique(&mut claims, CapabilityKind::Reranking, Provenance::Inferred);
    }
    if name_has("whisper") {
        push_unique(
            &mut claims,
            CapabilityKind::SpeechToText,
            Provenance::Inferred,
        );
        push_unique(&mut claims, CapabilityKind::Audio, Provenance::Inferred);
    }
    if name_has("tinyllama")
        || name_has("small")
        || name_has("phi-3-mini")
        || name_has("phi-4-mini")
    {
        push_unique(
            &mut claims,
            CapabilityKind::SmallModels,
            Provenance::Inferred,
        );
        push_unique(&mut claims, CapabilityKind::Local, Provenance::Inferred);
    }
    if name_has("experimental") || name_has("dev") {
        push_unique(
            &mut claims,
            CapabilityKind::Experimental,
            Provenance::Inferred,
        );
    }

    // Deduplicate while preserving order.
    let mut seen = std::collections::HashSet::new();
    claims.retain(|c| seen.insert((c.capability, c.provenance)));

    // Task entries: only for capabilities that expose tasks.
    let mut tasks = Vec::new();
    for claim in &claims {
        for task in tasks_for(claim.capability) {
            tasks.push(TaskEntry {
                capability: claim.capability,
                task: task.to_string(),
            });
        }
    }

    ModelCapabilities { claims, tasks }
}

fn claim(capability: CapabilityKind, provenance: Provenance) -> CapabilityClaim {
    CapabilityClaim {
        capability,
        provenance,
    }
}

fn push_unique(
    claims: &mut Vec<CapabilityClaim>,
    capability: CapabilityKind,
    provenance: Provenance,
) {
    if !claims
        .iter()
        .any(|c| c.capability == capability && c.provenance == provenance)
    {
        claims.push(claim(capability, provenance));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// The Fabric Intelligence layer uses ALL_NAMES as the planning model's
    /// only allowed vocabulary. This test pins it to the enum: adding a
    /// CapabilityKind variant without extending ALL_NAMES must fail here,
    /// otherwise the intelligence layer could never propose the new
    /// capability.
    #[test]
    fn all_names_covers_every_variant() {
        for name in CapabilityKind::ALL_NAMES {
            let parsed: CapabilityKind = name.parse().expect("ALL_NAMES entry must parse");
            assert_eq!(
                serde_json::to_value(parsed).unwrap(),
                serde_json::json!(name),
                "wire form drift between ALL_NAMES and serde rename"
            );
        }
        // And nothing extra: every variant appears exactly once in the list.
        assert_eq!(CapabilityKind::ALL_NAMES.len(), 26);
    }

    #[test]
    fn text_generation_pipeline_verified_generation_but_not_verified_chat() {
        let caps = classify(Some("text-generation"), &tags(&["gguf"]), "org/any-model");
        // The pipeline tag states text generation — VERIFIED.
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::TextGeneration && c.provenance == Provenance::Verified
        }));
        // It does NOT state chat capability — Chat must never be VERIFIED
        // (honesty §49), only INFERRED.
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Chat && c.provenance == Provenance::Inferred
        }));
        assert!(!caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Chat && c.provenance == Provenance::Verified
        }));
        assert!(caps.tasks.is_empty(), "plain chat exposes no task filters");
    }

    #[test]
    fn tools_tag_claims_tool_calling_verified() {
        let caps = classify(
            Some("text-generation"),
            &tags(&["gguf", "tools", "function-calling", "json-mode"]),
            "org/agent-model",
        );
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::ToolCalling && c.provenance == Provenance::Verified
        }));
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::StructuredOutput && c.provenance == Provenance::Verified
        }));
        // The tag states tool/function calling (VERIFIED) but not full agentic
        // capability — Agents must be INFERRED, never VERIFIED (honesty §49).
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Agents && c.provenance == Provenance::Inferred
        }));
        assert!(!caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Agents && c.provenance == Provenance::Verified
        }));
        // Tool calling exposes task filters.
        assert!(caps.tasks.iter().any(|t| t.task == "tool calling"));
    }

    #[test]
    fn coding_is_inferred_from_name_only() {
        let caps = classify(
            Some("text-generation"),
            &tags(&["gguf"]),
            "org/codestral-7b",
        );
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Coding && c.provenance == Provenance::Inferred
        }));
        // ...but NOT claimed verified.
        assert!(!caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Coding && c.provenance == Provenance::Verified
        }));
        // Coding tasks appear once coding is claimed.
        assert!(
            caps.tasks
                .iter()
                .any(|t| t.task == "repository understanding")
        );
    }

    #[test]
    fn encoder_named_models_never_claim_coding() {
        // Regression: "code" is a literal substring of encoder/decoder/codec/
        // vocoder — embedding/vision/audio models with those words in their
        // name must never be silently labeled Coding:Inferred.
        for id in [
            "org/bge-large-encoder-v1.5",
            "org/vae-decoder",
            "org/hifi-vocoder",
            "org/audio-codec-large",
        ] {
            let caps = classify(Some("feature-extraction"), &tags(&["gguf"]), id);
            assert!(
                !caps.claims.iter().any(|c| {
                    c.capability == CapabilityKind::Coding && c.provenance == Provenance::Inferred
                }),
                "{id} must not claim Coding"
            );
        }
    }

    #[test]
    fn real_coding_families_still_inferred() {
        // No loss of real detections: coder/starcoder/codestral/codellama.
        for id in [
            "org/coder-7b",
            "org/starcoder2-15b",
            "org/codestral-latest",
            "org/codellama-13b",
            "org/code-llama-34b",
        ] {
            let caps = classify(Some("text-generation"), &tags(&["gguf"]), id);
            assert!(
                caps.claims.iter().any(|c| {
                    c.capability == CapabilityKind::Coding && c.provenance == Provenance::Inferred
                }),
                "{id} must still claim Coding"
            );
        }
    }

    #[test]
    fn vision_pipeline_is_verified_multimodal() {
        let caps = classify(Some("image-text-to-text"), &tags(&["gguf"]), "org/vision-x");
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Vision && c.provenance == Provenance::Verified
        }));
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Multimodal && c.provenance == Provenance::Verified
        }));
    }

    #[test]
    fn unknown_pipeline_claims_nothing_false() {
        // A repo with an unmapped pipeline tag and an innocent name must not
        // claim any capability it cannot back.
        let caps = classify(
            Some("unknown-tag"),
            &tags(&["gguf", "license:mit"]),
            "org/innocent",
        );
        assert!(caps.claims.is_empty(), "claims: {:?}", caps.claims);
        assert!(caps.tasks.is_empty());
    }

    #[test]
    fn reranking_and_embeddings_claim_verified_from_tags() {
        let caps = classify(
            Some("feature-extraction"),
            &tags(&["gguf", "sentence-transformers", "reranking"]),
            "org/reranker",
        );
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Reranking && c.provenance == Provenance::Verified
        }));
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Embeddings && c.provenance == Provenance::Verified
        }));
    }
}

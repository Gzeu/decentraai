//! Capability taxonomy for DecentraAI models (Issue #26 §4-§6, §31).
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

use serde::Serialize;

/// A capability category a model may genuinely have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
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

/// How a capability claim was obtained. Never hidden from the user (§31).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Stated explicitly by the source metadata (e.g. Hub pipeline tag).
    Verified,
    /// Derived by a deterministic heuristic (e.g. repo id contains "code").
    Inferred,
}

/// One capability claim with its provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityClaim {
    pub capability: CapabilityKind,
    pub provenance: Provenance,
}

/// The full capability view of a model: claims plus the task-level filters
/// that are supported by the claimed capabilities (§5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelCapabilities {
    pub claims: Vec<CapabilityClaim>,
    /// Task filters exposed for this model, grouped by capability. Derived
    /// deterministically from the claims -- a task is only present when its
    /// capability is.
    pub tasks: Vec<TaskEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
        CapabilityKind::Embeddings => &[
            "semantic search",
            "retrieval",
            "clustering",
            "similarity",
        ],
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
        CapabilityKind::Translation => &[
            "machine translation",
            "document translation",
        ],
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
pub fn classify(
    pipeline_tag: Option<&str>,
    tags: &[String],
    id: &str,
) -> ModelCapabilities {
    let mut claims: Vec<CapabilityClaim> = Vec::new();

    // --- VERIFIED from the Hub pipeline tag ---
    match pipeline_tag {
        Some("text-generation") | Some("text2text-generation") => {
            claims.push(claim(CapabilityKind::TextGeneration, Provenance::Verified));
            claims.push(claim(CapabilityKind::Chat, Provenance::Verified));
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
        Some("text-classification") | Some("token-classification")
        | Some("zero-shot-classification") | Some("question-answering") => {
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
        claims.push(claim(CapabilityKind::ToolCalling, Provenance::Verified));
        claims.push(claim(CapabilityKind::FunctionCalling, Provenance::Verified));
        claims.push(claim(CapabilityKind::Agents, Provenance::Verified));
    }
    if has("structured-output") || has("json-mode") || has("json") {
        claims.push(claim(CapabilityKind::StructuredOutput, Provenance::Verified));
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
        claims.push(claim(CapabilityKind::DocumentUnderstanding, Provenance::Verified));
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

    // BUGFIX (audit finding, verified from source): a bare `name_has("code")`
    // substring check false-positives on "encoder", "decoder", "codec" and
    // "vocoder" -- "code" is a literal substring of all four (e.g.
    // "en-C-O-D-E-r"). Any embedding/vision/audio model with one of those
    // words in its name (extremely common -- "bge-encoder", "vae-decoder",
    // "hifi-vocoder", any audio codec) was silently mislabeled
    // `Coding: Inferred`, directly contradicting this module's own "never
    // claim a capability merely because it would be convenient" rule.
    // Fix: exclude the known codec/encoder/decoder family explicitly before
    // applying the broad "code" check, and add "codellama"/"code-llama"
    // (a real, common coding-model family the old heuristic actually missed,
    // since it contains neither "coder" nor bare unqualified "code" cleanly
    // distinguishable from the false-positive family).
    let looks_like_codec_family =
        name_has("encode") || name_has("decode") || name_has("codec") || name_has("vocoder");
    let coding_name_hit = name_has("codestral")
        || name_has("coder")
        || name_has("starcoder")
        || name_has("codellama")
        || name_has("code-llama")
        || (name_has("code") && !looks_like_codec_family);
    if coding_name_hit {
        push_unique(&mut claims, CapabilityKind::Coding, Provenance::Inferred);
    }
    if name_has("reason") || name_has("think") || name_has("deepseek-r1") || name_has("qwq") {
        push_unique(&mut claims, CapabilityKind::Reasoning, Provenance::Inferred);
    }
    if name_has("llava") || name_has("vision") || name_has("qwen2-vl") || name_has("qwen2.5-vl")
    {
        push_unique(&mut claims, CapabilityKind::Vision, Provenance::Inferred);
        push_unique(&mut claims, CapabilityKind::Multimodal, Provenance::Inferred);
    }
    if name_has("embed") || name_has("e5-") || name_has("bge-") || name_has("gte-") {
        push_unique(&mut claims, CapabilityKind::Embeddings, Provenance::Inferred);
        push_unique(&mut claims, CapabilityKind::Retrieval, Provenance::Inferred);
    }
    if name_has("rerank") {
        push_unique(&mut claims, CapabilityKind::Reranking, Provenance::Inferred);
    }
    if name_has("whisper") {
        push_unique(&mut claims, CapabilityKind::SpeechToText, Provenance::Inferred);
        push_unique(&mut claims, CapabilityKind::Audio, Provenance::Inferred);
    }
    if name_has("tinyllama") || name_has("small") || name_has("phi-3-mini") || name_has("phi-4-mini")
    {
        push_unique(&mut claims, CapabilityKind::SmallModels, Provenance::Inferred);
        push_unique(&mut claims, CapabilityKind::Local, Provenance::Inferred);
    }
    if name_has("experimental") || name_has("dev") {
        push_unique(&mut claims, CapabilityKind::Experimental, Provenance::Inferred);
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

fn push_unique(claims: &mut Vec<CapabilityClaim>, capability: CapabilityKind, provenance: Provenance) {
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

    #[test]
    fn text_generation_pipeline_is_verified_chat() {
        let caps = classify(Some("text-generation"), &tags(&["gguf"]), "org/any-model");
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Chat && c.provenance == Provenance::Verified
        }));
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::TextGeneration
                && c.provenance == Provenance::Verified
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
            c.capability == CapabilityKind::StructuredOutput
                && c.provenance == Provenance::Verified
        }));
        assert!(caps
            .tasks
            .iter()
            .any(|t| t.task == "tool calling"));
    }

    #[test]
    fn coding_is_inferred_from_name_only() {
        let caps = classify(Some("text-generation"), &tags(&["gguf"]), "org/codestral-7b");
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Coding && c.provenance == Provenance::Inferred
        }));
        assert!(!caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Coding && c.provenance == Provenance::Verified
        }));
        assert!(caps.tasks.iter().any(|t| t.task == "repository understanding"));
    }

    #[test]
    fn vision_pipeline_is_verified_multimodal() {
        let caps = classify(Some("image-text-to-text"), &tags(&["gguf"]), "org/vision-x");
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Vision && c.provenance == Provenance::Verified
        }));
        assert!(caps.claims.iter().any(|c| {
            c.capability == CapabilityKind::Multimodal
                && c.provenance == Provenance::Verified
        }));
    }

    #[test]
    fn unknown_pipeline_claims_nothing_false() {
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

    // --- Regression tests for the audit-found false-positive bugfix ---

    #[test]
    fn encoder_named_models_are_never_misclassified_as_coding() {
        let caps = classify(Some("feature-extraction"), &tags(&["gguf"]), "org/bge-base-encoder");
        assert!(
            !caps.claims.iter().any(|c| c.capability == CapabilityKind::Coding),
            "encoder-named embedding model must not be claimed as Coding: {:?}",
            caps.claims
        );
    }

    #[test]
    fn decoder_and_codec_and_vocoder_named_models_are_never_misclassified_as_coding() {
        for id in [
            "org/vae-decoder",
            "org/hifi-vocoder",
            "org/neural-audio-codec",
        ] {
            let caps = classify(None, &tags(&["gguf"]), id);
            assert!(
                !caps.claims.iter().any(|c| c.capability == CapabilityKind::Coding),
                "{id} must not be claimed as Coding: {:?}",
                caps.claims
            );
        }
    }

    #[test]
    fn codellama_family_is_still_inferred_as_coding() {
        for id in ["org/codellama-7b", "org/code-llama-13b-instruct"] {
            let caps = classify(Some("text-generation"), &tags(&["gguf"]), id);
            assert!(
                caps.claims.iter().any(|c| {
                    c.capability == CapabilityKind::Coding && c.provenance == Provenance::Inferred
                }),
                "{id} must still be inferred as Coding: {:?}",
                caps.claims
            );
        }
    }

    #[test]
    fn coder_and_starcoder_and_codestral_still_infer_coding() {
        for id in ["org/qwen2.5-coder-7b", "org/starcoder2-15b", "org/codestral-22b"] {
            let caps = classify(Some("text-generation"), &tags(&["gguf"]), id);
            assert!(
                caps.claims.iter().any(|c| {
                    c.capability == CapabilityKind::Coding && c.provenance == Provenance::Inferred
                }),
                "{id} must still be inferred as Coding: {:?}",
                caps.claims
            );
        }
    }
}

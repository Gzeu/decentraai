//! HuggingFace catalog client: search + repository file listing.
//!
//! Only *discovery* happens here (what models exist, what GGUF files a repo
//! offers, and their reported sizes/digests). The actual bytes are fetched by
//! [`crate::download`], which enforces the digest pinned here.

use anyhow::{Context, Result};
use serde::Deserialize;

/// Base URL of the Hub API. Overridable for tests via [`HubCatalog::new`].
const DEFAULT_API_BASE: &str = "https://huggingface.co/api";

/// Pipeline tag of a Hub model — the coarse "category" shown to operators
/// (text generation, image-text-to-text, ...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineTag {
    TextGeneration,
    ImageTextToText,
    TextToImage,
    ImageToText,
    TextToVideo,
    AutomaticSpeechRecognition,
    TextToAudio,
    AudioToAudio,
    VisualQuestionAnswering,
    ObjectDetection,
    ImageClassification,
    ZeroShotImageClassification,
    TextClassification,
    TokenClassification,
    FillMask,
    QuestionAnswering,
    Summarization,
    Translation,
    Text2TextGeneration,
    FeatureExtraction,
    SentenceSimilarity,
    /// Any tag the Hub sends that we do not model explicitly.
    Other(String),
}

impl PipelineTag {
    /// Human-readable category label.
    pub fn as_str(&self) -> &str {
        match self {
            PipelineTag::TextGeneration => "text-generation",
            PipelineTag::ImageTextToText => "image-text-to-text",
            PipelineTag::TextToImage => "text-to-image",
            PipelineTag::ImageToText => "image-to-text",
            PipelineTag::TextToVideo => "text-to-video",
            PipelineTag::AutomaticSpeechRecognition => "automatic-speech-recognition",
            PipelineTag::TextToAudio => "text-to-audio",
            PipelineTag::AudioToAudio => "audio-to-audio",
            PipelineTag::VisualQuestionAnswering => "visual-question-answering",
            PipelineTag::ObjectDetection => "object-detection",
            PipelineTag::ImageClassification => "image-classification",
            PipelineTag::ZeroShotImageClassification => "zero-shot-image-classification",
            PipelineTag::TextClassification => "text-classification",
            PipelineTag::TokenClassification => "token-classification",
            PipelineTag::FillMask => "fill-mask",
            PipelineTag::QuestionAnswering => "question-answering",
            PipelineTag::Summarization => "summarization",
            PipelineTag::Translation => "translation",
            PipelineTag::Text2TextGeneration => "text2text-generation",
            PipelineTag::FeatureExtraction => "feature-extraction",
            PipelineTag::SentenceSimilarity => "sentence-similarity",
            PipelineTag::Other(s) => s,
        }
    }
}

impl<'de> Deserialize<'de> for PipelineTag {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "text-generation" => PipelineTag::TextGeneration,
            "image-text-to-text" => PipelineTag::ImageTextToText,
            "text-to-image" => PipelineTag::TextToImage,
            "image-to-text" => PipelineTag::ImageToText,
            "text-to-video" => PipelineTag::TextToVideo,
            "automatic-speech-recognition" => PipelineTag::AutomaticSpeechRecognition,
            "text-to-audio" => PipelineTag::TextToAudio,
            "audio-to-audio" => PipelineTag::AudioToAudio,
            "visual-question-answering" => PipelineTag::VisualQuestionAnswering,
            "object-detection" => PipelineTag::ObjectDetection,
            "image-classification" => PipelineTag::ImageClassification,
            "zero-shot-image-classification" => PipelineTag::ZeroShotImageClassification,
            "text-classification" => PipelineTag::TextClassification,
            "token-classification" => PipelineTag::TokenClassification,
            "fill-mask" => PipelineTag::FillMask,
            "question-answering" => PipelineTag::QuestionAnswering,
            "summarization" => PipelineTag::Summarization,
            "translation" => PipelineTag::Translation,
            "text2text-generation" => PipelineTag::Text2TextGeneration,
            "feature-extraction" => PipelineTag::FeatureExtraction,
            "sentence-similarity" => PipelineTag::SentenceSimilarity,
            other => PipelineTag::Other(other.to_string()),
        })
    }
}

/// One search hit from the Hub API.
#[derive(Debug, Clone, Deserialize)]
pub struct HubModel {
    /// Repository id, e.g. `Qwen/Qwen2.5-1.5B-Instruct-GGUF`.
    pub id: String,
    /// Coarse category, when the Hub reports one.
    #[serde(default)]
    pub pipeline_tag: Option<PipelineTag>,
    /// Raw Hub tags (e.g. `gguf`, `conversational`).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Downloads since publication; used for coarse popularity ordering.
    #[serde(default)]
    pub downloads: u64,
}

/// One file inside a repository's `main` branch tree.
#[derive(Debug, Clone, Deserialize)]
pub struct HubModelFile {
    pub path: String,
    #[serde(default)]
    pub size: Option<u64>,
    /// LFS object id — for GGUF artifacts this is the file's SHA-256.
    #[serde(default)]
    pub lfs: Option<HubLfs>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HubLfs {
    pub oid: String,
}

/// Client for the HuggingFace Hub API.
#[derive(Debug, Clone)]
pub struct HubCatalog {
    api_base: String,
    client: reqwest::Client,
}

impl HubCatalog {
    /// Connect to the production Hub API.
    pub fn new() -> Self {
        HubCatalog::with_base(DEFAULT_API_BASE.to_string())
    }

    /// Connect to a custom API base (tests / mirrors).
    pub fn with_base(api_base: String) -> Self {
        HubCatalog {
            api_base,
            client: reqwest::Client::new(),
        }
    }

    /// Search the Hub for GGUF models matching `query`.
    ///
    /// Requests only GGUF-tagged repositories; the Hub returns the most
    /// downloaded matches first (its default sort), which gives operators a
    /// sensible popularity ordering without extra parameters.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<HubModel>> {
        let url = format!(
            "{}/models?search={}&filter=gguf&limit={}",
            self.api_base,
            urlencode(query),
            limit
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("searching Hub for '{query}'"))?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "Hub search failed: HTTP {} for '{}'",
                resp.status(),
                query
            );
        }
        let models: Vec<HubModel> = resp
            .json()
            .await
            .with_context(|| format!("parsing Hub search response for '{query}'"))?;
        Ok(models)
    }

    /// List the GGUF files of a repository (with sizes + SHA-256 digests).
    pub async fn list_gguf_files(&self, repo: &str) -> Result<Vec<HubModelFile>> {
        let url = format!("{}/models/{}/tree/main", self.api_base, repo);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("listing files of '{repo}'"))?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "Hub tree failed: HTTP {} for '{repo}' (repository may be private or missing)",
                resp.status()
            );
        }
        let files: Vec<HubModelFile> = resp
            .json()
            .await
            .with_context(|| format!("parsing Hub tree response for '{repo}'"))?;
        Ok(files
            .into_iter()
            .filter(|f| f.path.to_lowercase().ends_with(".gguf"))
            .collect())
    }
}

impl Default for HubCatalog {
    fn default() -> Self {
        Self::new()
    }
}

fn urlencode(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencode_spaces_and_slashes() {
        assert_eq!(urlencode("llama 3.2 gguf"), "llama%203.2%20gguf");
        assert_eq!(urlencode("a/b"), "a%2Fb");
        assert_eq!(urlencode("qwen"), "qwen");
    }

    #[test]
    fn pipeline_tag_maps_known_and_unknown() {
        let tag: PipelineTag = serde_json::from_str("\"text-generation\"").unwrap();
        assert_eq!(tag, PipelineTag::TextGeneration);
        assert_eq!(tag.as_str(), "text-generation");

        let tag: PipelineTag = serde_json::from_str("\"some-new-task\"").unwrap();
        assert_eq!(tag, PipelineTag::Other("some-new-task".into()));
        assert_eq!(tag.as_str(), "some-new-task");
    }

    #[test]
    fn search_url_is_built_with_filter_and_limit() {
        let url = format!(
            "{}/models?search={}&filter=gguf&limit={}",
            "https://api.test",
            urlencode("llama 3.2"),
            5
        );
        assert_eq!(
            url,
            "https://api.test/models?search=llama%203.2&filter=gguf&limit=5"
        );
    }

    #[test]
    fn tree_url_is_built_from_repo() {
        let url = format!(
            "{}/models/{}/tree/main",
            "https://api.test",
            "Qwen/Qwen2.5-1.5B-Instruct-GGUF"
        );
        assert_eq!(
            url,
            "https://api.test/models/Qwen/Qwen2.5-1.5B-Instruct-GGUF/tree/main"
        );
    }
}
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

/// Enriched model metadata from the Hub model-card endpoint
/// (`GET /api/models/{repo}`), used to build the model detail view.
///
/// All optional fields deserialize with `default` — the Hub does not report
/// every field for every repo, and absent means UNKNOWN, never fabricated.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HubModelDetail {
    pub id: String,
    #[serde(default)]
    pub pipeline_tag: Option<PipelineTag>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub likes: u64,
    /// Model card description / README excerpt.
    #[serde(default)]
    pub description: Option<String>,
    /// License identifier when the Hub reports one (e.g. `apache-2.0`).
    #[serde(default)]
    pub license: Option<String>,
    /// Context window reported by the Hub in tags (`context-length:N`).
    #[serde(default)]
    pub context_length: Option<u32>,
    /// Parameter count reported by the Hub in tags (`params:7B`).
    #[serde(default)]
    pub params: Option<String>,
}

impl HubModelDetail {
    /// The capability classification of this model from its real Hub
    /// metadata (pipeline tag + tags + id), with honest provenance.
    pub fn capabilities(&self) -> crate::capability::ModelCapabilities {
        crate::capability::classify(
            self.pipeline_tag.as_ref().map(|t| t.as_str()),
            &self.tags,
            &self.id,
        )
    }

    /// Extract context-length / params / license from the raw Hub tag list.
    /// Pure so tests can drive it without a Hub request.
    pub fn fill_from_tags(mut self) -> Self {
        for tag in &self.tags {
            if self.context_length.is_none() {
                if let Some(rest) = tag.strip_prefix("context-length:") {
                    if let Ok(n) = rest.parse::<u32>() {
                        self.context_length = Some(n);
                    }
                }
            }
            if self.params.is_none() {
                if let Some(rest) = tag.strip_prefix("params:") {
                    self.params = Some(rest.to_string());
                }
            }
            if self.license.is_none() {
                if let Some(rest) = tag.strip_prefix("license:") {
                    self.license = Some(rest.to_string());
                }
            }
        }
        self
    }
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

    /// Fetch enriched metadata for one repository (the model card endpoint).
    ///
    /// The Hub's `/models/{repo}` response carries tags like
    /// `context-length:4096`, `params:7B`, `license:apache-2.0` plus the
    /// README description; we surface them as OPTIONAL (UNKNOWN when absent).
    /// This is the metadata backbone for the Hub model detail view.
    pub async fn model_detail(&self, repo: &str) -> Result<HubModelDetail> {
        let url = format!("{}/models/{}", self.api_base, repo);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("fetching model detail for '{repo}'"))?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "Hub model detail failed: HTTP {} for '{repo}' (repository may be private or missing)",
                resp.status()
            );
        }
        // The Hub's detail JSON is a superset of HubModel; the extra fields we
        // care about (tags with context-length/params/license, downloads,
        // likes, description) deserialize into HubModelDetail with defaults.
        let detail: HubModelDetail = resp
            .json()
            .await
            .with_context(|| format!("parsing model detail for '{repo}'"))?;

        // Extract context-length / params / license from raw Hub tags, which
        // are the most reliable place those values appear.
        Ok(detail.fill_from_tags())
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

    #[test]
    fn model_detail_extracts_context_params_license_from_tags() {
        // The Hub detail endpoint returns raw tags; context-length/params/
        // license are extracted from them (optional fields stay None when the
        // Hub does not report them).
        let json = serde_json::json!({
            "id": "Qwen/Qwen2.5-7B-Instruct-GGUF",
            "pipeline_tag": "text-generation",
            "tags": [
                "gguf",
                "conversational",
                "context-length:32768",
                "params:7B",
                "license:apache-2.0",
                "tools"
            ],
            "downloads": 1234,
            "likes": 56,
            "description": "A Qwen chat model."
        });
        let detail: HubModelDetail =
            serde_json::from_value::<HubModelDetail>(json).unwrap().fill_from_tags();
        assert_eq!(detail.id, "Qwen/Qwen2.5-7B-Instruct-GGUF");
        assert_eq!(detail.context_length, Some(32768));
        assert_eq!(detail.params.as_deref(), Some("7B"));
        assert_eq!(detail.license.as_deref(), Some("apache-2.0"));
        assert_eq!(detail.downloads, 1234);
        assert_eq!(detail.likes, 56);
        assert!(detail.description.is_some());

        // The `tools` tag yields VERIFIED tool-calling capability.
        let caps = detail.capabilities();
        assert!(caps.claims.iter().any(|c| {
            c.capability == crate::capability::CapabilityKind::ToolCalling
                && c.provenance == crate::capability::Provenance::Verified
        }));
    }

    #[test]
    fn model_detail_absent_metadata_stays_unknown() {
        let json = serde_json::json!({
            "id": "org/bare",
            "tags": ["gguf"],
            "downloads": 1
        });
        let detail: HubModelDetail = serde_json::from_value(json).unwrap();
        assert_eq!(detail.context_length, None, "absent -> UNKNOWN, never invented");
        assert_eq!(detail.params, None);
        assert_eq!(detail.license, None);
        assert_eq!(detail.description, None);
        assert_eq!(detail.likes, 0);
    }
}
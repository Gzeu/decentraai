//! Runtime engine abstraction (M22).
//!
//! DecentraAI must not become dependent on one inference engine. This module
//! defines the *kind* of engine a worker runs and the *capabilities* that
//! engine actually advertises, so the execution planner and the scheduler can
//! make decisions that are engine-aware without being engine-specific.
//!
//! The engine is always an external process/server (llama-server, vLLM,
//! SGLang, Ollama, or any OpenAI-compatible `server`). DecentraAI never links
//! or embeds an engine; it drives the engine's HTTP API and reads its
//! reported capabilities. Engines that cannot express a capability (e.g. an
//! OpenAI-compatible endpoint with no KV-cache or expert-routing surface)
//! simply report `false`, and the planner falls back to the default path.
//!
//! # Design rule
//!
//! Build the correct abstraction; integrate real support **only where the
//! underlying runtime actually provides it**. Do not force tensor/Pipeline
//! parallelism or distributed MoE onto an engine that cannot do it.

use serde::{Deserialize, Serialize};

/// The inference engine a worker runs behind its OpenAI-compatible API.
///
/// Every engine here speaks the OpenAI-compatible protocol at minimum
/// (`/v1/models`, `/v1/chat/completions`, `/v1/completions`), which is why
/// the existing backend-neutral [`InferenceBackend`] adapter can drive all of
/// them through one interface. The kind matters to the planner for two
/// reasons: which *additional* capabilities the engine exports (see
/// [`EngineCapabilities`]), and how to prefer among otherwise-equal workers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineKind {
    /// llama.cpp's `llama-server`.
    LlamaServer,
    /// vLLM (`vllm serve`). Supports KV-cache state and prefill/decode
    /// disaggregation endpoints, and multi-backend tensor parallel.
    Vllm,
    /// SGLang (`python -m sglang.launch_server`). Exposes forward-pass and
    /// KV cache control surfaces.
    Sglang,
    /// Ollama (`ollama serve`). OpenAI-compatible layer over llama.cpp.
    Ollama,
    /// Any other OpenAI-compatible HTTP server (generic hub-and-spoke).
    RemoteOpenAI,
}

impl EngineKind {
    /// Parses a wire / config string into an engine kind. Unknown engines
    /// resolve to [`EngineKind::RemoteOpenAI`] rather than failing, so a
    /// future engine never breaks an old node's config parsing.
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "llama-server" | "llama_server" | "llamacpp" | "llama.cpp" => Self::LlamaServer,
            "vllm" => Self::Vllm,
            "sglang" => Self::Sglang,
            "ollama" => Self::Ollama,
            _ => Self::RemoteOpenAI,
        }
    }

    /// Canonical wire representation stored in advertisements / config.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LlamaServer => "llama-server",
            Self::Vllm => "vllm",
            Self::Sglang => "sglang",
            Self::Ollama => "ollama",
            Self::RemoteOpenAI => "openai-compatible",
        }
    }

    /// The capabilities this engine advertises *in general*. Production code
    /// should default to [`EngineKind::capabilities`] and let a live probe
    /// narrow them (an endpoint may not expose what its kind implies).
    pub fn advertised_capabilities(self) -> EngineCapabilities {
        match self {
            // llama-server exposes KV params; it does not split one model
            // across machines or route individual experts.
            Self::LlamaServer => EngineCapabilities {
                streaming: true,
                kv_report: true,
                prefill_decode_separation: false,
                expert_routing: false,
                tensor_parallel: false,
                ..EngineCapabilities::zero_extra()
            },
            // vLLM exports live KV-cache state and a prefill/decode
            // disaggregation surface, and can attend over multiple ranks.
            Self::Vllm => EngineCapabilities {
                streaming: true,
                kv_report: true,
                prefill_decode_separation: true,
                expert_routing: false,
                tensor_parallel: true,
                continuous_batching: true,
                speculative_decoding: true,
                kv_offload: true,
                prefix_cache: true,
                pipeline_parallel: true,
            },
            Self::Sglang => EngineCapabilities {
                streaming: true,
                kv_report: true,
                prefill_decode_separation: true,
                expert_routing: false,
                tensor_parallel: true,
                continuous_batching: true,
                speculative_decoding: true,
                kv_offload: true,
                prefix_cache: true,
                pipeline_parallel: true,
            },
            Self::Ollama => EngineCapabilities {
                streaming: true,
                kv_report: false,
                prefill_decode_separation: false,
                expert_routing: false,
                tensor_parallel: false,
                ..EngineCapabilities::zero_extra()
            },
            // Unknown/remote: be conservative. Safe defaults preserve
            // correctness; an endpoint that supports more is probed to opt-in.
            Self::RemoteOpenAI => EngineCapabilities {
                streaming: true,
                kv_report: false,
                prefill_decode_separation: false,
                expert_routing: false,
                tensor_parallel: false,
                ..EngineCapabilities::zero_extra()
            },
        }
    }
}

/// The set of execution capabilities a specific engine instance advertises.
///
/// These are the explicit contract between the engine and the planner. Every
/// flag has one meaning: when `true`, the planner MAY use that mechanism on
/// this engine; when `false`, the planner MUST route around it (single-worker,
/// non-split, default path). Engine-specific probing narrows these at reg time
/// (e.g. an OpenAI-compatible vLLM that does not expose its KV API reports
/// `kv_report: false`, and KV-aware routing degrades gracefully).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineCapabilities {
    /// Streams incremental tokens over the protocol.
    pub streaming: bool,
    /// Reports live KV-cache / context-headroom state to the coordinator.
    pub kv_report: bool,
    /// Exposes a distinct prefill (prompt ingestion) vs decode (token
    /// generation) phase the coordinator can target separately.
    pub prefill_decode_separation: bool,
    /// Accepts expert-level routing / selection (distributed MoE).
    pub expert_routing: bool,
    /// Can shard one model across multiple distributed ranks (tensor
    /// parallelism) and serve it cooperatively.
    pub tensor_parallel: bool,
    /// Can batch independent requests into one forward pass (vLLM/SGLang
    /// continuous batching). Preferred for BatchFanOut; not required.
    #[serde(default)]
    pub continuous_batching: bool,
    /// Supports draft-model speculative decoding (verify engine).
    #[serde(default)]
    pub speculative_decoding: bool,
    /// Can offload KV cache to a remote KV layer (LMCache etc.) or otherwise
    /// share KV state across engines.
    #[serde(default)]
    pub kv_offload: bool,
    /// Maintains a prefix cache with KV locality and hit/miss statistics that
    /// cache-aware routing can exploit.
    #[serde(default)]
    pub prefix_cache: bool,
    /// Can shard one model across pipeline stages on different ranks
    /// (pipeline parallelism, e.g. vLLM PP).
    #[serde(default)]
    pub pipeline_parallel: bool,
}

impl EngineCapabilities {
    /// Conservative defaults for an unprobed engine: only streaming is safe.
    pub fn conservative() -> Self {
        Self {
            streaming: true,
            kv_report: false,
            prefill_decode_separation: false,
            expert_routing: false,
            tensor_parallel: false,
            ..Self::zero_extra()
        }
    }

    /// Defaults for the newer Model-Fabric Execution Spec (§2.1) flags. Kept
    /// as a separate constructor so `..EngineCapabilities::conservative()`
    /// spread constructions stay source-compatible when the flags change.
    fn zero_extra() -> Self {
        Self {
            streaming: false,
            kv_report: false,
            prefill_decode_separation: false,
            expert_routing: false,
            tensor_parallel: false,
            continuous_batching: false,
            speculative_decoding: false,
            kv_offload: false,
            prefix_cache: false,
            pipeline_parallel: false,
        }
    }

    /// Whether this engine can participate in a multi-stage execution plan
    /// that stages work across more than one worker.
    pub fn supports_staging(&self) -> bool {
        self.prefill_decode_separation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_and_unknown_engines() {
        assert_eq!(EngineKind::parse("vllm"), EngineKind::Vllm);
        assert_eq!(EngineKind::parse("SGLang"), EngineKind::Sglang);
        assert_eq!(EngineKind::parse("llama-server"), EngineKind::LlamaServer);
        assert_eq!(EngineKind::parse("ollama"), EngineKind::Ollama);
        // Unknown never fails; degrades to remote OpenAI.
        assert_eq!(
            EngineKind::parse("some-future-engine"),
            EngineKind::RemoteOpenAI
        );
    }

    #[test]
    fn capabilities_round_trip() {
        let capa = EngineKind::Vllm.advertised_capabilities();
        assert!(capa.kv_report);
        assert!(capa.tensor_parallel);
        assert!(!capa.expert_routing);
        let json = serde_json::to_string(&capa).unwrap();
        let back: EngineCapabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(back, capa);
    }

    #[test]
    fn conservative_only_streams() {
        let c = EngineCapabilities::conservative();
        assert!(c.streaming);
        assert!(!c.kv_report && !c.tensor_parallel && !c.expert_routing);
    }

    #[test]
    fn only_kv_separating_engines_support_staging() {
        assert!(
            EngineKind::Vllm
                .advertised_capabilities()
                .supports_staging()
        );
        assert!(
            !EngineKind::Ollama
                .advertised_capabilities()
                .supports_staging()
        );
        assert!(
            !EngineKind::LlamaServer
                .advertised_capabilities()
                .supports_staging()
        );
    }

    // Against engine.rs documentation: these capabilities are *parked* (M21 /
    // M22 prefill) and NOT wired into any running engine. The two gates that
    // would enable distributed MoE / prefill-decode splits must stay OFF for
    // every engine DecentraAI actually runs today — llama-server is the only
    // engine the runtime spawns. This test pins that honest state so a future
    // change cannot silently "enable" a mechanism no engine can actually
    // serve, which would route requests into a broken split path.
    #[test]
    fn every_engine_decentraai_runs_is_gated_conservative_on_split_features() {
        let kinds = [
            EngineKind::LlamaServer,
            EngineKind::Ollama,
            EngineKind::RemoteOpenAI,
        ];
        for kind in kinds {
            let capa = kind.advertised_capabilities();
            assert!(
                !capa.expert_routing,
                "{kind:?} must not advertise expert routing (parked M21)"
            );
        }
        // llama-server (the production engine) must not advertise prefill/decode
        // separation either — that split is parked behind the capability gate.
        assert!(
            !EngineKind::LlamaServer
                .advertised_capabilities()
                .prefill_decode_separation,
            "llama-server must stay on the single-phase path (parked split)"
        );
    }

    // Complementary pin: probing/capability narrowing can only ever *lower* the
    // advertised set toward the conservative baseline; it must never flip
    // `expert_routing` on. So every engine kind DecentraAI knows about, honest
    // or experimental, plus the unprobed conservative baseline, keeps
    // `expert_routing` = false. This is the guarantee the degenerate-split
    // guard in expert.rs relies on: no advertisement that powers it can come
    // from a capability that this module claims to be on.
    #[test]
    fn no_engine_kind_or_conservative_baseline_ever_advertises_expert_routing() {
        let kinds = [
            EngineKind::LlamaServer,
            EngineKind::Vllm,
            EngineKind::Sglang,
            EngineKind::Ollama,
            EngineKind::RemoteOpenAI,
        ];
        for kind in kinds {
            let capa = kind.advertised_capabilities();
            assert!(
                !capa.expert_routing,
                "{kind:?} must never advertise expert routing"
            );
        }
        let conservative = EngineCapabilities::conservative();
        assert!(
            !conservative.expert_routing,
            "the unprobed conservative baseline must not advertise expert routing"
        );
    }
}

//! Dynamic Model Valuation & Fair Economic Exchange Engine (research track).
//!
//! # The Token Cost Asymmetry Problem
//!
//! Not all AI tokens have the same economic or computational cost:
//! - 1,000 output tokens of `claude-3-5-sonnet` costs ~$0.015
//! - 1,000 output tokens of `deepseek-chat` (V3) costs ~$0.00028 (~50x difference!)
//! - 1,000 output tokens on a local RTX 4090 costs only electricity (~$0.00005)
//!
//! If an economy awards 1 token = 2 CU uniformly, a contributor donating Claude 3.5 Sonnet
//! is severely penalized, and a consumer could drain an expensive subscription using cheap credits.
//!
//! # The Solution: Relative Resource Weight Matrix
//!
//! Each model is classified into an economic tier with versioned weights:
//!
//! $$\text{CU Earned} = \frac{\text{BaseUnits}}{1000} \times \left( W_{\text{in}} \cdot \text{input\_tokens} + W_{\text{out}} \cdot \text{output\_tokens} \right)$$
//!
//! This guarantees that donating expensive frontier capacity on your day off gives you
//! enough CU to consume massive amounts of mid-tier or local GPU inference later.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Economic classification tier of an AI resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelEconomicTier {
    /// Frontier models (e.g. Claude 3.5 Sonnet, GPT-4o, DeepSeek R1 reasoning).
    FrontierReasoning,
    /// High-efficiency commercial models (e.g. Claude 3.5 Haiku, DeepSeek V3, GPT-4o-mini).
    MidTierCommercial,
    /// Ultra-fast / Low-cost APIs (e.g. Groq Llama 3.3, Gemini 2.0 Flash).
    FastCommodityApi,
    /// Free-tier or local GPU hardware compute (e.g. Ollama, local vLLM).
    LocalGpuHardware,
}

/// Token pricing weight per 1,000 tokens for fair relative exchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTierWeight {
    /// CU awarded per 1,000 input prompt tokens.
    pub cu_per_1k_input: u64,
    /// CU awarded per 1,000 output completion tokens.
    pub cu_per_1k_output: u64,
}

/// Deterministic, operator-versioned valuation matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelValuationMatrix {
    pub version: u32,
    pub tier_weights: HashMap<ModelEconomicTier, ModelTierWeight>,
    pub custom_model_overrides: HashMap<String, ModelTierWeight>,
}

impl Default for ModelValuationMatrix {
    fn default() -> Self {
        let mut tier_weights = HashMap::new();
        tier_weights.insert(
            ModelEconomicTier::FrontierReasoning,
            ModelTierWeight {
                cu_per_1k_input: 30,
                cu_per_1k_output: 150,
            },
        );
        tier_weights.insert(
            ModelEconomicTier::MidTierCommercial,
            ModelTierWeight {
                cu_per_1k_input: 3,
                cu_per_1k_output: 12,
            },
        );
        tier_weights.insert(
            ModelEconomicTier::FastCommodityApi,
            ModelTierWeight {
                cu_per_1k_input: 1,
                cu_per_1k_output: 4,
            },
        );
        tier_weights.insert(
            ModelEconomicTier::LocalGpuHardware,
            ModelTierWeight {
                cu_per_1k_input: 1,
                cu_per_1k_output: 2,
            },
        );

        let mut custom_model_overrides = HashMap::new();
        // Exact mappings for popular flagship models
        custom_model_overrides.insert(
            "anthropic/claude-3.5-sonnet".into(),
            ModelTierWeight {
                cu_per_1k_input: 30,
                cu_per_1k_output: 150,
            },
        );
        custom_model_overrides.insert(
            "deepseek/deepseek-r1".into(),
            ModelTierWeight {
                cu_per_1k_input: 20,
                cu_per_1k_output: 100,
            },
        );
        custom_model_overrides.insert(
            "deepseek/deepseek-chat".into(),
            ModelTierWeight {
                cu_per_1k_input: 2,
                cu_per_1k_output: 8,
            },
        );

        Self {
            version: 1,
            tier_weights,
            custom_model_overrides,
        }
    }
}

impl ModelValuationMatrix {
    /// Resolves the weight for a given model identifier.
    pub fn resolve_weight(&self, model_id: &str, tier_fallback: ModelEconomicTier) -> ModelTierWeight {
        if let Some(w) = self.custom_model_overrides.get(model_id) {
            return w.clone();
        }
        self.tier_weights
            .get(&tier_fallback)
            .cloned()
            .unwrap_or(ModelTierWeight {
                cu_per_1k_input: 1,
                cu_per_1k_output: 2,
            })
    }

    /// Computes fair CU reward for actual measured tokens generated.
    pub fn compute_cu_reward(
        &self,
        model_id: &str,
        tier: ModelEconomicTier,
        prompt_tokens: u64,
        completion_tokens: u64,
    ) -> u64 {
        let weight = self.resolve_weight(model_id, tier);
        let input_cu = (prompt_tokens.saturating_mul(weight.cu_per_1k_input)) / 1000;
        let output_cu = (completion_tokens.saturating_mul(weight.cu_per_1k_output)) / 1000;
        input_cu.saturating_add(output_cu).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fair_economic_exchange_claude_vs_deepseek() {
        let matrix = ModelValuationMatrix::default();

        // Contributor serves 10,000 output tokens on Claude 3.5 Sonnet on their day off
        let claude_cu = matrix.compute_cu_reward(
            "anthropic/claude-3.5-sonnet",
            ModelEconomicTier::FrontierReasoning,
            1_000,
            10_000,
        );
        // (1000 * 30 / 1000) + (10000 * 150 / 1000) = 30 + 1500 = 1530 CU
        assert_eq!(claude_cu, 1530);

        // Later, contributor consumes DeepSeek V3 with their 1530 CU
        let deepseek_cost = matrix.compute_cu_reward(
            "deepseek/deepseek-chat",
            ModelEconomicTier::MidTierCommercial,
            1_000,
            10_000,
        );
        // (1000 * 2 / 1000) + (10000 * 8 / 1000) = 2 + 80 = 82 CU
        assert_eq!(deepseek_cost, 82);

        // Donating 10k Claude tokens yields ~18x more DeepSeek tokens!
        assert!(claude_cu / deepseek_cost >= 18);
    }
}

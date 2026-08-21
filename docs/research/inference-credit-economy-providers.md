# Provider Integration Guide — DecentraAI Inference Credit Economy

Status: experimental (research track)
Branch: `research/inference-credit-economy`

## How External Providers Connect to DecentraAI

DecentraAI allows nodes to share their existing AI subscriptions or API keys (OpenRouter, Anthropic/Claude, DeepSeek, OpenAI, Ollama, local vLLM) in exchange for durable **Contribution Credits (CU)**.

```text
  [Node Local Credential Vault] (API keys stay local; never broadcast)
               │
               ▼
   [Local Provider Adapter] ──(authenticated HTTP)──> [OpenRouter / Anthropic / DeepSeek API]
               │                                                    │
               │ (captures exact prompt + completion tokens)        │ (returns response)
               ▼                                                    ▼
      [P13 Signed Receipt]                               [Client (OpenCode / DecentraAI)]
               │
               ▼
  [CreditFabric Settlement] ──> Contributor earns durable CU (spendable on any model)
```

---

## Supported Provider Types

| Provider | Supported Models | How Quota is Handled |
|---|---|---|
| **OpenRouter** | Claude 3.5 Sonnet, Llama 3.3 70B, DeepSeek R1, GPT-4o | Daily credit limit configured locally; auto-pauses on exhaustion |
| **Anthropic** | Claude 3.5 Sonnet, Claude 3.5 Haiku, Claude 3 Opus | Monthly token allotment; token usage metered from response headers |
| **DeepSeek** | DeepSeek V3, DeepSeek R1 | Token rate-limit tracking; 1 token in / 2 tokens out CU weighting |
| **Ollama / Local vLLM** | Local GGUF / AWQ models on private GPU | GPU-ms and token metering; zero external API cost |

---

## Step-by-Step Sharing Flow

### 1. Configure the Provider Locally
The operator configures the provider in their local node. The raw key is stored in the in-memory `CredentialVault`, and an opaque handle (e.g. `key-openrouter-1724217000`) is assigned.

### 2. Advertisement Broadcast (No Secrets)
The node broadcasts a `ResourceAdvertisement` containing:
- Provider name & model ID
- Available daily token quota (e.g. 500,000 tokens)
- Rate limits (RPM, concurrency)
- Opaque `credential_ref` handle

### 3. Execution & Measured Settlement
When an inbound request arrives:
1. `CreditFabric` reserves CU from the consumer's balance.
2. The contributor's node executes the request against OpenRouter/Anthropic using the local secret.
3. The provider returns the completion along with exact token counts.
4. The contributor node signs a P13 `VerifiedComputeReceipt`.
5. `CreditFabric::complete_session` settles the session:
   - Contributor receives **durable CU**.
   - Provider quota is decremented.
   - Consumer CU is consumed.

### 4. Quota Expiration & Reset
When the daily provider quota expires or resets at midnight:
- The temporary `ProviderQuota` resets.
- **Already settled CU in DecentraAI remain valid and spendable permanently**.
- The contributor can now spend their earned CU on another provider's model (e.g. Qwen, GPT-4o, or remote GPU).

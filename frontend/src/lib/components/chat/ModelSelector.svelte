<script lang="ts">
  import type { ModelInfo } from '$lib/types/chat';
  import { chatSettings } from '$lib/stores/chat';

  let { models, selectedModel }: { models: ModelInfo[]; selectedModel: string } = $props();

  const filteredModels = $derived(
    models.filter(m => m.type === 'llm')
  );

  function selectModel(modelId: string) {
    chatSettings.update(settings => ({ ...settings, model: modelId }));
  }
</script>

<div class="model-selector">
  <label class="selector-label">Model</label>
  <select 
    bind:value={$chatSettings.model} 
    class="model-select"
    onchange={(e) => selectModel(e.target.value)}
  >
    {#each filteredModels as model}
      <option value={model.id}>
        {model.name} ({model.quantization}) - {model.tokens_per_sec.toFixed(0)} t/s
      </option>
    {/each}
  </select>
  <div class="model-info">
    {#if filteredModels.find(m => m.id === $chatSettings.model)}
      {@const selected = filteredModels.find(m => m.id === $chatSettings.model)}
      <div class="info-item">
        <span class="info-label">Context:</span>
        <span class="info-value">{selected?.context_length.toLocaleString()}</span>
      </div>
      <div class="info-item">
        <span class="info-label">VRAM:</span>
        <span class="info-value">{selected?.vram_usage_gb.toFixed(1)} GB</span>
      </div>
    {/if}
  </div>
</div>

<style>
  .model-selector {
    background: rgba(31, 41, 55, 0.8);
    border: 1px solid rgba(75, 85, 99, 0.5);
    border-radius: 0.5rem;
    padding: 1rem;
  }

  .selector-label {
    display: block;
    font-size: 0.875rem;
    font-weight: 600;
    margin-bottom: 0.5rem;
    color: #9ca3af;
  }

  .model-select {
    width: 100%;
    background: rgba(17, 24, 39, 0.8);
    border: 1px solid rgba(75, 85, 99, 0.5);
    border-radius: 0.375rem;
    padding: 0.5rem;
    color: white;
    font-size: 0.875rem;
    outline: none;
  }

  .model-select:focus {
    border-color: #3b82f6;
  }

  .model-info {
    display: flex;
    gap: 1rem;
    margin-top: 0.5rem;
    font-size: 0.75rem;
  }

  .info-item {
    display: flex;
    gap: 0.25rem;
  }

  .info-label {
    color: #9ca3af;
  }

  .info-value {
    color: #3b82f6;
    font-weight: 600;
  }
</style>
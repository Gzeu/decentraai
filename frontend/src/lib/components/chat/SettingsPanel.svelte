<script lang="ts">
  import { chatSettings } from '$lib/stores/chat';

  let { isOpen, onClose }: { isOpen: boolean; onClose: () => void } = $props();

  function updateSetting(key: keyof typeof $chatSettings, value: any) {
    chatSettings.update(settings => ({ ...settings, [key]: value }));
  }
</script>

{#if isOpen}
  <div class="settings-panel-overlay" on:click={onClose}>
    <div class="settings-panel" on:click|stopPropagation>
      <div class="settings-header">
        <h2>⚙️ Chat Settings</h2>
        <button on:click={onClose} class="close-button">✕</button>
      </div>

      <div class="settings-content">
        <div class="setting-group">
          <label class="setting-label">Temperature</label>
          <input 
            type="range" 
            min="0" 
            max="2" 
            step="0.1" 
            bind:value={$chatSettings.temperature}
            class="setting-slider"
          />
          <span class="setting-value">{$chatSettings.temperature.toFixed(1)}</span>
        </div>

        <div class="setting-group">
          <label class="setting-label">Max Tokens</label>
          <input 
            type="number" 
            min="100" 
            max="8192" 
            step="100"
            bind:value={$chatSettings.max_tokens}
            class="setting-input"
          />
        </div>

        <div class="setting-group">
          <label class="setting-label">Top P</label>
          <input 
            type="range" 
            min="0" 
            max="1" 
            step="0.1"
            bind:value={$chatSettings.top_p}
            class="setting-slider"
          />
          <span class="setting-value">{$chatSettings.top_p.toFixed(1)}</span>
        </div>

        <div class="setting-group">
          <label class="setting-label">Stream Responses</label>
          <label class="toggle-switch">
            <input type="checkbox" bind:checked={$chatSettings.stream} />
            <span class="toggle-slider"></span>
          </label>
        </div>

        <div class="setting-group">
          <label class="setting-label">System Prompt</label>
          <textarea 
            bind:value={$chatSettings.system_prompt}
            class="setting-textarea"
            rows="4"
          />
        </div>
      </div>

      <div class="settings-footer">
        <button on:click={() => {
          chatSettings.set({
            model: 'tinyllama-1.1b',
            temperature: 0.7,
            max_tokens: 2048,
            top_p: 0.9,
            stream: true,
            system_prompt: 'You are a helpful AI assistant. Provide clear, accurate, and concise responses.'
          });
        }} class="reset-button">
          Reset to Defaults
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .settings-panel-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .settings-panel {
    background: rgba(31, 41, 55, 0.95);
    border: 1px solid rgba(75, 85, 99, 0.5);
    border-radius: 1rem;
    padding: 1.5rem;
    width: 90%;
    max-width: 500px;
    max-height: 80vh;
    overflow-y: auto;
  }

  .settings-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 1.5rem;
  }

  .settings-header h2 {
    font-size: 1.25rem;
    font-weight: 600;
  }

  .close-button {
    background: transparent;
    border: none;
    color: white;
    font-size: 1.5rem;
    cursor: pointer;
  }

  .settings-content {
    display: flex;
    flex-direction: column;
    gap: 1.5rem;
  }

  .setting-group {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .setting-label {
    font-size: 0.875rem;
    font-weight: 600;
    color: #9ca3af;
  }

  .setting-slider {
    width: 100%;
  }

  .setting-value {
    font-size: 0.875rem;
    color: #3b82f6;
    font-weight: 600;
  }

  .setting-input {
    background: rgba(17, 24, 39, 0.8);
    border: 1px solid rgba(75, 85, 99, 0.5);
    border-radius: 0.375rem;
    padding: 0.5rem;
    color: white;
    font-size: 0.875rem;
  }

  .setting-textarea {
    background: rgba(17, 24, 39, 0.8);
    border: 1px solid rgba(75, 85, 99, 0.5);
    border-radius: 0.375rem;
    padding: 0.5rem;
    color: white;
    font-size: 0.875rem;
    resize: vertical;
  }

  .toggle-switch {
    position: relative;
    display: inline-block;
    width: 48px;
    height: 24px;
  }

  .toggle-switch input {
    opacity: 0;
    width: 0;
    height: 0;
  }

  .toggle-slider {
    position: absolute;
    cursor: pointer;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background-color: #4b5563;
    transition: 0.3s;
    border-radius: 24px;
  }

  .toggle-slider:before {
    position: absolute;
    content: "";
    height: 18px;
    width: 18px;
    left: 3px;
    bottom: 3px;
    background-color: white;
    transition: 0.3s;
    border-radius: 50%;
  }

  .toggle-switch input:checked + .toggle-slider {
    background-color: #3b82f6;
  }

  .toggle-switch input:checked + .toggle-slider:before {
    transform: translateX(24px);
  }

  .settings-footer {
    margin-top: 1.5rem;
    display: flex;
    justify-content: flex-end;
  }

  .reset-button {
    background: rgba(239, 68, 68, 0.2);
    border: 1px solid rgba(239, 68, 68, 0.5);
    border-radius: 0.5rem;
    padding: 0.5rem 1rem;
    color: #ef4444;
    cursor: pointer;
    transition: background 0.2s;
  }

  .reset-button:hover {
    background: rgba(239, 68, 68, 0.3);
  }
</style>
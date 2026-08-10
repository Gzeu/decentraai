<script lang="ts">
  import { onMount } from 'svelte';
  import { chatSettings } from '$lib/stores/chat';

  let { onSend, disabled = false }: { onSend: (message: string) => void; disabled?: boolean } = $props();

  let textarea: HTMLTextAreaElement;
  let message = $state('');

  onMount(() => {
    textarea?.focus();
  });

  function handleSend() {
    if (message.trim() && !disabled) {
      onSend(message.trim());
      message = '';
    }
  }

  function handleKeyDown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      handleSend();
    }
  }

  async function adjustHeight() {
    if (textarea) {
      textarea.style.height = 'auto';
      textarea.style.height = Math.min(textarea.scrollHeight, 200) + 'px';
    }
  }
</script>

<div class="chat-input-container">
  <textarea
    bind:this={textarea}
    bind:value={message}
    on:input={adjustHeight}
    on:keydown={handleKeyDown}
    placeholder="Type your message... (Shift+Enter for new line)"
    disabled={disabled}
    rows="1"
    class="chat-textarea"
  />
  <div class="chat-input-actions">
    <button 
      on:click={handleSend} 
      disabled={disabled || !message.trim()}
      class="send-button"
    >
      {disabled ? '⏳' : '📤 Send'}
    </button>
  </div>
</div>

<style>
  .chat-input-container {
    background: rgba(31, 41, 55, 0.8);
    border: 1px solid rgba(75, 85, 99, 0.5);
    border-radius: 0.75rem;
    padding: 1rem;
    margin-top: 1rem;
  }

  .chat-textarea {
    width: 100%;
    background: transparent;
    border: none;
    color: white;
    font-family: inherit;
    font-size: 1rem;
    resize: none;
    min-height: 44px;
    max-height: 200px;
    outline: none;
  }

  .chat-textarea::placeholder {
    color: rgba(156, 163, 175, 0.7);
  }

  .chat-textarea:disabled {
    opacity: 0.5;
  }

  .chat-input-actions {
    display: flex;
    justify-content: flex-end;
    margin-top: 0.5rem;
  }

  .send-button {
    background: linear-gradient(135deg, #3b82f6 0%, #1d4ed8 100%);
    border: none;
    border-radius: 0.5rem;
    padding: 0.5rem 1rem;
    color: white;
    font-weight: 600;
    cursor: pointer;
    transition: transform 0.2s, opacity 0.2s;
  }

  .send-button:hover:not(:disabled) {
    transform: scale(1.05);
  }

  .send-button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
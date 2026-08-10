<script lang="ts">
  import type { ChatMessage } from '$lib/types/chat';
  import { marked } from 'marked';
  import hljs from 'highlight.js/lib/core';
  import 'highlight.js/styles/github-dark.css';

  let { message }: { message: ChatMessage } = $props();

  const isUser = $derived(message.role === 'user');
  const isSystem = $derived(message.role === 'system');
  
  // Configure marked for code highlighting
  marked.setOptions({
    highlight: function(code, lang) {
      const language = hljs.getLanguage(lang) ? lang : 'plaintext';
      return hljs.highlight(code, { language }).value;
    },
    breaks: true,
    gfm: true
  });

  const renderedContent = $derived(marked.parse(message.content));

  function copyCode(event: MouseEvent) {
    const target = event.target as HTMLElement;
    const codeBlock = target.closest('pre');
    if (codeBlock) {
      const code = codeBlock.querySelector('code')?.textContent || '';
      navigator.clipboard.writeText(code);
    }
  }
</script>

<div class="chat-message {isUser ? 'user' : isSystem ? 'system' : 'assistant'}">
  <div class="message-header">
    <span class="message-role">
      {isUser ? '👤 You' : isSystem ? '⚙️ System' : '🤖 Assistant'}
    </span>
    <span class="message-time">
      {new Date(message.timestamp).toLocaleTimeString()}
    </span>
    {#if message.tokens}
      <span class="message-tokens">
        {message.tokens} tokens
      </span>
    {/if}
  </div>
  
  <div class="message-content">
    {@html renderedContent}
  </div>
  
  {#if !isUser && !isSystem}
    <div class="message-actions">
      <button on:click={copyCode} class="action-button" title="Copy code">
        📋 Copy
      </button>
      <button class="action-button" title="Regenerate">
        🔄 Regenerate
      </button>
    </div>
  {/if}
</div>

<style>
  .chat-message {
    padding: 1rem;
    margin-bottom: 1rem;
    border-radius: 0.75rem;
    max-width: 80%;
  }

  .chat-message.user {
    background: linear-gradient(135deg, #3b82f6 0%, #1d4ed8 100%);
    margin-left: auto;
  }

  .chat-message.assistant {
    background: rgba(31, 41, 55, 0.8);
    border: 1px solid rgba(75, 85, 99, 0.5);
  }

  .chat-message.system {
    background: rgba(234, 179, 8, 0.1);
    border: 1px solid rgba(234, 179, 8, 0.3);
    font-style: italic;
    max-width: 100%;
  }

  .message-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.5rem;
    font-size: 0.875rem;
  }

  .message-role {
    font-weight: 600;
  }

  .message-time {
    opacity: 0.7;
  }

  .message-tokens {
    margin-left: auto;
    opacity: 0.6;
    font-size: 0.75rem;
  }

  .message-content {
    line-height: 1.6;
  }

  .message-content :global(pre) {
    background: rgba(0, 0, 0, 0.3);
    border-radius: 0.5rem;
    padding: 1rem;
    overflow-x: auto;
    margin: 0.5rem 0;
  }

  .message-content :global(code) {
    font-family: 'JetBrains Mono', monospace;
    font-size: 0.875rem;
  }

  .message-content :global(p) {
    margin: 0.5rem 0;
  }

  .message-content :global(ul), .message-content :global(ol) {
    margin: 0.5rem 0;
    padding-left: 1.5rem;
  }

  .message-content :global(blockquote) {
    border-left: 3px solid #6b7280;
    padding-left: 1rem;
    margin: 0.5rem 0;
    opacity: 0.8;
  }

  .message-actions {
    display: flex;
    gap: 0.5rem;
    margin-top: 0.5rem;
  }

  .action-button {
    background: rgba(255, 255, 255, 0.1);
    border: none;
    border-radius: 0.375rem;
    padding: 0.25rem 0.5rem;
    color: white;
    cursor: pointer;
    font-size: 0.75rem;
    transition: background 0.2s;
  }

  .action-button:hover {
    background: rgba(255, 255, 255, 0.2);
  }
</style>
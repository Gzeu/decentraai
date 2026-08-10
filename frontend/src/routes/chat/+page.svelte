<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import Navigation from '$lib/components/Navigation.svelte';
  import {
    chatState,
    chatSettings,
    currentSession,
    currentMessages,
    isChatLoading,
    chatError,
    availableModels,
    createNewSession,
    addMessage,
    updateLastMessage,
    switchSession,
    deleteSession,
    clearChat,
    setLoading,
    setError,
    setModels,
    loadSessions,
    saveSessions,
    loadSettings,
    saveSettings
  } from '$lib/stores/chat';
  import ChatMessage from '$lib/components/chat/ChatMessage.svelte';
  import ChatInput from '$lib/components/chat/ChatInput.svelte';
  import ModelSelector from '$lib/components/chat/ModelSelector.svelte';
  import SettingsPanel from '$lib/components/chat/SettingsPanel.svelte';
  import type { ModelInfo } from '$lib/types/chat';

  // Disable SSR for this page
  export const ssr = false;

  let settingsOpen = $state(false);
  let sidebarOpen = $state(true);
  let chatContainer: HTMLDivElement;

  // Mock models data - should be fetched from registry
  const mockModels: ModelInfo[] = [
    {
      id: 'tinyllama',
      name: 'TinyLlama 1.1B',
      context_length: 2048,
      quantization: 'q4_k_m',
      tokens_per_sec: 45.2,
      vram_usage_gb: 2.5,
      type: 'llm'
    }
  ];

  onMount(() => {
    // Load from localStorage
    loadSessions();
    loadSettings();
    setModels(mockModels);
    saveSessions();
    saveSettings();

    // Create initial session if none exists
    if (!currentSession) {
      createNewSession($chatSettings.model);
    }

    // Load API token from localStorage (in production, this would be from user input)
    const storedToken = localStorage.getItem('decentraai_api_token');
    if (!storedToken) {
      // For demo purposes, use hardcoded token
      localStorage.setItem('decentraai_api_token', '32508c50e6bd1a7c149ac8a580c42cdf1afea10657a9c6961466b1897e240ba2');
    }
  });

  onDestroy(() => {
    // Cleanup will be handled by localStorage persistence
  });

  async function sendMessage(content: string) {
    try {
      setLoading(true);
      setError(null);

      // Add user message
      addMessage({
        id: crypto.randomUUID(),
        role: 'user',
        content,
        timestamp: new Date().toISOString(),
        model: $chatSettings.model
      });

      // Prepare API request
      const apiRequest = {
        model: $chatSettings.model,
        messages: [
          { role: 'system', content: $chatSettings.system_prompt },
          ...($currentMessages.filter(m => m.role !== 'assistant').map(m => ({
            role: m.role,
            content: m.content
          }))),
          { role: 'user', content }
        ],
        temperature: $chatSettings.temperature,
        max_tokens: $chatSettings.max_tokens,
        top_p: $chatSettings.top_p,
        stream: $chatSettings.stream
      };

      if ($chatSettings.stream) {
        // Streaming response
        console.log('Starting streaming request...');
        const response = await fetch('/api/chat', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(apiRequest)
        });

        if (!response.ok) {
          const errorText = await response.text();
          throw new Error(`API error: ${response.status} - ${errorText}`);
        }

        const reader = response.body?.getReader();
        const decoder = new TextDecoder();

        // Add assistant message for streaming
        addMessage({
          id: crypto.randomUUID(),
          role: 'assistant',
          content: '',
          timestamp: new Date().toISOString(),
          model: $chatSettings.model,
          streaming: true
        });

        if (reader) {
          let fullContent = '';
          while (true) {
            const { done, value } = await reader.read();
            if (done) {
              console.log('Streaming complete, final content:', fullContent);
              break;
            }

            const chunk = decoder.decode(value);
            console.log('Received chunk:', chunk);
            const lines = chunk.split('\n').filter(line => line.trim());

            for (const line of lines) {
              if (line.startsWith('data: ')) {
                const data = line.slice(6);
                console.log('SSE data:', data);
                if (data === '[DONE]') {
                  updateLastMessage(fullContent, true);
                } else {
                  try {
                    const parsed = JSON.parse(data);
                    console.log('Parsed SSE:', parsed);
                    if (parsed.choices?.[0]?.delta?.content) {
                      const newContent = parsed.choices[0].delta.content;
                      fullContent += newContent;
                      updateLastMessage(fullContent);
                      
                      // Auto-scroll to bottom
                      if (chatContainer) {
                        chatContainer.scrollTop = chatContainer.scrollHeight;
                      }
                    }
                  } catch (e) {
                    console.error('Error parsing SSE data:', e);
                  }
                }
              }
            }
          }
        }
      } else {
        // Non-streaming response
        const response = await fetch('/api/chat', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(apiRequest)
        });

        if (!response.ok) {
          const errorText = await response.text();
          throw new Error(`API error: ${response.status} - ${errorText}`);
        }

        const data = await response.json();
        console.log('API Response:', data);
        
        // Add assistant message
        addMessage({
          id: crypto.randomUUID(),
          role: 'assistant',
          content: data.choices?.[0]?.message?.content || 'No response generated',
          timestamp: new Date().toISOString(),
          model: $chatSettings.model,
          tokens: data.usage?.total_tokens
        });
      }

    } catch (err) {
      console.error('Chat error:', err);
      setError(err instanceof Error ? err.message : 'Failed to send message');
    } finally {
      setLoading(false);
    }
  }

  function handleNewChat() {
    createNewSession($chatSettings.model);
  }

  function handleKeyPress(event: KeyboardEvent) {
    // CMD/Ctrl + K for settings
    if ((event.metaKey || event.ctrlKey) && event.key === 'k') {
      event.preventDefault();
      settingsOpen = !settingsOpen;
    }
    
    // CMD/Ctrl + N for new chat
    if ((event.metaKey || event.ctrlKey) && event.key === 'n') {
      event.preventDefault();
      handleNewChat();
    }
    
    // CMD/Ctrl + B for sidebar toggle
    if ((event.metaKey || event.ctrlKey) && event.key === 'b') {
      event.preventDefault();
      sidebarOpen = !sidebarOpen;
    }
  }
</script>

<svelte:window on:keydown={handleKeyPress} />

<Navigation currentPage="chat" />

<div class="chat-container">
  <!-- Sidebar -->
  {#if sidebarOpen}
    <aside class="sidebar">
      <div class="sidebar-header">
        <h2>💬 Chats</h2>
        <button on:click={handleNewChat} class="new-chat-button" title="New Chat (Cmd+N)">
          ➕ New
        </button>
      </div>

      <div class="sidebar-content">
        {#each $chatState.sessions as session}
          <div 
            class="session-item {session.id === $currentSession?.id ? 'active' : ''}"
            on:click={() => switchSession(session.id)}
          >
            <div class="session-title">{session.title}</div>
            <div class="session-time">
              {new Date(session.updatedAt).toLocaleDateString()}
            </div>
            <button 
              on:click|stopPropagation={() => deleteSession(session.id)}
              class="delete-button"
              title="Delete chat"
            >
              🗑️
            </button>
          </div>
        {/each}
      </div>

      <div class="sidebar-footer">
        <button on:click={() => settingsOpen = true} class="settings-button" title="Settings (Cmd+K)">
          ⚙️ Settings
        </button>
        <button on:click={() => sidebarOpen = false} class="close-sidebar-button" title="Close sidebar (Cmd+B)">
          ◀
        </button>
      </div>
    </aside>
  {/if}

  <!-- Main Chat Area -->
  <main class="chat-main">
    <!-- Header -->
    <header class="chat-header">
      <div class="header-left">
        <button on:click={() => sidebarOpen = true} class="sidebar-toggle" title="Open sidebar (Cmd+B)">
          ▶
        </button>
        <h1>{$currentSession?.title || 'New Chat'}</h1>
      </div>
      <div class="header-right">
        <ModelSelector models={$availableModels} selectedModel={$chatSettings.model} />
        <button on:click={() => clearChat()} class="clear-button" title="Clear chat">
          🗑️ Clear
        </button>
        <button on:click={() => settingsOpen = true} class="settings-header-button" title="Settings (Cmd+K)">
          ⚙️
        </button>
      </div>
    </header>

    <!-- Messages -->
    <div bind:this={chatContainer} class="messages-container">
      {#if $currentMessages.length === 0}
        <div class="empty-state">
          <div class="empty-icon">💬</div>
          <h2>Start a conversation</h2>
          <p>Ask me anything! I'm here to help with your questions.</p>
          <div class="quick-prompts">
            <button on:click={() => sendMessage('What can you help me with?')}>
              What can you help me with?
            </button>
            <button on:click={() => sendMessage('Explain how AI works')}>
              Explain how AI works
            </button>
            <button on:click={() => sendMessage('Write a short story')}>
              Write a short story
            </button>
          </div>
        </div>
      {:else}
        {#each $currentMessages as message}
          <ChatMessage {message} />
        {/each}
      {/if}

      {#if $isChatLoading}
        <div class="typing-indicator">
          <div class="typing-dot"></div>
          <div class="typing-dot"></div>
          <div class="typing-dot"></div>
        </div>
      {/if}
    </div>

    <!-- Input -->
    <div class="input-container">
      {#if $chatError}
        <div class="error-message">
          ⚠️ {$chatError}
        </div>
      {/if}
      <ChatInput onSend={sendMessage} disabled={$isChatLoading} />
    </div>
  </main>

  <!-- Settings Panel -->
  <SettingsPanel isOpen={settingsOpen} onClose={() => settingsOpen = false} />
</div>

<style>
  .chat-container {
    display: flex;
    height: 100vh;
    background: #0f172a;
    color: white;
  }

  .sidebar {
    width: 280px;
    background: rgba(30, 41, 59, 0.95);
    border-right: 1px solid rgba(75, 85, 99, 0.5);
    display: flex;
    flex-direction: column;
    transition: width 0.3s;
  }

  .sidebar-header {
    padding: 1rem;
    border-bottom: 1px solid rgba(75, 85, 99, 0.5);
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .sidebar-header h2 {
    font-size: 1.125rem;
    font-weight: 600;
  }

  .new-chat-button {
    background: linear-gradient(135deg, #3b82f6 0%, #1d4ed8 100%);
    border: none;
    border-radius: 0.375rem;
    padding: 0.5rem 0.75rem;
    color: white;
    font-size: 0.75rem;
    cursor: pointer;
    transition: transform 0.2s;
  }

  .new-chat-button:hover {
    transform: scale(1.05);
  }

  .sidebar-content {
    flex: 1;
    overflow-y: auto;
    padding: 0.5rem;
  }

  .session-item {
    background: rgba(51, 65, 85, 0.5);
    border: 1px solid rgba(75, 85, 99, 0.3);
    border-radius: 0.5rem;
    padding: 0.75rem;
    margin-bottom: 0.5rem;
    cursor: pointer;
    transition: background 0.2s;
    position: relative;
  }

  .session-item:hover {
    background: rgba(51, 65, 85, 0.8);
  }

  .session-item.active {
    background: rgba(59, 130, 246, 0.2);
    border-color: rgba(59, 130, 246, 0.5);
  }

  .session-title {
    font-weight: 500;
    margin-bottom: 0.25rem;
  }

  .session-time {
    font-size: 0.75rem;
    color: #9ca3af;
  }

  .delete-button {
    position: absolute;
    top: 0.5rem;
    right: 0.5rem;
    background: transparent;
    border: none;
    color: #ef4444;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.2s;
  }

  .session-item:hover .delete-button {
    opacity: 1;
  }

  .sidebar-footer {
    padding: 1rem;
    border-top: 1px solid rgba(75, 85, 99, 0.5);
    display: flex;
    gap: 0.5rem;
  }

  .settings-button,
  .close-sidebar-button {
    flex: 1;
    background: rgba(51, 65, 85, 0.5);
    border: 1px solid rgba(75, 85, 99, 0.3);
    border-radius: 0.375rem;
    padding: 0.5rem;
    color: white;
    cursor: pointer;
    transition: background 0.2s;
  }

  .settings-button:hover,
  .close-sidebar-button:hover {
    background: rgba(51, 65, 85, 0.8);
  }

  .chat-main {
    flex: 1;
    display: flex;
    flex-direction: column;
  }

  .chat-header {
    background: rgba(30, 41, 59, 0.95);
    border-bottom: 1px solid rgba(75, 85, 99, 0.5);
    padding: 1rem;
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 1rem;
  }

  .sidebar-toggle {
    background: transparent;
    border: none;
    color: white;
    cursor: pointer;
    font-size: 1.25rem;
  }

  .chat-header h1 {
    font-size: 1.125rem;
    font-weight: 600;
  }

  .header-right {
    display: flex;
    align-items: center;
    gap: 1rem;
  }

  .clear-button,
  .settings-header-button {
    background: rgba(51, 65, 85, 0.5);
    border: 1px solid rgba(75, 85, 99, 0.3);
    border-radius: 0.375rem;
    padding: 0.5rem;
    color: white;
    cursor: pointer;
    transition: background 0.2s;
  }

  .clear-button:hover,
  .settings-header-button:hover {
    background: rgba(51, 65, 85, 0.8);
  }

  .messages-container {
    flex: 1;
    overflow-y: auto;
    padding: 1rem;
  }

  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    text-align: center;
  }

  .empty-icon {
    font-size: 4rem;
    margin-bottom: 1rem;
  }

  .empty-state h2 {
    font-size: 1.5rem;
    font-weight: 600;
    margin-bottom: 0.5rem;
  }

  .empty-state p {
    color: #9ca3af;
    margin-bottom: 2rem;
  }

  .quick-prompts {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    width: 100%;
    max-width: 400px;
  }

  .quick-prompts button {
    background: rgba(51, 65, 85, 0.5);
    border: 1px solid rgba(75, 85, 99, 0.3);
    border-radius: 0.5rem;
    padding: 0.75rem;
    color: white;
    cursor: pointer;
    transition: background 0.2s;
  }

  .quick-prompts button:hover {
    background: rgba(51, 65, 85, 0.8);
  }

  .typing-indicator {
    display: flex;
    gap: 0.25rem;
    padding: 1rem;
  }

  .typing-dot {
    width: 8px;
    height: 8px;
    background: #3b82f6;
    border-radius: 50%;
    animation: typing 1.4s infinite;
  }

  .typing-dot:nth-child(2) {
    animation-delay: 0.2s;
  }

  .typing-dot:nth-child(3) {
    animation-delay: 0.4s;
  }

  @keyframes typing {
    0%, 60%, 100% {
      transform: translateY(0);
    }
    30% {
      transform: translateY(-10px);
    }
  }

  .input-container {
    padding: 1rem;
    background: rgba(30, 41, 59, 0.95);
    border-top: 1px solid rgba(75, 85, 99, 0.5);
  }

  .error-message {
    background: rgba(239, 68, 68, 0.2);
    border: 1px solid rgba(239, 68, 68, 0.5);
    border-radius: 0.5rem;
    padding: 0.75rem;
    margin-bottom: 1rem;
    color: #ef4444;
  }
</style>
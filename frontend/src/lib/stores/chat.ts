import { writable, derived } from 'svelte/store';
import type { ChatMessage, ChatSession, ModelInfo, ChatSettings, ChatState } from '$lib/types/chat';

// Chat state store
export const chatState = writable<ChatState>({
  currentSession: null,
  sessions: [],
  models: [],
  isLoading: false,
  error: null
});

// Settings store
export const chatSettings = writable<ChatSettings>({
  model: 'tinyllama',
  temperature: 0.9, // Higher temperature for more variety
  max_tokens: 2048,
  top_p: 0.95,
  stream: true,
  system_prompt: '' // Empty system prompt for more natural responses
});

// Current session
export const currentSession = derived(
  chatState,
  ($state) => $state.currentSession
);

// Current messages
export const currentMessages = derived(
  chatState,
  ($state) => $state.currentSession?.messages || []
);

// Loading state
export const isChatLoading = derived(
  chatState,
  ($state) => $state.isLoading
);

// Error state
export const chatError = derived(
  chatState,
  ($state) => $state.error
);

// Available models
export const availableModels = derived(
  chatState,
  ($state) => $state.models
);

// Functions
export function createNewSession(model: string): ChatSession {
  const session: ChatSession = {
    id: crypto.randomUUID(),
    title: 'New Chat',
    messages: [],
    model,
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString()
  };
  
  chatState.update(state => ({
    ...state,
    currentSession: session,
    sessions: [session, ...state.sessions]
  }));
  
  return session;
}

export function addMessage(message: ChatMessage) {
  chatState.update(state => {
    if (!state.currentSession) return state;
    
    const updatedSession = {
      ...state.currentSession,
      messages: [...state.currentSession.messages, message],
      updatedAt: new Date().toISOString()
    };
    
    // Update session title based on first user message
    if (state.currentSession.messages.length === 0 && message.role === 'user') {
      updatedSession.title = message.content.slice(0, 30) + (message.content.length > 30 ? '...' : '');
    }
    
    return {
      ...state,
      currentSession: updatedSession,
      sessions: state.sessions.map(s => 
        s.id === updatedSession.id ? updatedSession : s
      )
    };
  });
}

export function updateLastMessage(content: string, complete: boolean = false) {
  chatState.update(state => {
    if (!state.currentSession) return state;
    
    const messages = [...state.currentSession.messages];
    const lastMessage = messages[messages.length - 1];
    
    if (lastMessage && lastMessage.role === 'assistant') {
      messages[messages.length - 1] = {
        ...lastMessage,
        content,
        streaming: !complete
      };
    }
    
    const updatedSession = {
      ...state.currentSession,
      messages,
      updatedAt: new Date().toISOString()
    };
    
    return {
      ...state,
      currentSession: updatedSession,
      sessions: state.sessions.map(s => 
        s.id === updatedSession.id ? updatedSession : s
      )
    };
  });
}

export function switchSession(sessionId: string) {
  chatState.update(state => ({
    ...state,
    currentSession: state.sessions.find(s => s.id === sessionId) || null
  }));
}

export function deleteSession(sessionId: string) {
  chatState.update(state => {
    const updatedSessions = state.sessions.filter(s => s.id !== sessionId);
    return {
      ...state,
      sessions: updatedSessions,
      currentSession: state.currentSession?.id === sessionId 
        ? (updatedSessions[0] || null)
        : state.currentSession
    };
  });
}

export function clearChat() {
  chatState.update(state => {
    if (!state.currentSession) return state;
    
    const clearedSession = {
      ...state.currentSession,
      messages: [],
      title: 'New Chat',
      updatedAt: new Date().toISOString()
    };
    
    return {
      ...state,
      currentSession: clearedSession,
      sessions: state.sessions.map(s => 
        s.id === clearedSession.id ? clearedSession : s
      )
    };
  });
}

export function setLoading(loading: boolean) {
  chatState.update(state => ({ ...state, isLoading: loading }));
}

export function setError(error: string | null) {
  chatState.update(state => ({ ...state, error }));
}

export function setModels(models: ModelInfo[]) {
  chatState.update(state => ({ ...state, models }));
}

// Load sessions from localStorage
export function loadSessions() {
  try {
    const saved = localStorage.getItem('chat_sessions');
    if (saved) {
      const sessions: ChatSession[] = JSON.parse(saved);
      chatState.update(state => ({
        ...state,
        sessions
      }));
    }
  } catch (err) {
    console.error('Failed to load sessions:', err);
  }
}

// Save sessions to localStorage
export function saveSessions() {
  chatState.subscribe(state => {
    try {
      localStorage.setItem('chat_sessions', JSON.stringify(state.sessions));
    } catch (err) {
      console.error('Failed to save sessions:', err);
    }
  });
}

// Load settings from localStorage
export function loadSettings() {
  try {
    const saved = localStorage.getItem('chat_settings');
    if (saved) {
      const settings: ChatSettings = JSON.parse(saved);
      chatSettings.set(settings);
    }
  } catch (err) {
    console.error('Failed to load settings:', err);
  }
}

// Save settings to localStorage
export function saveSettings() {
  chatSettings.subscribe(settings => {
    try {
      localStorage.setItem('chat_settings', JSON.stringify(settings));
    } catch (err) {
      console.error('Failed to save settings:', err);
    }
  });
}
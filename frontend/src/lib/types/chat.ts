// Chat types
export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  timestamp: string;
  model?: string;
  tokens?: number;
  streaming?: boolean;
}

export interface ChatSession {
  id: string;
  title: string;
  messages: ChatMessage[];
  model: string;
  createdAt: string;
  updatedAt: string;
}

export interface ModelInfo {
  id: string;
  name: string;
  context_length: number;
  quantization: string;
  tokens_per_sec: number;
  vram_usage_gb: number;
  type: 'llm' | 'embedding' | 'vision';
}

export interface ChatSettings {
  model: string;
  temperature: number;
  max_tokens: number;
  top_p: number;
  stream: boolean;
  system_prompt: string;
}

export interface ChatState {
  currentSession: ChatSession | null;
  sessions: ChatSession[];
  models: ModelInfo[];
  isLoading: boolean;
  error: string | null;
}
export interface ProviderPreset {
  id: string;
  name: string;
  provider: string; // ProviderType value
  baseUrl: string;
  models: { id: string; name: string; recommended?: boolean }[];
  requiresApiKey: boolean;
  icon: string; // emoji
  description: string;
}

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: 'openai',
    name: 'OpenAI',
    provider: 'open_ai',
    baseUrl: 'https://api.openai.com/v1',
    requiresApiKey: true,
    icon: '🤖',
    description: 'GPT-5, GPT-4o, o3/o4 series',
    models: [
      { id: 'gpt-5.2', name: 'GPT-5.2 (Latest)', recommended: true },
      { id: 'gpt-5.2-codex', name: 'GPT-5.2 Codex (Coding)' },
      { id: 'gpt-5.1', name: 'GPT-5.1' },
      { id: 'gpt-5', name: 'GPT-5' },
      { id: 'gpt-5-mini', name: 'GPT-5 Mini' },
      { id: 'gpt-4.1', name: 'GPT-4.1 (1M Context)' },
      { id: 'gpt-4.1-mini', name: 'GPT-4.1 Mini' },
      { id: 'gpt-4o', name: 'GPT-4o' },
      { id: 'gpt-4o-mini', name: 'GPT-4o Mini' },
      { id: 'o4-mini', name: 'o4-mini (Reasoning)' },
      { id: 'o3', name: 'o3 (Reasoning)' },
      { id: 'o3-mini', name: 'o3-mini' },
      { id: 'codex-mini-latest', name: 'Codex Mini' },
    ],
  },
  {
    id: 'anthropic',
    name: 'Anthropic',
    provider: 'anthropic',
    baseUrl: 'https://api.anthropic.com/v1',
    requiresApiKey: true,
    icon: '🧠',
    description: 'Claude Opus, Sonnet, Haiku',
    models: [
      { id: 'claude-opus-4-6', name: 'Claude Opus 4.6 (Most Intelligent)', recommended: true },
      { id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5 (Best Balance)' },
      { id: 'claude-haiku-4-5', name: 'Claude Haiku 4.5 (Fastest)' },
      { id: 'claude-opus-4-20250514', name: 'Claude Opus 4' },
      { id: 'claude-sonnet-4-20250514', name: 'Claude Sonnet 4' },
      { id: 'claude-3-5-sonnet-latest', name: 'Claude 3.5 Sonnet' },
      { id: 'claude-3-5-haiku-latest', name: 'Claude 3.5 Haiku' },
    ],
  },
  {
    id: 'google',
    name: 'Google',
    provider: 'google',
    baseUrl: 'https://generativelanguage.googleapis.com/v1beta',
    requiresApiKey: true,
    icon: '✨',
    description: 'Gemini 3, 2.5 Pro/Flash',
    models: [
      { id: 'gemini-2.5-pro', name: 'Gemini 2.5 Pro (Best)', recommended: true },
      { id: 'gemini-2.5-flash', name: 'Gemini 2.5 Flash (Fast)' },
      { id: 'gemini-2.5-flash-lite', name: 'Gemini 2.5 Flash Lite (Cheapest)' },
      { id: 'gemini-3-pro-preview', name: 'Gemini 3 Pro (Preview)' },
      { id: 'gemini-3-flash', name: 'Gemini 3 Flash' },
      { id: 'gemini-2.0-flash', name: 'Gemini 2.0 Flash' },
      { id: 'gemini-1.5-pro', name: 'Gemini 1.5 Pro (2M Context)' },
    ],
  },
  {
    id: 'deepseek',
    name: 'DeepSeek',
    provider: 'deep_seek',
    baseUrl: 'https://api.deepseek.com',
    requiresApiKey: true,
    icon: '🔮',
    description: 'DeepSeek V3, Reasoner',
    models: [
      { id: 'deepseek-chat', name: 'DeepSeek Chat (V3.2)', recommended: true },
      { id: 'deepseek-reasoner', name: 'DeepSeek Reasoner (Thinking)' },
    ],
  },
  {
    id: 'xai',
    name: 'xAI',
    provider: 'open_ai', // xAI uses OpenAI-compatible API
    baseUrl: 'https://api.x.ai/v1',
    requiresApiKey: true,
    icon: '🅧',
    description: 'Grok 4, Grok 3',
    models: [
      { id: 'grok-4', name: 'Grok 4 (Flagship)', recommended: true },
      { id: 'grok-4-1-fast', name: 'Grok 4-1 Fast (2M Context!)' },
      { id: 'grok-3', name: 'Grok 3' },
      { id: 'grok-3-mini', name: 'Grok 3 Mini' },
    ],
  },
  {
    id: 'mistral',
    name: 'Mistral',
    provider: 'open_ai', // Mistral uses OpenAI-compatible API
    baseUrl: 'https://api.mistral.ai/v1',
    requiresApiKey: true,
    icon: '🌊',
    description: 'Mistral Large, Codestral',
    models: [
      { id: 'mistral-large-2512', name: 'Mistral Large 3', recommended: true },
      { id: 'codestral-2508', name: 'Codestral (Code)' },
      { id: 'devstral-2-2512', name: 'Devstral 2 (SWE)' },
      { id: 'magistral-medium-2509', name: 'Magistral Medium (Reasoning)' },
      { id: 'mistral-small-2506', name: 'Mistral Small 3.2' },
    ],
  },
  {
    id: 'ollama',
    name: 'Ollama',
    provider: 'ollama',
    baseUrl: 'http://localhost:11434',
    requiresApiKey: false,
    icon: '🦙',
    description: 'Local models, no API key',
    models: [
      { id: 'llama3.3:latest', name: 'Llama 3.3 (Recommended)', recommended: true },
      { id: 'qwen2.5:latest', name: 'Qwen 2.5' },
      { id: 'mistral:latest', name: 'Mistral' },
      { id: 'phi4:latest', name: 'Phi-4' },
      { id: 'deepseek-r1:latest', name: 'DeepSeek R1' },
      { id: 'gemma2:latest', name: 'Gemma 2' },
    ],
  },
  {
    id: 'lmstudio',
    name: 'LM Studio',
    provider: 'lm_studio',
    baseUrl: 'http://localhost:1234/v1',
    requiresApiKey: false,
    icon: '🖥️',
    description: 'Local models via LM Studio',
    models: [], // LM Studio models are user-loaded
  },
];

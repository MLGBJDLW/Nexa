export interface ProviderPreset {
  id: string;
  name: string;
  provider: string; // ProviderType value
  baseUrl: string;
  models: { id: string; name: string; tagKey?: string; recommended?: boolean }[];
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
    description: 'GPT-5.4, GPT-4.1, o3/o4 series',
    models: [
      { id: 'gpt-5.4', name: 'GPT-5.4', tagKey: 'providers.tagLatest', recommended: true },
      { id: 'gpt-5.4-pro', name: 'GPT-5.4 Pro', tagKey: 'providers.tagMostIntelligent' },
      { id: 'gpt-5.3-codex', name: 'GPT-5.3 Codex', tagKey: 'providers.tagCoding' },
      { id: 'gpt-5.2', name: 'GPT-5.2' },
      { id: 'gpt-5-mini', name: 'GPT-5 Mini' },
      { id: 'gpt-5-nano', name: 'GPT-5 Nano' },
      { id: 'gpt-4.1', name: 'GPT-4.1 (1M Context)' },
      { id: 'gpt-4.1-mini', name: 'GPT-4.1 Mini' },
      { id: 'gpt-4.1-nano', name: 'GPT-4.1 Nano' },
      { id: 'gpt-4o', name: 'GPT-4o' },
      { id: 'gpt-4o-mini', name: 'GPT-4o Mini' },
      { id: 'o3-pro', name: 'o3-pro', tagKey: 'providers.tagReasoning' },
      { id: 'o4-mini', name: 'o4-mini', tagKey: 'providers.tagReasoning' },
      { id: 'o3', name: 'o3', tagKey: 'providers.tagReasoning' },
      { id: 'o3-mini', name: 'o3-mini' },
    ],
  },
  {
    id: 'anthropic',
    name: 'Anthropic',
    provider: 'anthropic',
    baseUrl: 'https://api.anthropic.com/v1',
    requiresApiKey: true,
    icon: '🧠',
    description: 'Claude Opus 4.6, Sonnet 4.6/4.5, Haiku 4.5',
    models: [
      { id: 'claude-opus-4-6', name: 'Claude Opus 4.6', tagKey: 'providers.tagMostIntelligent' },
      { id: 'claude-sonnet-4-6', name: 'Claude Sonnet 4.6', tagKey: 'providers.tagBestBalance', recommended: true },
      { id: 'claude-opus-4-5', name: 'Claude Opus 4.5' },
      { id: 'claude-sonnet-4-5', name: 'Claude Sonnet 4.5' },
      { id: 'claude-haiku-4-5', name: 'Claude Haiku 4.5', tagKey: 'providers.tagFastest' },
      { id: 'claude-opus-4-1-20250805', name: 'Claude Opus 4.1' },
      { id: 'claude-sonnet-4-20250514', name: 'Claude Sonnet 4' },
      { id: 'claude-3-7-sonnet-20250219', name: 'Claude 3.7 Sonnet' },
      { id: 'claude-3-5-sonnet-20241022', name: 'Claude 3.5 Sonnet' },
      { id: 'claude-3-5-haiku-20241022', name: 'Claude 3.5 Haiku' },
    ],
  },
  {
    id: 'google',
    name: 'Google',
    provider: 'google',
    baseUrl: 'https://generativelanguage.googleapis.com/v1beta',
    requiresApiKey: true,
    icon: '✨',
    description: 'Gemini 3.1 preview, Gemini 2.5 Pro/Flash',
    models: [
      { id: 'gemini-2.5-pro', name: 'Gemini 2.5 Pro', tagKey: 'providers.tagBest', recommended: true },
      { id: 'gemini-2.5-flash', name: 'Gemini 2.5 Flash', tagKey: 'providers.tagFast' },
      { id: 'gemini-2.5-flash-lite', name: 'Gemini 2.5 Flash Lite', tagKey: 'providers.tagCheapest' },
      { id: 'gemini-3.1-pro-preview', name: 'Gemini 3.1 Pro Preview', tagKey: 'providers.tagPreview' },
      { id: 'gemini-3-flash-preview', name: 'Gemini 3 Flash Preview', tagKey: 'providers.tagPreview' },
      { id: 'gemini-3.1-flash-lite-preview', name: 'Gemini 3.1 Flash-Lite Preview', tagKey: 'providers.tagPreview' },
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
    description: 'DeepSeek V3.2-Exp, Reasoner',
    models: [
      { id: 'deepseek-chat', name: 'DeepSeek Chat (V3.2-Exp)', recommended: true },
      { id: 'deepseek-reasoner', name: 'DeepSeek Reasoner (R1)', tagKey: 'providers.tagReasoning' },
    ],
  },
  {
    id: 'xai',
    name: 'xAI',
    provider: 'open_ai', // xAI uses OpenAI-compatible API
    baseUrl: 'https://api.x.ai/v1',
    requiresApiKey: true,
    icon: '🅧',
    description: 'Grok 4.1 Fast, Grok 4, Grok Code',
    models: [
      { id: 'grok-4-1-fast-reasoning', name: 'Grok 4.1 Fast Reasoning', tagKey: 'providers.tagLatest', recommended: true },
      { id: 'grok-4-1-fast-non-reasoning', name: 'Grok 4.1 Fast', tagKey: 'providers.tagFast' },
      { id: 'grok-4-fast-reasoning', name: 'Grok 4 Fast Reasoning' },
      { id: 'grok-4-fast-non-reasoning', name: 'Grok 4 Fast' },
      { id: 'grok-code-fast-1', name: 'Grok Code Fast 1', tagKey: 'providers.tagCode' },
      { id: 'grok-4', name: 'Grok 4', tagKey: 'providers.tagFlagship' },
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
    description: 'Mistral Large 3, Medium 3.1, Devstral 2',
    models: [
      { id: 'mistral-large-2512', name: 'Mistral Large 3', tagKey: 'providers.tagFlagship', recommended: true },
      { id: 'mistral-medium-2508', name: 'Mistral Medium 3.1', tagKey: 'providers.tagBestBalance' },
      { id: 'mistral-small-2506', name: 'Mistral Small 3.2', tagKey: 'providers.tagFast' },
      { id: 'devstral-2512', name: 'Devstral 2', tagKey: 'providers.tagCode' },
      { id: 'codestral-2508', name: 'Codestral 25.08', tagKey: 'providers.tagCoding' },
      { id: 'magistral-medium-2509', name: 'Magistral Medium', tagKey: 'providers.tagReasoning' },
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
      { id: 'llama3.3:latest', name: 'Llama 3.3', tagKey: 'providers.tagRecommended', recommended: true },
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
  {
    id: 'zhipu',
    name: 'Zhipu (智谱GLM)',
    provider: 'zhipu',
    baseUrl: 'https://open.bigmodel.cn/api/paas/v4/',
    requiresApiKey: true,
    icon: '🔷',
    description: 'GLM-4 series, GLM-4V vision',
    models: [
      { id: 'glm-4-plus', name: 'GLM-4 Plus', tagKey: 'providers.tagBest', recommended: true },
      { id: 'glm-4', name: 'GLM-4' },
      { id: 'glm-4-long', name: 'GLM-4 Long (1M Context)' },
      { id: 'glm-4-flash', name: 'GLM-4 Flash', tagKey: 'providers.tagFast' },
      { id: 'glm-4v-plus', name: 'GLM-4V Plus', tagKey: 'providers.tagVision' },
      { id: 'glm-4v', name: 'GLM-4V' },
    ],
  },
  {
    id: 'moonshot',
    name: 'Moonshot (Kimi)',
    provider: 'moonshot',
    baseUrl: 'https://api.moonshot.cn/v1/',
    requiresApiKey: true,
    icon: '🌙',
    description: 'Kimi / 月之暗面, long context',
    models: [
      { id: 'moonshot-v1-128k', name: 'Moonshot V1 128K', tagKey: 'providers.tagBest', recommended: true },
      { id: 'moonshot-v1-32k', name: 'Moonshot V1 32K' },
      { id: 'moonshot-v1-8k', name: 'Moonshot V1 8K', tagKey: 'providers.tagFast' },
    ],
  },
  {
    id: 'qwen',
    name: 'Qwen (通义千问)',
    provider: 'qwen',
    baseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1/',
    requiresApiKey: true,
    icon: '☁️',
    description: 'Alibaba Qwen series, vision support',
    models: [
      { id: 'qwen-max', name: 'Qwen Max', tagKey: 'providers.tagBest', recommended: true },
      { id: 'qwen-plus', name: 'Qwen Plus' },
      { id: 'qwen-turbo', name: 'Qwen Turbo', tagKey: 'providers.tagFast' },
      { id: 'qwen-long', name: 'Qwen Long' },
      { id: 'qwen-vl-max', name: 'Qwen VL Max', tagKey: 'providers.tagVision' },
      { id: 'qwen-vl-plus', name: 'Qwen VL Plus' },
    ],
  },
  {
    id: 'doubao',
    name: 'Doubao (豆包)',
    provider: 'doubao',
    baseUrl: 'https://ark.cn-beijing.volces.com/api/v3/',
    requiresApiKey: true,
    icon: '🫘',
    description: 'ByteDance Doubao / 豆包',
    models: [
      { id: 'doubao-pro-256k', name: 'Doubao Pro 256K', tagKey: 'providers.tagBest', recommended: true },
      { id: 'doubao-pro-128k', name: 'Doubao Pro 128K' },
      { id: 'doubao-pro-32k', name: 'Doubao Pro 32K' },
      { id: 'doubao-lite-128k', name: 'Doubao Lite 128K', tagKey: 'providers.tagFast' },
      { id: 'doubao-lite-32k', name: 'Doubao Lite 32K' },
    ],
  },
  {
    id: 'yi',
    name: 'Yi (零一万物)',
    provider: 'yi',
    baseUrl: 'https://api.lingyiwanwu.com/v1/',
    requiresApiKey: true,
    icon: '🌟',
    description: '01.AI Yi series',
    models: [
      { id: 'yi-large', name: 'Yi Large', tagKey: 'providers.tagBest', recommended: true },
      { id: 'yi-medium', name: 'Yi Medium' },
      { id: 'yi-spark', name: 'Yi Spark', tagKey: 'providers.tagFast' },
      { id: 'yi-large-turbo', name: 'Yi Large Turbo' },
    ],
  },
  {
    id: 'baichuan',
    name: 'Baichuan (百川)',
    provider: 'baichuan',
    baseUrl: 'https://api.baichuan-ai.com/v1/',
    requiresApiKey: true,
    icon: '🏔️',
    description: 'Baichuan / 百川智能',
    models: [
      { id: 'Baichuan4', name: 'Baichuan 4', tagKey: 'providers.tagBest', recommended: true },
      { id: 'Baichuan3-Turbo', name: 'Baichuan 3 Turbo', tagKey: 'providers.tagFast' },
      { id: 'Baichuan3-Turbo-128k', name: 'Baichuan 3 Turbo 128K' },
    ],
  },
];

function normalizePresetBaseUrl(baseUrl: string | null | undefined): string {
  return (baseUrl ?? '').trim().replace(/\/+$/, '').toLowerCase();
}

export function findProviderPreset(input: {
  provider: string;
  baseUrl?: string | null;
}): ProviderPreset | null {
  const provider = input.provider.trim();
  const normalizedBaseUrl = normalizePresetBaseUrl(input.baseUrl);

  if (normalizedBaseUrl) {
    const exactMatch = PROVIDER_PRESETS.find((preset) => (
      preset.provider === provider &&
      normalizePresetBaseUrl(preset.baseUrl) === normalizedBaseUrl
    ));
    if (exactMatch) {
      return exactMatch;
    }
  }

  const providerMatches = PROVIDER_PRESETS.filter((preset) => preset.provider === provider);
  if (providerMatches.length === 1) {
    return providerMatches[0];
  }

  return null;
}

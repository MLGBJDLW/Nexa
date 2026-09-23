import type { CSSProperties } from 'react';
import { GoCopilot } from 'react-icons/go';

interface ProviderIconMeta {
  asset?: string;
  glyph?: typeof GoCopilot;
  fallback: string;
  label: string;
  tone: string;
}

const PROVIDER_ICON_META: Record<string, ProviderIconMeta> = {
  sourceful: { asset: '/provider-icons/sourceful.svg', fallback: 'SF', label: 'Sourceful', tone: 'bg-text-primary/10 text-text-primary' },
  flux: { asset: '/provider-icons/flux.svg', fallback: 'FL', label: 'Black Forest Labs', tone: 'bg-text-primary/10 text-text-primary' },
  krea: { asset: '/provider-icons/krea.svg', fallback: 'KR', label: 'Krea', tone: 'bg-text-primary/10 text-text-primary' },
  microsoft: { asset: '/provider-icons/microsoft.svg', fallback: 'MI', label: 'Microsoft AI', tone: 'bg-text-primary/10 text-text-primary' },
  recraft: { asset: '/provider-icons/recraft.svg', fallback: 'RE', label: 'Recraft', tone: 'bg-text-primary/10 text-text-primary' },

  copilot: {
    glyph: GoCopilot,
    fallback: 'GH',
    label: 'GitHub Copilot',
    tone: 'bg-text-primary/10 text-text-primary',
  },
  openai: {
    asset: '/provider-icons/openai.svg',
    fallback: 'AI',
    label: 'OpenAI',
    tone: 'bg-text-primary/10 text-text-primary',
  },
  openrouter: {
    asset: '/provider-icons/openrouter.svg',
    fallback: 'OR',
    label: 'OpenRouter',
    tone: 'bg-slate-500/12 text-slate-300',
  },
  anthropic: {
    asset: '/provider-icons/anthropic.svg',
    fallback: 'A',
    label: 'Anthropic',
    tone: 'bg-amber-500/12 text-amber-500',
  },
  gemini: {
    asset: '/provider-icons/gemini.svg',
    fallback: 'G',
    label: 'Google Gemini',
    tone: 'bg-violet-500/12 text-violet-400',
  },
  google: {
    asset: '/provider-icons/google.svg',
    fallback: 'G',
    label: 'Google',
    tone: 'bg-blue-500/12 text-blue-500',
  },
  deepseek: {
    asset: '/provider-icons/deepseek.svg',
    fallback: 'DS',
    label: 'DeepSeek',
    tone: 'bg-blue-500/12 text-blue-400',
  },
  zhipu: {
    asset: '/provider-icons/zhipu.svg',
    fallback: 'GLM',
    label: 'Zhipu',
    tone: 'bg-indigo-500/12 text-indigo-400',
  },
  moonshot: {
    asset: '/provider-icons/moonshot.svg',
    fallback: 'K',
    label: 'Moonshot / Kimi',
    tone: 'bg-sky-500/12 text-sky-400',
  },
  qwen: {
    asset: '/provider-icons/qwen.svg',
    fallback: 'Q',
    label: 'Qwen',
    tone: 'bg-violet-500/12 text-violet-400',
  },
  doubao: {
    asset: '/provider-icons/doubao.svg',
    fallback: 'DB',
    label: 'Doubao',
    tone: 'bg-pink-500/12 text-pink-400',
  },
  yi: {
    asset: '/provider-icons/zeroone.svg',
    fallback: '01',
    label: '01.AI / Yi',
    tone: 'bg-rose-500/12 text-rose-400',
  },
  baichuan: {
    asset: '/provider-icons/baichuan.svg',
    fallback: 'BC',
    label: 'Baichuan',
    tone: 'bg-teal-500/12 text-teal-400',
  },
  ollama: {
    asset: '/provider-icons/ollama.svg',
    fallback: 'OL',
    label: 'Ollama',
    tone: 'bg-lime-500/12 text-lime-400',
  },
  lmstudio: {
    asset: '/provider-icons/lmstudio.svg',
    fallback: 'LM',
    label: 'LM Studio',
    tone: 'bg-fuchsia-500/12 text-fuchsia-400',
  },
  azureai: {
    asset: '/provider-icons/azureai.svg',
    fallback: 'AZ',
    label: 'Azure OpenAI',
    tone: 'bg-cyan-500/12 text-cyan-400',
  },
  mistral: {
    asset: '/provider-icons/mistral.svg',
    fallback: 'M',
    label: 'Mistral AI',
    tone: 'bg-orange-500/12 text-orange-400',
  },
  minimax: {
    asset: '/provider-icons/minimax.svg',
    fallback: 'MM',
    label: 'MiniMax',
    tone: 'bg-violet-500/12 text-violet-400',
  },
  meta: {
    asset: '/provider-icons/meta.svg',
    fallback: 'M',
    label: 'Meta',
    tone: 'bg-blue-500/12 text-blue-400',
  },
  elevenlabs: {
    asset: '/provider-icons/elevenlabs.svg',
    fallback: '11',
    label: 'ElevenLabs',
    tone: 'bg-text-primary/10 text-text-primary',
  },
  groq: {
    asset: '/provider-icons/groq.svg',
    fallback: 'GQ',
    label: 'Groq',
    tone: 'bg-orange-500/12 text-orange-400',
  },
  sherpa: {
    fallback: 'S',
    label: 'sherpa-onnx',
    tone: 'bg-emerald-500/12 text-emerald-400',
  },
  jina: {
    asset: '/provider-icons/jina.svg',
    fallback: 'J',
    label: 'Jina AI',
    tone: 'bg-cyan-500/12 text-cyan-400',
  },
  xai: {
    asset: '/provider-icons/xai.svg',
    fallback: 'xAI',
    label: 'xAI',
    tone: 'bg-text-primary/10 text-text-primary',
  },
  alibabacloud: {
    asset: '/provider-icons/alibabacloud.svg',
    fallback: 'Ali',
    label: 'Alibaba Cloud',
    tone: 'bg-orange-500/12 text-orange-400',
  },
  siliconflow: {
    asset: '/provider-icons/siliconflow.svg',
    fallback: 'SF',
    label: 'SiliconFlow',
    tone: 'bg-violet-500/12 text-violet-400',
  },
  bytedance: {
    asset: '/provider-icons/bytedance.svg',
    fallback: 'BD',
    label: 'ByteDance',
    tone: 'bg-blue-500/12 text-blue-400',
  },
  xiaomimimo: { asset: '/provider-icons/xiaomimimo.svg', fallback: 'Mi', label: 'Xiaomi MiMo', tone: 'bg-orange-500/12 text-orange-500' },
  cohere: { asset: '/provider-icons/cohere.svg', fallback: 'C', label: 'Cohere', tone: 'bg-emerald-500/12 text-emerald-500' },
  inception: { asset: '/provider-icons/inception.svg', fallback: 'I', label: 'Inception', tone: 'bg-text-primary/10 text-text-primary' },
  inference: { asset: '/provider-icons/inference.svg', fallback: 'IN', label: 'Inference', tone: 'bg-sky-500/12 text-sky-500' },
  sakana: { asset: '/provider-icons/sakana.svg', fallback: 'S', label: 'Sakana AI', tone: 'bg-red-500/12 text-red-500' },
  antgroup: { asset: '/provider-icons/antgroup.svg', fallback: 'A', label: 'InclusionAI / Ant Group', tone: 'bg-blue-500/12 text-blue-500' },
  prismml: { asset: '/provider-icons/prismml.svg', fallback: 'P', label: 'PrismML', tone: 'bg-text-primary/10 text-text-primary' },
  unbiased: { asset: '/provider-icons/unbiased.svg', fallback: 'U', label: 'Unbiased', tone: 'bg-text-primary/10 text-text-primary' },
  nexagi: { asset: '/provider-icons/nexagi.svg', fallback: 'N', label: 'Nex-AGI', tone: 'bg-text-primary/10 text-text-primary' },
  custom: {
    fallback: 'AI',
    label: 'Custom provider',
    tone: 'bg-text-tertiary/12 text-text-secondary',
  },
};

const PROVIDER_TYPE_TO_ICON: Record<string, string> = {
  github_copilot: 'copilot',
  openai_codex: 'openai',
  open_ai: 'openai',
  openrouter: 'openrouter',
  anthropic: 'anthropic',
  google: 'gemini',
  deep_seek: 'deepseek',
  zhipu: 'zhipu',
  moonshot: 'moonshot',
  qwen: 'qwen',
  alibaba_model_studio: 'alibabacloud',
  siliconflow: 'siliconflow',
  doubao: 'doubao',
  yi: 'yi',
  baichuan: 'baichuan',
  ollama: 'ollama',
  lm_studio: 'lmstudio',
  azure_open_ai: 'azureai',
  minimax: 'minimax',
  elevenlabs: 'elevenlabs',
  groq: 'groq',
  sherpa_onnx: 'sherpa',
  jina: 'jina',
  custom: 'custom',
};

const PRESET_ID_TO_ICON: Record<string, string> = {
  xiaomimimo: 'xiaomimimo',
  githubcopilot: 'copilot',
  openaicodex: 'openai',
  openai: 'openai',
  openrouter: 'openrouter',
  anthropic: 'anthropic',
  google: 'gemini',
  deepseek: 'deepseek',
  xai: 'xai',
  mistral: 'mistral',
  minimax: 'minimax',
  metamodelapi: 'meta',
  elevenlabs: 'elevenlabs',
  groq: 'groq',
  'sherpa-onnx': 'sherpa',
  jina: 'jina',
  ollama: 'ollama',
  lmstudio: 'lmstudio',
  zhipu: 'zhipu',
  moonshot: 'moonshot',
  qwen: 'qwen',
  qwencloudintl: 'qwen',
  'alibaba-model-studio': 'alibabacloud',
  siliconflow: 'siliconflow',
  doubao: 'doubao',
  yi: 'yi',
  baichuan: 'baichuan',
};

const BASE_URL_ICON_MATCHERS: Array<[RegExp, string]> = [
  [/xiaomimimo\.com|mimo\.mi\.com/i, 'xiaomimimo'],
  [/openrouter\.ai/i, 'openrouter'],
  [/anthropic\.com/i, 'anthropic'],
  [/googleapis\.com|generativelanguage/i, 'gemini'],
  [/deepseek\.com/i, 'deepseek'],
  [/bigmodel\.cn/i, 'zhipu'],
  [/moonshot\.cn|moonshot\.ai|kimi/i, 'moonshot'],
  [/siliconflow\.cn/i, 'siliconflow'],
  [/dashscope|aliyuncs|alibabacloud/i, 'alibabacloud'],
  [/volces|volcengine|byte/i, 'doubao'],
  [/lingyiwanwu|01\.ai/i, 'yi'],
  [/baichuan-ai|baichuan/i, 'baichuan'],
  [/localhost:11434|ollama/i, 'ollama'],
  [/localhost:1234|lmstudio|lm-studio/i, 'lmstudio'],
  [/azure\.com|openai\.azure/i, 'azureai'],
  [/mistral\.ai/i, 'mistral'],
  [/minimax\.io/i, 'minimax'],
  [/api\.meta\.ai/i, 'meta'],
  [/elevenlabs\.io/i, 'elevenlabs'],
  [/groq\.com/i, 'groq'],
  [/jina\.ai/i, 'jina'],
  [/x\.ai/i, 'xai'],
  [/openai\.com/i, 'openai'],
];

const LABEL_ICON_MATCHERS: Array<[RegExp, string]> = [
  [/sourceful|riverflow/i, 'sourceful'],
  [/black-forest-labs|flux/i, 'flux'],
  [/krea/i, 'krea'],
  [/microsoft|mai-image/i, 'microsoft'],
  [/recraft/i, 'recraft'],
  [/xiaomi|\bmimo\b/i, 'xiaomimimo'],
  [/cohere|command-a/i, 'cohere'],
  [/inception|mercury/i, 'inception'],
  [/inference-net|schematron/i, 'inference'],
  [/sakana|fugu/i, 'sakana'],
  [/inclusionai|ling-3|antgroup/i, 'antgroup'],
  [/prism-?ml|prism-ml|bonsai/i, 'prismml'],
  [/unbiased|pareto/i, 'unbiased'],
  [/nex-agi|nex-n2/i, 'nexagi'],
  [/openrouter/i, 'openrouter'],
  [/anthropic|claude/i, 'anthropic'],
  [/gemini|google/i, 'gemini'],
  [/deepseek/i, 'deepseek'],
  [/zhipu|glm/i, 'zhipu'],
  [/moonshot|kimi/i, 'moonshot'],
  [/siliconflow|硅基流动/i, 'siliconflow'],
  [/alibaba|dashscope|aliyun|百炼/i, 'alibabacloud'],
  [/qwen|tongyi/i, 'qwen'],
  [/doubao|byte/i, 'doubao'],
  [/\b01\b|01\.ai|\byi\b/i, 'yi'],
  [/baichuan/i, 'baichuan'],
  [/ollama/i, 'ollama'],
  [/lm\s*studio/i, 'lmstudio'],
  [/azure/i, 'azureai'],
  [/mistral/i, 'mistral'],
  [/minimax/i, 'minimax'],
  [/\bmeta\b|muse\s*spark/i, 'meta'],
  [/eleven\s*labs/i, 'elevenlabs'],
  [/groq|orpheus/i, 'groq'],
  [/sherpa(?:-onnx)?/i, 'sherpa'],
  [/\bjina\b/i, 'jina'],
  [/\bxai\b|\bx\.ai\b|grok/i, 'xai'],
  [/openai|gpt/i, 'openai'],
];

interface ResolveProviderIconInput {
  provider: string;
  model?: string | null;
  providerId?: string | null;
  baseUrl?: string | null;
  label?: string | null;
}

interface ProviderIconProps extends ResolveProviderIconInput {
  className?: string;
  size?: 'xs' | 'sm' | 'md' | 'lg';
}

const sizeClasses = {
  xs: 'h-4 w-4',
  sm: 'h-6 w-6',
  md: 'h-8 w-8',
  lg: 'h-10 w-10',
};

const glyphSizeClasses = {
  xs: 'h-2.5 w-2.5',
  sm: 'h-3.5 w-3.5',
  md: 'h-[18px] w-[18px]',
  lg: 'h-[22px] w-[22px]',
};

const fallbackTextClasses = {
  xs: 'text-[7px]',
  sm: 'text-[9px]',
  md: 'text-[10px]',
  lg: 'text-xs',
};

function normalizeKey(value: string | null | undefined): string {
  return (value ?? '').trim().toLowerCase().replace(/[^a-z0-9]+/g, '');
}

export function resolveProviderIconMeta({
  provider,
  model,
  providerId,
  baseUrl,
  label,
}: ResolveProviderIconInput): ProviderIconMeta {
  if (model) {
    const brand = LABEL_ICON_MATCHERS.find(([pattern]) => pattern.test(model));
    if (brand) return PROVIDER_ICON_META[brand[1]];
  }
  const presetIcon = PRESET_ID_TO_ICON[normalizeKey(providerId)];
  if (presetIcon) {
    return PROVIDER_ICON_META[presetIcon];
  }

  const normalizedBaseUrl = (baseUrl ?? '').trim();
  if (normalizedBaseUrl) {
    const match = BASE_URL_ICON_MATCHERS.find(([pattern]) => pattern.test(normalizedBaseUrl));
    if (match) {
      return PROVIDER_ICON_META[match[1]];
    }
  }

  const labelText = (label ?? '').trim();
  if (labelText) {
    const match = LABEL_ICON_MATCHERS.find(([pattern]) => pattern.test(labelText));
    if (match) {
      return PROVIDER_ICON_META[match[1]];
    }
  }

  return PROVIDER_ICON_META[PROVIDER_TYPE_TO_ICON[provider] ?? 'custom'];
}

export function ProviderIcon({
  provider,
  model,
  providerId,
  baseUrl,
  label,
  className = '',
  size = 'md',
}: ProviderIconProps) {
  const meta = resolveProviderIconMeta({ provider, model, providerId, baseUrl, label });
  const Glyph = meta.glyph;
  const maskStyle = meta.asset
    ? ({
        WebkitMask: `url("${meta.asset}") center / contain no-repeat`,
        mask: `url("${meta.asset}") center / contain no-repeat`,
        backgroundColor: 'currentColor',
      } satisfies CSSProperties)
    : undefined;

  return (
    <span
      className={`inline-flex shrink-0 items-center justify-center rounded-md ${sizeClasses[size]} ${meta.tone} ${className}`}
      title={meta.label}
      aria-hidden="true"
    >
      {Glyph ? (
        <Glyph className={glyphSizeClasses[size]} aria-hidden="true" />
      ) : meta.asset ? (
        <span className={glyphSizeClasses[size]} style={maskStyle} />
      ) : (
        <span className={`font-semibold leading-none tracking-normal ${fallbackTextClasses[size]}`}>
          {meta.fallback}
        </span>
      )}
    </span>
  );
}

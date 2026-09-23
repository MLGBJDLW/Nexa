import type { TextToSpeechConfig } from '../types/conversation';

const MAX_AUTO_SPEECH_CHARS = 4_000;

/** Same readiness rules as TextToSpeechConfig::is_configured in the runtime. */
export function isTextToSpeechConfigured(config: TextToSpeechConfig | null | undefined): boolean {
  if (!config) return false;
  if (config.apiStyle === 'sherpa_onnx') {
    return Boolean(config.executablePath?.trim() && config.modelPath?.trim() && config.tokensPath?.trim()
      && (!['kokoro', 'kitten'].includes(config.model.trim()) || config.voicesPath?.trim()));
  }
  return Boolean(config.apiKey.trim() && config.model.trim()
    && (config.apiStyle === 'dashscope_audio_generation' ? config.baseUrl?.trim() : config.voice.trim()));
}

export function finalAnswerToSpeechText(markdown: string): string {
  return markdown
    .replace(/```[\s\S]*?```/g, ' Code block omitted. ')
    .replace(/`([^`]+)`/g, '$1')
    .replace(/!\[[^\]]*\]\([^)]*\)/g, '')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/^\s{0,3}#{1,6}\s+/gm, '')
    .replace(/^\s*[-*+]\s+/gm, '')
    .replace(/^\s*\d+[.)]\s+/gm, '')
    .replace(/^\s*>\s?/gm, '')
    .replace(/[*_~|]/g, '')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, MAX_AUTO_SPEECH_CHARS)
    .trim();
}

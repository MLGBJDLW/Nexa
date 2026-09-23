import { isTextToSpeechConfigured } from '../src/lib/autoSpeech';
import type { TextToSpeechConfig } from '../src/types/conversation';
import {
  classifyMediaError,
  playableMediaType,
  speechCacheKeyMaterial,
} from '../src/features/voice/speechPlaybackRuntime';

function equal(actual: unknown, expected: unknown): void {
  if (actual !== expected) throw new Error(`Expected ${String(expected)}, received ${String(actual)}`);
}

equal(playableMediaType('audio/mpeg', 'probably'), true);
equal(playableMediaType('audio/wav', ''), false);
equal(classifyMediaError(2), 'asset_access');
equal(classifyMediaError(3), 'decode');
equal(classifyMediaError(4), 'unsupported_format');
equal(classifyMediaError(undefined), 'playback');

equal(
  speechCacheKeyMaterial({ provider: 'open_ai', model: 'tts-1', voice: 'alloy', speed: 1, outputFormat: 'mp3' }, '  hello\nworld '),
  'open_ai\u0000tts-1\u0000alloy\u00001\u0000mp3\u0000hello world',
);

const ttsNext = { apiStyle: 'dashscope_audio_generation', apiKey: 'key', model: 'qwen-audio-3.1-tts-next', voice: '', baseUrl: 'https://workspace.cn-beijing.maas.aliyuncs.com/api/v1/services/audio/tts/SpeechSynthesizer' } as TextToSpeechConfig;
equal(isTextToSpeechConfigured(ttsNext), true);
equal(isTextToSpeechConfigured({ ...ttsNext, baseUrl: null }), false);
equal(isTextToSpeechConfigured({ ...ttsNext, apiKey: '' }), false);
equal(isTextToSpeechConfigured({ ...ttsNext, apiStyle: 'openai_speech' }), false);
equal(isTextToSpeechConfigured({ ...ttsNext, apiStyle: 'openai_speech', voice: 'alloy' }), true);

import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

const { build } = createRequire(import.meta.resolve('vite'))('esbuild');
async function loadCatalog(name) {
  const compiled = await build({
    entryPoints: [fileURLToPath(new URL(`../src/lib/${name}.ts`, import.meta.url))],
    bundle: true, platform: 'node', format: 'esm', write: false,
  });
  return import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);
}
const tts = await loadCatalog('ttsProviderPresets');
const stt = await loadCatalog('sttProviderPresets');
const playback = await loadCatalog('../features/voice/speechPlaybackRequest');
const { finalAnswerToSpeechText } = await loadCatalog('autoSpeech');

test('explicit speech presets select documented active models without claiming credential acceptance', () => {
  for (const [presets, select] of [
    [tts.TTS_PROVIDER_PRESETS, tts.defaultTtsItem],
    [stt.STT_PROVIDER_PRESETS, stt.defaultSttItem],
  ]) {
    for (const preset of presets) {
      assert.ok(select(preset.models)?.id, `${preset.id} should have a usable preset default`);
      assert.equal(select(preset.models).descriptor.availableToCredential, null);
      assert.ok(preset.lastVerifiedAt && preset.documentationUrls.length);
    }
  }
  const openai = stt.STT_PROVIDER_PRESETS.find(p => p.id === 'openai');
  assert.equal(stt.defaultSttItem(openai.models).id, 'gpt-transcribe');
  for (const legacy of openai.models.filter(m => m.id !== 'gpt-transcribe')) {
    assert.equal(legacy.descriptor.lifecycle, 'deprecated');
    assert.equal(legacy.descriptor.replacementModelId, 'gpt-transcribe');
    assert.equal(stt.defaultSttItem([legacy]), null);
  }
  const retired = { ...openai.models[0], descriptor: { ...openai.models[0].descriptor, lifecycle: 'removed' } };
  assert.equal(stt.defaultSttItem([retired]), null);
  assert.equal(tts.defaultTtsItem([retired]), null);
});

test('TTS model changes replace incompatible system voices and preserve private voices', () => {
  const qwen = tts.TTS_PROVIDER_PRESETS.find(p => p.id === 'dashscope-cosyvoice');
  assert.equal(tts.ttsVoiceForModel(qwen, 'qwen-audio-3.0-tts-flash'), 'longanhuan_v3.6');
  assert.equal(tts.ttsVoiceForModel(qwen, 'qwen-audio-3.0-tts-plus', 'longanhuan_v3.6'), 'longanlingxin');
  assert.equal(tts.ttsVoiceForModel(qwen, 'cosyvoice-v3-flash', 'longanhuan_v3.6'), 'longanyang');
  assert.equal(tts.ttsVoiceForModel(qwen, 'cosyvoice-v3.5-flash', 'longanhuan_v3.6'), '');
  assert.equal(tts.ttsVoiceForModel(qwen, 'cosyvoice-v3.5-flash', 'my-cloned-voice'), 'my-cloned-voice');
  assert.equal(tts.ttsVoiceForModel(qwen, 'cosyvoice-v3.5-flash', 'longanhuan_v3.6', 'https://private.example/v1'), 'longanhuan_v3.6');
  assert.equal(tts.findTtsProviderPreset({ provider: 'qwen', apiStyle: 'dashscope_audio_generation' }).id, 'qwen-audio-generation');
  assert.equal(tts.findTtsProviderPreset({ provider: 'qwen', apiStyle: 'dashscope_speech' }).id, 'dashscope-cosyvoice');
  const openai = tts.TTS_PROVIDER_PRESETS.find(p => p.id === 'openai');
  assert.equal(tts.ttsVoiceForModel(openai, 'tts-1', 'marin'), 'coral');
  for (const base of [null, '', '  ', 'https://api.openai.com/v1/']) {
    assert.equal(tts.ttsPresetEndpointMatches(openai, base), true);
    assert.equal(tts.ttsVoiceForModel(openai, 'tts-1', 'marin', base), 'coral');
  }
  const groq = tts.TTS_PROVIDER_PRESETS.find(p => p.id === 'groq');
  assert.equal(tts.ttsVoiceForModel(groq, 'canopylabs/orpheus-arabic-saudi', 'hannah'), 'abdullah');
  assert.deepEqual(tts.ttsSpeedRange(groq), [0.5, 2]);
  assert.deepEqual(tts.ttsSpeedRange(groq, 'https://private.example/v1'), [0.5, 2]);
  assert.deepEqual(tts.ttsSpeedRange(tts.TTS_PROVIDER_PRESETS.find(p => p.id === 'elevenlabs')), [0.7, 1.2]);
});

test('new models are bound to implemented transports and unknown realtime models stay final-only', () => {
  const eleven = tts.TTS_PROVIDER_PRESETS.find(p => p.id === 'elevenlabs');
  assert.ok(eleven.models.some(m => m.id === 'eleven_v4'));
  assert.ok(!eleven.models.some(m => ['eleven_v4_turbo', 'eleven_v3_conversational'].includes(m.id)));
  const config = { provider: 'alibaba_model_studio', apiStyle: 'dashscope_realtime_asr', model: 'qwen3-asr-flash-realtime-2026-02-10' };
  assert.equal(stt.sttRuntimeCapabilities(config).transcriptDelivery, 'interimAndFinal');
  assert.equal(stt.sttRuntimeCapabilities({ ...config, model: 'qwen3-asr-flash-realtime-2099-01-01' }).transcriptDelivery, 'finalOnly');
  const local = stt.STT_PROVIDER_PRESETS.find(p => p.id === 'local-whisper');
  assert.equal(stt.defaultSttItem(local.models).id, 'whisper-local');
  const rust = readFileSync(new URL('../../../crates/core/src/app_settings.rs', import.meta.url), 'utf8');
  assert.match(rust, /fn default_stt_model\(\) -> String \{\s*"whisper-local"\.to_string\(\)/);
});

test('every curated voice model binding resolves inside its own provider', () => {
  for (const preset of tts.TTS_PROVIDER_PRESETS) {
    const models = new Set(preset.models.map(m => m.id));
    for (const voice of preset.voices) {
      for (const model of voice.modelIds ?? []) assert.ok(models.has(model), `${preset.id}: ${voice.id} -> ${model}`);
    }
  }
});

const groqConfig = {
  provider: 'groq', apiStyle: 'openai_speech', apiKey: 'test-only',
  baseUrl: 'https://api.groq.com/openai/v1', model: 'canopylabs/orpheus-v1-english',
  voice: 'hannah', outputFormat: 'wav', speed: 1,
};

test('read-aloud preserves text above 4000 characters and enforces Unicode model limits before synthesis', async () => {
  const longReply = '中'.repeat(4500) + '🎧完';
  assert.equal(finalAnswerToSpeechText(longReply), longReply);
  let calls = 0;
  const dependencies = {
    loadConfig: async () => groqConfig,
    synthesize: async text => { calls += 1; return text; },
    isCurrent: () => true,
  };
  assert.deepEqual(await playback.requestSpeechPlayback('🎧'.repeat(201), dependencies), {
    kind: 'input_limit', model: groqConfig.model, actual: 201, limit: 200,
  });
  assert.equal(calls, 0);
  assert.equal((await playback.requestSpeechPlayback('🎧'.repeat(200), dependencies)).kind, 'ready');
  assert.equal(calls, 1);
  const privateConfig = { ...groqConfig, baseUrl: 'https://private.example/v1' };
  const result = await playback.requestSpeechPlayback(longReply, { ...dependencies, loadConfig: async () => privateConfig });
  assert.equal(result.kind, 'ready');
  assert.equal(result.preview, longReply);
  assert.equal(calls, 2);
  const openaiConfig = { ...groqConfig, provider: 'open_ai', baseUrl: 'https://api.openai.com/v1', model: 'gpt-4o-mini-tts', voice: 'coral' };
  assert.equal(playback.speechPlaybackInputLimit(openaiConfig), 4096);
  assert.equal((await playback.requestSpeechPlayback(longReply, { ...dependencies, loadConfig: async () => openaiConfig })).kind, 'input_limit');
  assert.equal(calls, 2);
});

test('read-aloud cancellation during config lookup causes no synthesis and settings are snapshotted', async () => {
  let resolveConfig;
  let current = true;
  let calls = 0;
  const pending = playback.requestSpeechPlayback('Hello.', {
    loadConfig: () => new Promise(resolve => { resolveConfig = resolve; }),
    synthesize: async () => { calls += 1; return 'unexpected'; },
    isCurrent: () => current,
  });
  current = false;
  resolveConfig(groqConfig);
  assert.deepEqual(await pending, { kind: 'cancelled' });
  assert.equal(calls, 0);
  const source = { ...groqConfig };
  const result = await playback.requestSpeechPlayback('Hello.', {
    loadConfig: async () => source,
    synthesize: async (text, snapshot) => {
      calls += 1;
      source.model = 'another-model';
      assert.notEqual(snapshot, source);
      assert.equal(snapshot.model, groqConfig.model);
      return text;
    },
    isCurrent: () => true,
  });
  assert.equal(result.kind, 'ready');
  assert.equal(calls, 1);
});

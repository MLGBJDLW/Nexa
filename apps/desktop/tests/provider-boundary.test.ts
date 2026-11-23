import { credentialFeatureEndpoint, findSharedProviderCredential, providerCredentialScope } from '../src/lib/providerCredentials';

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertEqual<T>(actual: T, expected: T, message: string): void {
  assert(Object.is(actual, expected), `${message}: expected ${String(expected)}, got ${String(actual)}`);
}

const alibabaChat = 'https://dashscope.aliyuncs.com/compatible-mode/v1';
const alibabaRealtimeSpeech = 'https://dashscope.aliyuncs.com/api-ws/v1';
assertEqual(
  providerCredentialScope('alibaba_model_studio', alibabaChat),
  'alibaba-model-studio:beijing',
  'canonical Alibaba chat endpoint should share its regional credential',
);
assertEqual(
  providerCredentialScope('alibaba_model_studio', alibabaRealtimeSpeech),
  'alibaba-model-studio:beijing',
  'canonical Alibaba realtime speech should share the Beijing credential',
);

for (const endpoint of [
  'https://dashscope.aliyuncs.com/evil',
  `${alibabaChat}?workspace=other`,
  `${alibabaChat}#edited`,
  'http://dashscope.aliyuncs.com/compatible-mode/v1',
  'https://dashscope.aliyuncs.com:8443/compatible-mode/v1',
]) {
  assert(
    providerCredentialScope('alibaba_model_studio', endpoint).startsWith('endpoint:'),
    `edited endpoint must keep an isolated credential scope: ${endpoint}`,
  );
}

assertEqual(
  providerCredentialScope('open_ai', 'https://api.meta.ai/v1'),
  'meta-model-api',
  'the exact Meta Model API endpoint should keep its own credential scope',
);
for (const endpoint of [
  'https://api.meta.ai/v2',
  'https://api.meta.ai.evil.example/v1',
  'http://api.meta.ai/v1',
]) {
  assert(
    providerCredentialScope('open_ai', endpoint).startsWith('endpoint:'),
    `edited Meta endpoints must keep isolated credentials: ${endpoint}`,
  );
}

assertEqual(
  providerCredentialScope('zhipu', 'https://open.bigmodel.cn/api/paas/v4'),
  'zhipu:china',
  'China Zhipu Model API should keep its regional credential scope',
);
assertEqual(
  providerCredentialScope('zhipu', 'https://api.z.ai/api/paas/v4'),
  'zhipu:international',
  'international Z.ai Model API should keep a separate credential scope',
);
assertEqual(
  providerCredentialScope('zhipu'),
  'zhipu:china',
  'the legacy/default Zhipu provider identity should continue to resolve to China',
);
for (const endpoint of [
  'https://open.bigmodel.cn/api/coding/paas/v4',
  'https://api.z.ai/api/coding/paas/v4',
  'https://api.z.ai/api/paas/v4?workspace=other',
  'http://api.z.ai/api/paas/v4',
]) {
  assert(
    providerCredentialScope('zhipu', endpoint).startsWith('endpoint:'),
    `Coding Plan and edited Z.ai routes must keep isolated credentials: ${endpoint}`,
  );
}

for (const [provider, chat, speech] of [
  ['google', 'https://generativelanguage.googleapis.com/v1beta', 'wss://generativelanguage.googleapis.com/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent'],
  ['mistral', 'https://api.mistral.ai/v1', 'https://api.mistral.ai/v1/audio/transcriptions/realtime'],
]) {
  assertEqual(providerCredentialScope(provider, chat), providerCredentialScope(provider, speech), `${provider} realtime speech reuses its official chat key`);
}

const sgWorkspace = 'https://ws-a.ap-southeast-1.maas.aliyuncs.com';
const saved = { provider: 'alibaba_model_studio', baseUrl: sgWorkspace + '/compatible-mode/v1', apiKey: 'fixture', name: 'workspace' };
const tts = credentialFeatureEndpoint(saved, 'tts', { id: 'qwen-audio-generation' });
assertEqual(tts, sgWorkspace + '/api/v1/services/audio/tts/SpeechSynthesizer', 'audio generation keeps the selected workspace');
assertEqual(findSharedProviderCredential([saved], 'qwen', tts), saved, 'the selected tenant owns both paths');
assertEqual(findSharedProviderCredential([saved], 'qwen', tts?.replace('ws-a.', 'ws-b.')), null, 'workspaces do not share keys');
assertEqual(credentialFeatureEndpoint(saved, 'systemOne', { id: 'alibaba-model-studio-cn' }), null, 'the selected region must match');
assertEqual(credentialFeatureEndpoint({provider:'qwen', baseUrl:'https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1'}, 'tts', {id:'qwen-audio-generation'}), null, 'Token Plan does not provide a PAYG workspace key');
assertEqual(credentialFeatureEndpoint({provider:'custom',baseUrl:'https://proxy.example/v1'}, 'embedding', {id:'openai',baseUrl:'https://api.openai.com/v1'}), null, 'a proxy key stays with the proxy');
assertEqual(credentialFeatureEndpoint({provider:'deep_seek',baseUrl:'https://api.deepseek.com'}, 'webSearch', {id:'deepseek',baseUrl:'https://api.deepseek.com/v1/tools/search'}), 'https://api.deepseek.com/v1/tools/search', 'DeepSeek Search accepts its native key');

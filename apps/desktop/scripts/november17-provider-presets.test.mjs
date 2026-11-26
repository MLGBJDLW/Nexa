import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
const read = name => JSON.parse(readFileSync(new URL(`../../../shared/${name}.json`, import.meta.url), 'utf8'));
const text = read('provider-presets');
const model = (p,id) => text.find(x=>x.id===p).models.find(x=>x.id===id);
test('Hy4 Flash is opt-in and exact router controls do not invent effort levels',()=>{
 const m=model('openrouter','tencent/hy4-flash');assert.equal(m.contextTokens,1048576);assert.equal(m.maxOutputTokens,65536);assert.equal(m.recommended,false);assert.equal(m.supportsTools,true);assert.equal(m.supportsStructuredOutput,true);assert.deepEqual(m.inputModalities,['text','image']);assert.deepEqual(m.capabilities.reasoning,{mode:'optional',defaultEnabled:true});
});
test('Qwen4 Token Plan keeps the deprecated preview ID and discloses vendor billing',()=>{
 for(const [p,r] of [['qwen-token-plan-cn','cn-beijing'],['qwen-token-plan-global','ap-southeast-1']]) {const m=model(p,'qwen4-max');assert.deepEqual(m.regions,[r]);assert.equal(m.access,'account_enablement');assert.equal(m.recommended,true);const legacy=model(p,'qwen4-max-preview');assert.equal(legacy.deprecationDate,'2026-11-25');assert.equal(legacy.status,'deprecated');assert.equal(legacy.replacementModelId,'qwen4-max');assert.match(legacy.name,/standard rate/);assert.equal(legacy.recommended,false);}
});
test('Liquid D1.2 remains on Decisions, not chat',()=>{
 const p=read('system-one-provider-presets').find(x=>x.id==='openrouter');for(const id of ['liquid/d1.2','liquid/d1.2-ultra']) {const m=p.models.find(x=>x.id===id);assert.equal(m.contextTokens,65536);assert.equal(m.recommended,false);assert.deepEqual(m.outputModalities,['decisions']);assert.equal(text.some(p=>p.models?.some(m=>m.id===id)),false);}
});
test('GPT Image 2 has independently documented sunsets without a replacement route', () => {
  const presets = read('image-provider-presets');
  const native = presets.find(p => p.id === 'openai').models.find(m => m.id === 'gpt-image-2');
  const routed = presets.find(p => p.id === 'openrouter-images').models.find(m => m.id === 'openai/gpt-image-2');
  for (const model of [native, routed]) {
    assert.equal(model.status, 'deprecated');
    assert.equal(model.deprecationDate, '2027-02-10');
    assert.equal(model.recommended, false);
    assert.equal(model.replacementModelId, undefined);
  }
  assert.match(native.documentationRef, /^https:\/\/developers\.openai\.com\//);
  assert.equal(routed.documentationRef, 'https://openrouter.ai/openai/gpt-image-2');
});
test('Seedance Flash uses confirmed frame operations and explicit durations only',()=>{
 const m=read('video-provider-presets').find(x=>x.id==='openrouter-video').models.find(x=>x.modelId==='bytedance/seedance-2.5-flash');assert.equal(m.selectable,true);assert.deepEqual(m.operationCapabilities.map(x=>x.operation),['text_to_video','image_to_video','first_last_frame']);for(const op of m.operationCapabilities){assert.equal(op.supportsAudio,true);assert.equal(op.supportsSeed,true);assert.deepEqual(op.durationOptions.map(x=>x.resolution),['480p','720p']);for(const o of op.durationOptions){assert.equal(o.durationsSeconds[0],4);assert.equal(o.durationsSeconds.at(-1),30);assert.equal(o.durationsSeconds.includes(-1),false);}}
});
test('expired embedding with no routing endpoints retains its saved ID and no replacement', () => {
  const router = read('embedding-provider-presets').find(provider => provider.id === 'openrouter');
  const retired = router.models.find(model => model.id === 'nvidia/llama-nemotron-embed-vl-1b-v2:free');
  assert.equal(retired.status, 'removed');
  assert.equal(retired.deprecationDate, '2026-11-16');
  assert.equal(retired.replacementModelId, undefined);
});
test('Cohere light v3 models keep their fixed dimensions alongside Embed 5 Fast',()=>{
 const p=read('embedding-provider-presets').find(x=>x.id==='cohere');for(const id of ['embed-english-light-v3.0','embed-multilingual-light-v3.0']){const m=p.models.find(x=>x.id===id);assert.equal(m.dimensions,384);assert.equal(m.supportsDimensionOverride,false);assert.equal(m.recommended,false);}assert.equal(p.models.find(x=>x.id==='embed-v5.0-fast').recommended,true);assert.equal(p.models.find(x=>x.id==='embed-english-v3.0').dimensions,1024);
});

test('November 26 lifecycle changes respect route and announcement scope', () => {
  for (const id of ['unbiased/pareto-26.10-preview', 'unbiased/pareto-26.10', 'unbiased/pareto-26.9']) {
    const m = model('openrouter', id);
    assert.equal(m.status, 'removed');
    assert.equal(m.deprecationDate, '2026-11-25');
    assert.equal(m.replacementModelId, undefined);
  }
  assert.equal(model('openrouter', '~unbiased/pareto-latest').status, 'deprecated');
  for (const id of ['gemini-2.5-flash', 'gemini-2.5-pro']) {
    const routed = model('openrouter', 'google/' + id);
    assert.equal(routed.status, 'deprecated');
    assert.equal(routed.deprecationDate, '2026-12-01');
    assert.equal(routed.replacementModelId, undefined);
    assert.ok(model('google', id).replacementModelId);
  }
});

test('OpenRouter Chimera keeps exact hosted limits and stays explicit', () => {
  const hosted = model('openrouter', 'tngtech/deepseek-r1t2-chimera');
  assert.equal(hosted.contextTokens, 131_072);
  assert.equal(hosted.maxOutputTokens, 32_768);
  assert.equal(hosted.supportsTools, false);
  assert.equal(hosted.supportsStructuredOutput, true);
  assert.equal(hosted.capabilities.reasoning.mode, 'always');
  assert.deepEqual(hosted.inputModalities, ['text']);
  assert.deepEqual(hosted.outputModalities, ['text']);
  assert.equal(hosted.recommended, false);
  assert.equal(hosted.productReadiness, 'known');
  assert.equal(model('deepseek', hosted.id), undefined);
});

test('stable Alibaba Decision Model is opt-in on the two supported workspace routes', () => {
  const decisions = read('system-one-provider-presets');
  const routes = decisions.filter(preset => preset.models.some(row => row.id === 'decision-model'));
  assert.deepEqual(routes.map(route => route.id), ['alibaba-model-studio-cn', 'alibaba-model-studio-intl']);
  for (const [route, region] of [[routes[0], 'cn-beijing'], [routes[1], 'ap-southeast-1']]) {
    const row = route.models.find(row => row.id === 'decision-model');
    assert.equal(route.baseUrl, '');
    assert.equal(route.baseUrlPlaceholder, `https://<WorkspaceId>.${region}.maas.aliyuncs.com/compatible-mode/v1`);
    assert.deepEqual(row.regions, [region]);
    assert.equal(row.access, 'account_enablement');
    assert.equal(row.recommended, false);
    assert.equal(row.contextTokens, 268_288);
    assert.deepEqual(row.inputModalities, ['text', 'image']);
    assert.deepEqual(row.outputModalities, ['decisions']);
    assert.equal(route.models.find(m => m.id === 'decision-model-preview').status, 'preview');
  }
  assert.equal(text.some(preset => preset.models.some(row => row.id === 'decision-model')), false);
});

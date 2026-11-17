import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
const read = name => JSON.parse(readFileSync(new URL(`../../../shared/${name}.json`, import.meta.url), 'utf8'));
const text = read('provider-presets');
const model = (p,id) => text.find(x=>x.id===p).models.find(x=>x.id===id);
test('Hy4 Flash is opt-in and exact router controls do not invent effort levels',()=>{
 const m=model('openrouter','tencent/hy4-flash');assert.equal(m.contextTokens,1048576);assert.equal(m.maxOutputTokens,65536);assert.equal(m.recommended,false);assert.equal(m.supportsTools,true);assert.equal(m.supportsStructuredOutput,true);assert.deepEqual(m.inputModalities,['text','image']);assert.deepEqual(m.capabilities.reasoning,{mode:'optional',defaultEnabled:true});
});
test('Qwen4 Token Plan stays region/account scoped and keeps preview',()=>{
 for(const [p,r] of [['qwen-token-plan-cn','cn-beijing'],['qwen-token-plan-global','ap-southeast-1']]) {const m=model(p,'qwen4-max');assert.deepEqual(m.regions,[r]);assert.equal(m.access,'account_enablement');assert.equal(m.recommended,false);assert.equal(model(p,'qwen4-max-preview').deprecationDate,'2026-11-26');}
});
test('Liquid D1.2 remains on Decisions, not chat',()=>{
 const p=read('system-one-provider-presets').find(x=>x.id==='openrouter');for(const id of ['liquid/d1.2','liquid/d1.2-ultra']) {const m=p.models.find(x=>x.id===id);assert.equal(m.contextTokens,65536);assert.equal(m.recommended,false);assert.deepEqual(m.outputModalities,['decisions']);assert.equal(text.some(p=>p.models?.some(m=>m.id===id)),false);}
});
test('GPT Image 2 future shutdown remains deprecated and router scope is untouched',()=>{
 const ps=read('image-provider-presets');const m=ps.find(x=>x.id==='openai').models.find(x=>x.id==='gpt-image-2');assert.equal(m.status,'deprecated');assert.equal(m.deprecationDate,'2027-02-10');assert.equal(m.recommended,false);assert.equal(m.replacementModelId,undefined);const routed=ps.find(x=>x.id==='openrouter-images').models.find(x=>x.id==='openai/gpt-image-2');if(routed)assert.notEqual(routed.deprecationDate,'2027-02-10');
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
test('Cohere light v3 additions do not change default or existing dimensions',()=>{
 const p=read('embedding-provider-presets').find(x=>x.id==='cohere');for(const id of ['embed-english-light-v3.0','embed-multilingual-light-v3.0']){const m=p.models.find(x=>x.id===id);assert.equal(m.dimensions,384);assert.equal(m.supportsDimensionOverride,false);assert.equal(m.recommended,false);}assert.equal(p.models.find(x=>x.id==='embed-v4.0').recommended,true);assert.equal(p.models.find(x=>x.id==='embed-english-v3.0').dimensions,1024);
});

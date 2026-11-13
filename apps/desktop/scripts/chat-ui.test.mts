import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parseChatUi, MAX_UI_BYTES } from '../../../crates/core/assets/skills/visual-explanations/resources/chat-ui.ts';
import { transformMarkdownProse, markdownProse } from '../src/lib/markdownCodeProtection.ts';
import { parseFollowupSuggestions, latestFollowupSuggestions, stripFollowupBlocks } from '../src/lib/followupSuggestions.ts';
import { localFileReference, isInlineFileReference } from '../src/features/preview/localFileReference.ts';
import { invalidateStorageUsage, readStorageUsage, storageUsageSnapshot } from '../src/lib/storageUsageCache.ts';

test('storage measurements survive tab remounts and coalesce refresh after a mutation', async () => {
  const first = { totalBytes: 10 } as any;
  const second = { totalBytes: 20 } as any;
  let finish!: (value: any) => void;
  let calls = 0;
  const service = { usage: () => { calls++; return new Promise(resolve => { finish = resolve; }); } } as any;
  const pending = readStorageUsage(service);
  await Promise.resolve();
  assert.equal(readStorageUsage(service), pending);
  finish(first);
  assert.equal(await pending, first);
  assert.equal(storageUsageSnapshot(service), first);
  assert.equal(await readStorageUsage(service), first);
  assert.equal(calls, 1);
  invalidateStorageUsage(service);
  const refreshed = readStorageUsage(service);
  await Promise.resolve();
  invalidateStorageUsage(service);
  assert.equal(readStorageUsage(service), refreshed);
  assert.equal(storageUsageSnapshot(service), first);
  finish(first);
  await Promise.resolve();
  assert.equal(calls, 3);
  finish(second);
  assert.equal(await refreshed, second);
  assert.equal(storageUsageSnapshot(service), second);
});

test('failed storage refresh preserves the last measurement and can retry', async () => {
  let fail = false;
  const value = { totalBytes: 10 } as any;
  const service = { usage: async () => { if (fail) throw new Error('disk unavailable'); return value; } } as any;
  await readStorageUsage(service);
  invalidateStorageUsage(service);
  fail = true;
  await assert.rejects(readStorageUsage(service), /disk unavailable/);
  assert.equal(storageUsageSnapshot(service), value);
  fail = false;
  assert.equal(await readStorageUsage(service), value);
});

test('automatic inline images reject network and device references before native reads', () => {
  for (const path of ['//example.invalid/share/photo.png', '\\\\example.invalid\\share\\photo.png', 'file://example.invalid/share/photo.png', '%2f%2fexample.invalid/share/photo.png', '\\\\?\\C:\\photo.png', 'javascript:alert(1)']) {
    assert.equal(isInlineFileReference(localFileReference(path)), false, path);
  }
  for (const path of ['C:/workspace/photo.png', 'file:///C:/workspace/photo.png', '/workspace/photo.png', 'images/photo.png']) {
    assert.equal(isInlineFileReference(localFileReference(path)), true, path);
  }
});

test('completed replies have local suggestions when the provider omits optional metadata', () => {
  const items = [{label:'Explain',prompt:'Explain the result in more detail.'},{label:'Next steps',prompt:'Suggest useful next steps.'}];
  const message = {id:'completed',role:'assistant',content:'The requested analysis is complete and the output is ready to inspect.'};
  assert.deepEqual(latestFollowupSuggestions([message] as any, items), {key:'completed',items});
  assert.deepEqual(latestFollowupSuggestions([{...message,content:'Done.\n\n```nexa-followups\n['}] as any, items), {key:'completed',items});
  assert.equal(latestFollowupSuggestions([{...message,artifacts:{phase:'commentary'}}] as any, items), null);
});

test('followups emitted as newline-delimited objects remain actionable', () => {
  const items = [{label:'项目体检',prompt:'检查 D:\\coder\\Nexa 的项目结构。'}, {label:'继续工作',prompt:'继续处理剩余问题。'}];
  const jsonl = items.map(item => JSON.stringify(item)).join('\n');
  const content = '检查已完成。\n\n```nexa-followups\n' + jsonl + '\n```';
  assert.deepEqual(parseFollowupSuggestions(jsonl), items);
  assert.deepEqual(latestFollowupSuggestions([{id:'final',role:'assistant',content,toolCalls:[],artifacts:null} as any]), {key:'final',items});
});

test('followups require a complete final assistant block and never activate quoted examples or later user turns', () => {
  const items = [{label:'Inspect',prompt:'Inspect the result'}, {label:'Extend',prompt:'Extend the example'}];
  const json = JSON.stringify(items);
  assert.deepEqual(parseFollowupSuggestions(json), items);
  const message = {id:'final',role:'assistant',content:'Done.\n\n```nexa-followups\n'+json+'\n```',toolCalls:[],artifacts:null} as any;
  assert.deepEqual(latestFollowupSuggestions([message]), {key:'final',items});
  for (const marker of ['nexa-followup', 'nexa_followups json', 'json nexa-followups']) {
    const block = '\`\`\`' + marker + '\n' + JSON.stringify({suggestions:items}, null, 2);
    assert.equal(stripFollowupBlocks('Done.\n' + block), 'Done.\n');
    assert.deepEqual(latestFollowupSuggestions([{...message, content:'Done.\n' + block + '\n\`\`\`'}]), {key:'final',items});
  }
  for (const content of [message.content.slice(0,-3), '````md\n'+message.content+'\n````', message.content+'\nMore prose']) {
    assert.equal(latestFollowupSuggestions([{...message,content}]), null);
  }
  assert.equal(latestFollowupSuggestions([message,{...message,id:'user',role:'user'}]), null);
  assert.equal(latestFollowupSuggestions([{...message,artifacts:{phase:'commentary'}}]), null);
  assert.deepEqual(parseFollowupSuggestions(JSON.stringify([items[0]])), [items[0]]);
  for (const fence of ['~~~', '````']) assert.deepEqual(latestFollowupSuggestions([{...message,content:`Done.\n\n${fence}NEXA-FOLLOWUPS\n${json}\n${fence}`}]), {key:'final',items});
  for (const input of ['{}',JSON.stringify([items[0],items[0]]),JSON.stringify([{label:'Run',prompt:'/compact'},items[1]])]) assert.deepEqual(parseFollowupSuggestions(input), []);
});

const sample = JSON.parse(readFileSync(new URL('../../../crates/core/assets/skills/visual-explanations/resources/examples.json', import.meta.url), 'utf8'));
test('chat UI shares the skill contract for all components and numeric data', () => {
  assert.deepEqual(parseChatUi(JSON.stringify(sample)).document, sample);
  const parsed = parseChatUi(JSON.stringify({version:1,title:'Plain <b>text</b>',blocks:[{type:'text',text:'<script>no()</script>'},{type:'table',columns:['A','B'],rows:[[null,0]]}]}));
  assert.ok(parsed.document);
});
test('chat UI streams only complete root blocks and does not repair final JSON', () => {
  const first = {type:'text',text:'Quoted } ], \\" still text 中文'};
  const prefix = '{"version":1,"title":"Live","blocks":[' + JSON.stringify(first) + ',{"type":"text","text":"par';
  assert.deepEqual(parseChatUi(prefix,true).document?.blocks,[first]);
  assert.equal(parseChatUi(prefix).document, undefined);
  assert.equal(parseChatUi(prefix,true).partial,true);
});
test('chat UI rejects malformed data instead of inventing components or executing properties', () => {
  for (const value of [
    {version:2,title:'x',blocks:[]}, {version:1,title:'x',blocks:[],onClick:'run_shell'},
    {version:1,title:'x',blocks:[{type:'iframe',src:'https://example.test'}]},
    {version:1,title:'x',blocks:[{type:'table',columns:['a'],rows:[['x','y']]}]},
    {version:1,title:'x',blocks:[{type:'bars',items:[{label:'x',value:'25'}]}]},
    {version:1,title:'x',blocks:[{type:'text',text:{html:'x'}}]},
    {version:1,title:'x',blocks:[{type:'constructor',text:'x'}]},
    {version:1,title:'x',blocks:[{type:'steps',items:[{title:'x',status:['complete']}]}]},
    {version:1,title:'x',blocks:[{type:'steps',items:[{title:'x',status:{value:'complete'}}]}]},
  ]) assert.ok(parseChatUi(JSON.stringify(value)).error, JSON.stringify(value));
  assert.ok(parseChatUi('{"version":1,"title":"x","blocks":[{"type":"bars","items":[{"label":"x","value":1e999}]}]}').error);
  assert.ok(parseChatUi('a'.repeat(MAX_UI_BYTES+1)).error);
  assert.ok(parseChatUi('汉'.repeat(MAX_UI_BYTES/2)).error);
});
test('chat UI limits nested work and reports a useful property path', () => {
  let block:any = {type:'text',text:'leaf'};
  for(let i=0;i<8;i++) block={type:'tabs',tabs:[{label:'nest',blocks:[block]}]};
  assert.match(parseChatUi(JSON.stringify({version:1,title:'nested',blocks:[block]})).error!,/nested UI/);
  assert.match(parseChatUi('{"version":1,"title":"x","blocks":[{"type":"text","text":42}]}').error!,/blocks\[0\]\.text/);
});

test('prose transforms preserve closed and streaming fences, spans and citation numbering', () => {
  const code = 'const label = `[source: dataset.csv]`;\nconst cite="[cite:literal]";';
  for(const fence of ['```','````','~~~']) for(const closed of [true,false]) {
    const original='[source: outside.csv]\n'+fence+'nexa-html\n'+code+(closed?'\n'+fence+'\n[cite:outside]':'');
    const transformed=transformMarkdownProse(original,value=>value.replaceAll('[source:', '[changed:').replaceAll('[cite:', '[ref:'));
    assert.ok(transformed.includes(code));assert.ok(transformed.startsWith('[changed:'));
    assert.ok(!markdownProse(original).includes('dataset.csv'));
    if(closed) assert.ok(transformed.endsWith('[ref:outside]'));
  }
  assert.equal(transformMarkdownProse('`[cite:literal]` [cite:outside]',value=>value.replaceAll('[cite:','[ref:')),'`[cite:literal]` [ref:outside]');
  const spoof='\0nexa-code-0\0\n```html\n[source:literal]\n```';
  assert.equal(transformMarkdownProse(spoof,value=>value),spoof);
});

test('code protection follows CommonMark containers, indentation and matched spans', () => {
  const rewrite = (value: string) => value.replaceAll('[cite:', '[ref:');
  for (const falseEnd of ['> ```', '- ```', '    ```', '\t```']) {
    const original = '```nexa-html\n<pre>\n' + falseEnd + '\n</pre>\n<p>[cite:literal]</p>\n```\n[cite:outside]';
    assert.equal(transformMarkdownProse(original, rewrite), original.replace('[cite:outside]', '[ref:outside]'));
    assert.equal(markdownProse(original).includes('[cite:literal]'), false);
  }
  for (const original of [
    '> ```html\n> [cite:literal]\n> ```\n\n[cite:outside]',
    '- ```html\n  [cite:literal]\n  ```\n\n[cite:outside]',
    '    [cite:literal]\n\n[cite:outside]',
    '``[cite:literal] ` inside`` [cite:outside]',
  ]) assert.equal(transformMarkdownProse(original, rewrite), original.replace('[cite:outside]', '[ref:outside]'));
  const unmatched = 'A literal ` character is supported. [cite:outside]';
  assert.equal(transformMarkdownProse(unmatched, rewrite), unmatched.replace('[cite:outside]', '[ref:outside]'));
  assert.equal(markdownProse(unmatched), unmatched);
});

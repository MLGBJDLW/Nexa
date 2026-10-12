import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { parseChatUi, MAX_UI_BYTES } from '../../../crates/core/assets/skills/visual-explanations/resources/chat-ui.ts';

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

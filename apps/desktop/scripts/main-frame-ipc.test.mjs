import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const source=readFileSync(new URL('../src-tauri/src/main_frame_ipc.js',import.meta.url),'utf8');
const script=source.replace('__INVOKE_KEY__','"fixture-key"').replace('__NEXA_OS__','"windows"');
function runtime({frame=false,preview=false,failFetch=false}={}){
  const requests=[];const messages=[];const callbacks=[];
  const window={__TAURI_INTERNALS__:{convertFileSrc:command=>'http://ipc.localhost/'+command,runCallback:(...args)=>callbacks.push(args)},ipc:{postMessage:value=>messages.push(JSON.parse(value))}};
  window.top=frame?{}:window;
  const context=vm.createContext({window,location:{protocol:'http:',hostname:preview?'nexa-ui.localhost':'tauri.localhost'},Headers,console:{warn(){}},
    fetch:async(url,options)=>{requests.push({url,options});if(failFetch)throw new Error('blocked');return{headers:new Headers({'Tauri-Response':'ok','Content-Type':'application/json'}),json:async()=>({ok:true})};}});
  vm.runInContext(script,context);
  return {context,window,requests,messages,callbacks};
}
const settle=()=>new Promise(resolve=>setImmediate(resolve));
test('IPC never installs a key-bearing transport in frames or preview-origin top-level pages',()=>{
  for(const options of [{frame:true},{preview:true},{frame:true,preview:true}]){
    const {window,requests,messages}=runtime(options);
    assert.equal(window.__TAURI_INTERNALS__.postMessage,undefined);
    assert.deepEqual(requests,[]);assert.deepEqual(messages,[]);
  }
});
test('main-frame IPC retains Tauri JSON, Map, byte and channel serialization',async()=>{
  const {context,requests,callbacks,window}=runtime();
  vm.runInContext(`window.__TAURI_INTERNALS__.postMessage({cmd:'fixture',callback:7,error:8,payload:{map:new Map([['x',1]]),bytes:new Uint8Array([2,3]),channel:{__TAURI_TO_IPC_KEY__(){return '__CHANNEL__:9'}}}})`,context);
  await settle();
  const request=requests[0];assert.equal(request.options.headers.get('Tauri-Invoke-Key'),'fixture-key');
  assert.equal(request.options.headers.get('Content-Type'),'application/json');
  assert.deepEqual(JSON.parse(request.options.body),{map:{x:1},bytes:[2,3],channel:'__CHANNEL__:9'});
  assert.deepEqual(callbacks,[[7,{ok:true}]]);
  assert.ok(!window.__TAURI_INTERNALS__.postMessage.toString().includes('fixture-key'));
  vm.runInContext(`window.__TAURI_INTERNALS__.postMessage({cmd:'binary',callback:1,error:2,payload:new Uint8Array([4,5])})`,context);
  await settle();assert.equal(requests[1].options.headers.get('Content-Type'),'application/octet-stream');
  assert.deepEqual(Array.from(requests[1].options.body),[4,5]);
});
test('main-frame IPC preserves the native postMessage fallback after CSP/protocol failure',async()=>{
  const {context,messages}=runtime({failFetch:true});
  vm.runInContext(`window.__TAURI_INTERNALS__.postMessage({cmd:'fixture',callback:7,error:8,payload:{value:42}})`,context);
  await settle();
  assert.deepEqual(messages,[{cmd:'fixture',callback:7,error:8,options:{customProtocolIpcBlocked:true},payload:{value:42},__TAURI_INVOKE_KEY__:'fixture-key'}]);
});
test('the adapted transport stays bound to the reviewed Tauri version',()=>{
  const cargo=readFileSync(new URL('../src-tauri/Cargo.toml',import.meta.url),'utf8');
  const version=cargo.match(/tauri = \{ version = "=([^"]+)"/)[1];
  assert.ok(source.includes(`Tauri ${version}`),'Review upstream IPC changes when upgrading Tauri');
});

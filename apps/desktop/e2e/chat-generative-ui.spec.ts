import { test, expect, type Page } from '@playwright/test';
import { createServer, type Server } from 'node:http';
import { readFileSync } from 'node:fs';
import { randomUUID } from 'node:crypto';

// Exercise the shipping policies in a real browser. Native/remote Rust tests
// separately assert these headers are actually attached to the served routes.
const policySource = readFileSync(new URL('../../../crates/remote/src/html_preview.rs', import.meta.url),'utf8');
const policy=(name:string)=>policySource.match(new RegExp(`pub const ${name}: &str = "([^"]+)"`))![1];
let server:Server; let origin:string; const previews=new Map<string,string>(); const released:string[]=[];
test.beforeAll(async()=>{
  server=createServer((request,response)=>{
    const [,route,id,document]=request.url!.split('/');
    if(route!=='preview'||!previews.has(id)){response.writeHead(404).end();return;}
    response.writeHead(200,{'Content-Type':'text/html; charset=utf-8','Content-Security-Policy':policy(document?'DOCUMENT_CSP':'HOST_CSP'),'Permissions-Policy':policy('PERMISSIONS_POLICY'),'Cache-Control':'no-store','Referrer-Policy':'no-referrer'});
    response.end(document?previews.get(id):`<!doctype html><style>html,body,iframe{margin:0;width:100%;height:100%;border:0}</style><iframe title="Generated content" sandbox="allow-scripts" src="/preview/${id}/document"></iframe>`);
  });
  await new Promise<void>(resolve=>server.listen(0,'127.0.0.1',resolve));
  origin=`http://127.0.0.1:${(server.address() as any).port}`;
});
test.afterAll(async()=>{await new Promise<void>(resolve=>server.close(()=>resolve()));});
test.beforeEach(async({page})=>{
  previews.clear();released.length=0;
  await page.addInitScript(()=>{localStorage.setItem('nexa-locale','en');});
  await page.exposeFunction('publishUiFixture',(html:string)=>{const id=randomUUID();previews.set(id,html);return {id,url:`${origin}/preview/${id}`};});
  await page.exposeFunction('releaseUiFixture',(id:string)=>{released.push(id);previews.delete(id);});
  await page.goto('/e2e/fixtures/generative-ui.html');
  await expect(page.getByTestId('generative-ui')).toBeVisible();
});
async function show(page:Page,code:string,language='nexa-html',streaming=false){
  await page.evaluate(({code,language,streaming})=>(window as any).setUiFixture('```'+language+'\n'+code+(streaming?'':'\n```'),streaming),{code,language,streaming});
}

test('generative UI renders native data, filters, sorts, and restores from message text',async({page},info)=>{
  await expect(page.getByRole('heading',{name:'A clearer weekly review'})).toBeVisible();
  await expect(page.locator('.nexa-ui-bar[data-negative=true]')).toHaveCount(1);
  await page.getByRole('tab',{name:'By team'}).focus(); await page.keyboard.press('ArrowRight');
  await expect(page.getByRole('tab',{name:'Details'})).toHaveAttribute('aria-selected','true');
  await page.getByRole('button',{name:'Completed',exact:true}).click();
  await expect(page.locator('tbody tr').first()).toContainText('Research');
  await page.getByRole('searchbox',{name:'Filter rows'}).fill('Engineering');
  await expect(page.locator('tbody tr')).toHaveCount(1);
  await page.getByRole('searchbox',{name:'Filter rows'}).fill('not-present');
  await expect(page.getByText('No matching rows')).toBeVisible();
  await page.setViewportSize({width:390,height:844});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth)).toBeLessThanOrEqual(390);
  await page.reload(); await expect(page.getByRole('heading',{name:'A clearer weekly review'})).toBeVisible();
  await page.getByTestId('generative-ui').screenshot({path:info.outputPath('generative-ui-mobile.png')});
  await page.setViewportSize({width:1100,height:850});
  await page.getByTestId('generative-ui').screenshot({path:info.outputPath('generative-ui-desktop.png')});
});

test('generative UI streams complete blocks and degrades malformed final output without losing the answer',async({page})=>{
  const first='{"version":1,"title":"Live view","blocks":[{"type":"text","text":"First fact"},{"type":"metrics","items":[';
  await show(page,first,'nexa-ui',true);
  await expect(page.getByRole('heading',{name:'Live view'})).toBeVisible();
  await expect(page.locator('.nexa-ui-text')).toHaveText('First fact');
  await show(page,first,'nexa-ui');
  await expect(page.getByRole('alert')).toContainText('Unable to render');
  await show(page,'{"version":1,"title":"Recovered","blocks":[]}','nexa-ui');
  await expect(page.getByRole('heading',{name:'Recovered'})).toBeVisible();
  await expect(page.getByRole('alert')).toHaveCount(0);
});

test('generative HTML streaming is inert and runs only on request under production parent CSP',async({page},info)=>{
  await page.evaluate(()=>{const meta=document.createElement('meta');meta.httpEquiv='Content-Security-Policy';meta.content="default-src 'self';script-src 'self';style-src 'self' 'unsafe-inline';frame-src http: https:;img-src 'self' data:";document.head.appendChild(meta);});
  const html='<style>body{font:16px system-ui;padding:20px;color:#263347}button{padding:10px 20px;background:#dbeafe;border:0;border-radius:8px}</style><h1>Interactive model</h1><p>Adjust the value locally.</p><button onclick="document.querySelector(\'output\').textContent=\'42\'">Calculate</button><output>21</output><script>document.body.dataset.executed="yes"</script>';
  await show(page,html,'nexa-html',true);
  const card=page.getByTestId('html-preview'); const staticFrame=card.locator('iframe').contentFrame();
  await expect(staticFrame.getByRole('heading',{name:'Interactive model'})).toBeVisible();
  await expect(staticFrame.locator('body')).not.toHaveAttribute('data-executed','yes');
  await expect(page.getByRole('button',{name:'Run scripts'})).toBeDisabled();
  expect(previews.size).toBe(0);
  await show(page,html); await page.getByRole('button',{name:'Run scripts'}).click();
  const inner=card.locator('iframe').contentFrame().locator('iframe').contentFrame();
  await expect(inner.locator('body')).toHaveAttribute('data-executed','yes');
  await inner.getByRole('button',{name:'Calculate'}).click();
  await expect(inner.locator('output')).toHaveText('42');
  await card.screenshot({path:info.outputPath('interactive-html.png')});
  await page.getByRole('button',{name:'Stop',exact:true}).click();
  await expect.poll(()=>previews.size).toBe(0);
  await expect(card.locator('iframe')).toHaveAttribute('sandbox','');
  await page.getByRole('button',{name:'Run scripts'}).click();await expect.poll(()=>previews.size).toBe(1);
  await page.evaluate(()=>(window as any).hideUiFixture());await expect.poll(()=>previews.size).toBe(0);
});

test('generative HTML sandbox blocks parent access, storage, network, and external self-navigation',async({page})=>{
  let escaped=0;await page.route('**/exfil**',async route=>{escaped++;await route.abort();});
  const target='https://example.invalid/exfil';
  const html=`<h1>Sandbox test</h1><output id="result"></output><button id="nav">Navigate</button><script>
    const result={};
    try{result.parent=!!parent.document}catch{result.parent=false}
    try{localStorage.setItem('escape','yes');result.storage=true}catch{result.storage=false}
    result.ipc=typeof window.__TAURI_INTERNALS__!=='undefined';
    fetch('${target}').then(()=>result.network=true).catch(()=>result.network=false).finally(()=>document.querySelector('output').textContent=JSON.stringify(result));
    new Image().src='${target}?image';
    document.getElementById('nav').onclick=()=>location.href='${target}?navigate';
    </script>`;
  await show(page,html);await page.getByRole('button',{name:'Run scripts'}).click();
  const inner=page.getByTestId('html-preview').locator('iframe').contentFrame().locator('iframe').contentFrame();
  await expect(inner.locator('output')).toHaveText('{"parent":false,"storage":false,"ipc":false,"network":false}');
  await inner.getByRole('button',{name:'Navigate'}).click();
  await page.waitForTimeout(300);
  expect(escaped).toBe(0); expect(page.url()).toContain('generative-ui.html');
});

test('generative HTML source edits revoke active execution and ordinary html remains opt-in',async({page})=>{
  await show(page,'<h1>Code example</h1>','html');
  await expect(page.getByTestId('html-preview')).toHaveCount(0);
  await page.getByRole('button',{name:'Preview',exact:true}).click();
  await expect(page.getByTestId('html-preview')).toBeVisible();
  await page.getByRole('button',{name:'Run scripts'}).click();await expect.poll(()=>previews.size).toBe(1);
  await show(page,'<h1>Changed example</h1>','html');
  await expect.poll(()=>previews.size).toBe(0);
  await expect(page.getByTestId('html-preview').locator('iframe')).toHaveAttribute('sandbox','');
});

test('generative HTML preserves citation-like literals and paths while streaming and replaying',async({page})=>{
  const html='<output>pending</output><script>const label = `[source: dataset.csv]`;document.querySelector("output").textContent=label+" [cite:literal] ./data/report.json";</script>';
  await show(page,html,'nexa-html',true);
  await page.getByText('View source',{exact:true}).click();
  await expect(page.getByTestId('html-preview').locator('pre code')).toHaveText(html);
  await show(page,html);await page.getByRole('button',{name:'Run scripts'}).click();
  const inner=page.getByTestId('html-preview').locator('iframe').contentFrame().locator('iframe').contentFrame();
  await expect(inner.locator('output')).toHaveText('[source: dataset.csv] [cite:literal] ./data/report.json');
  await page.reload();await expect(page.getByTestId('html-preview').locator('iframe')).toHaveAttribute('sandbox','');
  await page.getByRole('button',{name:'Run scripts'}).click();
  await expect(inner.locator('output')).toHaveText('[source: dataset.csv] [cite:literal] ./data/report.json');
});

import { expect, test } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale','en');
    const fixture = {
      config: {mode:'local',provider:'qdrant',endpoint:'',apiKey:'',collectionPrefix:'nexa',database:'default',account:'root'},
      status: {localVectors:3,uploadedVectors:0,pendingUploads:3,pendingDeletes:1,syncing:false,paused:false,lastError:null},
      tests:0,syncs:0,complete:null as null | (()=>void),
    };
    Object.assign(window,{vectorFixture:fixture,__TAURI_INTERNALS__:{invoke:async (command:string,args:any)=>{
      switch(command){
        case 'get_vector_store_config_cmd':return structuredClone(fixture.config);
        case 'get_vector_store_status_cmd':return structuredClone(fixture.status);
        case 'save_vector_store_config_cmd':fixture.config=structuredClone(args.config);return null;
        case 'test_vector_store_connection_cmd':fixture.tests+=1;return null;
        case 'sync_vector_store_cmd':fixture.syncs+=1;fixture.status.syncing=true;fixture.status.paused=false;await new Promise<void>(resolve=>{fixture.complete=resolve;});return {uploaded:3,deleted:1,busy:false};
        case 'cancel_vector_store_sync_cmd':fixture.status.syncing=false;fixture.status.paused=true;fixture.complete?.();return null;
        default:return null;
      }
    }}});
  });
  await page.goto('/e2e/fixtures/vector-store.html');
  await page.getByRole('button',{name:'Vector storage',exact:true}).click();
});

test('local stays default and all five cloud protocols are optional with isolated keys', async ({page},testInfo)=>{
  await expect(page.getByRole('combobox',{name:'Retrieval mode'})).toContainText('Local only');
  expect(await page.evaluate(()=>(window as any).vectorFixture.tests)).toBe(0);
  await page.getByRole('combobox',{name:'Retrieval mode'}).click();
  await page.getByRole('option',{name:'Local + cloud fusion',exact:true}).click();
  for(const name of ['Qdrant / Qdrant Cloud','Pinecone','Alibaba DashVector','Milvus / Zilliz Cloud','Tencent VectorDB']){
    await page.getByRole('combobox',{name:'Vector store provider'}).click();
    await page.getByRole('option',{name,exact:true}).click();
    await expect(page.getByLabel('Vector store API key',{exact:true})).toHaveValue('');
    await page.getByLabel('Vector store API key',{exact:true}).fill('fixture-secret');
  }
  await expect(page.getByLabel('Existing database',{exact:true})).toBeVisible();
  await expect(page.getByLabel('Account',{exact:true})).toBeVisible();
  await page.getByLabel('Cloud instance / index endpoint',{exact:true}).fill('https://vector.example');
  await page.getByRole('button',{name:'Test Connection',exact:true}).click();
  await expect(page.getByText('Connection successful')).toBeVisible();
  expect(await page.evaluate(()=>(window as any).vectorFixture.syncs)).toBe(0);
  await page.getByRole('button',{name:'Save Config',exact:true}).click();
  await expect(page.getByTestId('vector-store-sync-status')).toContainText('0/3');
  expect(await page.evaluate(()=>(window as any).vectorFixture.config.provider)).toBe('tencent');
  await page.setViewportSize({width:390,height:840});
  await expect.poll(()=>page.getByTestId('vector-store-settings').evaluate(e=>e.scrollWidth-e.clientWidth)).toBeLessThanOrEqual(1);
  await page.screenshot({path:testInfo.outputPath('cloud-vector-stores.png'),fullPage:true});
});

test('sync exposes pending deletes, can pause, and switching local preserves stored data',async({page})=>{
  await page.getByRole('combobox',{name:'Retrieval mode'}).click();
  await page.getByRole('option',{name:'Cloud with local fallback',exact:true}).click();
  await page.getByLabel('Cloud instance / index endpoint',{exact:true}).fill('http://localhost:6333');
  await page.getByRole('button',{name:'Save Config',exact:true}).click();
  await expect(page.getByTestId('vector-store-sync-status')).toContainText('1 deletions pending');
  await page.getByRole('button',{name:'Sync now',exact:true}).click();
  await page.getByRole('button',{name:'Pause synchronization',exact:true}).click();
  await expect(page.getByTestId('vector-store-sync-status')).toContainText('paused');
  await page.getByRole('combobox',{name:'Retrieval mode'}).click();
  await page.getByRole('option',{name:'Local only (default)',exact:true}).click();
  await page.getByRole('button',{name:'Save Config',exact:true}).click();
  await expect(page.getByTestId('vector-store-sync-status')).toHaveCount(0);
  expect(await page.evaluate(()=>(window as any).vectorFixture.status.localVectors)).toBe(3);
  expect(await page.evaluate(()=>(window as any).vectorFixture.config.mode)).toBe('local');
});

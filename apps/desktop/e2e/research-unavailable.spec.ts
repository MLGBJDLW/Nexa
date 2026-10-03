import { test, expect } from '@playwright/test';

test('research identifies unavailable pending documents and keeps review disabled after retry', async ({page}) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale','en');
    let stored:any=null;
    (window as any).__TAURI_INTERNALS__={invoke:async(command:string,args:any={})=>{
      if(command==='list_research_sets')return stored ? [stored.summary] : [];
      if(command==='create_research_set')stored={summary:{id:'missing-document',title:args.input.title,revision:1,updatedAt:'2026-10-03'},questions:args.input.questions,documents:[{reference:args.input.documents[0],title:'Evidence restored',path:'C:/notes/report.md',unavailable:true,cells:[{questionIndex:0,reviewState:'pending',stale:true,note:'',evidence:[]}]}]};
      return structuredClone(stored);
    }};
  });
  await page.goto('/e2e/fixtures/knowledge-evidence.html?mode=research');
  const workspace=page.getByTestId('research-workspace');
  await workspace.locator('summary').click();
  await workspace.getByRole('checkbox',{name:'Evidence restored Reports'}).check();
  await workspace.getByRole('textbox',{name:'Research name'}).fill('Deleted source');
  await workspace.getByRole('textbox',{name:'Questions (one per line, up to 6)'}).fill('What is the limit?');
  await workspace.getByRole('button',{name:'Save research set'}).click();
  const unavailable=workspace.getByText('Source unavailable or without searchable text. Select a current document in a new research set.',{exact:true});
  await expect(unavailable).toBeVisible();
  await expect(workspace.getByText('Pending search',{exact:true})).toHaveCount(0);
  await expect(workspace.getByRole('button',{name:'Review',exact:true})).toBeDisabled();
  await workspace.getByRole('button',{name:'Refresh / continue'}).click();
  await expect(unavailable).toBeVisible();
  await expect(workspace.getByRole('button',{name:'Review',exact:true})).toBeDisabled();
});

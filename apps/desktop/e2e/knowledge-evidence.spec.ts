import { expect, test } from '@playwright/test';

test('DOCX citations retain native paragraph identity when empty paragraphs are omitted', async ({page}) => {
  await page.addInitScript(() => localStorage.setItem('nexa-locale','en'));
  await page.goto('/e2e/fixtures/knowledge-evidence.html?mode=docx');
  const paragraphs=page.getByTestId('file-preview-structured-document').locator('article p');
  await expect(paragraphs).toHaveCount(2);
  await expect(paragraphs.nth(0)).toHaveAttribute('data-evidence-anchor','true');
  await expect(paragraphs.nth(1)).not.toHaveAttribute('data-evidence-anchor','true');
});

test('DOCX citations highlight the native data row and its copied header context', async ({page}) => {
  await page.addInitScript(() => localStorage.setItem('nexa-locale','en'));
  await page.goto('/e2e/fixtures/knowledge-evidence.html?mode=docx&table=1');
  await expect(page.locator('[data-docx-table="2"] [data-docx-row="2"]')).toHaveAttribute('data-evidence-anchor','true');
  await expect(page.locator('[data-docx-table="2"] [data-docx-row="1"]')).toHaveAttribute('data-evidence-context-anchor','true');
});

test('citations expose read failures, recover on retry and fit a small viewport', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('nexa-locale', 'en'));
  await page.setViewportSize({ width: 280, height: 240 });
  await page.goto('/e2e/fixtures/knowledge-evidence.html');
  const citation = page.getByRole('button', { name: 'Read citation' });
  await citation.click();
  await expect(page.getByRole('alert')).toContainText('Temporary evidence read failure');
  await page.getByRole('button', { name: 'Retry' }).click();
  const popup = page.getByRole('dialog');
  await expect(popup).toContainText('Evidence restored');
  const bounds = await popup.boundingBox();
  expect(bounds).not.toBeNull();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.y).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(280);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(240);
  await page.keyboard.press('Escape');
  await expect(popup).toHaveCount(0);
  await expect(citation).toBeFocused();
});

test('historical reader preserves original text, rejects late sections and opens current files without old anchors', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('nexa-locale', 'en'));
  await page.goto('/e2e/fixtures/knowledge-evidence.html?mode=reader');
  await page.getByRole('button', {name:'Open historical evidence'}).click();
  const reader = page.getByTestId('evidence-reader');
  await expect(reader.getByTestId('selected-evidence-block')).toContainText('Original limit: 500 yuan.');
  await reader.getByRole('button', {name:'Slow section'}).click();
  await expect.poll(() => page.evaluate(() => typeof (window as any).__resolveSlow)).toBe('function');
  await reader.getByRole('button', {name:'Fast section'}).click();
  await expect(reader.getByTestId('selected-evidence-block')).toContainText('Evidence section fast');
  await page.evaluate(() => (window as any).__resolveSlow());
  await expect(reader.getByTestId('selected-evidence-block')).toContainText('Evidence section fast');
  await reader.getByRole('button', {name:'More sections'}).click();
  await reader.getByRole('button', {name:'Appendix'}).click();
  await expect(reader.getByTestId('selected-evidence-block')).toContainText('Evidence section appendix');
  await page.screenshot({path:'.artifacts/knowledge-reader-final.png',fullPage:true});
  await reader.getByRole('button', {name:'Open current source'}).click();
  await expect(reader).toHaveCount(0);
  await expect(page.getByTestId('opened-position')).toContainText('current-file-without-historical-anchor');
});

test('evidence context errors remain recoverable', async ({ page }) => {
  await page.addInitScript(() => { localStorage.setItem('nexa-locale', 'en'); (window as any).__failEvidence=true; });
  await page.goto('/e2e/fixtures/knowledge-evidence.html?mode=reader');
  await page.getByRole('button', {name:'Open historical evidence'}).click();
  await expect(page.getByRole('alert')).toContainText('Context temporarily unavailable');
  await page.evaluate(() => { (window as any).__failEvidence=false; });
  await page.getByRole('button', {name:'Retry'}).click();
  await expect(page.getByTestId('selected-evidence-block')).toContainText('500 yuan');
});

test('multi-page extracted evidence explains its location limit and opens without a false PDF page', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('nexa-locale', 'en'));
  await page.goto('/e2e/fixtures/knowledge-evidence.html?mode=reader&extracted=1');
  await page.getByRole('button', {name:'Open historical evidence'}).click();
  const reader=page.getByTestId('evidence-reader');
  await expect(reader.getByText('Extracted text; an exact source location is unavailable.', {exact:true})).toBeVisible();
  await reader.getByRole('button', {name:'Open current source'}).click();
  await expect(page.getByTestId('opened-position')).toContainText('"kind":"extracted"');
  await expect(page.getByTestId('opened-position')).not.toContainText('"page":');
});


test('workbook evidence locates actual cell coordinates when the used range starts at B3', async ({page}) => {
  await page.addInitScript(() => localStorage.setItem('nexa-locale', 'en'));
  await page.goto('/e2e/fixtures/knowledge-evidence.html?mode=workbook');
  const selected=page.locator('[data-cell-address="C4"]');
  await expect(selected).toHaveAttribute('data-evidence-anchor','true');
  await expect(selected).toContainText('17');
  await expect(page.locator('[data-cell-address="B3"]')).toContainText('金额(元)');
  await expect(page.locator('[data-cell-address="B3"]')).toHaveAttribute('data-evidence-context-anchor','true');
  await expect(page.locator('[data-cell-address="C3"]')).toHaveAttribute('data-evidence-context-anchor','true');
  await expect(page.locator('[data-cell-address="A1"]')).toHaveCount(0);
  await page.screenshot({path:'.artifacts/knowledge-sheet-anchor.png',fullPage:true});
});

test('research saves a comparison, reviews cited evidence and restores stale state after remount', async ({page}) => {
  await page.addInitScript(() => {
    localStorage.setItem('nexa-locale','en');
    let stored:any=null;
    (window as any).__TAURI_INTERNALS__={invoke:async(command:string,args:any={})=>{
      if(command==='list_research_sets')return stored ? [stored.summary] : [];
      if(command==='create_research_set') {
        stored={summary:{id:'research-one',title:args.input.title,revision:1,updatedAt:'2026-10-03'},questions:args.input.questions,documents:[{reference:args.input.documents[0],title:'Evidence restored',path:'C:/notes/report.md',cells:[{questionIndex:0,reviewState:'pending',stale:false,note:'',evidence:[]}]}]};
      }
      if(command==='refresh_research_set') {
        stored.summary.revision++;
        const doc=stored.documents[0];
        const updated=Boolean((window as any).__fixtureChanged);
        doc.reference={...doc.reference,revision:updated?'revision-2026-10':'revision-2026-09',status:'current'};
        doc.cells[0]={...doc.cells[0],reviewState:'needs_review',stale:false,evidence:[{chunkId:'old',documentId:'fixture-doc',sourceId:'fixture-source',sourceName:'Reports',documentPath:doc.path,documentTitle:doc.title,content:updated?'Updated limit: 600 yuan.':'Original limit: 500 yuan.',score:0.8,headingPath:[],highlights:[],evidenceRef:doc.reference}]};
        (window as any).__fixtureChanged=false;
      }
      if(command==='review_research_cell') {
        if(args.input.expectedRevision!==stored.summary.revision)throw new Error('Concurrent change');
        stored.summary.revision++;stored.documents[0].cells[0].reviewState=args.input.reviewState;stored.documents[0].cells[0].note=args.input.note;
      }
      if(command==='get_research_set' && (window as any).__fixtureChanged)stored.documents[0].cells[0].stale=true;
      if(command==='delete_research_set'){stored=null;return null;}
      return structuredClone(stored);
    }};
  });
  await page.goto('/e2e/fixtures/knowledge-evidence.html?mode=research');
  const workspace=page.getByTestId('research-workspace');
  await workspace.locator('summary').click();
  await workspace.getByRole('checkbox',{name:'Evidence restored Reports'}).check();
  await workspace.getByRole('textbox',{name:'Research name'}).fill('Expense policy comparison');
  await workspace.getByRole('textbox',{name:'Questions (one per line, up to 6)'}).fill('What is the limit?');
  await workspace.getByRole('button',{name:'Save research set'}).click();
  await expect(workspace.getByText('Pending search',{exact:true})).toBeVisible();
  await workspace.getByRole('button',{name:'Refresh / continue'}).click();
  await expect(workspace.getByRole('button',{name:'Original limit: 500 yuan.'})).toBeVisible();
  await workspace.getByRole('button',{name:'Original limit: 500 yuan.'}).click();
  await expect(page.getByTestId('research-opened')).toContainText('revision-2026-09');
  await workspace.getByRole('button',{name:'Review',exact:true}).click();
  const dialog=page.getByRole('dialog');
  await dialog.getByRole('textbox',{name:'Your conclusion / notes'}).fill('Limit confirmed at 500 yuan.');
  await dialog.getByRole('combobox',{name:'Review'}).selectOption('supported');
  await dialog.getByRole('button',{name:'Save',exact:true}).click();
  await expect(workspace.getByText('Supported (reviewed)',{exact:true})).toBeVisible();
  await page.evaluate(()=>{(window as any).__fixtureChanged=true;});
  await page.getByRole('button',{name:'Toggle workspace'}).click();
  await page.getByRole('button',{name:'Toggle workspace'}).click();
  await workspace.locator('summary').click();
  await expect(workspace.getByText('Evidence changed — refresh required',{exact:true})).toBeVisible();
  await expect(workspace.getByRole('button',{name:'Review',exact:true})).toBeDisabled();
  await workspace.getByRole('button',{name:'Refresh / continue'}).click();
  await expect(workspace.getByRole('button',{name:'Updated limit: 600 yuan.'})).toBeVisible();
  await expect(workspace.getByText('Needs review',{exact:true})).toBeVisible();
  await page.screenshot({path:'.artifacts/knowledge-research-final.png',fullPage:true});
});

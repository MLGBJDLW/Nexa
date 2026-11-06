import { expect, test, type Locator, type Page } from '@playwright/test';

const fixture = '/e2e/fixtures/mobile-shell.html?kind=chat&projects';
const project = 'Field notes';
const root = 'Ungrouped chats';
const button = (scope: Page | Locator, name: string) => scope.getByRole('button', {name, exact:true});
test.use({viewport:{width:360, height:780}, reducedMotion:'reduce'});

async function openDrawer(page: Page) {
  await page.locator('.mobile-app').getByRole('button', { name: 'Conversation', exact: true }).click();
  return page.getByRole('dialog', { name: 'Conversation', exact: true });
}

async function createProject(page: Page, name: string) {
  await page.getByRole('button', { name: 'New Project', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'New Project', exact: true });
  await dialog.getByRole('textbox').fill(name);
  await dialog.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(dialog).not.toBeVisible();
}

test('dragging a chat into a Project persists on this phone and keeps paired hosts separate', async ({page}) => {
  await page.goto(fixture);
  const drawer = await openDrawer(page);
  await createProject(page, project);
  const folder = button(drawer, project);
  const chat = button(drawer, 'Previous work');
  await button(drawer, root).click();
  const grip = (await button(drawer, 'Move to Project: Previous work').boundingBox())!;
  const destination = (await folder.boundingBox())!;
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await page.mouse.down();
  await page.mouse.move(destination.x + destination.width / 2, destination.y + destination.height / 2, {steps:12});
  await page.mouse.up();
  await expect(chat).toHaveCount(0);
  await folder.click();
  await expect(chat).toBeVisible();
  await page.goto(fixture + '&host=another-computer');
  await openDrawer(page);
  await expect(button(drawer, 'New Project')).toBeVisible();
  await expect(folder).toHaveCount(0);
  await expect(chat).toBeVisible();
  await page.goto(fixture);
  await openDrawer(page);
  await expect(chat).toHaveCount(0);
  await folder.click();
  await expect(chat).toBeVisible();
  await button(drawer, 'Move to Project: Previous work').press('Enter');
  await button(page.getByRole('dialog', {name:'Move to Project',exact:true}), 'Remove from Project').click();
  await expect(chat).toHaveCount(0);
  await button(drawer, root).click();
  await expect(chat).toBeVisible();
});

test('touch drag files a conversation without opening it, and archive and restore retain its Project', async ({page}) => {
  await page.goto(fixture);
  const drawer = await openDrawer(page);
  await createProject(page, project);
  await button(drawer, root).click();
  const folder = button(drawer, project);
  const chat = button(drawer, 'Previous work');
  const from = (await chat.boundingBox())!;
  const to = (await folder.boundingBox())!;
  const session = await page.context().newCDPSession(page);
  const start = { x:from.x + from.width / 2, y:from.y + from.height / 2 };
  const end = { x:to.x + to.width / 2, y:to.y + to.height / 2 };
  await session.send('Input.dispatchTouchEvent', {type:'touchStart',touchPoints:[start]});
  await page.waitForTimeout(500); // Long press on the title, like a phone user.
  for (let step = 1; step <= 6; step++) {
    const x = start.x + (end.x - start.x) * step / 6, y = start.y + (end.y - start.y) * step / 6;
    await session.send('Input.dispatchTouchEvent', {type:'touchMove',touchPoints:[{x,y}]});
  }
  await session.send('Input.dispatchTouchEvent', {type:'touchEnd',touchPoints:[]});
  await expect(drawer).toBeVisible();
  await expect(chat).toHaveCount(0);
  await folder.click();
  await expect(chat).toBeVisible();
  await button(drawer, 'Archive: Previous work').click();
  await button(page.getByRole('dialog',{name:'Archive',exact:true}), 'Archive').click();
  await expect(chat).toHaveCount(0);
  await drawer.getByText('Archived conversations',{exact:true}).click();
  await button(drawer, 'Unarchive: Previous work').click();
  await expect(chat).toBeVisible();
  await button(drawer, 'More options').click();
  await button(page.getByRole('dialog', {name:project,exact:true}), 'Edit project').click();
  const editor = page.getByRole('dialog', {name:'Edit project',exact:true});
  await editor.getByRole('textbox').fill('Checked reports');
  await button(editor, 'Save').click();
  await expect(button(drawer, 'Checked reports')).toBeVisible();
  await button(drawer, 'More options').click();
  await button(page.getByRole('dialog', {name:'Checked reports',exact:true}), 'Delete Project').click();
  await button(page.getByRole('dialog', {name:'Delete Project',exact:true}), 'Delete').click();
  await expect(button(drawer, 'Checked reports')).toHaveCount(0);
  await expect(chat).toBeVisible();
});

test('opening a Project finds older chats and retrying a first send preserves the new chat and its Project', async ({page}) => {
  await page.goto(fixture + '&host=paired&paged&interrupt');
  let drawer = await openDrawer(page);
  await createProject(page, project);
  await button(drawer, root).click();
  await button(drawer, 'More conversations').click();
  await expect(button(drawer, 'Middle chat')).toBeVisible();
  await button(drawer, 'More conversations').click();
  await button(drawer, 'Move to Project: Earliest chat').click();
  await button(page.getByRole('dialog',{name:'Move to Project',exact:true}), project).click();
  await page.reload();
  drawer = await openDrawer(page);
  await button(drawer, project).click();
  await expect(button(drawer, 'Earliest chat')).toBeVisible();
  await expect(button(drawer, 'Middle chat')).toHaveCount(0);
  await button(drawer, 'New conversation').click();
  await expect(drawer).not.toBeVisible();
  const composer = page.getByRole('textbox', {name:'Message',exact:true});
  await composer.fill('Save this in the project');
  await expect(composer).toHaveValue('Save this in the project');
  await button(page, 'Send').click();
  await expect(page.getByRole('alert').filter({hasText:'Project move interrupted'})).toBeVisible();
  await expect(composer).toHaveValue('Save this in the project');
  const launches = () => page.evaluate(() => (window as any).__MOBILE_CONTINUITY__.actions
    .map((event: {method:string}) => event.method).filter((method:string) => method === 'chat.start' || method === 'chat.create'));
  expect(await launches()).toEqual(['chat.create']);
  await button(page, 'Send').click();
  await expect(button(page, 'Stop')).toBeVisible();
  expect(await launches()).toEqual(['chat.create','chat.start']);
  await openDrawer(page);
  await expect(button(drawer, 'New work')).toBeVisible();
  await button(drawer, root).click();
  await expect(button(drawer, 'New work')).toHaveCount(0);
});

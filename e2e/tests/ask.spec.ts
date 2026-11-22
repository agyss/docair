import { expect, test, type Page } from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import { fakeAi, fixture, generate, go, ready, select, serve, withFakeAi, type ServedCopy } from './helpers';

/** Ask in the page's chat and wait for the model's reply; a reply with a walkthrough writes the documentation again, and the page reloads with it. */
async function ask(page: Page, question: string) {
  const before = await page.getByTestId('msg-ai').count();
  const reloaded = page.waitForEvent('load', { timeout: 20_000 }).catch(() => null);
  await page.getByTestId('question').fill(question);
  await page.getByTestId('question').press('Enter');
  await expect(page.getByTestId('msg-ai')).toHaveCount(before + 1);
  if (await page.getByTestId('msg-ai').last().getByTestId('play').count()) {
    await reloaded;
    await ready(page);
  }
  return page.getByTestId('msg-ai').last();
}

/** Apply what waits: the files are written, and the page reloads with the documentation written again. */
async function apply(page: Page) {
  const reloaded = page.waitForEvent('load', { timeout: 20_000 });
  await page.getByTestId('msg-ai').last().getByTestId('apply').click();
  await reloaded;
  await ready(page);
  await expect(page.getByTestId('msg-ai').last().getByTestId('apply')).toContainText('Applied');
}

/** The graph `docair serve` wrote for the project. */
function servedGraph(s: ServedCopy): any {
  return JSON.parse(fs.readFileSync(path.join(s.folder, '.docair', 'docair.graph.json'), 'utf8'));
}

/***
 * Clarify with AI: the chat in the page `docair serve` serves; replies bring walkthroughs to play, view changes and edits, applied to the project's files and the documentation.
 *
 * @id      e2e.ask
 * @kind    test
 * @part-of e2e
 ***/
test.describe('clarify with AI', () => {
  test.skip(!withFakeAi, 'needs the fake model');
  let served: ServedCopy;
  test.beforeEach(async ({ page }) => {
    served = await serve('shop');
    await page.goto(`${served.url}/`);
    await ready(page);
    await page.getByTestId('ask-button').click();
    await expect(page.getByTestId('chat')).toBeVisible();
    await expect(page.getByTestId('model')).toHaveText('fake-model');
  });
  test.afterEach(async () => {
    await served?.stop();
  });

  /***
   * A question gets an answer and a walkthrough; the documentation is written again with it, and Play walks it.
   *
   * @part-of e2e.ask
   * @covers  docair-viewer.doc-page.chat "checks that a question gets a reply, kept across the reload"
   * @covers  docair-server.ask "checks that the server answers"
   * @covers  docair-server.project.answered "checks the walkthrough is recorded and the documentation written again"
   * @covers  docair-ai.ask.ask "checks the model's reply against the graph"
   * @covers  docair-viewer.page_ai.reply_html "checks the reply as the chat shows it"
   ***/
  test('a question becomes a walkthrough that plays on the chart', async ({ page }) => {
    const reply = await ask(page, 'How does checkout work?');
    await expect(reply).toContainText('Checkout goes from the cart');
    await expect(reply.getByTestId('play')).toContainText('3 steps');
    // The documentation is written again: the walkthrough is a file of the project, and a picture of the page.
    expect(fs.readdirSync(path.join(served.folder, 'docs', 'walkthroughs')).some((f) => f.endsWith('.json'))).toBe(true);
    await page.getByTestId('msg-ai').last().getByTestId('play').click();
    await expect(page).toHaveURL(/#slice\/walkthrough:/);
    const card = page.getByTestId('step-card');
    await expect(card).toContainText('step 1 of 3');
    await expect(card).toContainText('The shopper clicks Buy');
  });

  /***
   * A follow-up is sent with the earlier turns, so the model knows what it refers to.
   *
   * @part-of e2e.ask
   * @covers  docair-viewer.doc-page.chat "checks the follow-up carries the conversation"
   * @covers  docair-ai.ask.ask "checks earlier turns reach the model"
   ***/
  test('a follow-up carries the conversation', async ({ page }) => {
    await ask(page, 'Is checkout tested?');
    const reply = await ask(page, 'and then?');
    await expect(reply).toContainText('Turns so far: 1.');
  });

  /***
   * "I don't care about …" hides that section in the page at once, as the reader's own hiding; Undo shows it again, and docair.toml is never written.
   *
   * @part-of e2e.ask
   * @covers  docair-server.project.answered "checks a view change does not wait to be applied"
   * @covers  docair-viewer.doc-page.chat "checks the view change hides at once, and Undo"
   ***/
  test("I don't care about it hides it, and Undo shows it again", async ({ page }) => {
    const toml = fs.readFileSync(path.join(served.folder, 'docair.toml'), 'utf8');
    const reply = await ask(page, "I don't care about the infrastructure");
    await expect(reply.getByTestId('view-change')).toContainText('infra');
    await expect(page.getByTestId('hidden-chip')).toHaveText('infra host ×');
    await expect(reply.getByTestId('apply')).toHaveCount(0);
    await reply.getByTestId('undo').click();
    await expect(page.getByTestId('hidden-bar')).toBeHidden();
    await expect(reply).toContainText('undone');
    expect(fs.readFileSync(path.join(served.folder, 'docair.toml'), 'utf8')).toBe(toml);
  });

  /***
   * A correction becomes an edit of a doc block: the diff can be reviewed and downloaded, and applying it writes the project's file and the documentation again.
   *
   * @part-of e2e.ask
   * @covers  docair-server.project.apply "checks applying the edit writes the file"
   * @covers  docair-core.edit.patch "checks the files get new texts"
   * @covers  docair-viewer.page_ai.reply_html "checks the edits, the diff and the patch"
   ***/
  test('a correction becomes an edit, applied to the code and the page', async ({ page }) => {
    // A read line runs from the store to the part that reads it.
    const reads = (g: any) => g.edges.some((e: any) => e.kind === 'read' && e.from === 'orders-db' && e.to === 'orders.list');
    expect(reads(servedGraph(served))).toBe(false);
    const reply = await ask(page, 'Listing orders also reads the orders table; that is missing.');
    await expect(reply.getByTestId('edits')).toContainText('orders.list');
    await reply.getByTestId('review').click();
    await expect(reply.getByTestId('diff')).toContainText('+ * @reads   orders-db -> Order');
    await expect(reply.getByTestId('diff')).toContainText('+++ b/api/src/lib.rs');
    const [download] = await Promise.all([page.waitForEvent('download'), reply.getByTestId('download-patch').click()]);
    expect(download.suggestedFilename()).toBe('docair.patch');
    expect(fs.readFileSync(await download.path(), 'utf8')).toContain('diff --git a/api/src/lib.rs b/api/src/lib.rs');
    await apply(page);
    expect(fs.readFileSync(path.join(served.folder, 'api', 'src', 'lib.rs'), 'utf8')).toContain('@reads   orders-db -> Order');
    expect(reads(servedGraph(served))).toBe(true);
    await select(page, 'orders.list');
    await expect(page.locator('#details')).toContainText('orders-db');
    await expect(page.getByTestId('msg-ai')).toHaveCount(1);
  });

  /***
   * Word the lines: the model's words are listed in the reply; applied, they are written into the doc blocks, and the documentation's lines carry them.
   *
   * @part-of e2e.ask
   * @covers  docair-server.word "checks the model words the lines"
   * @covers  docair-ai.word.word "checks the model fills the lines"
   * @covers  docair-viewer.doc-page.chat "checks the words wait to be applied"
   ***/
  test('Word the lines lists the words and writes them into the code', async ({ page }) => {
    const covers = (g: any) => g.edges.filter((e: any) => e.kind === 'cover').map((e: any) => e.text ?? '');
    expect(covers(servedGraph(served))).not.toContain('checks Cart');
    await page.getByTestId('word-lines').click();
    const reply = page.getByTestId('msg-ai').last();
    await expect(reply.getByTestId('wording')).toContainText('Words for 3 lines');
    await expect(reply.getByTestId('wording')).toContainText('checks Cart');
    await reply.getByTestId('review').click();
    await expect(reply.getByTestId('diff')).toContainText('@covers  shop-ui.cart "checks Cart"');
    await apply(page);
    expect(covers(servedGraph(served))).toContain('checks Cart');
    // The picture counts test lines ("1 test"); the test's details say what it checks.
    await select(page, 'cart-spec.buying-stores-the-order');
    await expect(page.locator('#details')).toContainText('checks Cart');
  });

  /***
   * Explain the terms: the model's entries are listed in the reply; applied, they are written into the glossary, and the documentation explains them.
   *
   * @part-of e2e.ask
   * @covers  docair-server.glossary "checks the server asks for the entries"
   * @covers  docair-ai.glossary.explain "checks only terms of the docs are kept"
   * @covers  docair-viewer.doc-page.chat "checks the entries wait to be applied"
   ***/
  test('Explain the terms lists the entries and writes the glossary', async ({ page }) => {
    await select(page, 'orders-db');
    // The reader chooses how much the model reasons; the reply says how it went (docair-8l6.89).
    await page.getByTestId('glossary-reasoning').selectOption('medium');
    await page.getByTestId('explain-terms').click();
    const reply = page.getByTestId('msg-ai').last();
    await expect(reply.getByTestId('glossary-terms')).toContainText('2 terms explained');
    await expect(reply).toContainText('reasoning medium');
    // Tests share the fake model: some glossary request of this run asked for medium.
    const asked = (await (await page.request.get(`${fakeAi}/requests`)).json()).filter((r: any) => (r.messages[0]?.content || '').startsWith('You write the glossary'));
    expect(asked.some((r: any) => r.reasoning_effort === 'medium')).toBe(true);
    await expect(reply).toContainText('orders table');
    await expect(reply).toContainText('1 parts left out');
    await reply.getByTestId('review').click();
    await expect(reply.getByTestId('diff')).toContainText('+## orders table');
    await apply(page);
    expect(servedGraph(served).glossary.map((t: any) => t.term)).toEqual(['order', 'cart', 'orders table', 'cart page']);
    await expect(page.getByTestId('glossary-button')).toContainText('Glossary (4)');
  });

  /***
   * What is selected goes along with the question.
   *
   * @part-of e2e.ask
   * @covers  docair-viewer.doc-page.chat "checks the selection joins the question"
   ***/
  test('the selected part goes along as context', async ({ page }) => {
    await select(page, 'orders.create');
    await expect(page.getByTestId('context')).toContainText('create_order');
    // A question no other test asks: the fake model serves every test at once.
    await ask(page, 'How does checkout work for the selected part?');
    const all = await (await page.request.get(`${fakeAi}/requests`)).json();
    const question = all
      .map((r: any) => r.messages.filter((m: any) => m.role === 'user').at(-1).content as string)
      .filter((q: string) => q.includes('How does checkout work for the selected part?'))
      .at(-1);
    expect(question).toContain('orders.create');
  });

  /***
   * A question can point at several things at once: parts, data, terms and lines, as typed refs; one that names nothing is said to be left out.
   *
   * @part-of e2e.ask
   * @covers  docair-ai.ask_context.about "checks typed refs become words for the model"
   * @covers  docair-server.ask "checks the server takes a list as context"
   ***/
  test('a question points at parts, data, terms and lines', async ({ page }) => {
    const res = await page.request.post(`${served.url}/api/ask`, {
      data: { question: 'How do these fit together, pointed at?', context: ['part:orders.create', 'data:data.order', 'term:order', 'edges:0', 'part:ghost'] },
    });
    expect(res.status()).toBe(200);
    const answer = await res.json();
    expect(answer.reply.problems.join(' ')).toContain('part:ghost');
    const all = await (await page.request.get(`${fakeAi}/requests`)).json();
    const sent = all
      .map((r: any) => r.messages.filter((m: any) => m.role === 'user').at(-1).content as string)
      .filter((q: string) => q.includes('How do these fit together, pointed at?'))
      .at(-1);
    expect(sent).toContain('- part `orders.create`');
    expect(sent).toContain('- datum `data.order`');
    expect(sent).toContain('- term "order"');
    expect(sent).toContain('- line `');
  });

  /***
   * ✦ Ask about this, in a part's details, a line's details and a datum's page, puts each into the question as a chip; the question carries them as typed refs, and the model gets the facts of all three. Sent, the question keeps them as chips in its bubble, each opening its thing, across a reload, and a follow-up still tells the model what it pointed at.
   *
   * @part-of e2e.ask
   * @covers  docair-viewer.doc-page.chat "checks Ask about this puts things into the question, and the question keeps them"
   * @covers  docair-ai.ask.ask "checks a follow-up carries what an earlier question pointed at"
   * @covers  docair-viewer.page_panels.panels "checks a part's details offer Ask about this"
   ***/
  test('✦ Ask about this puts a part, a line and a datum into one question', async ({ page }) => {
    await select(page, 'orders.create');
    await page.locator('#details [data-test="ask-about"]').click();
    // A line's words in sight, free of the legend and of data chips: a click there picks the line.
    const at = await page.evaluate(() => {
      for (const w of document.querySelectorAll('#pic .words')) {
        if (w.querySelector('[data-data]')) continue;
        const b = w.querySelector('rect')!.getBoundingClientRect();
        const [x, y] = [b.left + b.width / 2, b.top + b.height / 2];
        if (document.elementFromPoint(x, y)?.closest('#pic .words') === w) return { x, y };
      }
      return null;
    });
    expect(at, 'a line to point at').toBeTruthy();
    await page.mouse.click(at!.x, at!.y);
    await expect(page.getByTestId('line-details')).toBeVisible();
    await page.locator('#details [data-test="ask-about"]').click();
    await page.getByTestId('data-button').click();
    await page.locator('#details [data-about="data.order"]').first().click();
    await expect(page.getByTestId('data-title')).toBeVisible();
    await page.locator('#details [data-test="ask-about"]').click();
    const refs = await page.locator('#chat [data-test="context"] .chip-x').evaluateAll((bs) => bs.map((b) => (b as HTMLElement).title));
    expect(refs).toHaveLength(3);
    expect(refs[0]).toBe('part:orders.create');
    expect(refs[1]).toMatch(/^edge:/);
    expect(refs[2]).toBe('data:data.order');
    await ask(page, 'How do a part, a line and a datum fit together, pointed at in the page?');
    const all = await (await page.request.get(`${fakeAi}/requests`)).json();
    const sent = all
      .map((r: any) => r.messages.filter((m: any) => m.role === 'user').at(-1).content as string)
      .filter((q: string) => q.includes('pointed at in the page?'))
      .at(-1);
    expect(sent).toContain('- part `orders.create`');
    expect(sent).toContain('- line `');
    expect(sent).toContain('- datum `data.order`');
    // The question keeps what it was about, as chips in its bubble; the chips above it go (docair-rgz).
    const bubble = page.getByTestId('msg-user').last();
    const asked = bubble.getByTestId('asked');
    await expect(asked).toHaveCount(3);
    await expect(asked.nth(0)).toContainText('create');
    await expect(asked.nth(1)).toContainText('→');
    // Only the selection's chip stays above the question: it follows the selection (decision docair-t2z).
    await expect(page.locator('#chat [data-test="context"] .chip-x')).toHaveText(['create_order ×']);
    // Each opens its thing: the line is selected, the part opens.
    await asked.nth(1).click();
    await expect(page.getByTestId('line-details')).toBeVisible();
    await asked.nth(0).click();
    await expect(page).toHaveURL(/#part\/orders\.create$/);
    // Kept with the conversation, across a reload.
    await page.reload();
    await expect(page.getByTestId('msg-user').last().getByTestId('asked')).toHaveCount(3);
    // A follow-up still tells the model what the earlier question pointed at.
    await ask(page, 'And which of them comes first, as a follow-up?');
    const later = (await (await page.request.get(`${fakeAi}/requests`)).json())
      .map((r: any) => r.messages.filter((m: any) => m.role === 'user').map((m: any) => m.content as string))
      .filter((qs: string[]) => qs.at(-1)!.includes('as a follow-up?'))
      .at(-1);
    const earlier = later.find((q: string) => q.includes('pointed at in the page?'));
    expect(earlier).toContain('(What the question points at:');
    expect(earlier).toContain('- part `orders.create`');
    expect(earlier).toContain('- datum `data.order`');
  });

  /***
   * New conversation starts over: the questions and answers go, also after a reload, and the next question goes to the model without the earlier ones.
   *
   * @part-of e2e.ask
   * @covers  docair-viewer.doc-page.chat "checks New conversation starts over"
   ***/
  test('New conversation starts over', async ({ page }) => {
    await ask(page, 'What does the cart do, before starting over?');
    await expect(page.getByTestId('msg-user')).toHaveCount(1);
    await page.getByTestId('new-chat').click();
    await expect(page.getByTestId('msg-user')).toHaveCount(0);
    await expect(page.getByTestId('msg-ai')).toHaveCount(0);
    await page.reload();
    await ready(page);
    await expect(page.getByTestId('msg-user')).toHaveCount(0);
    await ask(page, 'And a first question after starting over?');
    const sent = (await (await page.request.get(`${fakeAi}/requests`)).json())
      .filter((r: any) => (r.messages.at(-1)?.content || '').includes('after starting over?'))
      .at(-1);
    // The prompt, the graph and its answer, then the question: no earlier turn.
    expect(sent.messages.filter((m: any) => m.role === 'user')).toHaveLength(2);
  });
});

/***
 * The AI in an uploaded page: the server keeps the upload as a project, and the page in the web app asks it, applies edits to the server's copy and shows them.
 *
 * @id      e2e.ask-upload
 * @kind    test
 * @part-of e2e
 ***/
test.describe('clarify with AI in an uploaded page', () => {
  test.skip(!withFakeAi, 'needs the fake model');

  /***
   * A question in the uploaded page gets a walkthrough to play; a correction is applied to the server's copy, the frame reloads with it, and the conversation stays.
   *
   * @part-of e2e.ask-upload
   * @covers  docair-server.scan_upload "checks the upload is kept as a project"
   * @covers  docair-server.kept_upload "checks the page asks and applies on the kept upload"
   * @covers  docair-web.result-view "checks the frame shows the upload's page by its address"
   ***/
  test('an uploaded page asks the server that keeps it', async ({ page }) => {
    await generate(page, fixture('shop'));
    const f = page.frameLocator('[data-test="page"]');
    const frame = () => page.frames().find((x) => x.url().includes('/api/projects/'))!;
    expect(frame()).toBeTruthy();
    await f.getByTestId('ask-button').click();
    await expect(f.getByTestId('model')).toHaveText('fake-model');
    const ask = async (question: string) => {
      const before = await f.getByTestId('msg-ai').count();
      await f.getByTestId('question').fill(question);
      await f.getByTestId('question').press('Enter');
      await expect(f.getByTestId('msg-ai')).toHaveCount(before + 1);
      return f.getByTestId('msg-ai').last();
    };
    // A walkthrough writes the documentation again: the frame reloads with it.
    const written = page.waitForEvent('framenavigated', { predicate: (x) => x.url().includes('/api/projects/'), timeout: 20_000 });
    const walk = await ask('How does checkout work in the upload?');
    await expect(walk.getByTestId('play')).toContainText('3 steps');
    await written;
    await expect(f.locator('body[data-ready="true"]')).toBeAttached();
    await f.getByTestId('msg-ai').last().getByTestId('play').click();
    await expect(f.getByTestId('step-card')).toContainText('step 1 of 3');
    const fix = await ask('Listing orders also reads the orders table; that is missing.');
    await expect(fix.getByTestId('edits')).toContainText('orders.list');
    // The frame reloads from the server with the edit in; the conversation is kept.
    const reloaded = page.waitForEvent('framenavigated', { predicate: (x) => x.url().includes('/api/projects/'), timeout: 20_000 });
    await fix.getByTestId('apply').click();
    await reloaded;
    await expect(f.locator('body[data-ready="true"]')).toBeAttached();
    await expect(f.getByTestId('msg-ai').last().getByTestId('apply')).toContainText('Applied');
    await expect(f.getByTestId('msg-ai')).toHaveCount(2);
    await frame().evaluate(() => (location.hash = '#part/orders.list'));
    await expect(f.locator('#details .meta')).toContainText('orders.list');
    await expect(f.locator('#details')).toContainText('orders-db');
  });
});

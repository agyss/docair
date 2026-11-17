import { expect, test } from '@playwright/test';
import fs from 'node:fs';
import { fakeAi, fixture, generate, withFakeAi } from './helpers';

/***
 * Clarify with AI: a conversation in the web app whose replies play on the chart, change the view, and edit the code.
 *
 * @id      e2e.ask
 * @kind    test
 * @part-of e2e
 ***/
test.describe('clarify with AI', () => {
  test.skip(!withFakeAi, 'needs the fake model');

  test.beforeEach(async ({ page }) => {
    await generate(page, fixture('shop'));
    await expect(page.getByTestId('chat')).toBeVisible();
  });

  async function ask(page: import('@playwright/test').Page, question: string) {
    const before = await page.getByTestId('msg-ai').count();
    await page.getByTestId('question').fill(question);
    await page.getByTestId('question').press('Enter');
    await expect(page.getByTestId('msg-ai')).toHaveCount(before + 1);
    return page.getByTestId('msg-ai').last();
  }

  /***
   * A question gets an answer and a walkthrough, which plays on the chart at once.
   *
   * @part-of e2e.ask
   * @covers  docair-web.chat-panel "checks that a question gets a reply"
   * @covers  docair-web.conversation.ask "checks the question is sent"
   * @covers  docair-server.ask "checks that the server answers"
   * @covers  docair-ai.ask.ask "checks the model's reply against the graph"
   * @covers  docair-viewer.receive "checks the walkthrough plays on the chart"
   ***/
  test('a question becomes a walkthrough that plays on the chart', async ({ page }) => {
    const reply = await ask(page, 'How does checkout work?');
    await expect(reply).toContainText('Checkout goes from the cart');
    await expect(reply.getByTestId('play')).toContainText('3 steps');
    const card = page.frameLocator('[data-test="page"]').getByTestId('step-card');
    await expect(card).toContainText('step 1 of 3');
    await expect(card).toContainText('The shopper clicks Buy');
  });

  /***
   * A follow-up is sent with the earlier turns, so the model knows what it refers to.
   *
   * @part-of e2e.ask
   * @covers  docair-web.conversation.ask "checks the follow-up carries the conversation"
   * @covers  docair-ai.ask.ask "checks earlier turns reach the model"
   ***/
  test('a follow-up carries the conversation', async ({ page }) => {
    await ask(page, 'How does checkout work?');
    const reply = await ask(page, 'and then?');
    await expect(reply).toContainText('Turns so far: 1.');
  });

  /***
   * "I don't care about …" hides that section on the chart; Undo brings it back.
   *
   * @part-of e2e.ask
   * @covers  docair-web.result-view.ask "checks the hide request applies"
   * @covers  docair-viewer.sections "checks hiding and undoing a section"
   ***/
  test("I don't care about it hides it, and Undo shows it again", async ({ page }) => {
    const reply = await ask(page, "I don't care about the infrastructure");
    await expect(reply.getByTestId('view-change')).toContainText('Hid infra');
    const frame = page.frameLocator('[data-test="page"]');
    await expect(frame.getByTestId('hidden-chip')).toHaveText('infra ×');
    await reply.getByTestId('undo').click();
    await expect(frame.getByTestId('hidden-bar')).toBeHidden();
    await expect(reply).toContainText('undone');
  });

  /***
   * A correction becomes an edit of a doc block: the diff can be reviewed and downloaded, and applying it regenerates the page.
   *
   * @part-of e2e.ask
   * @covers  docair-web.result-view.apply "checks applying the edit regenerates the page"
   * @covers  docair-web.result-view.patch "checks the edits become a diff"
   * @covers  docair-server.patch "checks the server applies the edits"
   * @covers  docair-core.edit.patch "checks the files get new texts"
   ***/
  test('a correction becomes an edit, applied to the code and the page', async ({ page }) => {
    await expect(page.locator('.stats')).toContainText('8 edges');
    const reply = await ask(page, 'Listing orders also reads the orders table; that is missing.');
    await expect(reply.getByTestId('edits')).toContainText('orders.list');
    await reply.getByTestId('review').click();
    await expect(reply.getByTestId('diff')).toContainText('+ * @reads   orders-db -> Order');
    await expect(reply.getByTestId('diff')).toContainText('+++ b/api/src/lib.rs');
    const [download] = await Promise.all([page.waitForEvent('download'), reply.getByTestId('download-patch').click()]);
    expect(download.suggestedFilename()).toBe('docair.patch');
    expect(fs.readFileSync(await download.path(), 'utf8')).toContain('diff --git a/api/src/lib.rs b/api/src/lib.rs');
    await reply.getByTestId('apply').click();
    await expect(page.locator('.stats')).toContainText('9 edges');
    await expect(reply.getByTestId('apply')).toContainText('Applied');
    await expect(page.getByTestId('msg-ai')).toHaveCount(1);
  });

  /***
   * Word the lines: the model's words show on the chart at once, marked ✦, and applying them writes them into the doc blocks.
   *
   * @part-of e2e.ask
   * @covers  docair-web.result-view.word "checks the words show at once"
   * @covers  docair-server.word "checks the model words the lines"
   * @covers  docair-ai.word.word "checks the model fills the lines"
   * @covers  docair-viewer.wording "checks the proposals show at once"
   ***/
  test('Word the lines shows words at once and writes them into the code', async ({ page }) => {
    const frame = page.frame({ url: /about:srcdoc/ }) ?? page.frames()[1];
    const words = () =>
      frame.evaluate(() => {
        const D = (window as any).docair;
        return D.state.graph.edges.filter((e: any) => e.kind === 'cover').map((e: any) => D.edgeText(e));
      });
    expect(await words()).toEqual(['tests', 'tests', 'tests']);
    await page.getByTestId('word-lines').click();
    const reply = page.getByTestId('msg-ai').last();
    await expect(reply.getByTestId('wording')).toContainText('Words for 3 lines');
    await expect.poll(words).toContain('✦ checks Cart');
    await reply.getByTestId('review').click();
    await expect(reply.getByTestId('diff')).toContainText('@covers  shop-ui.cart "checks Cart"');
    await reply.getByTestId('apply').click();
    await expect(reply.getByTestId('apply')).toContainText('Applied');
    const fresh = () => (page.frame({ url: /about:srcdoc/ }) ?? page.frames()[1]).evaluate(() => {
      const D = (window as any).docair;
      return D?.state?.graph?.edges.filter((e: any) => e.kind === 'cover').map((e: any) => D.edgeText(e)) ?? [];
    }).catch(() => []);
    await expect.poll(fresh, { timeout: 20000 }).toContain('checks Cart');
  });

  /***
   * Explain the terms: the model's entries are underlined on the page at once, marked as proposed; applying them writes the glossary.
   *
   * @part-of e2e.ask
   * @covers  docair-web.result-view.glossary "checks proposed terms show at once"
   * @covers  docair-server.glossary "checks the server asks for the entries"
   * @covers  docair-ai.glossary.explain "checks only terms of the docs are kept"
   * @covers  docair-viewer.glossary-preview "checks the preview underlines the terms"
   ***/
  test('Explain the terms underlines them at once and writes the glossary', async ({ page }) => {
    const frame = () => page.frame({ url: /about:srcdoc/ }) ?? page.frames()[1];
    await frame().evaluate(() => (window as any).docair.select('orders-db', true));
    await page.getByTestId('explain-terms').click();
    const reply = page.getByTestId('msg-ai').last();
    await expect(reply.getByTestId('glossary-terms')).toContainText('2 terms explained');
    await expect(reply).toContainText('orders table');
    await expect(reply).toContainText('1 parts left out');
    const f = page.frameLocator('[data-test="page"]');
    await expect(f.locator('#details .term.preview[data-term="orders-table"]')).toBeVisible();
    await reply.getByTestId('review').click();
    await expect(reply.getByTestId('diff')).toContainText('+## orders table');
    await reply.getByTestId('apply').click();
    await expect(reply.getByTestId('apply')).toContainText('Applied');
    await expect
      .poll(() => frame().evaluate(() => (window as any).docair?.state?.graph?.glossary?.map((t: any) => t.term) ?? []).catch(() => []), { timeout: 20000 })
      .toEqual(['order', 'cart', 'orders table', 'cart page']);
  });

  /***
   * What is selected on the chart goes along with the question.
   *
   * @part-of e2e.ask
   * @covers  docair-web.chat-panel "checks the selection joins the question"
   * @covers  docair-viewer.show-details "checks the picked part goes along"
   ***/
  test('the selected part goes along as context', async ({ page }) => {
    const frame = page.frame({ url: /about:srcdoc/ }) ?? page.frames()[1];
    await frame.evaluate(() => (window as any).docair.select('orders.create', true));
    await expect(page.getByTestId('context')).toContainText('create_order');
    // A question no other test asks: the fake model serves every test at once.
    await ask(page, 'How does checkout work for the selected part?');
    const all = await (await page.request.get(`${fakeAi}/requests`)).json();
    const question = all
      .map((r: any) => r.messages.filter((m: any) => m.role === 'user').at(-1).content as string)
      .filter((q: string) => q.includes('How does checkout work for the selected part?'))
      .at(-1);
    expect(question).toContain('Selected on the chart: orders.create');
  });
});

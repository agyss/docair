import { expect, test } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { docair, drawn, fixture, openStatic, out } from './helpers';

/***
 * Code and data: parts are drawn where they run; data has its own list and can be followed along its way.
 *
 * @id      e2e.data
 * @kind    test
 * @part-of e2e
 ***/
test.describe('code and data', () => {
  /***
   * A part whose code a server delivers but which runs in the browser is drawn in the browser, and says where its code comes from.
   *
   * @part-of e2e.data
   * @covers  docair-core.build.build "checks @runs-in places the part where it runs"
   ***/
  test('the shop front end is drawn in the browser, where it runs', async ({ page }) => {
    await openStatic(page, 'shop');
    const node = await page.evaluate(() => (window as any).docair.state.byId.get('shop-ui'));
    expect(node.part_of).toBe('browser');
    expect(node.code_from).toBe('shop');
    await page.evaluate(() => (window as any).docair.select('shop-ui', true));
    await expect(page.locator('#details')).toContainText('runs in');
    await expect(page.locator('#details')).toContainText('code from');
  });

  /***
   * Data is not a box among the parts; the Data list says who makes it and how many lines carry it, and Follow walks its way.
   *
   * @part-of e2e.data
   * @covers  docair-viewer.data "checks the data list"
   * @covers  docair-viewer.data-tour "checks Follow walks the data's way"
   * @covers  docair-viewer.follow "checks the lines that carry it light up"
   ***/
  test('the order is data: made by the cart, followed along its way', async ({ page }) => {
    await openStatic(page, 'shop');
    await page.evaluate(() => (window as any).docair.showLayer('code'));
    expect(await drawn(page)).not.toContain('data.order');
    await page.getByTestId('data-button').click();
    const list = page.getByTestId('data-list');
    await expect(list).toContainText('order');
    await expect(list).toContainText('2 lines');
    await expect(list).toContainText('made by Cart');
    await list.getByTestId('follow').click();
    await expect(page.getByTestId('follow-bar')).toContainText('order');
    const card = page.getByTestId('step-card');
    await expect(card).toContainText('step 1 of 4');
    await expect(page.getByTestId('carried')).toHaveText('Carried by 2 lines, all in this walk.');
    await expect(card.locator('h3')).toHaveText('Cart makes ◇ order');
    await page.evaluate(() => (window as any).docair.goStep(1));
    await expect(card.locator('h3')).toHaveText('Cart calls create_order');
    await expect(card.locator('.hop-carry .data-ref').first()).toHaveText('CreateOrder');
    await page.evaluate(() => (window as any).docair.goStep(2));
    await expect(card.locator('h3')).toHaveText('create_order writes to orders-db');
    // the call carries the order there, and its answer brings it back
    await page.evaluate(() => (window as any).docair.goStep(3));
    await expect(card.locator('h3')).toHaveText('create_order answers Cart');
    await expect(card.locator('.hop.return')).toHaveCount(1);
    expect(await page.evaluate(() => (window as any).docair.state.cy.edges('.carry').length)).toBe(2);
    await page.getByTestId('unfollow').click();
    await expect(page.getByTestId('follow-bar')).toBeHidden();
  });

  /***
   * A datum a function makes goes back to its caller in the answer, and on from there; an answer the model
   * makes is followed back along the calls; a line the walk cannot reach from where the datum is made says so.
   *
   * @part-of e2e.data
   * @covers  docair-viewer.data-tour "checks data is followed back along answers, and on"
   * @covers  docair-viewer.messages "checks a step of a data walk is the answer that brings it back"
   * @covers  docair-viewer.data-cover "checks the card says what the walk covers"
   * @covers  docair-viewer.follow "checks the walk's own parts do not fade while it plays"
   ***/
  test('data that a function makes and returns is followed back to its caller, and on', async ({ page }) => {
    execFileSync(docair, ['scan', fixture('answer'), '--out', path.join(out, 'answer')]);
    await openStatic(page, 'answer');
    const walk = () =>
      page.evaluate(() => {
        const t = (window as any).docair.state.flow;
        return (t.base || t).steps.map((st: any) => `${st.title} | ${st.text}`);
      });
    await page.evaluate(() => (window as any).docair.followData('data.summary'));
    await expect
      .poll(walk)
      .toEqual([
        'summarize makes ◇ summary | Sums up the notes.',
        'summarize answers ask | brings ◇ summary back inside String',
        'ask calls model | asks the model · sends question + summary · gets answer back',
        'report calls mail | Not reached from where ◇ summary is made.\n\nmails the report · sends summary',
      ]);
    // the card says what the walk covers
    await expect(page.getByTestId('carried')).toContainText('Carried by 2 lines, all in this walk; 1 of them starts a stretch of its own');
    // at the code layer each step is one line; the answer goes back as an answer
    await page.evaluate(() => (window as any).docair.setLayer('code'));
    await page.evaluate(() => (window as any).docair.goStep(1));
    const card = page.getByTestId('step-card');
    await expect(card.locator('h3')).toHaveText('summarize answers ask');
    await expect(card.locator('.hop.return')).toHaveCount(1);
    // while the walk plays, the lines that carry it stay lit, and the walk's own parts do not fade
    expect(await page.evaluate(() => (window as any).docair.state.cy.edges('.carry').length)).toBe(2);
    expect(await page.evaluate(() => (window as any).docair.state.cy.$('.carry-dim').map((x: any) => x.id()))).not.toContain('app.summarize');
    await page.evaluate(() => (window as any).docair.followData('data.answer'));
    await expect.poll(walk).toEqual(['model makes ◇ answer | Writes the answer.', 'model answers ask | brings ◇ answer back', 'ask answers asker | brings ◇ answer back']);
    expect(await page.evaluate(() => (window as any).docair.state.cy.edges('.carry').length)).toBe(2);
  });
});

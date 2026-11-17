import { expect, test } from '@playwright/test';
import { drawn, openStatic } from './helpers';

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
    await expect(card).toContainText('step 1 of 3');
    await expect(card.locator('h3')).toHaveText('Cart makes order');
    await page.evaluate(() => (window as any).docair.goStep(1));
    await expect(card.locator('h3')).toHaveText('Cart calls create_order');
    await expect(card.locator('.hop-carry .data-ref').first()).toHaveText('CreateOrder');
    await page.evaluate(() => (window as any).docair.goStep(2));
    await expect(card.locator('h3')).toHaveText('create_order writes to orders-db');
    expect(await page.evaluate(() => (window as any).docair.state.cy.edges('.carry').length)).toBe(2);
    await page.getByTestId('unfollow').click();
    await expect(page.getByTestId('follow-bar')).toBeHidden();
  });
});

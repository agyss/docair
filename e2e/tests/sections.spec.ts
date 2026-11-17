import { expect, test } from '@playwright/test';
import { drawn, openStatic, shown } from './helpers';

/***
 * Sections: leave out what you don't care about, and bring it back.
 *
 * @id      e2e.sections
 * @kind    test
 * @part-of e2e
 ***/
test.describe('add and remove sections', () => {
  /***
   * The eye in the tree hides a section with everything inside it; its chip above the picture shows it again.
   *
   * @part-of e2e.sections
   * @covers  docair-viewer.sections "checks the eye hides a section"
   ***/
  test('the eye hides a section and its chip shows it again', async ({ page }) => {
    await openStatic(page, 'shop');
    await page.evaluate(() => (window as any).docair.showLayer('code'));
    expect(await drawn(page)).toContain('tf.cluster');
    const row = page.locator('#tree-list li[data-id="infra"] > .row');
    await row.hover();
    await row.getByTestId('eye').click();
    await expect.poll(() => drawn(page)).not.toContain('tf.cluster');
    expect(await drawn(page)).not.toContain('infra');
    const chip = page.getByTestId('hidden-chip');
    await expect(chip).toHaveText('infra ×');
    await chip.click();
    await expect(page.getByTestId('hidden-bar')).toBeHidden();
    await expect.poll(() => drawn(page)).toContain('tf.cluster');
  });

  /***
   * The details of a node offer to hide it.
   *
   * @part-of e2e.sections
   * @covers  docair-viewer.sections "checks the details offer to hide"
   * @covers  docair-viewer.show-details "checks the details show the node"
   ***/
  test('Hide this section in the details', async ({ page }) => {
    await openStatic(page, 'shop');
    await page.evaluate(() => (window as any).docair.select('shop-ui', true));
    await page.getByTestId('hide-section').click();
    await expect(page.getByTestId('hidden-chip')).toHaveText('shop-ui ×');
    await expect.poll(() => drawn(page)).not.toContain('shop-ui');
  });

  /***
   * In a hidden section, a part can be shown: the section comes back with only that part in it, its siblings are hidden instead, at every level down.
   *
   * @part-of e2e.sections
   * @covers  docair-viewer.sections "checks a part shows inside its hidden section"
   ***/
  test('showing a part inside a hidden section hides its siblings instead', async ({ page }) => {
    await openStatic(page, 'shop');
    await page.evaluate(() => (window as any).docair.showLayer('code'));
    const row = (id: string) => page.locator(`#tree-list li[data-id="${id}"] > .row`);
    await row('infra').hover();
    await row('infra').getByTestId('eye').click();
    await expect.poll(() => drawn(page)).not.toContain('orders.create');
    // One level down: the cluster comes back inside infra; its sibling, the Terraform resource, goes.
    // (The code layer dissolves the machine boxes: they are shown, but not drawn as boxes.)
    await row('cluster').hover();
    await row('cluster').getByTestId('eye').click();
    await expect.poll(() => drawn(page)).toContain('orders.create');
    const now = await drawn(page);
    expect(now).toEqual(expect.arrayContaining(['orders.create', 'orders-db']));
    expect(now).not.toContain('tf.cluster');
    expect(await shown(page, ['infra', 'cluster', 'tf.cluster'])).toEqual(['infra', 'cluster']);
    await expect(page.getByTestId('hidden-chip')).toHaveText(['kind_cluster.shop ×']);
    // Two levels down: hide infra again, then show the API inside the cluster.
    await page.evaluate(() => (window as any).docair.setHidden(['infra'], []));
    await row('cluster').locator('.caret').click();
    await row('orders-api').hover();
    await row('orders-api').getByTestId('eye').click();
    await expect.poll(() => drawn(page)).toContain('orders-api');
    const deeper = await drawn(page);
    expect(deeper).toEqual(expect.arrayContaining(['orders-api', 'orders.create']));
    expect(deeper).not.toContain('orders-db');
    expect(deeper).not.toContain('tf.cluster');
    expect(await shown(page, ['infra', 'cluster', 'orders-db'])).toEqual(['infra', 'cluster']);
    await page.getByTestId('show-all').click();
    await expect.poll(() => drawn(page)).toContain('orders-db');
  });

  /***
   * Show only this hides everything beside a part and beside each group around it; the hidden parts of one group are one chip.
   *
   * @part-of e2e.sections
   * @covers  docair-viewer.sections "checks Show only this and grouped chips"
   ***/
  test('Show only this keeps one part and the groups around it', async ({ page }) => {
    await openStatic(page, 'shop');
    await page.evaluate(() => (window as any).docair.showLayer('code'));
    await page.evaluate(() => (window as any).docair.select('orders-api', true));
    await page.getByTestId('show-only').click();
    await expect.poll(() => drawn(page)).not.toContain('shopper');
    const only = await drawn(page);
    expect(only).toEqual(expect.arrayContaining(['orders-api', 'orders.create', 'orders.list']));
    expect(await shown(page, ['infra', 'cluster'])).toEqual(['infra', 'cluster']);
    for (const gone of ['shopper', 'shop-ui', 'tf.cluster', 'orders-db']) expect(only).not.toContain(gone);
    await expect(page.getByTestId('hidden-group')).toHaveText('shop: 3 hidden ×');
    await page.getByTestId('hidden-group').click();
    await expect.poll(() => drawn(page)).toContain('shopper');
  });

  /***
   * `[view] show` in docair.toml shows a part inside a hidden section when the page opens.
   *
   * @part-of e2e.sections
   * @covers  docair-viewer.sections "checks [view] show opens a part in a hidden section"
   ***/
  test('docair.toml [view] show opens a part inside a hidden section', async ({ page }) => {
    await openStatic(page, 'shop-show');
    await page.evaluate(() => (window as any).docair.showLayer('code'));
    const now = await drawn(page);
    expect(now).toContain('orders.create');
    expect(now).not.toContain('tf.cluster');
    expect(await shown(page, ['infra', 'cluster', 'tf.cluster'])).toEqual(['infra', 'cluster']);
  });

  /***
   * `[view] hide` in docair.toml decides what the page leaves out when it opens.
   *
   * @part-of e2e.sections
   * @covers  docair-viewer.sections "checks the page opens partly hidden"
   * @covers  docair-core.config.load "checks the config supplies the hide list"
   ***/
  test('docair.toml [view] hide is the default', async ({ page }) => {
    await openStatic(page, 'shop-hidden');
    await expect(page.getByTestId('hidden-chip')).toHaveText('infra ×');
    expect(await drawn(page)).not.toContain('infra');
  });
});

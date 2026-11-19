import { expect, test, type Page } from '@playwright/test';
import { openStatic, pictured, select } from './helpers';

/** Which of these parts the tree says are shown: not hidden, and not inside a hidden one. */
function shown(page: Page, ids: string[]): Promise<string[]> {
  return page.evaluate(
    (list) =>
      list.filter((id) => {
        const li = document.querySelector(`#tree-list li[data-id="${CSS.escape(id)}"]`);
        return li && !li.classList.contains('hid') && !li.classList.contains('hid-in');
      }),
    ids,
  );
}

/***
 * Sections: leave out what you don't care about, and bring it back, in the picture docair drew.
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
   * @covers  docair-viewer.doc-page.hide "checks the eye hides a section and the chip shows it"
   ***/
  test('the eye hides a section and its chip shows it again', async ({ page }) => {
    await openStatic(page, 'shop', '#layer/code');
    expect(await pictured(page)).toContain('tf.cluster');
    const row = page.locator('#tree-list li[data-id="infra"] > .row');
    await row.hover();
    await row.getByTestId('eye').click();
    await expect.poll(() => pictured(page)).not.toContain('tf.cluster');
    await expect(page.locator('#pic g.part[data-id="tf.cluster"]')).toHaveClass(/\bhidden\b/);
    // The chip says what the box says in the picture.
    const chip = page.getByTestId('hidden-chip');
    await expect(chip).toHaveText('infra host ×');
    await chip.click();
    await expect(page.getByTestId('hidden-bar')).toBeHidden();
    await expect.poll(() => pictured(page)).toContain('tf.cluster');
  });

  /***
   * The details of a part offer to hide it.
   *
   * @part-of e2e.sections
   * @covers  docair-viewer.doc-page.hide "checks the details offer to hide"
   * @covers  docair-viewer.page_panels.panel-graph.details "checks the details show the part"
   ***/
  test('Hide this section in the details', async ({ page }) => {
    await openStatic(page, 'shop', '#layer/apps');
    await select(page, 'shop-ui');
    await page.getByTestId('hide-section').click();
    await expect(page.getByTestId('hidden-chip')).toHaveText('shop-ui ×');
    await expect.poll(() => pictured(page)).not.toContain('shop-ui');
    await expect(page.getByTestId('show-section')).toBeVisible();
  });

  /***
   * In a hidden section, a part can be shown: the section comes back with only that part in it, its siblings are hidden instead, at every level down.
   *
   * @part-of e2e.sections
   * @covers  docair-viewer.doc-page.hide "checks a part shows inside its hidden section"
   ***/
  test('showing a part inside a hidden section hides its siblings instead', async ({ page }) => {
    await openStatic(page, 'shop', '#layer/code');
    const row = (id: string) => page.locator(`#tree-list li[data-id="${id}"] > .row`);
    const eye = async (id: string) => {
      await row(id).hover();
      await row(id).getByTestId('eye').click();
    };
    await eye('infra');
    await expect.poll(() => pictured(page)).not.toContain('orders.create');
    // One level down: the cluster comes back inside infra; its sibling, the Terraform resource, goes.
    // (The code layer dissolves the machine boxes: they are shown, but not drawn as boxes.)
    await eye('cluster');
    await expect.poll(() => pictured(page)).toContain('orders.create');
    const now = await pictured(page);
    expect(now).toEqual(expect.arrayContaining(['orders.create', 'orders-db']));
    expect(now).not.toContain('tf.cluster');
    expect(await shown(page, ['infra', 'cluster', 'tf.cluster'])).toEqual(['infra', 'cluster']);
    await expect(page.getByTestId('hidden-chip')).toHaveText(['kind_cluster.shop ×']);
    // Two levels down: hide infra again, then show the API inside the cluster.
    await page.getByTestId('hidden-chip').click();
    await eye('infra');
    await expect.poll(() => pictured(page)).not.toContain('orders.create');
    if (await page.locator('#tree-list li[data-id="cluster"]').evaluate((li) => li.classList.contains('closed'))) await row('cluster').locator('.caret').click();
    await eye('orders-api');
    await expect.poll(() => pictured(page)).toContain('orders-api');
    const deeper = await pictured(page);
    expect(deeper).toEqual(expect.arrayContaining(['orders-api', 'orders.create']));
    expect(deeper).not.toContain('orders-db');
    expect(deeper).not.toContain('tf.cluster');
    expect(await shown(page, ['infra', 'cluster', 'orders-db'])).toEqual(['infra', 'cluster']);
    await page.getByTestId('show-all').click();
    await expect.poll(() => pictured(page)).toContain('orders-db');
  });

  /***
   * `[view] show` in docair.toml shows a part inside a hidden section when the page opens: the section comes back with only that part, its siblings hidden, as chips.
   *
   * @part-of e2e.sections
   * @covers  docair-viewer.doc-page.hide "checks [view] show opens a part in a hidden section"
   ***/
  test('docair.toml [view] show opens a part inside a hidden section', async ({ page }) => {
    await openStatic(page, 'shop-show', '#layer/code');
    const now = await pictured(page);
    expect(now).toEqual(expect.arrayContaining(['orders.create', 'orders-db']));
    expect(now).not.toContain('tf.cluster');
    await expect(page.locator('#pic g.part[data-id="tf.cluster"]')).toHaveClass(/\bhidden\b/);
    expect(await shown(page, ['infra', 'cluster', 'tf.cluster'])).toEqual(['infra', 'cluster']);
    await expect(page.getByTestId('hidden-chip')).toHaveText(['kind_cluster.shop ×']);
  });

  /***
   * `[view] hide` in docair.toml decides what the page starts with hidden; docair draws it all the same, so its chip shows it again (docair-702.13).
   *
   * @part-of e2e.sections
   * @covers  docair-viewer.doc-page.hide "checks the page opens partly hidden, and the chip shows it"
   * @covers  docair-core.config.load "checks the config supplies the hide list"
   ***/
  test('docair.toml [view] hide is the default', async ({ page }) => {
    await openStatic(page, 'shop-hidden', '#layer/code');
    await expect(page.getByTestId('hidden-chip')).toHaveText('infra host ×');
    expect(await pictured(page)).not.toContain('tf.cluster');
    await expect(page.locator('#pic g.part[data-id="tf.cluster"]')).toHaveClass(/\bhidden\b/);
    await page.getByTestId('hidden-chip').click();
    await expect.poll(() => pictured(page)).toContain('tf.cluster');
  });
});

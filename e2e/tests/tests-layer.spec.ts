import { expect, test } from '@playwright/test';
import { openStatic, pictured, select } from './helpers';

/***
 * Tests on the page: what is covered, did it pass, and a switch to leave tests out.
 *
 * @id      e2e.tests
 * @kind    test
 * @part-of e2e
 ***/
test.describe('tests in the docs', () => {
  /***
   * Test nodes carry the result of their last run, and the Tests switch shows each picture without them, as docair drew it, and back.
   *
   * @part-of e2e.tests
   * @covers  docair-viewer.doc_page.spots "checks each picture is drawn with tests and without"
   * @covers  docair-viewer.doc-page.show "checks the switch shows the other picture, keeping the view"
   * @covers  docair-core.test_results.attach "checks results land on test nodes"
   ***/
  test('tests show with their results and can be hidden', async ({ page }) => {
    await openStatic(page, 'shop', '#layer/code');
    const toggle = page.getByTestId('tests-toggle');
    await expect(toggle).toContainText('✓1');
    await expect(toggle).toContainText('✗1');
    await expect(toggle).toHaveClass(/\bon\b/);
    await expect(page.locator('#pic g.part[data-id="cart-spec.buying-stores-the-order"]')).toHaveClass(/t-passed/);
    await expect(page.locator('#pic g.part[data-id="cart-spec.the-total-adds-up"]')).toHaveClass(/t-failed/);
    await toggle.click();
    await expect(toggle).not.toHaveClass(/\bon\b/);
    await expect(toggle.locator('.tests-off')).toBeVisible();
    await expect.poll(() => pictured(page)).not.toContain('shop-tests');
    expect(await pictured(page)).not.toContain('cart-spec.buying-stores-the-order');
    expect(await pictured(page)).toContain('orders.create');
    await toggle.click();
    await expect.poll(() => pictured(page)).toContain('cart-spec.buying-stores-the-order');
  });

  /***
   * The details of a part list the tests that cover it, with their results.
   *
   * @part-of e2e.tests
   * @covers  docair-viewer.page_panels.panel-graph.details "checks the details list covering tests"
   ***/
  test('a part lists the tests that cover it', async ({ page }) => {
    await openStatic(page, 'shop');
    await select(page, 'orders.create');
    const list = page.getByTestId('tested-by');
    await expect(list).toContainText('buying stores the order');
    await expect(list).toContainText('passed');
    await select(page, 'shop-ui');
    await expect(page.getByTestId('tested-by')).toContainText('the total adds up');
    await expect(page.getByTestId('tested-by')).toContainText('failed');
  });
});

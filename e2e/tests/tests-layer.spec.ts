import { expect, test } from '@playwright/test';
import { drawn, openStatic } from './helpers';

/***
 * Tests on the page: what is covered, did it pass, and a switch to leave tests out.
 *
 * @id      e2e.tests
 * @kind    test
 * @part-of e2e
 ***/
test.describe('tests in the docs', () => {
  /***
   * Test nodes carry the result of their last run, and the Tests switch hides and shows them.
   *
   * @part-of e2e.tests
   * @covers  docair-viewer.tests-layer "checks the switch hides the tests"
   * @covers  docair-core.test_results.attach "checks results land on test nodes"
   ***/
  test('tests show with their results and can be hidden', async ({ page }) => {
    await openStatic(page, 'shop');
    await page.evaluate(() => (window as any).docair.showLayer('code'));
    const toggle = page.getByTestId('tests-toggle');
    await expect(toggle).toContainText('✓1');
    await expect(toggle).toContainText('✗1');
    const classes = await page.evaluate(() => {
      const cy = (window as any).docair.state.cy;
      return ['cart-spec.buying-stores-the-order', 'cart-spec.the-total-adds-up'].map((id) => cy.getElementById(id).classes().join(' '));
    });
    expect(classes[0]).toContain('t-passed');
    expect(classes[1]).toContain('t-failed');
    await toggle.click();
    await expect(toggle).toContainText('hidden');
    await expect.poll(() => drawn(page)).not.toContain('shop-tests');
    expect(await drawn(page)).not.toContain('cart-spec.buying-stores-the-order');
    await toggle.click();
    await expect.poll(() => drawn(page)).toContain('cart-spec.buying-stores-the-order');
  });

  /***
   * The details of a part list the tests that cover it, with their results.
   *
   * @part-of e2e.tests
   * @covers  docair-viewer.show-details "checks the details list covering tests"
   * @covers  docair-viewer.tests-layer "checks the tests show again"
   ***/
  test('a part lists the tests that cover it', async ({ page }) => {
    await openStatic(page, 'shop');
    await page.evaluate(() => (window as any).docair.select('orders.create', true));
    const list = page.getByTestId('tested-by');
    await expect(list).toContainText('buying stores the order');
    await expect(list).toContainText('passed');
    await page.evaluate(() => (window as any).docair.select('shop-ui', true));
    await expect(page.getByTestId('tested-by')).toContainText('the total adds up');
    await expect(page.getByTestId('tested-by')).toContainText('failed');
  });
});

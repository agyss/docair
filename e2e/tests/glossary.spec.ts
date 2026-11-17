import { expect, test } from '@playwright/test';
import { openStatic } from './helpers';

/***
 * The glossary: the words of the docs explain themselves where they are used.
 *
 * @id      e2e.glossary
 * @kind    test
 * @part-of e2e
 ***/
test.describe('what the words mean', () => {
  /***
   * A term in a description is underlined; hovering shows its short definition, Read more opens the linked doc inside the page at its heading.
   *
   * @part-of e2e.glossary
   * @covers  docair-viewer.terms "checks that terms explain themselves on hover"
   * @covers  docair-viewer.reader "checks the linked doc opens at its heading"
   * @covers  docair-core.glossary.library "checks the linked doc is in the page"
   ***/
  test('hovering a term explains it, Read more opens the doc', async ({ page }) => {
    await openStatic(page, 'shop');
    await page.evaluate(() => (window as any).docair.select('shop-ui.cart', true));
    const term = page.locator('#details .term[data-term="order"]');
    await expect(term).toHaveText('order');
    await term.hover();
    const pop = page.getByTestId('term-pop');
    await expect(pop).toContainText('What the shopper buys');
    await expect(pop).toContainText('see create_order');
    await pop.getByTestId('pop-more').click();
    const reader = page.getByTestId('reader');
    await expect(reader).toBeVisible();
    await expect(reader.locator('.flash')).toHaveText('How an order moves');
    await page.getByTestId('reader-close').click();
    await expect(reader).toBeHidden();
  });

  /***
   * Clicking a term opens its entry, with the parts that use it; a code span that names a part links to it; the Glossary lists every term.
   *
   * @part-of e2e.glossary
   * @covers  docair-viewer.glossary "checks the entry and the list"
   * @covers  docair-core.glossary.parse "checks terms, aliases and see"
   ***/
  test('a term opens its entry; the glossary lists them all', async ({ page }) => {
    await openStatic(page, 'shop');
    await page.evaluate(() => (window as any).docair.select('shop-ui.cart', true));
    await page.locator('#details .term[data-term="order"]').click();
    await expect(page.getByTestId('term-title')).toHaveText('¶order');
    await expect(page.locator('#details')).toContainText('also: orders');
    await expect(page.locator('#details')).toContainText('Used by');
    await page.locator('#details code.node-ref', { hasText: 'orders.create' }).click();
    await expect(page.locator('#details h2')).toContainText('create_order');
    await page.getByTestId('glossary-button').click();
    await expect(page.getByTestId('glossary').locator('dt')).toHaveText(['cart', 'order']);
    await page.getByTestId('glossary-search').fill('ord');
    await expect(page.getByTestId('glossary').locator('dt')).toHaveText(['order']);
  });
});

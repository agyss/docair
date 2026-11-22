import { expect, test } from '@playwright/test';
import { go, openStatic, pictured } from './helpers';

/***
 * A part reached by its address: it stays where it is when the picture on screen draws it, and otherwise comes in its home picture, its box's section, with its details.
 *
 * @id      e2e.layout
 * @kind    test
 * @part-of e2e
 ***/
test.describe('pick a part by its address', () => {
  /***
   * A function picked at Apps, where only its service's closed box is drawn, comes in that box's section; its siblings come too, the rest stays closed.
   *
   * @part-of e2e.layout
   * @covers  docair-viewer.doc_page.homes "checks a part's home is its box's section"
   * @covers  docair-viewer.doc-page.show "checks a part not drawn here goes to its home, selected"
   ***/
  test('a function picked at Apps comes in its box, with its details', async ({ page }) => {
    await openStatic(page, 'shop', '#layer/apps');
    expect(await pictured(page)).not.toContain('orders.create');
    await go(page, '#part/orders.create');
    await expect(page.locator('#pic g.part.sel[data-id="orders.create"]')).toBeAttached();
    await expect(page.locator('#details h2')).toContainText('create_order');
    // The box is open now, around what it holds; the cart's box stays closed.
    await expect(page.locator('#pic g.box[data-id="orders-api"]')).toBeAttached();
    expect(await pictured(page)).toEqual(expect.arrayContaining(['orders.create', 'orders.list']));
    expect(await pictured(page)).not.toContain('shop-ui.cart.total');
    await expect(page.locator('#crumbs')).toContainText('orders-api');
  });

  /***
   * A part the picture on screen draws stays where it is; one it does not draw goes to its box's section, from any layer.
   *
   * @part-of e2e.layout
   * @covers  docair-viewer.doc-page.show "checks a part drawn here stays, one not drawn goes to its home"
   ***/
  test('a part stays where it is drawn, else goes to its box', async ({ page }) => {
    await openStatic(page, 'shop', '#layer/code');
    await go(page, '#part/orders.create');
    await expect(page.locator('#pic g.part.sel[data-id="orders.create"]')).toBeAttached();
    await expect(page.locator('#pic svg.docair-pic[data-layer="code"]')).toBeAttached();
    expect(await pictured(page)).toContain('shop-ui');
    // Infrastructure draws no box around the cart: its function comes in the cart's section.
    await go(page, '#layer/infra');
    await expect(page.locator('#pic svg.docair-pic[data-layer="infra"]')).toBeAttached();
    await go(page, '#part/shop-ui.cart.total');
    await expect(page.locator('#pic g.part.sel[data-id="shop-ui.cart.total"]')).toBeAttached();
    await expect(page.locator('#pic g.box[data-id="shop-ui.cart"]')).toBeAttached();
  });
});

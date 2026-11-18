import { expect, test, type Page } from '@playwright/test';
import { openStatic } from './helpers';

/** Where the camera left a part: selected, readable, all of it on the canvas and in its middle, its halo, not dimmed. */
function framing(page: Page, id: string) {
  return page.evaluate((x) => {
    const d = (window as any).docair;
    const cy = d.state.cy;
    const n = cy.getElementById(d.rep(x));
    const b = n.renderedBoundingBox({ includeLabels: true });
    const [w, h] = [cy.width(), cy.height()];
    return {
      selected: n.selected(),
      // As a walk's step (SPEC §10.4): zoom × the scale the part is drawn at.
      readable: cy.zoom() * (n.data('sc') ?? 1) >= 0.8,
      onCanvas: b.x1 >= 0 && b.y1 >= 0 && b.x2 <= w && b.y2 <= h,
      inMiddle: Math.abs((b.x1 + b.x2) / 2 - w / 2) < 2 && Math.abs((b.y1 + b.y2) / 2 - h / 2) < 2,
      halo: Number(n.style('underlay-opacity')),
      opacity: Number(n.style('opacity')),
    };
  }, id);
}

async function showCode(page: Page): Promise<void> {
  await page.evaluate(async () => {
    await (window as any).docair.showLayer('code');
    await (window as any).docair.state.lastRender;
  });
}

/***
 * Picking a part: from a link it comes close and stands out; on the canvas it is selected where it is.
 *
 * @id      e2e.pick
 * @kind    test
 * @part-of e2e
 ***/
test.describe('pick a part', () => {
  /***
   * A part picked by a link in the details comes close: in the middle of the canvas, readable, with a halo a search hit does not have, and not dimmed by the search.
   *
   * @part-of e2e.pick
   * @covers  docair-viewer.show-picked "checks a part picked by a link comes close, readable, with a halo"
   ***/
  test('a part picked by a link comes close and stands out', async ({ page }) => {
    await openStatic(page, 'shop');
    await showCode(page);
    // The cart's details link to what it calls; the camera is far out, away from the API.
    await page.evaluate(() => {
      const d = (window as any).docair;
      d.select('shop-ui.cart');
      d.state.cy.viewport({ zoom: 0.2, pan: { x: -400, y: -400 } });
    });
    // A search is on: list_orders is a hit, create_order is not.
    await page.locator('#search').fill('list');
    await page.locator('#details a.link[title="orders.create"]').first().click();
    await expect
      .poll(() => framing(page, 'orders.create'))
      .toMatchObject({ selected: true, readable: true, onCanvas: true, inMiddle: true, opacity: 1 });
    // Once it has swelled twice, the halo stays; the search's hit has none.
    await expect.poll(async () => (await framing(page, 'orders.create')).halo).toBeGreaterThan(0.2);
    expect((await framing(page, 'orders.list')).halo).toBe(0);
  });

  /***
   * A part clicked on the canvas is selected where it is: the camera does not move.
   *
   * @part-of e2e.pick
   * @covers  docair-viewer.show-details "checks a click on the canvas selects without moving the camera"
   ***/
  test('a part clicked on the canvas is selected where it is', async ({ page }) => {
    await openStatic(page, 'shop');
    await showCode(page);
    const before = await page.evaluate(() => {
      const cy = (window as any).docair.state.cy;
      const r = cy.container().getBoundingClientRect();
      const p = cy.getElementById('orders.list').renderedPosition();
      return { x: r.left + p.x, y: r.top + p.y, zoom: cy.zoom(), pan: cy.pan() };
    });
    await page.mouse.click(before.x, before.y);
    await expect.poll(() => page.evaluate(() => (window as any).docair.state.selected)).toBe('orders.list');
    // Long enough for a camera move to show.
    await page.waitForTimeout(600);
    const after = await page.evaluate(() => ({ zoom: (window as any).docair.state.cy.zoom(), pan: (window as any).docair.state.cy.pan() }));
    expect(after.zoom).toBeCloseTo(before.zoom, 5);
    expect(after.pan.x).toBeCloseTo(before.pan.x, 1);
    expect(after.pan.y).toBeCloseTo(before.pan.y, 1);
  });
});

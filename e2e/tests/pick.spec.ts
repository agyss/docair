import { expect, test, type Page } from '@playwright/test';
import { go, openStatic, select } from './helpers';

/** Where the camera left a part: selected, readable, all of it on the canvas and in its middle, and not dimmed. */
function framing(page: Page, id: string) {
  return page.evaluate((x) => {
    const g = document.querySelector(`#pic g.part[data-id="${CSS.escape(x)}"], #pic g.box[data-id="${CSS.escape(x)}"]`) as SVGGElement | null;
    if (!g) return null;
    const pic = document.querySelector('#pic')!.getBoundingClientRect();
    const b = (g.querySelector('.sh') as SVGGraphicsElement).getBoundingClientRect();
    const text = g.querySelector('text') as SVGTextElement;
    // The words of the part on the screen, in px: readable from 0.8 of the page's 12 px.
    const px = text.getBoundingClientRect().height / 1.2;
    return {
      selected: g.classList.contains('sel'),
      readable: px >= 0.8 * 12 * 0.9,
      onCanvas: b.left >= pic.left && b.top >= pic.top && b.right <= pic.right && b.bottom <= pic.bottom,
      inMiddle: Math.abs((b.left + b.right) / 2 - (pic.left + pic.right) / 2) < 2 && Math.abs((b.top + b.bottom) / 2 - (pic.top + pic.bottom) / 2) < 2,
      dimmed: g.classList.contains('dim'),
    };
  }, id);
}

/***
 * Picking a part: from a link it comes close and stands out; in the picture it is selected where it is.
 *
 * @id      e2e.pick
 * @kind    test
 * @part-of e2e
 ***/
test.describe('pick a part', () => {
  /***
   * A part picked by a link in the details comes close: in the middle of the canvas, readable, marked, and not dimmed by the search.
   *
   * @part-of e2e.pick
   * @covers  docair-viewer.doc-page.select "checks a part picked by a link comes close, readable and marked"
   ***/
  test('a part picked by a link comes close and stands out', async ({ page }) => {
    await openStatic(page, 'shop', '#layer/code');
    await select(page, 'shop-ui.cart');
    // The camera far out, away from the API.
    const pic = await page.locator('#pic').boundingBox();
    await page.mouse.move(pic!.x + 20, pic!.y + 20);
    for (let k = 0; k < 6; k++) await page.mouse.wheel(0, 60);
    // A search is on: list_orders is a hit, create_order is not.
    await page.locator('#search').fill('list');
    await page.locator('#details a.link[title="orders.create"]').first().click();
    await expect(page).toHaveURL(/#part\/orders\.create$/);
    await expect.poll(() => framing(page, 'orders.create')).toMatchObject({ selected: true, readable: true, onCanvas: true, inMiddle: true, dimmed: false });
    await expect(page.locator('#details h2')).toContainText('create_order');
  });

  /***
   * A part clicked in the picture is selected where it is: the camera does not move, and its details show.
   *
   * @part-of e2e.pick
   * @covers  docair-viewer.doc-page.move "checks a click in the picture selects without moving the camera"
   * @covers  docair-viewer.page_panels.panel-graph.details "checks the details of the part clicked"
   ***/
  test('a part clicked on the canvas is selected where it is', async ({ page }) => {
    await openStatic(page, 'shop', '#layer/code');
    const before = await page.locator('#pic svg.docair-pic').getAttribute('viewBox');
    await page.locator('#pic g.part[data-id="orders.list"]').click();
    await expect(page.locator('#pic g.part.sel[data-id="orders.list"]')).toBeAttached();
    await expect(page.locator('#details h2')).toContainText('list_orders');
    // Long enough for a camera move to show.
    await page.waitForTimeout(400);
    expect(await page.locator('#pic svg.docair-pic').getAttribute('viewBox')).toBe(before);
    await go(page, '#layer/apps');
    await expect(page.locator('#pic svg.docair-pic[data-layer="apps"]')).toBeAttached();
  });
});

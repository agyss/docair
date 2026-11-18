import { expect, test, type Frame, type Page } from '@playwright/test';
import { drawn, fixture, generate, openStatic } from './helpers';

/** The viewer's layout worker: a top-level const of the page's script. */
declare const LAYOUT: { off: boolean; next: number };

/** Where the page stands after a pick: the layer, the boxes opened, whether the part is drawn as itself and selected. */
function picked(page: Page, id: string) {
  return page.evaluate((x) => {
    const d = (window as any).docair;
    const S = d.state;
    return { layer: S.layer, opened: [...S.opened], itself: d.drawnRep(x) === x, selected: S.cy.getElementById(x).selected(), drawn: S.shownRun === S.layoutRun };
  }, id);
}

/** Every drawn part's place, rounded: to see that a picture comes back the same. */
function places(page: Page) {
  return page.evaluate(() =>
    (window as any).docair.state.cy
      .nodes()
      .map((n: any) => `${n.id()} ${Math.round(n.position('x'))},${Math.round(n.position('y'))}`)
      .sort(),
  );
}

/** The layout worker as the page sees it: whether it runs, and how many layouts it was sent. */
function worker(page: Page | Frame) {
  return page.evaluate(() => ({ off: LAYOUT.off, jobs: LAYOUT.next }));
}

/***
 * Laying out and picking: a picked part comes with its box opened instead of a whole other layer, the details show at once, a layer laid out once comes back at once, and the layout runs beside the page.
 *
 * @id      e2e.layout
 * @kind    test
 * @part-of e2e
 ***/
test.describe('lay out and pick', () => {
  /***
   * A function picked at Apps, where only its service's closed box is drawn, comes with that box opened on the same layer; its details show before the picture is drawn again.
   *
   * @part-of e2e.layout
   * @covers  docair-viewer.pick-way "checks a pick opens the part's box on the layer the reader is on"
   * @covers  docair-viewer.show-details "checks the details show before the layout"
   ***/
  test('a function picked at Apps comes with its box opened, details first', async ({ page }) => {
    await openStatic(page, 'shop');
    expect(await page.evaluate(() => (window as any).docair.state.layer)).toBe('apps');
    expect(await drawn(page)).not.toContain('orders.create');
    // The details at once: in the same task as the click, while the picture is still to be drawn.
    const now = await page.evaluate(() => {
      const d = (window as any).docair;
      d.select('orders.create', true);
      return { title: document.querySelector('#details h2')?.textContent, waiting: d.state.shownRun !== d.state.layoutRun };
    });
    expect(now).toEqual({ title: expect.stringContaining('create_order'), waiting: true });
    await expect.poll(() => picked(page, 'orders.create')).toEqual({ layer: 'apps', opened: ['orders-api'], itself: true, selected: true, drawn: true });
    await expect(page.locator('#crumbs')).toContainText('orders-api');
    // The other function of the box shows too; the cart's box stays closed.
    expect(await drawn(page)).toEqual(expect.arrayContaining(['orders.create', 'orders.list', 'shop-ui']));
    expect(await drawn(page)).not.toContain('shop-ui.cart');
  });

  /***
   * A function picked on a layer that draws a box around it opens there; one picked on a layer that draws no box around it comes at Apps with its box opened, not with the whole Code layer; inside a closed box at Code, the box opens there.
   *
   * @part-of e2e.layout
   * @covers  docair-viewer.pick-way "checks a pick stays on the layer, goes to Apps with the box opened, and opens a closed box at Code"
   ***/
  test('a function the layer has no box for comes at Apps', async ({ page }) => {
    await openStatic(page, 'shop');
    // Infrastructure draws no box around the cart (the browser is not drawn there): its function comes at Apps.
    await page.evaluate(() => (window as any).docair.showLayer('infra'));
    await page.evaluate(() => (window as any).docair.select('shop-ui.cart.total', true));
    await expect.poll(() => picked(page, 'shop-ui.cart.total')).toEqual({ layer: 'apps', opened: ['shop-ui.cart'], itself: true, selected: true, drawn: true });
    await expect(page.locator('#toast')).toContainText('Apps · inside Cart');
    // It draws the cluster: the service in it opens there.
    await page.evaluate(() => (window as any).docair.showLayer('infra'));
    await page.evaluate(() => (window as any).docair.select('orders.create', true));
    await expect.poll(() => picked(page, 'orders.create')).toEqual({ layer: 'infra', opened: ['orders-api'], itself: true, selected: true, drawn: true });

    await page.evaluate(() => (window as any).docair.showLayer('code'));
    expect(await page.evaluate(() => (window as any).docair.drawnRep('shop-ui.cart.total'))).toBe('shop-ui.cart');
    await page.evaluate(() => (window as any).docair.select('shop-ui.cart.total', true));
    await expect.poll(() => picked(page, 'shop-ui.cart.total')).toEqual({ layer: 'code', opened: ['shop-ui.cart'], itself: true, selected: true, drawn: true });
  });

  /***
   * A layer laid out once comes back the same without a new layout; a changed picture is laid out again. The layouts run in the worker.
   *
   * @part-of e2e.layout
   * @covers  docair-viewer.layout "checks a kept layout comes back the same, and a changed picture is laid out again"
   * @covers  docair-viewer.layout-worker "checks the layouts run in the worker"
   ***/
  test('a layer comes back as it was laid out, without a new layout', async ({ page }) => {
    await openStatic(page, 'shop');
    await page.evaluate(() => (window as any).docair.showLayer('code'));
    const first = await places(page);
    const sent = await worker(page);
    expect(sent.off).toBe(false);
    expect(sent.jobs).toBeGreaterThan(0);
    await page.evaluate(() => (window as any).docair.showLayer('apps'));
    await page.evaluate(() => (window as any).docair.showLayer('code'));
    expect(await places(page)).toEqual(first);
    expect((await worker(page)).jobs).toBe(sent.jobs);
    // Tests switched off: another picture, laid out again.
    await page.evaluate(() => (window as any).docair.setTests(false));
    await expect.poll(async () => (await worker(page)).jobs).toBe(sent.jobs + 1);
  });

  /***
   * In the web app the page runs in a sandboxed frame; its layouts run in the worker there too.
   *
   * @part-of e2e.layout
   * @covers  docair-viewer.layout-worker "checks the worker starts inside the web app's sandboxed frame"
   ***/
  test('the worker lays out inside the web app too', async ({ page }) => {
    await generate(page, fixture('shop'));
    const frame = page.frame({ url: /about:srcdoc/ }) ?? page.frames()[1];
    const w = await worker(frame);
    expect(w.off).toBe(false);
    expect(w.jobs).toBeGreaterThan(0);
  });
});

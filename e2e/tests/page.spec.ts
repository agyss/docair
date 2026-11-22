import { expect, test } from '@playwright/test';
import { openStatic, pictured, pictureLines } from './helpers';

/** The bounding box of each part and box the picture draws, by id, in the picture's own units. */
function boxesOf(page: import('@playwright/test').Page): Promise<Record<string, { x: number; y: number; w: number; h: number; part: boolean }>> {
  return page.evaluate(() => {
    const out: Record<string, any> = {};
    document.querySelectorAll('#pic svg.docair-pic g.part, #pic svg.docair-pic g.box').forEach((g) => {
      const shape = g.querySelector('.sh') as SVGGraphicsElement | null;
      if (!shape) return;
      const b = shape.getBBox();
      out[(g as HTMLElement).dataset.id as string] = { x: b.x, y: b.y, w: b.width, h: b.height, part: g.classList.contains('part') };
    });
    return out;
  });
}

/** Each line's points (the corners between its straight pieces) and the tip of its arrow. */
function waysOf(page: import('@playwright/test').Page): Promise<{ id: string; from: string; to: string; points: [number, number][]; tip: [number, number] | null }[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll('#pic svg.docair-pic g.line')].map((g) => {
      const d = (g as HTMLElement).dataset;
      const nums = (s: string) => (s.match(/-?\d+(\.\d+)?/g) || []).map(Number);
      // A way is M x,y then L x,y …; a Q x1,y1 x2,y2 rounds a corner: its control point is the corner itself.
      const points: [number, number][] = [];
      const path = g.querySelector('path.ln')?.getAttribute('d') || '';
      for (const [, cmd, args] of path.matchAll(/([MLQ])([^MLQ]*)/g)) {
        const n = nums(args);
        if (cmd === 'Q') points.push([n[0], n[1]]);
        else points.push([n[0], n[1]]);
      }
      const ah = nums(g.querySelector('path.ah')?.getAttribute('d') || '');
      return { id: d.id as string, from: d.from as string, to: d.to as string, points, tip: ah.length >= 2 ? [ah[0], ah[1]] : null };
    }),
  );
}

const onBorder = (p: [number, number], b: { x: number; y: number; w: number; h: number }, tol = 1.5) => {
  const inX = p[0] >= b.x - tol && p[0] <= b.x + b.w + tol;
  const inY = p[1] >= b.y - tol && p[1] <= b.y + b.h + tol;
  const atX = Math.abs(p[0] - b.x) <= tol || Math.abs(p[0] - (b.x + b.w)) <= tol;
  const atY = Math.abs(p[1] - b.y) <= tol || Math.abs(p[1] - (b.y + b.h)) <= tol;
  return (inX && atY) || (inY && atX);
};

/***
 * The page the CLI writes: every picture laid out and drawn by docair itself, reached by its address, the layers as links, a box opened into its section (docs/SLICES.md §4).
 *
 * @id      e2e.page
 * @kind    test
 * @part-of e2e
 ***/
test.describe('the page the CLI writes', () => {
  /***
   * Each layer is a picture of what its question is about, and its link says it is the one shown.
   *
   * @part-of e2e.page
   * @covers  docair-core.layer_view.picture "checks each layer draws its own parts"
   * @covers  docair-viewer.doc-page.show "checks a layer's link shows its picture"
   ***/
  test('each layer is a picture of what its question is about', async ({ page }) => {
    await openStatic(page, 'shop');
    await expect(page.locator('#layers a.seg')).toHaveText(['Overview', 'Infrastructure', 'Apps', 'Code']);
    const show = async (key: string) => {
      await page.locator(`#layers a.seg[data-layer="${key}"]`).click();
      await expect(page).toHaveURL(new RegExp(`#layer/${key}$`));
      await expect(page.locator(`#pic svg.docair-pic[data-layer="${key}"]`)).toBeAttached();
      await expect(page.locator('#layers a.seg.on')).toHaveAttribute('data-layer', key);
      return pictured(page);
    };
    const top = await show('overview');
    expect(top).toContain('infra');
    expect(top).not.toContain('orders.create');
    const infra = await show('infra');
    expect(infra).toEqual(expect.arrayContaining(['infra', 'tf.cluster', 'cluster']));
    expect(infra).not.toContain('shop-ui');
    const apps = await show('apps');
    expect(apps).toEqual(expect.arrayContaining(['shopper', 'shop-ui', 'orders-api', 'orders-db']));
    // The machine and the cluster around the API are dissolved: the apps stand side by side.
    expect(apps).not.toContain('cluster');
    expect(apps).not.toContain('orders.create');
    expect(await show('code')).toContain('orders.create');
  });

  /***
   * The front end is drawn in the browser, where it runs, though its code comes from a container.
   *
   * @part-of e2e.page
   * @covers  docair-layout.lay_out "checks a part is placed inside its box"
   ***/
  test('the shop front end is drawn inside the browser', async ({ page }) => {
    await openStatic(page, 'shop', '#layer/apps');
    const at = await boxesOf(page);
    const ui = at['shop-ui'];
    const browser = at['browser'];
    expect(ui && browser).toBeTruthy();
    expect(ui.x >= browser.x && ui.y >= browser.y && ui.x + ui.w <= browser.x + browser.w && ui.y + ui.h <= browser.y + browser.h).toBe(true);
  });

  /***
   * A request through the network is one line from the shopper to the API at the apps layer, and its words say what it passes; the network layer shows the way in.
   *
   * @part-of e2e.page
   * @covers  docair-core.layer_view.picture "checks a line through left-out parts says what it passes"
   ***/
  test('a request through the network is one line between the apps', async ({ page }) => {
    await openStatic(page, 'platform', '#layer/network');
    const net = await pictured(page);
    expect(net).toEqual(expect.arrayContaining(['shopper', 'k8s.ingress.shop', 'k8s.svc.api', 'k8s.deploy.api']));
    expect(net).not.toContain('terraform');
    await page.locator('#layers a.seg[data-layer="apps"]').click();
    await expect(page.locator('#pic svg.docair-pic[data-layer="apps"]')).toBeAttached();
    const lines = await pictureLines(page);
    const toApi = lines.filter((l) => l.from === 'shopper' && l.to === 'api');
    expect(toApi).toHaveLength(1);
    expect(toApi[0].words).toContain('GET /api/items');
    expect(toApi[0].words).toContain('via Ingress shop');
    expect(lines.filter((l) => l.from === 'shopper' && l.to === 'web')).toEqual([]);
    expect(await pictured(page)).not.toContain('k8s.ingress.shop');
  });

  /***
   * Every line of every layer has words: what it carries, or what it does.
   *
   * @part-of e2e.page
   * @covers  docair-viewer.svg.draw "checks every line is drawn with its words"
   ***/
  test('every line has words', async ({ page }) => {
    for (const [name, layers] of [
      ['shop', ['overview', 'infra', 'apps', 'code']],
      ['platform', ['overview', 'infra', 'network', 'k8s', 'apps']],
    ] as const) {
      for (const key of layers) {
        await openStatic(page, name, `#layer/${key}`);
        await expect(page.locator(`#pic svg.docair-pic[data-layer="${key}"]`)).toBeAttached();
        const silent = (await pictureLines(page)).filter((l) => !l.words.trim());
        expect(silent, `${name} ${key}`).toEqual([]);
      }
    }
  });

  /***
   * Every line runs at right angles, from the border of the part it leaves to the border of the part its arrow points at, and through no other part.
   *
   * @part-of e2e.page
   * @covers  docair-layout.route "checks the lines run at right angles, end on their parts and pass no part"
   ***/
  test('every line runs at right angles, ends on its parts and passes no part', async ({ page }) => {
    for (const [name, layers] of [
      ['shop', ['overview', 'infra', 'apps', 'code']],
      ['platform', ['overview', 'infra', 'network', 'k8s', 'apps']],
    ] as const) {
      for (const key of layers) {
        await openStatic(page, name, `#layer/${key}`);
        await expect(page.locator(`#pic svg.docair-pic[data-layer="${key}"]`)).toBeAttached();
        const at = await boxesOf(page);
        for (const w of await waysOf(page)) {
          const where = `${name} ${key}: ${w.from} → ${w.to}`;
          expect(w.points.length, where).toBeGreaterThanOrEqual(2);
          for (let k = 1; k < w.points.length; k++) {
            const [a, b] = [w.points[k - 1], w.points[k]];
            const straight = Math.abs(a[0] - b[0]) < 0.5 || Math.abs(a[1] - b[1]) < 0.5;
            expect(straight, `${where}: ${a} → ${b}`).toBe(true);
          }
          expect(onBorder(w.points[0], at[w.from]), `${where} starts on ${w.from}`).toBe(true);
          if (w.tip) expect(onBorder(w.tip, at[w.to]), `${where} ends on ${w.to}`).toBe(true);
          // Through no part: a long straight piece keeps out of every part (corners and ends may touch borders).
          for (const [id, b] of Object.entries(at)) {
            if (!b.part) continue;
            for (let k = 1; k < w.points.length; k++) {
              const [a, c] = [w.points[k - 1], w.points[k]];
              const [x1, x2, y1, y2] = [Math.min(a[0], c[0]), Math.max(a[0], c[0]), Math.min(a[1], c[1]), Math.max(a[1], c[1])];
              const inside = x2 > b.x + 2 && x1 < b.x + b.w - 2 && y2 > b.y + 2 && y1 < b.y + b.h - 2;
              expect(inside, `${where} passes ${id}`).toBe(false);
            }
          }
        }
      }
    }
  });

  /***
   * A click on a closed box opens it: the page goes to the box's section, its address says so, and Back comes back to the layer.
   *
   * @part-of e2e.page
   * @covers  docair-core.layer_view.section_picture "checks a box's section holds what is in it"
   * @covers  docair-viewer.doc-page.move "checks a click on a closed box goes to its section, and Back comes back"
   ***/
  test('a closed box opens into its section, and Back comes back', async ({ page }) => {
    await openStatic(page, 'shop', '#layer/apps');
    await page.locator('#pic g.part.closed[data-id="orders-api"]').click();
    await expect(page).toHaveURL(/#box\/orders-api$/);
    // Open now: a box around what it holds.
    await expect(page.locator('#pic g.box[data-id="orders-api"]')).toBeAttached();
    expect(await pictured(page)).toEqual(expect.arrayContaining(['orders-api', 'orders.create', 'orders.list']));
    await page.goBack();
    await expect(page).toHaveURL(/#layer\/apps$/);
    await expect(page.locator('#pic g.part.closed[data-id="orders-api"]')).toBeAttached();
  });

  /***
   * Parts docair only found in the code wait until their box is opened: the box says how many, its section shows them.
   *
   * @part-of e2e.page
   * @covers  docair-core.layer_view.section_picture "checks a section shows the parts found only in the code"
   ***/
  test('parts found only in the code wait until their box is opened', async ({ page }) => {
    await openStatic(page, 'fold', '#layer/code');
    await expect(page.locator('#pic g.box[data-id="billing"] text.bn')).toContainText('2 more');
    expect(await pictured(page)).not.toContain('billing.money');
    await openStatic(page, 'fold', '#box/billing');
    expect(await pictured(page)).toEqual(expect.arrayContaining(['billing.bill', 'billing.money', 'billing.tax', 'billing.fee']));
  });

  /***
   * A part reached by its address comes in a picture that draws it, and is marked as the one picked.
   *
   * @part-of e2e.page
   * @covers  docair-viewer.doc-page.move "checks a part's address shows it in its picture, picked"
   ***/
  test('a part reached by its address is drawn and picked', async ({ page }) => {
    await openStatic(page, 'shop', '#part/orders.create');
    await expect(page.locator('#pic g.part.sel[data-id="orders.create"]')).toBeAttached();
  });

  /***
   * A click on a part selects it where it is: the picture stays, the part and its lines are marked.
   *
   * @part-of e2e.page
   * @covers  docair-viewer.doc-page.move "checks a click selects a part where it is"
   ***/
  test('a part clicked in the picture is selected where it is', async ({ page }) => {
    await openStatic(page, 'shop', '#layer/apps');
    const before = await page.locator('#pic svg.docair-pic').getAttribute('viewBox');
    await page.locator('#pic g.part[data-id="shopper"]').click();
    await expect(page.locator('#pic g.part.sel[data-id="shopper"]')).toBeAttached();
    await expect(page.locator('#pic g.line.hl[data-from="shopper"]')).toHaveCount(1);
    await expect(page.locator('#pic svg.docair-pic[data-layer="apps"]')).toBeAttached();
    expect(await page.locator('#pic svg.docair-pic').getAttribute('viewBox')).toBe(before);
  });
});

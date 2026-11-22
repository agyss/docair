import { expect, test, type Page } from '@playwright/test';
import { go, openStatic, pictured } from './helpers';

/** How big the words of the step's parts are on the screen, in px, and whether the step's first part is in view. */
function stepView(page: Page) {
  return page.evaluate(() => {
    const svg = document.querySelector('#pic svg.docair-pic') as SVGSVGElement;
    const pic = document.querySelector('#pic')!.getBoundingClientRect();
    const [, , w] = svg.getAttribute('viewBox')!.split(' ').map(Number);
    const perUnit = svg.getBoundingClientRect().width / w;
    const cur = [...svg.querySelectorAll('g.part.flow-current, g.box.flow-current')];
    const px = cur.map((g) => Number(g.querySelector('text')?.getAttribute('font-size') ?? 12) * perUnit);
    const first = cur[0]?.querySelector('.sh')?.getBoundingClientRect();
    const inView = !!first && first.right > pic.left && first.left < pic.right && first.bottom > pic.top && first.top < pic.bottom;
    return { perUnit, smallest: Math.min(...px), inView };
  });
}

/** Each line's corners, from the part it leaves to the part its arrow points at. */
function waysOf(page: Page, selector = 'g.line') {
  return page.evaluate((sel) => {
    const nums = (s: string) => (s.match(/-?\d+(\.\d+)?/g) || []).map(Number);
    return [...document.querySelectorAll(`#pic svg.docair-pic ${sel}`)].map((g) => {
      const d = (g as HTMLElement).dataset;
      const points: [number, number][] = [];
      for (const [, cmd, args] of (g.querySelector('path.ln')?.getAttribute('d') || '').matchAll(/([MLQ])([^MLQ]*)/g)) {
        const n = nums(args);
        points.push(cmd === 'Q' ? [n[0], n[1]] : [n[0], n[1]]);
      }
      return { id: d.id as string, from: d.from as string, to: d.to as string, points };
    });
  }, selector);
}

/***
 * Layers, sections and walks: the same system seen one concern at a time, a box zoomed into, and a walk through it step by step.
 *
 * @id      e2e.levels
 * @kind    test
 * @part-of e2e
 ***/
test.describe('look at it layer by layer', () => {
  /***
   * A step tells its messages: who calls whom and why, what is sent, the answer going back as a hollow token, and what the step has at its end.
   *
   * @part-of e2e.levels
   * @covers  docair-core.slice.messages "checks calls, answers and what the step has"
   * @covers  docair-viewer.doc-page.walk "checks the answer token goes back"
   ***/
  test('a step says what each call does and what comes back', async ({ page }) => {
    await openStatic(page, 'shop', '#slice/flow:checkout');
    const card = page.getByTestId('step-card');
    await card.locator('[data-step="1"]').click();
    const first = page.getByTestId('hops').locator('li');
    await expect(first).toHaveCount(1);
    await expect(first).toContainText('shopper calls Cart');
    await expect(first).toContainText('sends clicks Buy');
    await card.locator('[data-step="1"]').click();
    const hops = page.getByTestId('hops').locator('li');
    await expect(hops).toHaveCount(2);
    await expect(hops.nth(0)).toContainText('Cart calls create_order');
    await expect(hops.nth(0)).toContainText('sends CreateOrder · gets Order back');
    await expect(hops.nth(1)).toContainText('create_order answers Cart: with Order');
    await expect(hops.nth(1)).toHaveClass(/\breturn\b/);
    await expect(page.locator('#pic g.tokens g.token.reply text', { hasText: 'Order' })).toBeAttached();
    await expect(page.getByTestId('built-up')).toContainText('Order from create_order');
    await expect(hops.nth(1)).toHaveClass(/done|cur/);
  });

  /***
   * One call stack for the whole walk: in the keep scenario the API's answer to the cart comes in the step after, once the API has written the order.
   *
   * @part-of e2e.levels
   * @covers  docair-core.slice.messages "checks an answer waits for the work of a later step"
   * @covers  docair-viewer.doc-page.walk "checks the step cards show the answer after the work"
   ***/
  test('an answer comes back after the work it waits for', async ({ page }) => {
    await openStatic(page, 'shop', '#slice/flow:keep');
    const card = page.getByTestId('step-card');
    const hops = page.getByTestId('hops').locator('li');
    await expect(card).toContainText('step 1 of 3');
    await card.locator('[data-step="1"]').click();
    await expect(hops).toHaveCount(1);
    await expect(hops.nth(0)).toContainText('Cart calls create_order');
    await expect(card.locator('.hop.return')).toHaveCount(0);
    await card.locator('[data-step="1"]').click();
    await expect(hops).toHaveCount(2);
    await expect(hops.nth(0)).toContainText('create_order writes to orders-db');
    await expect(hops.nth(1)).toContainText('create_order answers Cart');
    await expect(hops.nth(1)).toHaveClass(/\breturn\b/);
  });

  /***
   * In a small window, a walk never zooms out below a readable size: each step comes to rest on its parts; Whole step shows everything.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.page_walks.steps "checks each step's camera box"
   * @covers  docair-viewer.doc-page.walk "checks the camera keeps a step readable"
   ***/
  test('every step stays readable in a small window', async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 640 });
    await openStatic(page, 'shop', '#slice/flow:checkout');
    const card = page.getByTestId('step-card');
    for (let k = 0; k < 3; k++) {
      await page.waitForTimeout(700);
      const r = await stepView(page);
      expect(r.smallest, `step ${k + 1}`).toBeGreaterThanOrEqual(0.79 * 12);
      expect(r.inView, `step ${k + 1}`).toBe(true);
      if (k < 2) await card.locator('[data-step="1"]').click();
    }
    const before = (await stepView(page)).perUnit;
    await page.getByTestId('whole-step').click();
    await page.waitForTimeout(700);
    expect((await stepView(page)).perUnit).toBeLessThanOrEqual(before + 0.01);
  });

  /***
   * Zooming into a closed box opens it: the page goes to the box's section, with a breadcrumb back; zooming far out goes back up.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.doc-page.move "checks zooming into a box opens its section, and out goes back up"
   ***/
  test('zooming into a box opens it', async ({ page }) => {
    await openStatic(page, 'shop', '#layer/overview');
    const box = await page.locator('#pic g.part.closed[data-id="infra"] .sh').boundingBox();
    await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
    for (let k = 0; k < 40 && !page.url().endsWith('#box/infra'); k++) {
      await page.mouse.wheel(0, -60);
      await page.waitForTimeout(30);
    }
    await expect(page).toHaveURL(/#box\/infra$/);
    await expect(page.locator('#crumbs')).toContainText('infra');
    expect(await pictured(page)).toContain('tf.cluster');
    for (let k = 0; k < 40 && page.url().endsWith('#box/infra'); k++) {
      await page.mouse.wheel(0, 60);
      await page.waitForTimeout(30);
    }
    await expect(page).not.toHaveURL(/#box\/infra$/);
    await go(page, '#box/infra');
    await page.locator('#crumbs a', { hasText: 'Overview' }).click();
    await expect(page).toHaveURL(/#layer\/overview$/);
  });

  /***
   * At the code layer, parts docair only found in the code wait until their box is opened; the tree still lists them, and picking one there goes to its box.
   *
   * @part-of e2e.levels
   * @covers  docair-core.layer_view.picture "checks parts found only in the code wait until their box is opened"
   * @covers  docair-viewer.doc-page.select "checks picking such a part in the tree goes to its box"
   ***/
  test('parts found only in the code wait until their box is opened', async ({ page }) => {
    await openStatic(page, 'fold', '#layer/code');
    // billing.tax has a line (billing.bill calls it); billing.fee and the module billing.money have none.
    expect((await pictured(page)).sort()).toEqual(['billing', 'billing.bill', 'billing.tax']);
    await expect(page.locator('#pic g.box[data-id="billing"] text.bn')).toHaveText('billing · 2 more');
    // The tree lists them; picking one goes to its box.
    await page.locator('#search').fill('fee');
    await page.locator('#tree-list li[data-id="billing.fee"] > .row').click();
    await expect(page.locator('#pic g.part.sel[data-id="billing.fee"]')).toBeAttached();
    expect(await pictured(page)).toEqual(expect.arrayContaining(['billing.fee', 'billing.money']));
    expect(await pictured(page)).not.toContain('billing.money.round');
    await go(page, '#box/billing.money');
    await expect.poll(() => pictured(page)).toEqual(expect.arrayContaining(['billing.money.round', 'billing.money.amount']));
  });

  /***
   * A walk is a picture of its own: the walks list in the empty details opens it, Next and Back go through it, a step's parts link to them, and closing it gives the reader's picture back.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.page_walks.walked "checks a walk is a picture of its own"
   * @covers  docair-viewer.doc-page.walk "checks Next, Back and closing the walk"
   ***/
  test('a walk is a picture, and a step links to its parts', async ({ page }) => {
    await openStatic(page, 'shop', '#layer/overview');
    await page.locator('#details .tour', { hasText: 'checkout' }).click();
    await expect(page).toHaveURL(/#slice\/flow:checkout$/);
    const card = page.getByTestId('step-card');
    await expect(card).toContainText('step 1 of 3');
    await expect(card.locator('[data-step="-1"]')).toBeDisabled();
    await card.locator('[data-step="1"]').click();
    await card.locator('[data-step="1"]').click();
    await expect(card).toContainText('step 3 of 3');
    await expect(card.locator('h3')).toHaveText('the API stores the order');
    await expect(card.locator('[data-step="1"]')).toBeDisabled();
    await card.locator('[data-step="-1"]').click();
    await expect(card).toContainText('step 2 of 3');
    await card.locator('a.chip[data-part="shop-ui.cart"]').click();
    await expect(page.locator('#pic g.sel[data-id="shop-ui.cart"]')).toBeAttached();
    await card.locator('.card-close').click();
    await expect(card).toHaveCount(0);
  });

  /***
   * A walk's lines point ahead in its picture: each one it follows goes along the layout's direction, not back.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.layout_input.hints "checks a walk's lines point ahead"
   ***/
  test("a walk's lines point ahead", async ({ page }) => {
    await openStatic(page, 'shop', '#slice/flow:checkout');
    const dir = await page.locator('#pic svg.docair-pic').getAttribute('data-direction');
    const card = page.getByTestId('step-card');
    for (let k = 0; k < 3; k++) {
      const back = (await waysOf(page, 'g.line.flow-active'))
        .filter((w) => {
          const [a, z] = [w.points[0], w.points[w.points.length - 1]];
          return dir === 'right' ? z[0] < a[0] - 20 : z[1] < a[1] - 20;
        })
        .map((w) => `${w.from} → ${w.to}`);
      expect(back, `step ${k + 1}`).toEqual([]);
      if (k < 2) await card.locator('[data-step="1"]').click();
    }
  });

  /***
   * The legend shows from the start: what each shape and line stands for. The header's Legend button hides it and shows it again.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.page_panels.panels "checks the legend shows from the start"
   ***/
  test('the legend shows from the start', async ({ page }) => {
    await openStatic(page, 'shop');
    await expect(page.locator('#legend')).toBeVisible();
    await expect(page.locator('#legend')).toContainText('function');
    await page.locator('#btn-legend').click();
    await expect(page.locator('#legend')).toBeHidden();
    await page.locator('#btn-legend').click();
    await expect(page.locator('#legend')).toBeVisible();
  });

  /***
   * With nothing selected and no walk to offer, the details say only what to do; with walks, they offer them.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.page_walks.walked "checks the details offer the walks"
   ***/
  test('the details before anything is selected', async ({ page }) => {
    await openStatic(page, 'platform');
    await expect(page.locator('#details')).toHaveText('Select a node to see its documentation.');
    await openStatic(page, 'shop');
    await expect(page.locator('#details .tour', { hasText: 'checkout' })).toBeVisible();
    await expect(page.locator('#details')).not.toContainText('null');
  });

  /***
   * A part dragged on the canvas stands where it is put down, and its lines follow, already while it is held: docair's router, built to WebAssembly, routes them again in the page, at right angles, ending on the part's border and crossing no part. A click still selects and moves nothing.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.doc-page.move-part "checks a part moved by hand takes its lines along"
   * @covers  docair-layout.hand_move.move_part "checks the lines of a moved part end on it and cross no part"
   ***/
  test('lines follow a part that is moved', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(String(e)));
    await openStatic(page, 'shop', '#layer/apps');
    const id = 'orders-db';
    // In screen pixels: each part's outline (without its stroke, which selecting thickens) and the lines that touch the part.
    const around = () =>
      page.evaluate((id) => {
        const svg = document.querySelector('#pic svg.docair-pic')!;
        const px = (e: Element, x: number, y: number): [number, number] => {
          const p = new DOMPoint(x, y).matrixTransform((e as SVGGraphicsElement).getScreenCTM()!);
          return [p.x, p.y];
        };
        const nums = (s: string) => (s.match(/-?\d+(\.\d+)?/g) || []).map(Number);
        const parts: Record<string, { x: number; y: number; w: number; h: number }> = {};
        for (const g of svg.querySelectorAll('g.part')) {
          const sh = g.querySelector('.sh') as SVGGraphicsElement;
          const b = sh.getBBox();
          const [[x, y], [r, t]] = [px(sh, b.x, b.y), px(sh, b.x + b.width, b.y + b.height)];
          parts[(g as HTMLElement).dataset.id!] = { x, y, w: r - x, h: t - y };
        }
        const lines = [...svg.querySelectorAll('g.line')]
          .filter((g) => (g as HTMLElement).dataset.from === id || (g as HTMLElement).dataset.to === id)
          .map((g) => {
            const d = (g as HTMLElement).dataset;
            const [ln, ah] = [g.querySelector('path.ln')!, g.querySelector('path.ah')!];
            const points = [...(ln.getAttribute('d') || '').matchAll(/([MLQ])([^MLQ]*)/g)].map(([, , a]) => px(ln, nums(a)[0], nums(a)[1]));
            const tip = px(ah, nums(ah.getAttribute('d') || '')[0], nums(ah.getAttribute('d') || '')[1]);
            return { id: d.id!, d: ln.getAttribute('d'), points, end: d.from === id ? points[0] : tip };
          });
        return { parts, lines };
      }, id);
    const before = await around();
    const was = before.parts[id];
    expect(before.lines.length, 'orders-db has lines').toBeGreaterThan(0);
    // A click selects, and moves nothing.
    const part = page.locator(`#pic svg.docair-pic g.part[data-id="${id}"]`);
    await part.click();
    await expect(page.locator('#details .meta')).toContainText(id);
    expect((await around()).parts[id]).toEqual(was);
    // Dragged 80 px to the right and 60 down, into the empty space below.
    const c = { x: was.x + was.w / 2, y: was.y + was.h / 2 };
    await page.mouse.move(c.x, c.y);
    await page.mouse.down();
    await page.mouse.move(c.x + 40, c.y + 30, { steps: 4 });
    await page.mouse.move(c.x + 80, c.y + 60, { steps: 4 });
    // While it is still held, its lines follow already.
    await expect.poll(async () => (await around()).lines[0].d).not.toBe(before.lines[0].d);
    await page.mouse.up();
    const after = await around();
    const now = after.parts[id];
    expect(Math.abs(now.x - was.x - 80), 'it stands where it was put down').toBeLessThan(1.5);
    expect(Math.abs(now.y - was.y - 60)).toBeLessThan(1.5);
    for (const l of after.lines) {
      const right = l.points.every((p, i) => i === 0 || Math.abs(p[0] - l.points[i - 1][0]) < 0.6 || Math.abs(p[1] - l.points[i - 1][1]) < 0.6);
      expect(right, `line ${l.id} runs at right angles`).toBe(true);
      const [x, y] = l.end;
      const t = 2;
      const inside = x > now.x - t && x < now.x + now.w + t && y > now.y - t && y < now.y + now.h + t;
      const edge = Math.min(Math.abs(x - now.x), Math.abs(x - now.x - now.w), Math.abs(y - now.y), Math.abs(y - now.y - now.h));
      expect(inside && edge < t, `line ${l.id} ends on the moved part's border`).toBe(true);
      const crossed = Object.entries(after.parts)
        .filter(([, b]) =>
          l.points.some((p, i) => {
            if (i === 0) return false;
            const q = l.points[i - 1];
            return Math.min(p[0], q[0]) < b.x + b.w - 3 && Math.max(p[0], q[0]) > b.x + 3 && Math.min(p[1], q[1]) < b.y + b.h - 3 && Math.max(p[1], q[1]) > b.y + 3;
          }),
        )
        .map(([pid]) => pid);
      expect(crossed, `line ${l.id} crosses no part`).toEqual([]);
    }
    expect(errors).toEqual([]);
  });

  /***
   * docair draws the lines itself: nothing is loaded from the web, no part lies on another, no two lines run on top of each other, and no line's words cover a part.
   *
   * @part-of e2e.levels
   * @covers  docair-layout.lay_out "checks no part lies on another"
   * @covers  docair-layout.route "checks the lines keep off each other"
   * @covers  docair-viewer.svg.draw "checks no line's words cover a part"
   ***/
  test('docair draws its own lines, off the parts and off each other', async ({ page }) => {
    const fetched: string[] = [];
    page.on('request', (r) => {
      if (!/^(file|data|blob|about):/.test(r.url())) fetched.push(r.url());
    });
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(String(e)));
    for (const [name, layer] of [
      ['platform', 'infra'],
      ['platform', 'network'],
      ['platform', 'k8s'],
      ['shop', 'apps'],
      ['shop', 'code'],
    ]) {
      await openStatic(page, name, `#layer/${layer}`);
      const r = await page.evaluate(() => {
        const svg = document.querySelector('#pic svg.docair-pic')!;
        const rect = (g: Element) => (g.querySelector('.sh') as SVGGraphicsElement).getBBox();
        const parts = [...svg.querySelectorAll('g.part')].map(rect);
        const overlapping = parts.filter((a, i) => parts.some((b, j) => j > i && a.x < b.x + b.width && a.x + a.width > b.x && a.y < b.y + b.height && a.y + a.height > b.y)).length;
        const nums = (s: string) => (s.match(/-?\d+(\.\d+)?/g) || []).map(Number);
        const pieces: { id: string; a: number[]; b: number[] }[] = [];
        for (const g of svg.querySelectorAll('g.line')) {
          const pts: number[][] = [];
          for (const [, cmd, args] of (g.querySelector('path.ln')?.getAttribute('d') || '').matchAll(/([MLQ])([^MLQ]*)/g)) {
            const n = nums(args);
            pts.push(cmd === 'Q' ? [n[0], n[1]] : [n[0], n[1]]);
          }
          for (let i = 1; i < pts.length; i++) pieces.push({ id: (g as HTMLElement).dataset.id!, a: pts[i - 1], b: pts[i] });
        }
        let onTop = 0;
        for (let i = 0; i < pieces.length; i++) {
          for (let j = i + 1; j < pieces.length; j++) {
            const [s, t] = [pieces[i], pieces[j]];
            const flat = Math.abs(s.a[1] - s.b[1]) < 0.5;
            if (s.id === t.id || flat !== Math.abs(t.a[1] - t.b[1]) < 0.5) continue;
            const gap = flat ? Math.abs(s.a[1] - t.a[1]) : Math.abs(s.a[0] - t.a[0]);
            const [p, q] = flat ? [[s.a[0], s.b[0]], [t.a[0], t.b[0]]] : [[s.a[1], s.b[1]], [t.a[1], t.b[1]]];
            if (gap < 2 && Math.min(Math.max(...p), Math.max(...q)) - Math.max(Math.min(...p), Math.min(...q)) > 2) onTop++;
          }
        }
        const covered = [...svg.querySelectorAll('g.words > rect')].filter((w) => {
          const a = (w as SVGGraphicsElement).getBBox();
          return parts.some((b) => a.x < b.x + b.width - 2 && a.x + a.width > b.x + 2 && a.y < b.y + b.height - 2 && a.y + a.height > b.y + 2);
        }).length;
        return { lines: svg.querySelectorAll('g.line').length, overlapping, onTop, covered };
      });
      expect(r.lines, `${name} ${layer}`).toBeGreaterThan(3);
      expect(r, `${name} ${layer}`).toMatchObject({ overlapping: 0, onTop: 0, covered: 0 });
    }
    expect(fetched).toEqual([]);
    expect(errors).toEqual([]);
  });
});

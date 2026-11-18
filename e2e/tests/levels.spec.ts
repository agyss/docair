import { expect, test } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { docair, drawn, fixture, openStatic, out } from './helpers';

/** Every drawn line's way and the place of its words; also kept on the page as `__before` for comparisons there. */
async function drawnLines(page: import('@playwright/test').Page): Promise<Record<string, { route: { x: number; y: number }[]; words: number[] | null }>> {
  return page.evaluate(() => {
    const cy = (window as any).docair.state.cy;
    const out: Record<string, any> = {};
    cy.edges().forEach((e: any) => {
      const l = cy.getElementById(`${e.id()}::label`);
      out[e.id()] = { route: (e.data('route') || []).map((q: any) => ({ x: Math.round(q.x * 10) / 10, y: Math.round(q.y * 10) / 10 })), words: l.nonempty() ? [Math.round(l.position('x')), Math.round(l.position('y'))] : null };
    });
    (window as any).__before ??= out;
    return out;
  });
}

/***
 * Layers: the same system seen one concern at a time (the overview, the infrastructure, the network, Kubernetes, the apps, the code), by zooming into a box, and by walking through.
 *
 * @id      e2e.levels
 * @kind    test
 * @part-of e2e
 ***/
test.describe('look at it layer by layer', () => {
  /***
   * Each layer draws what its question is about: the overview the big pieces, the infrastructure the machines and Terraform, the apps the programs without the machines around them, the code every function.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.show-layer "checks each layer draws its own parts"
   * @covers  docair-viewer.layer-view "checks what a layer leaves out and dissolves"
   * @covers  docair-viewer.render "checks each layer redraws the graph"
   ***/
  test('each layer shows what its question is about', async ({ page }) => {
    await openStatic(page, 'shop');
    await expect(page.locator('#layers button')).toHaveText(['Overview', 'Infrastructure', 'Apps', 'Code']);
    const show = async (name: string) => {
      await page.locator('#layers button', { hasText: name }).click();
      await expect(page.locator('#layers button.on')).toHaveText(name);
      return drawn(page);
    };
    const top = await show('Overview');
    expect(top).toContain('infra');
    expect(top).not.toContain('orders.create');
    const infra = await show('Infrastructure');
    expect(infra).toEqual(expect.arrayContaining(['infra', 'tf.cluster', 'cluster']));
    expect(infra).not.toContain('shop-ui');
    const apps = await show('Apps');
    expect(apps).toEqual(expect.arrayContaining(['shopper', 'shop-ui', 'orders-api', 'orders-db']));
    // The machine and the cluster around the API are dissolved: the apps stand side by side.
    expect(apps).not.toContain('cluster');
    expect(apps).not.toContain('orders.create');
    const code = await show('Code');
    expect(code).toContain('orders.create');
  });

  /***
   * The network layer shows the way a request takes in; the apps layer draws it as one line from the shopper to the API that says what it passes, and the route sends it to the API only.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.layer-view "checks the network layer and the line through it"
   * @covers  docair-viewer.render "checks a line through left-out parts says what it passes"
   ***/
  test('a request through the network is one line between the apps', async ({ page }) => {
    await openStatic(page, 'platform');
    await expect(page.locator('#layers button')).toHaveText(['Overview', 'Infrastructure', 'Network', 'Kubernetes', 'Apps']);
    await page.evaluate(() => (window as any).docair.showLayer('network'));
    const net = await drawn(page);
    expect(net).toEqual(expect.arrayContaining(['shopper', 'k8s.ingress.shop', 'k8s.svc.api', 'k8s.deploy.api']));
    expect(net).not.toContain('terraform');
    await page.evaluate(() => (window as any).docair.showLayer('apps'));
    const lines = await page.evaluate(() =>
      (window as any).docair.state.cy.edges().map((e: any) => `${e.data('source')} → ${e.data('target')}: ${(e.data('labelText') || '').replace(/\n/g, ' ')}`),
    );
    expect(lines).toContain('shopper → api: GET /api/items · via Ingress shop');
    expect(lines.filter((l: string) => l.startsWith('shopper → web'))).toEqual([]);
    expect(await drawn(page)).not.toContain('k8s.ingress.shop');
  });

  /***
   * A step tells its messages: who calls whom and why, what is sent, the answer going back as a hollow token, and what the step has at its end.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.messages "checks calls, answers and what the step has"
   * @covers  docair-viewer.play-messages "checks the answer token goes back"
   ***/
  test('a step says what each call does and what comes back', async ({ page }) => {
    await openStatic(page, 'shop');
    await page.evaluate(() => (window as any).docair.setFlow('checkout', 1));
    const first = page.getByTestId('hops').locator('li');
    await expect(first).toHaveCount(1);
    await expect(first).toContainText('shopper calls Cart');
    await expect(first).toContainText('sends clicks Buy');
    const answer = page.waitForFunction(() => (window as any).docair.state.cy.$('node.token.reply').map((n: any) => n.data('label')).includes('Order'), null, { timeout: 15000 });
    await page.evaluate(() => (window as any).docair.goStep(2));
    const hops = page.getByTestId('hops').locator('li');
    await expect(hops).toHaveCount(2);
    await expect(hops.nth(0)).toContainText('Cart calls create_order');
    await expect(hops.nth(0)).toContainText('sends CreateOrder · gets Order back');
    await expect(hops.nth(1)).toContainText('create_order answers Cart: with Order');
    await answer;
    await expect(page.getByTestId('built-up')).toContainText('Order from create_order');
    await expect(hops.nth(1)).toHaveClass(/done/);
  });

  /***
   * One call stack for the whole walk: a call's answer goes back only after the work it waits for, even when that work is a later step.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.messages "checks answers wait for the later steps inside the call"
   ***/
  test('an answer comes back after the work it waits for', async ({ page }) => {
    await openStatic(page, 'shop');
    const order = await page.evaluate(() => {
      const D = (window as any).docair;
      const edges = D.state.graph.edges;
      const at = (from: string, to: string) => edges.findIndex((e: any) => e.from === from && e.to === to);
      // Step 1: the cart calls the API. Step 2: inside that call, the API writes the order.
      const words = (per: any[][]) => per.map((ms) => ms.map((m) => `${m.kind} ${m.e.from}>${m.e.to}`));
      const nested = D.walkMessages([{ path: [at('shop-ui.cart', 'orders.create')] }, { path: [at('orders.create', 'orders-db')] }]);
      // A line from a part that is not on the stack does not close the API's call: the API still writes later.
      const between = D.walkMessages([
        { path: [at('shop-ui.cart', 'orders.create')] },
        { path: [at('tf.cluster', 'cluster')] },
        { path: [at('orders.create', 'orders-db')] },
      ]);
      return { nested: words(nested), between: words(between) };
    });
    expect(order.nested).toEqual([
      ['call shop-ui.cart>orders.create'],
      ['write orders.create>orders-db', 'return shop-ui.cart>orders.create'],
    ]);
    expect(order.between).toEqual([
      ['call shop-ui.cart>orders.create'],
      ['create tf.cluster>cluster'],
      ['write orders.create>orders-db', 'return shop-ui.cart>orders.create'],
    ]);
  });

  /***
   * In a small window, a walkthrough never zooms out below a readable size: each step comes to rest on its parts; Whole step shows everything.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.play-step
   ***/
  test('every step stays readable in a small window', async ({ page }) => {
    await page.setViewportSize({ width: 1000, height: 640 });
    await openStatic(page, 'shop');
    await page.evaluate(() => (window as any).docair.setFlow('checkout'));
    for (let k = 0; k < 3; k++) {
      await page.evaluate((x) => (window as any).docair.goStep(x), k);
      await page.waitForTimeout(900);
      const r = await page.evaluate(() => {
        const D = (window as any).docair, cy = D.state.cy, st = D.state.flow.steps[D.state.step];
        const bb = cy.getElementById(D.rep(st.nodes[0])).boundingBox();
        const ext = cy.extent();
        return { zoom: cy.zoom(), inView: bb.x1 < ext.x2 && bb.x2 > ext.x1 && bb.y1 < ext.y2 && bb.y2 > ext.y1 };
      });
      expect(r.zoom, `step ${k + 1}`).toBeGreaterThanOrEqual(0.79);
      expect(r.inView, `step ${k + 1}`).toBe(true);
    }
    const before = await page.evaluate(() => (window as any).docair.state.cy.zoom());
    await page.getByTestId('whole-step').click();
    await page.waitForTimeout(700);
    expect(await page.evaluate(() => (window as any).docair.state.cy.zoom())).toBeLessThanOrEqual(before + 0.01);
  });

  /***
   * Every line on the page has words: its own, the callee's signature from the code, or what it does.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.render "checks that every rendered line has words"
   * @covers  docair-core.signature.ts_signature "checks that call edges carry wording"
   ***/
  test('every line has words', async ({ page }) => {
    await openStatic(page, 'shop');
    for (const level of await page.evaluate(() => (window as any).docair.state.layers.map((L: any) => L.key))) {
      await page.evaluate((l) => (window as any).docair.showLayer(l), level);
      const bare = await page.evaluate(() =>
        (window as any).docair.state.cy
          .edges()
          .filter((e: any) => !(e.data('labelText') || e.data('label')))
          .map((e: any) => `${e.data('source')} → ${e.data('target')}`),
      );
      expect(bare, `layer ${level}`).toEqual([]);
    }
    const covers = await page.evaluate(() =>
      (window as any).docair.state.graph.edges.filter((e: any) => e.kind === 'cover').map((e: any) => (window as any).docair.edgeText(e)),
    );
    expect(covers).toContain('tests');
  });

  /***
   * Zooming into a box opens it where it is, with a breadcrumb back to the layer.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.drill "checks zooming opens the box"
   ***/
  test('zooming into a box opens it', async ({ page }) => {
    await openStatic(page, 'shop');
    await page.evaluate(() => (window as any).docair.showLayer('overview'));
    await page.evaluate(() => (window as any).docair.drill('infra'));
    await expect(page.locator('#crumbs')).toContainText('infra');
    expect(await drawn(page)).toContain('tf.cluster');
    await page.locator('#crumbs a', { hasText: 'Overview' }).click();
    await expect(page.locator('#crumbs')).toBeHidden();
  });

  /***
   * At the code layer, parts docair only found in the code wait until their box is zoomed into: a box of code whose parts are all such stands closed, and in a box that also holds documented parts, those without a line are left out and the box says how many. A part with a line stays. The tree still lists them; picking one there opens its box.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.layer-view "checks parts found only in the code wait until their box is opened"
   * @covers  docair-viewer.drill "checks zooming into the box shows them"
   * @covers  docair-viewer.show-details "checks picking such a part in the tree opens its box"
   ***/
  test('parts found only in the code wait until their box is opened', async ({ page }) => {
    execFileSync(docair, ['scan', fixture('fold'), '--out', path.join(out, 'fold')]);
    await openStatic(page, 'fold');
    const code = () =>
      page.evaluate(async () => {
        await (window as any).docair.showLayer('code');
        await (window as any).docair.state.lastRender;
      });
    const label = (id: string) => page.evaluate((x) => (window as any).docair.state.cy.getElementById(x).data('label'), id);
    await code();
    // billing.tax has a line (billing.bill calls it); billing.fee and the module billing.money have none
    expect((await drawn(page)).sort()).toEqual(['billing', 'billing.bill', 'billing.tax']);
    expect(await label('billing')).toBe('billing · 2 more');
    // the tree lists them; picking one opens its box
    await page.locator('#search').fill('fee');
    await page.locator('#tree-list li[data-id="billing.fee"] > .row').click();
    await expect.poll(() => drawn(page)).toEqual(expect.arrayContaining(['billing.fee', 'billing.money']));
    await page.locator('#search').fill('');
    // zooming into the boxes shows the rest; the module of parts found only in the code stood closed
    await code();
    await page.evaluate(() => (window as any).docair.drill('billing'));
    await expect.poll(() => drawn(page)).toEqual(expect.arrayContaining(['billing.fee', 'billing.money']));
    expect(await drawn(page)).not.toContain('billing.money.round');
    await page.evaluate(() => (window as any).docair.drill('billing.money'));
    await expect.poll(() => drawn(page)).toEqual(expect.arrayContaining(['billing.money.round', 'billing.money.amount']));
  });

  /***
   * While a picture is laid out, the page says what it draws: the layer, its parts and its lines; the
   * note goes once the picture is there.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.drawing "checks the page says what it draws while it lays out"
   ***/
  test('the page says what it draws while it lays out', async ({ page }) => {
    await openStatic(page, 'shop');
    await expect(page.getByTestId('drawing')).not.toHaveClass(/show/);
    const note = await page.evaluate(async () => {
      const box = document.querySelector('#drawing')!;
      const done = (window as any).docair.showLayer('code');
      const during = { show: box.classList.contains('show'), text: box.textContent };
      await done;
      return { ...during, after: box.classList.contains('show') };
    });
    expect(note.show).toBe(true);
    expect(note.text).toMatch(/^Drawing Code · \d+ parts, \d+ lines…$/);
    expect(note.after).toBe(false);
  });

  /***
   * A walk plays at the layer that is about it: the boxes there stand for what is inside them, so the picture stays put; Open walks into a step's box, Back returns, and closing the walk gives the reader's layer back.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.layer "checks a walk keeps one layer and opens a step"
   * @covers  docair-viewer.walk "checks the walk gives the layer back at the end"
   ***/
  test('a walk keeps one layer, and a step opens into its box', async ({ page }) => {
    await openStatic(page, 'shop');
    await page.evaluate(() => (window as any).docair.showLayer('overview'));
    await page.locator('#details .tour', { hasText: 'checkout' }).click();
    const card = page.getByTestId('step-card');
    await expect(card).toContainText('step 1 of 3');
    // The checkout happens between programs: it plays at the apps.
    await expect(page.getByTestId('layer')).toHaveValue('apps');
    await page.keyboard.press('ArrowRight');
    await page.keyboard.press('ArrowRight');
    await expect(card).toContainText('step 3 of 3');
    await expect(card.locator('h3')).toHaveText('the API stores the order');
    // The same layer: the API is one box.
    await expect.poll(() => drawn(page)).toContain('orders-api');
    expect(await drawn(page)).not.toContain('orders.create');
    await card.getByTestId('open-step').click();
    await expect(card.getByTestId('close-step')).toBeVisible();
    await expect.poll(() => drawn(page)).toContain('orders.create');
    await card.getByTestId('close-step').click();
    await expect(card).toContainText('step 3 of 3');
    await expect.poll(() => drawn(page)).not.toContain('orders.create');
    // Another layer, chosen by hand.
    await page.getByTestId('layer').selectOption('code');
    await expect.poll(() => drawn(page)).toContain('orders.create');
    await card.locator('.card-close').click();
    await expect(card).toHaveCount(0);
    await expect(page.locator('#layers button.on')).toHaveText('Overview');
    expect(await page.evaluate(() => (window as any).docair.state.layer)).toBe('overview');
  });

  /***
   * While a walk plays, a layer that draws none of its parts is off, and picking it leaves the walk where it is instead of fading everything.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.layer "checks a walk is never shown at a layer without its parts"
   ***/
  test('a layer without any part of the walk is off while it plays', async ({ page }) => {
    await openStatic(page, 'shop');
    await page.evaluate(() =>
      (window as any).docair.addWalkthrough({
        id: 'cart-total',
        title: 'How the cart adds up',
        steps: [
          { title: 'the cart holds the items', nodes: ['shop-ui.cart'] },
          { title: 'the total sums them', nodes: ['shop-ui.cart.total'] },
        ],
      }),
    );
    await expect(page.getByTestId('step-card')).toContainText('step 1 of');
    // The cart is code in the browser: the infrastructure draws none of it.
    const infra = page.locator('#layers button[data-layer="infra"]');
    await expect(infra).toBeDisabled();
    await expect(page.locator('[data-test="layer"] option[value="infra"]')).toBeDisabled();
    const layer = await page.evaluate(() => (window as any).docair.state.flow.layer);
    await page.evaluate(() => (window as any).docair.setLayer('infra'));
    expect(await page.evaluate(() => (window as any).docair.state.flow.layer)).toBe(layer);
    expect(await page.evaluate(() => (window as any).docair.state.cy.nodes('.flow-current').length)).toBeGreaterThan(0);
  });

  /***
   * The legend shows from the start: what each shape and line stands for. The header's Legend button hides it and shows it again.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.boot "checks the legend shows from the start"
   ***/
  test('the legend shows from the start', async ({ page }) => {
    await openStatic(page, 'shop');
    await expect(page.locator('#legend')).toBeVisible();
    await expect(page.locator('#legend')).toContainText('function');
    await expect(page.locator('#btn-legend')).toHaveClass(/\bon\b/);
    await page.locator('#btn-legend').click();
    await expect(page.locator('#legend')).toBeHidden();
    await page.locator('#btn-legend').click();
    await expect(page.locator('#legend')).toBeVisible();
  });

  /***
   * With nothing selected and no walkthrough to offer, the details say only what to do; with a walkthrough, they offer it.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.show-details "checks the details before anything is selected"
   ***/
  test('the details before anything is selected', async ({ page }) => {
    await openStatic(page, 'platform');
    await expect(page.locator('#details')).toHaveText('Select a node to see its documentation.');
    await openStatic(page, 'shop');
    await expect(page.locator('#details .tour', { hasText: 'checkout' })).toBeVisible();
    await expect(page.locator('#details')).not.toContainText('null');
  });

  /***
   * A part moved by hand takes its lines along: while it is held, they are routed again where it stands, at right angles and ending on it, and their words wait. A click moves nothing.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.render "checks lines follow a part that is moved"
   ***/
  test('lines follow a part that is moved', async ({ page }) => {
    await openStatic(page, 'platform');
    await page.evaluate(() => (window as any).docair.showLayer('network'));
    const at = await page.evaluate(() => {
      const cy = (window as any).docair.state.cy;
      cy.fit(undefined, 30);
      const r = cy.getElementById('k8s.svc.api').renderedPosition();
      const box = document.querySelector('#cy')!.getBoundingClientRect();
      return { x: box.left + r.x, y: box.top + r.y };
    });
    const before = await drawnLines(page);
    await page.mouse.click(at.x, at.y);
    expect(await drawnLines(page)).toEqual(before);
    await page.mouse.move(at.x, at.y);
    await page.mouse.down();
    await page.mouse.move(at.x + 160, at.y + 70, { steps: 8 });
    const lines = await page.evaluate(() => {
      const cy = (window as any).docair.state.cy;
      const part = cy.getElementById('k8s.svc.api');
      const bb = part.boundingBox({ includeLabels: false, includeOverlays: false });
      return part.connectedEdges().map((e: any) => {
        const pts = [e.sourceEndpoint(), ...(e.segmentPoints() || []), e.targetEndpoint()];
        const q = e.source().id() === part.id() ? pts[0] : pts[pts.length - 1];
        return {
          slanted: pts.some((p: any, i: number) => i > 0 && Math.abs(p.x - pts[i - 1].x) > 1.5 && Math.abs(p.y - pts[i - 1].y) > 1.5),
          onPart: q.x > bb.x1 - 3 && q.x < bb.x2 + 3 && q.y > bb.y1 - 3 && q.y < bb.y2 + 3,
          waiting: cy.getElementById(`${e.id()}::label`).hasClass('away'),
        };
      });
    });
    expect(lines.length).toBeGreaterThan(0);
    for (const l of lines) expect(l).toEqual({ slanted: false, onPart: true, waiting: true });
    await page.mouse.up();
  });

  /***
   * Every line is drawn as it was routed: only horizontal and vertical pieces, and both ends on the border of the part or box it joins.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.render "checks lines are drawn at right angles and end on their parts"
   * @covers  docair-viewer.top-down "checks the boxes are the layout's boxes"
   ***/
  test('every line is drawn at right angles and ends on its part', async ({ page }) => {
    for (const [fixture, layer] of [['platform', 'network'], ['platform', 'k8s'], ['shop', 'apps'], ['shop', 'code']]) {
      await openStatic(page, fixture);
      await page.evaluate((l) => (window as any).docair.showLayer(l), layer);
      const bad = await page.evaluate(() => {
        const cy = (window as any).docair.state.cy;
        const out: string[] = [];
        cy.edges('[?routed]').forEach((e: any) => {
          const pts = [e.sourceEndpoint(), ...(e.segmentPoints() || []), e.targetEndpoint()];
          for (let i = 1; i < pts.length; i++) {
            if (Math.abs(pts[i].x - pts[i - 1].x) > 1.5 && Math.abs(pts[i].y - pts[i - 1].y) > 1.5) out.push(`${e.id()} slants`);
          }
          for (const [pt, n] of [[pts[0], e.source()], [pts[pts.length - 1], e.target()]]) {
            const bb = n.boundingBox({ includeLabels: false, includeOverlays: false });
            const off = Math.min(Math.abs(pt.x - bb.x1), Math.abs(pt.x - bb.x2), Math.abs(pt.y - bb.y1), Math.abs(pt.y - bb.y2));
            const within = pt.x >= bb.x1 - 3 && pt.x <= bb.x2 + 3 && pt.y >= bb.y1 - 3 && pt.y <= bb.y2 + 3;
            if (off > 4 || !within) out.push(`${e.id()} ends ${off.toFixed(0)}px off ${n.id()}`);
          }
        });
        return out;
      });
      expect(bad, `${fixture} ${layer}`).toEqual([]);
    }
  });

  /***
   * A walk's lines point ahead in the picture: each one it follows to a part it has not been to goes along the layout's direction, not back.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.top-down "checks a walk's lines point ahead"
   ***/
  test("a walk's lines point ahead", async ({ page }) => {
    await openStatic(page, 'shop');
    for (let k = 0; k < 3; k++) {
      await page.evaluate((x) => (window as any).docair.setFlow('checkout', x), k);
      const back = await page.evaluate(async () => {
        const S = (window as any).docair.state;
        await S.lastRender;
        return S.cy
          .edges('.flow-active')
          .filter((e: any) => {
            const r = e.data('route');
            if (!r) return false;
            const [a, z] = [r[0], r[r.length - 1]];
            return S.dirUsed === 'RIGHT' ? z.x < a.x - 20 : z.y < a.y - 20;
          })
          .map((e: any) => `${e.source().id()} → ${e.target().id()}`);
      });
      expect(back, `step ${k + 1}`).toEqual([]);
    }
  });

  /***
   * Top-down draws its lines with docair's own router: nothing is loaded from the web, every line is drawn at right angles from one of its parts to the other, none goes through a part, none runs on top of another, and no line's words cover a part.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.top-down "checks every box is placed and nothing overlaps"
   * @covers  docair-viewer.grid-lines "checks the lines keep off the parts and off each other"
   ***/
  test('top-down draws its own lines, off the parts and off each other', async ({ page }) => {
    const fetched: string[] = [];
    page.on('request', (r) => {
      if (!/^(file|data|blob|about):/.test(r.url())) fetched.push(r.url());
    });
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(String(e)));
    await openStatic(page, 'platform');
    for (const layer of ['infra', 'network', 'k8s']) {
      await page.evaluate((l) => (window as any).docair.showLayer(l), layer);
      const r = await page.evaluate(async () => {
        const S = (window as any).docair.state;
        await S.lastRender;
        const cy = S.cy;
        const parts = cy.nodes().filter((n: any) => !n.isParent() && !n.hasClass('elabel') && !n.hasClass('token'));
        const box = (n: any) => ({ x1: n.position().x - n.data('w') / 2, x2: n.position().x + n.data('w') / 2, y1: n.position().y - n.data('h') / 2, y2: n.position().y + n.data('h') / 2 });
        const overlapping = parts.filter((a: any) => parts.some((b: any) => a.id() < b.id() && box(a).x1 < box(b).x2 && box(a).x2 > box(b).x1 && box(a).y1 < box(b).y2 && box(a).y2 > box(b).y1)).length;
        const lines = cy.edges('[?routed]');
        const pieces: { id: string; a: any; b: any }[] = [];
        let slanted = 0;
        let through = 0;
        let loose = 0;
        lines.forEach((e: any) => {
          const pts = e.data('route');
          for (let i = 1; i < pts.length; i++) {
            if (Math.abs(pts[i].x - pts[i - 1].x) > 0.5 && Math.abs(pts[i].y - pts[i - 1].y) > 0.5) slanted++;
            pieces.push({ id: e.id(), a: pts[i - 1], b: pts[i] });
          }
          // Both ends lie on the border of the line's parts.
          for (const [n, q] of [[e.source(), pts[0]], [e.target(), pts[pts.length - 1]]] as [any, any][]) {
            const bb = n.boundingBox({ includeLabels: false, includeOverlays: false });
            const off = Math.min(Math.abs(q.x - bb.x1), Math.abs(q.x - bb.x2), Math.abs(q.y - bb.y1), Math.abs(q.y - bb.y2));
            const within = q.x >= bb.x1 - 3 && q.x <= bb.x2 + 3 && q.y >= bb.y1 - 3 && q.y <= bb.y2 + 3;
            if (off > 4 || !within) loose++;
          }
          parts.forEach((n: any) => {
            if (n.id() === e.source().id() || n.id() === e.target().id()) return;
            const b = box(n);
            if (pts.some((q: any, i: number) => i && Math.max(q.x, pts[i - 1].x) > b.x1 + 1 && Math.min(q.x, pts[i - 1].x) < b.x2 - 1 && Math.max(q.y, pts[i - 1].y) > b.y1 + 1 && Math.min(q.y, pts[i - 1].y) < b.y2 - 1)) through++;
          });
        });
        // Two lines on top of each other: side by side pieces less than 2 px apart that share a stretch.
        let onTop = 0;
        for (let i = 0; i < pieces.length; i++) {
          for (let j = i + 1; j < pieces.length; j++) {
            const [s, t] = [pieces[i], pieces[j]];
            const flat = Math.abs(s.a.y - s.b.y) < 0.5;
            if (s.id === t.id || flat !== Math.abs(t.a.y - t.b.y) < 0.5) continue;
            const gap = flat ? Math.abs(s.a.y - t.a.y) : Math.abs(s.a.x - t.a.x);
            const [p, q] = flat ? [[s.a.x, s.b.x], [t.a.x, t.b.x]] : [[s.a.y, s.b.y], [t.a.y, t.b.y]];
            if (gap < 2 && Math.min(Math.max(...p), Math.max(...q)) - Math.max(Math.min(...p), Math.min(...q)) > 2) onTop++;
          }
        }
        // Words on a part: a line's words drawn over a part that is not a box.
        const covered = cy.nodes('.elabel').filter((l: any) => {
          const a = l.boundingBox({ includeLabels: false });
          return parts.some((n: any) => {
            const b = n.boundingBox({ includeLabels: false });
            return a.x1 < b.x2 - 2 && a.x2 > b.x1 + 2 && a.y1 < b.y2 - 2 && a.y2 > b.y1 + 2;
          });
        }).length;
        return { all: cy.edges().length, lines: lines.length, overlapping, slanted, through, loose, onTop, covered, noWay: S.noWay };
      });
      expect(r.lines, layer).toBeGreaterThan(3);
      expect(r, layer).toMatchObject({ lines: r.all, overlapping: 0, slanted: 0, through: 0, loose: 0, onTop: 0, covered: 0, noWay: 0 });
    }
    expect(fetched).toEqual([]);
    expect(errors).toEqual([]);
  });

  /***
   * Once a part moved by hand is put down, its lines are routed again where it stands and their words come back; every other line keeps its way and its words, except those the part was put down on.
   *
   * @part-of e2e.levels
   * @covers  docair-viewer.grid-lines "checks only the lines the move touched are routed again"
   ***/
  test('top-down routes the lines again after a part is moved', async ({ page }) => {
    await openStatic(page, 'platform');
    await page.evaluate(() => (window as any).docair.showLayer('network'));
    const at = await page.evaluate(() => {
      const cy = (window as any).docair.state.cy;
      cy.fit(undefined, 30);
      const r = cy.getElementById('k8s.svc.api').renderedPosition();
      const box = document.querySelector('#cy')!.getBoundingClientRect();
      return { x: box.left + r.x, y: box.top + r.y, before: cy.getElementById('k8s.svc.api').position() };
    });
    const before = await drawnLines(page);
    await page.mouse.move(at.x, at.y);
    await page.mouse.down();
    await page.mouse.move(at.x + 120, at.y + 40, { steps: 8 });
    await page.mouse.up();
    await expect
      .poll(() =>
        page.evaluate(() => {
          const cy = (window as any).docair.state.cy;
          const lines = cy.getElementById('k8s.svc.api').connectedEdges();
          return lines.length > 0 && lines.every((e: any) => e.data('routed') && !cy.getElementById(`${e.id()}::label`).hasClass('away'));
        }),
      )
      .toBe(true);
    const r = await page.evaluate(() => {
      const cy = (window as any).docair.state.cy;
      const part = cy.getElementById('k8s.svc.api');
      const bb = part.boundingBox({ includeLabels: false, includeOverlays: false });
      const lines = part.connectedEdges();
      return {
        moved: part.position(),
        slanted: lines.filter((e: any) => e.data('route').some((q: any, i: number, pts: any[]) => i && Math.abs(q.x - pts[i - 1].x) > 0.5 && Math.abs(q.y - pts[i - 1].y) > 0.5)).length,
        // each line still ends on the part where it now stands
        loose: lines.filter((e: any) => {
          const pts = e.data('route');
          const q = e.source().id() === part.id() ? pts[0] : pts[pts.length - 1];
          return !(q.x > bb.x1 - 3 && q.x < bb.x2 + 3 && q.y > bb.y1 - 3 && q.y < bb.y2 + 3);
        }).length,
      };
    });
    expect(Math.abs(r.moved.x - at.before.x) + Math.abs(r.moved.y - at.before.y)).toBeGreaterThan(20);
    expect(r.slanted).toBe(0);
    expect(r.loose).toBe(0);
    // The other lines and their words stay, unless the part now stands where they ran.
    const after = await drawnLines(page);
    const moved = await page.evaluate((ids) => {
      const cy = (window as any).docair.state.cy;
      const part = cy.getElementById('k8s.svc.api');
      const bb = part.boundingBox({ includeLabels: false, includeOverlays: false });
      const mine = new Set(part.connectedEdges().map((e: any) => e.id()));
      return ids.filter((id: string) => !mine.has(id)).filter((id: string) => {
        const r = (window as any).__before[id].route;
        return !r.some((q: any, i: number) => i > 0 && Math.max(q.x, r[i - 1].x) > bb.x1 - 8 && Math.min(q.x, r[i - 1].x) < bb.x2 + 8 && Math.max(q.y, r[i - 1].y) > bb.y1 - 8 && Math.min(q.y, r[i - 1].y) < bb.y2 + 8);
      });
    }, Object.keys(after).filter((id) => JSON.stringify(after[id]) !== JSON.stringify(before[id])));
    expect(moved).toEqual([]);
  });
});

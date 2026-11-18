import { expect, test } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { docair, fixture, graphOf, openStatic, out } from './helpers';

test.beforeAll(() => {
  execFileSync(docair, ['scan', fixture('apis'), '--out', path.join(out, 'apis')]);
});

/***
 * API descriptions read next to the code: OpenAPI and AsyncAPI files meet the handlers and topics they describe (docs/IMPORTS.md).
 *
 * @id      e2e.apis
 * @kind    test
 * @part-of e2e
 ***/
test.describe('API descriptions', () => {
  /***
   * Declared routes merge into their handlers, callers' lines carry the description's words, and of a vendored description only the called operation is drawn.
   *
   * @part-of e2e.apis
   * @covers  docair-lang-apis.api-adapter.project "checks descriptions are read"
   * @covers  docair-core.declare.settle "checks operations merge into their handlers"
   * @covers  docair-core.declare.word_and_prune "checks callers carry the description's words"
   * @covers  docair-viewer.svg.draw "checks a description is drawn as a page with a folded corner"
   * @covers  docair-viewer.page_panels.look.details "checks the handler's details show the description's words"
   ***/
  test('descriptions meet the code', async ({ page }) => {
    const graph = graphOf('apis');
    const node = (id: string) => graph.nodes.find((n: any) => n.id === id);
    expect(node('api.openapi').kind).toBe('api');
    expect(node('api.openapi').part_of).toBe('orders-api');
    expect(node('api.openapi.create-order')).toBeUndefined();
    expect(node('api.openapi.cancel-order').kind).toBe('endpoint');
    expect(node('vendor.payments-openapi.create-charge')).toBeDefined();
    expect(node('vendor.payments-openapi.list-charges')).toBeUndefined();
    const line = graph.edges.find((e: any) => e.from === 'shop-ui.cart' && e.to === 'orders.create');
    expect(line.label).toBe('(CreateOrder) → Order');
    expect(line.from_api).toBe('api/openapi.yaml');
    // A description is drawn as a page with a folded corner.
    await openStatic(page, 'apis', '#box/orders-api');
    await expect(page.locator('#pic g.part.k-api[data-id="api.openapi"] .sh')).toHaveJSProperty('tagName', 'polygon');
    // The handler's details: the caller's line in the description's words, and the operation as a doc.
    await openStatic(page, 'apis', '#part/orders.create');
    await expect(page.locator('#details')).toContainText('(CreateOrder) → Order');
    await expect(page.locator('#details')).toContainText('Docs (1)');
  });

  /***
   * Where a description and the code disagree, the page lists the warnings: a renamed route, a route no code serves, one the description leaves out, an event no code receives.
   *
   * @part-of e2e.apis
   * @covers  docair-core.declare.settle "checks the disagreements become warnings"
   * @covers  docair-viewer.page_panels.panels "checks the diagnostics list names them"
   ***/
  test('disagreements are warnings', async ({ page }) => {
    const codes = graphOf('apis').diagnostics.map((d: any) => `${d.code} ${d.node}`).sort();
    expect(codes).toEqual(['W103 orders.update', 'W114 api.openapi.cancel-order', 'W114 mailer.asyncapi', 'W115 orders.list']);
    await openStatic(page, 'apis');
    await page.locator('#btn-diag').click();
    const rows = page.locator('#drawer .row[data-part]');
    await expect(rows).toHaveCount(4);
    await rows.filter({ hasText: 'W115' }).click();
    await expect(page).toHaveURL(/#part\/orders\.list$/);
  });
});

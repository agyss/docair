import { expect, test } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { copyOf, docair, fakeAiEnv, ready, withFakeAi } from './helpers';

/**
 * Two commits of the shop: after A, the checkout gets a new step, an audit entry, and the README says so;
 * the commit says what it does. With `alsoEdit`, the cart's total is edited too, a code edit no change explains.
 */
function twoCommits(alsoEdit = false): string {
  const shop = copyOf('shop');
  const git = (...args: string[]) =>
    execFileSync('git', ['-C', shop, '-c', 'user.name=docair test', '-c', 'user.email=test@example.invalid', ...args], { encoding: 'utf8' });
  git('init', '-q', '-b', 'main');
  git('add', '.');
  git('commit', '-q', '-m', 'a');
  const lib = path.join(shop, 'api', 'src', 'lib.rs');
  fs.writeFileSync(
    lib,
    fs.readFileSync(lib, 'utf8') +
      '\n/***\n * Keep an audit entry of the order.\n *\n * @id      orders.audit\n * @part-of orders-api\n * @flow    checkout 4 "the order is audited"\n ***/\npub fn audit() {}\n',
  );
  fs.appendFileSync(path.join(shop, 'README.md'), '\nEvery order is audited: the API keeps an entry of each one.\n');
  if (alsoEdit) {
    const cart = path.join(shop, 'ui', 'src', 'cart.ts');
    fs.writeFileSync(cart, fs.readFileSync(cart, 'utf8').replace('this.items.length * 100', 'this.items.length * 120'));
  }
  git('commit', '-q', '-am', 'Audit every order\n\nThe auditors want a trail of each order.');
  return shop;
}

/** The page of the shop with its changes since the commit before. */
function buildWithChanges(shop: string): string {
  const out = path.join(path.dirname(shop), 'out');
  execFileSync(docair, ['build', shop, '--out', out, '--changes', 'HEAD~1'], { stdio: ['ignore', 'pipe', 'pipe'] });
  return pathToFileURL(path.join(out, 'docair.html')).href;
}

/***
 * Changes in the page: `docair build --changes <A>` lists the flows the working tree changes since A, and each opens as its slice before and after, side by side.
 *
 * @id      e2e.changes
 * @kind    test
 * @part-of e2e
 ***/
test.describe('what changed since a commit', () => {
  /***
   * A new step in the checkout, committed after A: the page's Changes list the checkout as changed, and its one picture shows it before and after, the new step marked green and what stayed faded.
   *
   * @part-of e2e.changes
   * @covers  docair-viewer.page_changes.change_view "checks a changed flow is listed and drawn before and after"
   * @covers  docair-viewer.page_changes.review_view "checks the review says the AI writes it when there is none"
   * @covers  docair-viewer.page_changes.changes_walk "checks a changed flow opens on its change"
   * @covers  docair-viewer.doc_page.render_page_with "checks the page has the changes section"
   ***/
  test('a changed flow is listed and drawn before and after', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', (e) => errors.push(String(e)));
    await page.goto(buildWithChanges(twoCommits()));
    await ready(page);
    await expect(page.getByTestId('changes-button')).toContainText('Δ Changes · ');
    await page.getByTestId('changes-button').click();
    // No review yet: the AI writes it (docair-8l6.26); the flows are listed below.
    await expect(page.getByTestId('review-needs-ai')).toContainText('docair explain');
    const list = page.getByTestId('changes-list');
    await expect(list).toContainText('Scenario checkout');
    await list.getByRole('link', { name: 'Scenario checkout' }).click();
    await expect(page).toHaveURL(/#changes\/flow:checkout$/);
    const svg = page.locator('#pic svg.docair-pic.changes');
    await expect(svg).toHaveCount(1);
    expect(await svg.locator('g.part.ch-same').count()).toBeGreaterThan(2);
    await expect(svg.locator('g.part.ch-added[data-id="orders.audit"]')).toHaveCount(1);
    // It opens on its change, not on the whole flow: the walk over the changes, one step per change.
    await expect(page.getByTestId('step-card')).toContainText('change 1 of 1');
    await expect(page.locator('#pic g.part.flow-current[data-id="orders.audit"]')).toHaveCount(1);
    await expect(page.locator('#details')).toContainText('Now goes through');
    await expect(page.locator('#details')).toContainText('audit');
    // A change's picture is still, not frozen: a drag pans it, and nothing throws.
    await page.waitForTimeout(400);
    const view = await svg.getAttribute('viewBox');
    const pic = (await page.locator('#pic').boundingBox())!;
    await page.mouse.move(pic.x + pic.width - 60, pic.y + pic.height / 2);
    await page.mouse.down();
    await page.mouse.move(pic.x + pic.width - 160, pic.y + pic.height / 2 - 40, { steps: 5 });
    await page.mouse.up();
    expect(await svg.getAttribute('viewBox')).not.toBe(view);
    expect(errors).toEqual([]);
  });

  /***
   * Reviewed by the model (docair explain, docs/EXPLAIN.md §9): the changes list leads with the review's change, titled from the commit, the flows right after it, the cart's code edit, which no change explains, a folded other edit; the change opens on its own picture on what and why, headed by its title, then the how, its pieces with the model's words over docair's, the route after on the card, the new step lit; its details say why with the commit and the README as sources.
   *
   * @part-of e2e.changes
   * @covers  docair-viewer.page_changes.review_view "checks the review's change opens on its picture with its why and other edits"
   * @covers  docair-viewer.page_changes.changes_walk "checks the model's words show on the change's piece"
   ***/
  test('a reviewed change opens on its own picture, with why and other edits', async ({ page }) => {
    test.skip(!withFakeAi, 'needs the fake model');
    const shop = twoCommits(true);
    execFileSync(docair, ['explain', 'HEAD~1', '--root', shop], { env: { ...process.env, ...fakeAiEnv }, stdio: ['ignore', 'pipe', 'pipe'] });
    await page.goto(buildWithChanges(shop));
    await ready(page);
    await expect(page.getByTestId('changes-button')).toContainText('Δ Changes · 1');
    await page.getByTestId('changes-button').click();
    const review = page.getByTestId('review-list');
    await expect(review).toContainText('Audit every order: the flow now reaches orders.audit.');
    // The flows right after the changes; the other edits last, folded (docair-8l6.42).
    await expect(page.getByTestId('flows-head')).toBeVisible();
    await expect(page.getByTestId('changes-list')).toContainText('Scenario checkout');
    await expect(page.getByTestId('other-edits')).toContainText('total');
    const flowsFirst = await page.evaluate(() => {
      const f = document.querySelector('[data-test="flows-head"]');
      const o = document.querySelector('[data-test="other-edits-box"]');
      return !!f && !!o && !!(f.compareDocumentPosition(o) & Node.DOCUMENT_POSITION_FOLLOWING) && !o.hasAttribute('open');
    });
    expect(flowsFirst).toBe(true);
    await page.getByTestId('review-change').first().click();
    await expect(page).toHaveURL(/#review\/1$/);
    // It opens on what was done and why, headed by the change's title (docair-8l6.43) ...
    await expect(page.getByTestId('step-card')).toContainText('What and why');
    await expect(page.getByTestId('step-card')).toContainText('Audit every order');
    await expect(page.getByTestId('step-card').getByTestId('review-why')).toContainText('The commit says: Audit every order.');
    // ... then the how: the new step, and the cart's own block the model put with it.
    await page.getByTestId('step-card').locator('[data-step="1"]').click();
    await expect(page.getByTestId('step-card')).toContainText('How · 1 of 2');
    await expect(page.getByTestId('step-words')).toContainText('now reaches');
    await expect(page.getByTestId('route-after')).toContainText('audit');
    await expect(page.locator('#pic g.part.flow-current[data-id="orders.audit"]')).toHaveCount(1);
    await expect(page.locator('#details > [data-test="review-why"], #details h2 ~ [data-test="review-why"]').first()).toContainText('The commit says: Audit every order.');
    await expect(page.locator('[data-test="review-sources"]').first()).toContainText('doc:README.md');
    await expect(page.getByTestId('review-flows')).toContainText('Scenario checkout');
  });
});

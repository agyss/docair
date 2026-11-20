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
   * Reviewed by the model (docair explain, docs/EXPLAIN.md §9): the changes list leads with the review's change, titled from the commit, the flows right after it, then the cart's code edit, which the first request left, as a change of its own (every piece is a change, docair-8l6.63); the change opens on its own picture on what and why, headed by its title, then the how, its pieces with the model's words over docair's, the route after on the card, the new step lit; its details say why with the commit and the README as sources.
   *
   * @part-of e2e.changes
   * @covers  docair-viewer.page_changes.review_view "checks the review's change opens on its picture with its why, and a left piece is a change of its own"
   * @covers  docair-viewer.page_changes.changes_walk "checks the model's words show on the change's piece"
   * @covers  docair-viewer.page_changes.piece_code "checks a piece's parts link to their file and line"
   ***/
  test('a reviewed change opens on its own picture, with why, and every piece is a change', async ({ page }) => {
    test.skip(!withFakeAi, 'needs the fake model');
    const shop = twoCommits(true);
    execFileSync(docair, ['explain', 'HEAD~1', '--root', shop], { env: { ...process.env, ...fakeAiEnv }, stdio: ['ignore', 'pipe', 'pipe'] });
    await page.goto(buildWithChanges(shop));
    await ready(page);
    await expect(page.getByTestId('changes-button')).toContainText('Δ Changes · 2');
    await page.getByTestId('changes-button').click();
    const review = page.getByTestId('review-list');
    await expect(review).toContainText('Audit every order: the flow now reaches orders.audit.');
    // Every piece is a change (docair-8l6.63): the cart's edit, which the first request left, is one of its own.
    await expect(review.getByTestId('review-change')).toHaveText([/^Audit every order/, 'An item in the cart costs more.']);
    await expect(page.getByTestId('other-edits-box')).toHaveCount(0);
    // The flows right after the changes (docair-8l6.42).
    await expect(page.getByTestId('flows-head')).toBeVisible();
    await expect(page.getByTestId('changes-list')).toContainText('Scenario checkout');
    // Every flow of the system, folded under them (docair-8l6.57): checkout marked, to its change.
    const allFlows = page.getByTestId('all-flows');
    await expect(allFlows.locator('summary')).toContainText(/All \d+ flows of the system/);
    await allFlows.locator('summary').click();
    await expect(allFlows).toContainText('Scenarios ·');
    await expect(allFlows.locator('a[href="#slice/flow:checkout"]')).toHaveText('checkout');
    await expect(allFlows.locator('a[href="#changes/flow:checkout"]')).toBeVisible();
    await page.getByTestId('review-change').first().click();
    await expect(page).toHaveURL(/#review\/1$/);
    // It opens on what was done and why, headed by the change's title (docair-8l6.43) ...
    await expect(page.getByTestId('step-card')).toContainText('What and why');
    await expect(page.getByTestId('step-card')).toContainText('Audit every order');
    // The AI's own why, and what it found off, on the card (docair-8l6.55).
    await expect(page.getByTestId('step-card').getByTestId('review-why')).toContainText('Checkout now goes one step further');
    // What it read for it, folded: the commit and the README (docair-8l6.71).
    await expect(page.getByTestId('step-card').getByTestId('review-sources').locator('summary')).toHaveText('Sources · 2');
    // Found off by kind, each under its heading, the clear contradictions first (docair-8l6.62).
    const groups = page.getByTestId('step-card').getByTestId('off-group');
    await expect(groups.locator('h5')).toHaveText([/^Said, but the code does otherwise/, /^Done, but said nowhere/, /^Looks wrong or risky/]);
    // A contradiction: what was said, and by what, over what the code does (docair-8l6.58).
    const offs = page.getByTestId('step-card').locator('[data-test="off-item"][data-kind="contradicts"]');
    await expect(offs.first().getByTestId('off-said')).toHaveText('The commit “Audit every order”: Every order is audited.');
    await expect(offs.first().getByTestId('off-code')).toContainText('Only checkout reaches orders.audit.');
    // Where the code does it, a link to the part (docair-8l6.61) ...
    await expect(offs.first().getByTestId('off-parts').locator('.node-ref')).toHaveAttribute('data-node', 'orders.audit');
    // What nothing mentions, and a risk: what the code does, nothing said.
    const unsaid = page.getByTestId('step-card').locator('[data-test="off-item"][data-kind="unsaid"]');
    await expect(unsaid.getByTestId('off-said')).toHaveCount(0);
    await expect(unsaid.getByTestId('off-code')).toContainText('nothing announces it');
    const risk = page.getByTestId('step-card').locator('[data-test="off-item"][data-kind="risk"]');
    await expect(risk.getByTestId('off-said')).toHaveCount(0);
    await expect(risk.getByTestId('off-code')).toContainText('Nothing tests orders.audit.');
    // ... and where its code is: the cart's file, after as a link to the working tree (docair-8l6.82).
    await expect(unsaid.getByTestId('piece-code').getByTestId('code-link')).toHaveAttribute('href', /^vscode:\/\/file\/.*\/ui\/src\/cart\.ts:\d+$/);
    await expect(unsaid.locator('.code-diff')).toHaveCount(0);
    await expect(page.getByTestId('step-card')).not.toContainText('commit:');
    // The details below hold only what the card does not (docair-8l6.54).
    await expect(page.getByTestId('review-meta')).toContainText('change 1 of 2');
    await expect(page.locator('[data-test="review-why"]')).toHaveCount(1);
    // ... then the how: the new step, and the cart's own block the model put with it.
    await page.getByTestId('step-card').locator('[data-step="1"]').click();
    await expect(page.getByTestId('step-card')).toContainText('How · 1 of 2');
    await expect(page.getByTestId('step-words')).toContainText('now reaches');
    await expect(page.getByTestId('route-after')).toContainText('audit');
    // The new step's code on its card: a link to where the new function is.
    const auditCode = page.getByTestId('step-card').getByTestId('piece-code');
    await expect(auditCode.getByTestId('code-part')).toContainText('new');
    await expect(auditCode.getByTestId('code-link')).toHaveAttribute('href', /^vscode:\/\/file\/.*:\d+$/);
    await expect(page.locator('#pic g.part.flow-current[data-id="orders.audit"]')).toHaveCount(1);
    await expect(page.getByTestId('review-flows')).toContainText('Scenario checkout');
    // The cart's change, down to the code (docair-8l6.49, 8l6.82): where it is, before and after.
    await page.evaluate(() => (location.hash = '#review/2'));
    await expect(page.getByTestId('step-card')).toContainText('An item in the cart costs more.');
    await expect(page.getByTestId('step-card').getByTestId('review-why')).toContainText('The price of an item went up.');
    await page.getByTestId('step-card').locator('[data-step="1"]').click();
    await expect(page.getByTestId('step-words')).toContainText('Each item now counts for more in the total.');
    const cartCode = page.getByTestId('step-card').getByTestId('piece-code').filter({ hasText: 'edited' }).first();
    // No remote in the test repository: before is its file and line, after a link to the working tree.
    await expect(cartCode).toContainText('ui/src/cart.ts:');
    await expect(cartCode.getByTestId('code-link')).toHaveText(/^after, ui\/src\/cart\.ts:\d+$/);
  });
});

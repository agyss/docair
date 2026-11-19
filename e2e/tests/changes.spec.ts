import { expect, test } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { copyOf, docair, fakeAiEnv, ready, withFakeAi } from './helpers';

/** Two commits of the shop: after A, the checkout gets a new step, an audit entry. */
function twoCommits(): string {
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
  git('commit', '-q', '-am', 'b');
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
   * A new step in the checkout, committed after A: the page's Changes list the checkout as changed, and its picture shows it before and after, the new step marked.
   *
   * @part-of e2e.changes
   * @covers  docair-viewer.page_changes.change_view "checks a changed flow is listed and drawn before and after"
   * @covers  docair-viewer.doc_page.render_page_with "checks the page has the changes section"
   ***/
  test('a changed flow is listed and drawn before and after', async ({ page }) => {
    await page.goto(buildWithChanges(twoCommits()));
    await ready(page);
    await expect(page.getByTestId('changes-button')).toContainText('Δ Changes · ');
    await page.getByTestId('changes-button').click();
    const list = page.getByTestId('changes-list');
    await expect(list).toContainText('Scenario checkout');
    await list.getByRole('link', { name: 'Scenario checkout' }).click();
    await expect(page).toHaveURL(/#changes\/flow:checkout$/);
    const svg = page.locator('#pic svg.docair-pic.changes');
    await expect(svg.locator('.side-title')).toHaveText([/^Before · /, /^After · /]);
    await expect(svg.locator('g.part.ch-added[data-id="orders.audit"]')).toHaveCount(1);
    await expect(page.locator('#details')).toContainText('Now goes through');
    await expect(page.locator('#details')).toContainText('audit');
  });

  /***
   * Explained by the model (docair explain), the checkout's change shows the explanation in its details, and its workflow after the change is walked through on its side: a step lights the part it is in.
   *
   * @part-of e2e.changes
   * @covers  docair-viewer.page_changes.workflow_walk "checks a workflow is walked through on its side"
   ***/
  test("an explained flow shows the model's words and walks its workflow", async ({ page }) => {
    test.skip(!withFakeAi, 'needs the fake model');
    const shop = twoCommits();
    execFileSync(docair, ['explain', 'HEAD~1', '--root', shop], { env: { ...process.env, ...fakeAiEnv }, stdio: ['ignore', 'pipe', 'pipe'] });
    await page.goto(buildWithChanges(shop) + '#changes/flow:checkout');
    await ready(page);
    await expect(page.getByTestId('explanation')).toContainText('changed, so this flow goes another way.');
    await page.getByTestId('workflow-after').click();
    await expect(page).toHaveURL(/#changes\/flow:checkout\/after$/);
    await expect(page.getByTestId('step-card')).toContainText('step 1 of 1');
    await expect(page.locator('#pic g.flow-current[data-id="orders.audit"]')).toHaveCount(1);
  });
});

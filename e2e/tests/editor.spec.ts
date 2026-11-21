import { expect, test } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { copyOf, serveFolder } from './helpers';

/** The shop in git: commit A, then the audit step committed on top. */
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
  git('commit', '-q', '-am', 'Audit every order');
  return shop;
}

/***
 * The page in the editor (docs/EDITOR.md §4, §5): `docair serve` is told which two versions to compare while it runs, and the page in a frame, with `?editor` in its address, sends a place in the code to the frame around it instead of following its link.
 *
 * @id      e2e.editor
 * @kind    test
 * @part-of e2e
 ***/
test.describe('the page in the editor', () => {
  /***
   * POST /api/changes compares HEAD~1 with the working tree while serve runs; framed as the VS Code panel frames it, the page posts docair:open with the part's place when its source is clicked.
   *
   * @part-of e2e.editor
   * @covers  docair-server.project.compare "checks serve compares the pair it is given while it runs"
   * @covers  docair-server.project.changes "checks the pair is read back once it is compared"
   * @covers  docair-viewer.doc-page.popups "checks a place in the code is posted to the frame around the page"
   ***/
  test('a place in the code is sent to the frame around the page', async ({ page }) => {
    const served = await serveFolder(twoCommits());
    try {
      const set = await fetch(`${served.url}/api/changes`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ old: 'HEAD~1' }),
      });
      expect(set.status).toBe(202);
      await expect
        .poll(async () => (await (await fetch(`${served.url}/api/changes`)).json()).comparing, { timeout: 60_000 })
        .toBe(false);
      const now = await (await fetch(`${served.url}/api/changes`)).json();
      expect([now.old, now.new, now.error]).toEqual(['HEAD~1', null, null]);

      // The frame around the page, as the extension's panel holds it.
      await page.setContent(
        `<iframe id="docair" src="${served.url}/?editor=vscode#part/orders.create" style="width:1400px;height:900px"></iframe>` +
          `<script>window.got = []; addEventListener('message', (e) => e.data?.type === 'docair:open' && window.got.push(e.data));</script>`,
      );
      const frame = page.frameLocator('#docair');
      await expect(frame.locator('body[data-editor="true"][data-ready="true"]')).toBeAttached();
      await expect(frame.getByTestId('changes-button')).toBeVisible();
      await expect(frame.locator('#details .meta')).toContainText('orders.create');
      const source = frame.locator('#details code[data-open]');
      await expect(source).toContainText('api/src/lib.rs:');
      await source.click();
      await expect.poll(() => page.evaluate(() => (window as any).got.length)).toBe(1);
      const sent = await page.evaluate(() => (window as any).got[0]);
      expect(sent.old).toBeNull();
      expect(sent.new.rev).toBeNull();
      expect(sent.new.path).toBe('api/src/lib.rs');
      expect(sent.new.line).toBeGreaterThan(0);
    } finally {
      await served.stop();
    }
  });
});

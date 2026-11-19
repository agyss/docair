import { expect, test } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { copyOf, docair, ready, serve } from './helpers';

/***
 * Tidy in the page: the tags the code repeats are counted in the header, announced under it and listed with the code that says the same; docair serve tidies them, and a page opened as a file offers the command.
 *
 * @id      e2e.tidy
 * @kind    test
 * @part-of e2e
 ***/
test.describe('tidy the tags the code repeats', () => {
  /***
   * A page opened as a file counts the repeated tag, lists it in the panel and offers the command; what needs docair serve stays hidden. A closed banner stays closed until the count changes.
   *
   * @part-of e2e.tidy
   * @covers  docair-viewer.page_tidy.tidy "checks the button, the banner and the panel of a page opened as a file"
   ***/
  test('a page opened as a file counts them and offers the command', async ({ page }) => {
    const folder = copyOf('tidy');
    const out = path.join(path.dirname(folder), 'out');
    execFileSync(docair, ['scan', folder, '--out', out]);
    await page.goto(pathToFileURL(path.join(out, 'docair.html')).href);
    await ready(page);
    await expect(page.getByTestId('tidy-button')).toHaveText('✂ Tidy · 1');
    await expect(page.getByTestId('tidy-banner')).toContainText('1 tag repeats what the code says');
    await page.getByTestId('tidy-review').click();
    await expect(page.getByTestId('tidy-list')).toContainText('@calls app.load');
    await expect(page.getByTestId('tidy-list')).toContainText('the code says');
    await expect(page.getByTestId('tidy-apply')).toBeHidden();
    await expect(page.getByTestId('tidy-copy')).toBeVisible();
    await page.locator('[data-tidy-close]').click();
    await expect(page.getByTestId('tidy-banner')).toBeHidden();
    await page.reload();
    await ready(page);
    await expect(page.getByTestId('tidy-banner')).toBeHidden();
  });

  /***
   * Served by docair serve, the panel shows the patch and tidies the ticked tag: the file loses it, and the page, written again, says nothing repeats.
   *
   * @part-of e2e.tidy
   * @covers  docair-viewer.page_tidy.tidy "checks a served page tidies the ticked tags"
   * @covers  docair-server.project.tidy_apply "checks the server tidies what the page sends"
   ***/
  test('docair serve tidies the ticked tags, and then nothing repeats', async ({ page }) => {
    const served = await serve('tidy');
    try {
      await page.goto(`${served.url}/`);
      await ready(page);
      await page.getByTestId('tidy-button').click();
      await expect(page.getByTestId('tidy-apply')).toBeVisible();
      await expect(page.getByTestId('tidy-diff')).toContainText('@calls   app.load');
      const reloaded = page.waitForEvent('load');
      await page.getByTestId('tidy-apply').click();
      await reloaded;
      await ready(page);
      await expect(page.getByTestId('tidy-button')).toHaveText('✂ Tidy ✓');
      await expect(page.getByTestId('tidy-banner')).toHaveCount(0);
      expect(fs.readFileSync(path.join(served.folder, 'src', 'lib.rs'), 'utf8')).not.toContain('@calls');
    } finally {
      await served.stop();
    }
  });
});

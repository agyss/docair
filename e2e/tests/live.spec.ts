/***
 * The page follows each save (docs/REGENERATE.md): docair serve writes the page again after a file
 * changes, from the pictures the change touched; the page learns of it at once and opens again where
 * the reader was; and `docair build --from-serve` takes serve's layouts as its base.
 *
 * @id      e2e.live
 * @kind    test
 * @part-of e2e
 * @calls   e2e.serve "serves a copy of the shop"
 ***/
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { docair, ready, serve } from './helpers';

test('a save shows on the page within seconds, where the reader was', async ({ page }) => {
  const served = await serve('shop');
  try {
    await page.goto(`${served.url}/#box/orders-api`);
    await ready(page);
    // The reader picks a part and zooms in on the picture.
    await page.locator('#pic .part[data-id="orders.create"]').click();
    await expect(page.locator('#pic .part.sel[data-id="orders.create"]')).toBeAttached();
    const pic = page.locator('#pic');
    const box = (await pic.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.wheel(0, -400);
    const width = () => page.evaluate(() => Number(document.querySelector('#pic > svg')!.getAttribute('viewBox')!.split(' ')[2]));
    const zoomed = await width();

    // A save: a doc block's words in the shop's README.
    const reloaded = page.waitForEvent('load', { timeout: 15_000 });
    const readme = path.join(served.folder, 'README.md');
    const saved = Date.now();
    fs.writeFileSync(readme, fs.readFileSync(readme, 'utf8').replace('The shopper, in a browser.', 'The shopper, in a browser, who buys.'));
    await reloaded;
    await ready(page);
    expect(Date.now() - saved, 'the page shows the save within seconds').toBeLessThan(10_000);

    // Where the reader was: the same picture, the part still picked, the zoom as it was.
    expect(page.url()).toContain('#box/orders-api');
    await expect(page.locator('#pic .part.sel[data-id="orders.create"]')).toBeAttached();
    expect(Math.abs((await width()) - zoomed) / zoomed).toBeLessThan(0.05);

    // serve's layouts, kept on disk, are the base of a build of the same code: every picture taken.
    expect(fs.existsSync(path.join(served.folder, '.docair', 'serve', 'state.json'))).toBe(true);
    const said = execFileSync(docair, ['build', served.folder, '--from-serve'], { encoding: 'utf8' });
    expect(said).toMatch(/from docair serve's layouts: \d+ pictures taken as they were, 0 updated to the code, 0 laid out/);
  } finally {
    await served.stop();
  }
});

test('lay out again writes the picture again from scratch', async ({ page }) => {
  const served = await serve('shop');
  try {
    await page.goto(`${served.url}/#layer/apps`);
    await ready(page);
    const button = page.getByTestId('relayout');
    await expect(button).toBeVisible();
    const reloaded = page.waitForEvent('load', { timeout: 15_000 });
    await button.click();
    await reloaded;
    await ready(page);
    expect(page.url()).toContain('#layer/apps');
  } finally {
    await served.stop();
  }
});

import { expect, test } from '@playwright/test';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fixture, generate } from './helpers';

/***
 * Generate the docs: a project folder goes into the web app, a docair page comes out.
 *
 * @id      e2e.generate
 * @kind    test
 * @part-of e2e
 ***/
test.describe('generate the docs', () => {
  /***
   * A picked folder is uploaded, scanned by the server and shown as a page that fills the window.
   *
   * @part-of e2e.generate
   * @covers  docair-web.upload-panel "checks a picked folder uploads"
   * @covers  docair-web.app.generate "checks the page appears after upload"
   * @covers  docair-server.scan_upload "checks the server scans the uploaded files"
   * @covers  docair-server.run_scan "checks the pipeline renders the page"
   * @covers  docair-viewer.doc_page.render_page "checks the page docair wrote fills the window"
   ***/
  test('a picked folder becomes a page', async ({ page }) => {
    await generate(page, fixture('shop'));
    await expect(page.locator('.stats')).toContainText('17 nodes');
    await expect(page.locator('.stats')).toContainText('0 errors');
    const box = await page.getByTestId('page').boundingBox();
    expect(box!.height).toBeGreaterThan(page.viewportSize()!.height - 70);
  });

  /***
   * A generation says where it is, step by step: the picked files read, the upload in bytes and percent,
   * docair reading the files on the server, and the page coming back; the page, drawn by docair already,
   * says when it is ready.
   *
   * @part-of e2e.generate
   * @covers  docair-web.progress "checks each step of a generation shows how far it is"
   * @covers  docair-web.app.generate "checks the steps follow each other"
   * @covers  docair-web.upload-panel.take "checks the picked files are read first"
   * @covers  docair-web.result-view "checks the frame says when the page is ready"
   ***/
  test('each step of a generation shows how far it is', async ({ page }) => {
    // Every line the user sees, in order.
    await page.addInitScript(() => {
      const w = window as any;
      w.__seen = [];
      const look = () => {
        const now = [...document.querySelectorAll<HTMLElement>('[data-test="progress"]')].map((e) => `${e.dataset['phase']}: ${e.textContent!.replace(/\s+/g, ' ').trim()}`).join(' | ');
        if (now !== w.__seen[w.__seen.length - 1]) w.__seen.push(now);
      };
      new MutationObserver(look).observe(document, { subtree: true, childList: true, characterData: true, attributes: true });
    });
    // The shop with some notes, so the upload has a size worth showing.
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docair-e2e-'));
    const shop = path.join(dir, 'shop');
    fs.cpSync(fixture('shop'), shop, { recursive: true });
    fs.writeFileSync(path.join(shop, 'docs', 'notes.md'), `# Notes\n\n${'The shop keeps its notes here, so that the upload has some size.\n'.repeat(9000)}`);
    await page.goto('/');
    await page.getByTestId('folder-input').setInputFiles(shop);
    const files = (await page.getByTestId('picked').innerText()).match(/(\d+) files/)![1];
    // A slow network, so that each step lasts long enough to be seen.
    const cdp = await page.context().newCDPSession(page);
    await cdp.send('Network.enable');
    await cdp.send('Network.emulateNetworkConditions', { offline: false, latency: 150, uploadThroughput: 200 * 1024, downloadThroughput: 2 * 1024 * 1024 });
    await page.getByTestId('generate').click();
    await expect(page.getByTestId('progress')).toContainText(new RegExp(`1 of 3 Uploading ${files} files · \\d+ kB of \\d+ kB · [1-9]\\d?%`));
    await expect(page.getByTestId('result')).toBeVisible();
    await expect(page.frameLocator('[data-test="page"]').locator('body[data-ready="true"]')).toBeAttached();
    await expect(page.getByTestId('frame')).toHaveAttribute('data-ready', 'true');
    const seen: string[] = await page.evaluate(() => (window as any).__seen.filter(Boolean));
    const phases = seen.map((line) => line.split(':')[0]).filter((p, i, all) => p !== all[i - 1]);
    // The picked files are read into memory first, so a file changed on disk later cannot break the upload.
    expect(phases).toEqual(['read', 'upload', 'scan', 'download']);
    expect(seen).toContainEqual(expect.stringMatching(/^read: Reading the folder · \d+ files so far/));
    expect(seen).toContainEqual(expect.stringMatching(new RegExp(`^scan: 2 of 3 docair reads the ${files} files on the server`)));
    expect(seen).toContainEqual(expect.stringMatching(/^download: 3 of 3 Receiving the page · [\d.]+ (kB|MB) of [\d.]+ (kB|MB) · \d+%/));
    fs.rmSync(dir, { recursive: true, force: true });
  });

  /***
   * `.env` and Terraform state stay on the machine: the upload panel leaves them out.
   *
   * @part-of e2e.generate
   * @covers  docair-web.upload-panel "checks secret files never upload"
   ***/
  test('secret files are never uploaded', async ({ page }) => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docair-e2e-'));
    const shop = path.join(dir, 'shop');
    fs.cpSync(fixture('shop'), shop, { recursive: true });
    fs.writeFileSync(path.join(shop, '.env'), 'DOCAIR_AI_API_KEY=not-for-upload\n');
    fs.writeFileSync(path.join(shop, 'infra', 'terraform.tfstate'), '{"secret": true}\n');
    await page.goto('/');
    await page.getByTestId('folder-input').setInputFiles(shop);
    await expect(page.getByTestId('picked')).toContainText('12 files');
    await expect(page.getByTestId('picked')).toContainText('2 left out');
    await page.getByTestId('generate').click();
    await expect(page.locator('.stats li').first()).toHaveText('12 files');
    fs.rmSync(dir, { recursive: true, force: true });
  });

  /***
   * New upload leaves the page and goes back to the upload panel.
   *
   * @part-of e2e.generate
   * @covers  docair-web.app.reset "checks a new upload clears the page"
   * @covers  docair-web.result-view "checks the old page closes"
   ***/
  test('New upload goes back to the upload page', async ({ page }) => {
    await generate(page, fixture('shop'));
    await page.getByTestId('new-upload').click();
    await expect(page.locator('app-upload-panel')).toBeVisible();
    await expect(page.getByTestId('result')).toHaveCount(0);
  });
});

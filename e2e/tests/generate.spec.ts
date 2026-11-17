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
   * @covers  docair-viewer.page "checks the page fills the window"
   ***/
  test('a picked folder becomes a page', async ({ page }) => {
    await generate(page, fixture('shop'));
    await expect(page.locator('.stats')).toContainText('17 nodes');
    await expect(page.locator('.stats')).toContainText('0 errors');
    const box = await page.getByTestId('page').boundingBox();
    expect(box!.height).toBeGreaterThan(page.viewportSize()!.height - 70);
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
    await expect(page.getByTestId('picked')).toContainText('11 files');
    await expect(page.getByTestId('picked')).toContainText('2 left out');
    await page.getByTestId('generate').click();
    await expect(page.locator('.stats li').first()).toHaveText('11 files');
    fs.rmSync(dir, { recursive: true, force: true });
  });

  /***
   * New upload leaves the page and the conversation, and goes back to the upload panel.
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

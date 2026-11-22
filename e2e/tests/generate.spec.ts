import { expect, test } from '@playwright/test';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fixture, generate } from './helpers';

/** Clicks on links inside the framed page, which must move inside the page, not away from it. */
async function framedLinksStay(page: import('@playwright/test').Page, how: 'src' | 'srcdoc') {
    if (how === 'srcdoc') {
      // A server that keeps no upload: the frame gets the page as a srcdoc.
      await page.route('**/api/scan', async (route) => {
        const res = await route.fetch();
        const body = await res.json();
        delete body.page;
        await route.fulfill({ response: res, json: body });
      });
    }
    await generate(page, fixture('shop'));
    const f = page.frameLocator('[data-test="page"]');
    const frame = () => page.frames().find((x) => x !== page.mainFrame())!;
    await frame().evaluate(() => (location.hash = '#part/shop-ui.cart'));
    await expect(f.locator('#details .meta')).toContainText('shop-ui.cart');
    // A plain link to a part in the details.
    await f.locator('#details a.link[title="orders.create"]').first().click();
    await expect(f.locator('#details .meta')).toContainText('orders.create');
    await expect(f.locator('body[data-ready="true"]')).toBeAttached();
    await expect(f.locator('#pic svg.docair-pic')).toBeAttached();
    // A code span that names a part, in a glossary popup's entry.
    await frame().evaluate(() => (location.hash = '#part/shop-ui.cart'));
    await f.locator('#details .term[data-term="order"]').click();
    await f.locator('#details code.node-ref', { hasText: 'orders.create' }).click();
    await expect(f.locator('#details h2')).toContainText('create_order');
    await expect(page.getByTestId('result')).toBeVisible();
    expect(await page.locator('app-upload-panel').count()).toBe(0);
}

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
   * What git ignores stays out of an upload, as out of the CLI's scan: the rules of every folder's
   * `.gitignore`, each for its own folder, and `.git/info/exclude`; nothing in a folder left out
   * comes back, and a `!` rule brings a file back where git would.
   *
   * @part-of e2e.generate
   * @covers  docair-web.upload-panel.take "checks what git ignores is left out"
   ***/
  test('what git ignores is not uploaded', async ({ page }) => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docair-e2e-'));
    const shop = path.join(dir, 'shop');
    fs.cpSync(fixture('shop'), shop, { recursive: true });
    const put = (rel: string, text: string) => {
      fs.mkdirSync(path.dirname(path.join(shop, rel)), { recursive: true });
      fs.writeFileSync(path.join(shop, rel), text);
    };
    // test-results/ is ignored too, but the shop's docair.toml names its results.json: it stays.
    put('.gitignore', '/backup/\n*.log\n!keep.log\ntest-results/\n');
    put('backup/old.rs', '/*** Old. ***/\npub fn old() {}\n');
    put('backup/.gitignore', '!old.rs\n');
    put('ui/.gitignore', 'out/\n');
    put('ui/out/bundle.ts', 'export const x = 1;\n');
    put('.git/info/exclude', '/copy/\n');
    put('copy/api/src/lib.rs', fs.readFileSync(path.join(shop, 'api', 'src', 'lib.rs'), 'utf8'));
    put('debug.log', 'left out\n');
    put('keep.log', 'kept\n');
    await page.goto('/');
    await page.getByTestId('folder-input').setInputFiles(shop);
    // The 12 files of the shop, its two .gitignore files and keep.log.
    await expect(page.getByTestId('picked')).toContainText('15 files');
    await expect(page.getByTestId('picked')).toContainText('6 left out');
    await page.getByTestId('generate').click();
    await expect(page.locator('.stats li').first()).toHaveText('15 files');
    fs.rmSync(dir, { recursive: true, force: true });
  });

  /***
   * A dropped folder is walked by the page, which goes into no folder its rules leave out: a big
   * folder git ignores costs nothing, unlike a chosen folder, which the browser lists in full.
   * Playwright cannot drop a real folder, so the drop carries entries made in the page, which
   * note each folder the walk reads.
   *
   * @part-of e2e.generate
   * @covers  docair-web.upload-panel.drop "checks a dropped folder walks no ignored folder"
   ***/
  test('a dropped folder walks no folder git ignores', async ({ page }) => {
    await page.goto('/');
    await expect(page.getByTestId('drop-hint')).toContainText('Drag its folder in');
    await page.evaluate(() => {
      const files: Record<string, string> = {
        'shop/README.md': '# Shop\n',
        'shop/api/src/lib.rs': '/*** Orders. ***/\npub fn create_order() {}\n',
        'shop/ui/src/cart.ts': 'export const cart = 1;\n',
        'shop/.gitignore': '/backup/\ntest-results/\n',
        'shop/docair.toml': '[tests]\nresults = ["test-results/results.json"]\n',
        'shop/test-results/results.json': '{}\n',
        'shop/backup/old.rs': 'pub fn old() {}\n',
        'shop/ui/.gitignore': 'out/\n',
        'shop/ui/out/bundle.ts': 'export const x = 1;\n',
        'shop/.git/HEAD': 'ref: refs/heads/main\n',
        'shop/.git/info/exclude': '/copy/\n',
        'shop/copy/api/src/lib.rs': 'pub fn create_order() {}\n',
      };
      const dirs = new Set<string>();
      for (const p of Object.keys(files)) {
        const parts = p.split('/');
        for (let k = 1; k < parts.length; k++) dirs.add(parts.slice(0, k).join('/'));
      }
      const read: string[] = [];
      (window as unknown as { read: string[] }).read = read;
      const entry = (path: string): unknown => {
        const name = path.split('/').pop()!;
        if (!dirs.has(path)) {
          return { isFile: true, isDirectory: false, name, fullPath: `/${path}`, file: (ok: (f: File) => void) => ok(new File([files[path]], name)) };
        }
        const kids = [...Object.keys(files), ...dirs].filter((p) => p.startsWith(`${path}/`) && !p.slice(path.length + 1).includes('/'));
        return {
          isFile: false,
          isDirectory: true,
          name,
          fullPath: `/${path}`,
          createReader: () => {
            read.push(path);
            let done = false;
            return { readEntries: (ok: (e: unknown[]) => void) => ok(done ? [] : ((done = true), [...new Set(kids)].map(entry))) };
          },
          getFile: (rel: string, _: unknown, ok: (e: unknown) => void, fail: (e: unknown) => void) =>
            `${path}/${rel}` in files ? ok(entry(`${path}/${rel}`)) : fail(new DOMException('not found', 'NotFoundError')),
        };
      };
      const drop = new Event('drop', { bubbles: true, cancelable: true });
      Object.defineProperty(drop, 'dataTransfer', { value: { items: [{ webkitGetAsEntry: () => entry('shop') }] } });
      document.querySelector('app-upload-panel section.drop')!.dispatchEvent(drop);
    });
    // README, the API, the cart, the two .gitignore files, docair.toml and the results it names;
    // .git/info/exclude is read, not sent.
    await expect(page.getByTestId('picked')).toContainText('7 files');
    await expect(page.getByTestId('picked')).toContainText('1 left out');
    const read = await page.evaluate(() => (window as unknown as { read: string[] }).read.sort());
    expect(read).toEqual(['shop', 'shop/api', 'shop/api/src', 'shop/test-results', 'shop/ui', 'shop/ui/src']);
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

  /***
   * A link clicked inside the framed page moves inside the page: a part's link in the details, a code span naming a part, with the page loaded by its address (docair-sra).
   *
   * @part-of e2e.generate
   * @covers  docair-web.result-view "checks clicks inside the frame stay in the page"
   * @covers  docair-viewer.doc-page.move "checks fragment links in a framed page"
   ***/
  test('a click on a link in the framed page stays in the page (src)', async ({ page }) => {
    await framedLinksStay(page, 'src');
  });

  /***
   * The same with the page given to the frame as a srcdoc, as a server that keeps no upload does.
   *
   * @part-of e2e.generate
   * @covers  docair-web.result-view "checks clicks inside a srcdoc frame stay in the page"
   ***/
  test('a click on a link in the framed page stays in the page (srcdoc)', async ({ page }) => {
    await framedLinksStay(page, 'srcdoc');
  });

});

import { Page, expect } from '@playwright/test';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

export const root = path.resolve(__dirname, '..', '..');
export const docair = path.join(root, 'target', 'release', 'docair');
export const out = path.resolve(__dirname, '..', '.out');
export const fixture = (name: string) => path.resolve(__dirname, '..', 'fixtures', name);
export const fakeAi = 'http://127.0.0.1:9555';
/** The tests that need the fake model run only against the local server. */
export const withFakeAi = !process.env.E2E_BASE_URL;

/** Open a generated page from global-setup and wait until the viewer is ready. */
export async function openStatic(page: Page, name: string): Promise<void> {
  await page.goto(pathToFileURL(path.join(out, name, 'docair.html')).href);
  await expect(page.locator('body[data-ready="true"]')).toBeAttached();
}

/** Upload a folder in the web app and wait for the generated page. */
export async function generate(page: Page, folder: string): Promise<void> {
  await page.goto('/');
  await page.getByTestId('folder-input').setInputFiles(folder);
  await page.getByTestId('generate').click();
  await expect(page.getByTestId('result')).toBeVisible();
  await expect(page.frameLocator('[data-test="page"]').locator('body[data-ready="true"]')).toBeAttached();
}

/** The ids of the nodes drawn on the canvas (no edge labels, no tokens). */
export function drawn(page: Page | import('@playwright/test').Frame): Promise<string[]> {
  return page.evaluate(() =>
    (window as any).docair.state.cy
      .nodes()
      .filter((n: any) => !n.hasClass('elabel') && !n.hasClass('token'))
      .map((n: any) => n.id()),
  );
}

/** Which of these parts are not hidden (by a section, or the tests switch). */
export function shown(page: Page, ids: string[]): Promise<string[]> {
  return page.evaluate((list) => list.filter((id) => !(window as any).docair.hiddenNode(id)), ids);
}

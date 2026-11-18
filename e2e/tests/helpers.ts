import { Page, expect } from '@playwright/test';
import { ChildProcess, spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

export const root = path.resolve(__dirname, '..', '..');
export const docair = path.join(root, 'target', 'release', 'docair');
export const out = path.resolve(__dirname, '..', '.out');
export const fixture = (name: string) => path.resolve(__dirname, '..', 'fixtures', name);
/** The fake model (fake-ai.mjs); DOCAIR_E2E_FAKE_AI points the tests at another one. */
export const fakeAi = process.env.DOCAIR_E2E_FAKE_AI ?? 'http://127.0.0.1:9555';
/** The tests that need the fake model run only against the local server. */
export const withFakeAi = !process.env.E2E_BASE_URL;
/** The AI settings that point docair at the fake model. */
export const fakeAiEnv = { DOCAIR_AI_ENDPOINT: `${fakeAi}/v1`, DOCAIR_AI_MODEL: 'fake-model', DOCAIR_AI_API_KEY: 'fake-key' };

/** Open a page that docair wrote (global-setup, or a test's own scan), at an address, and wait until its picture shows. */
export async function openStatic(page: Page, name: string, address = ''): Promise<void> {
  await page.goto(pathToFileURL(path.join(out, name, 'docair.html')).href + address);
  await ready(page);
}

/** Wait until the page shows its first picture. */
export async function ready(page: Page): Promise<void> {
  await expect(page.locator('body[data-ready="true"]')).toBeAttached();
  await expect(page.locator('#pic svg.docair-pic')).toBeAttached();
}

/** Go to an address on the page that is open, as a link would. */
export async function go(page: Page, address: string): Promise<void> {
  await page.evaluate((a) => (location.hash = a), address);
}

/** Pick a part by its address, and wait until its details show. */
export async function select(page: Page, id: string): Promise<void> {
  await go(page, `#part/${id}`);
  await expect(page.locator('#details .meta')).toContainText(id);
}

/** The graph docair wrote next to a page. */
export function graphOf(name: string): any {
  return JSON.parse(fs.readFileSync(path.join(out, name, 'docair.graph.json'), 'utf8'));
}

/** Upload a folder in the web app and wait for the generated page. */
export async function generate(page: Page, folder: string): Promise<void> {
  await page.goto('/');
  await page.getByTestId('folder-input').setInputFiles(folder);
  await page.getByTestId('generate').click();
  await expect(page.getByTestId('result')).toBeVisible();
  await expect(page.frameLocator('[data-test="page"]').locator('body[data-ready="true"]')).toBeAttached();
}

/** The ids of the parts and boxes the picture on screen draws and does not hide. */
export function pictured(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll('#pic svg.docair-pic g.part, #pic svg.docair-pic g.box')]
      .filter((g) => !g.classList.contains('hidden'))
      .map((g) => (g as HTMLElement).dataset.id as string),
  );
}

/** The ids the picture on screen draws but hides. */
export function hidden(page: Page): Promise<string[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll('#pic svg.docair-pic g.part.hidden, #pic svg.docair-pic g.box.hidden')].map((g) => (g as HTMLElement).dataset.id as string),
  );
}

/** The lines of the picture on screen: their ends, the lines of the graph they stand for, their words and classes. */
export function pictureLines(page: Page): Promise<{ id: string; from: string; to: string; edges: string; words: string; cls: string }[]> {
  return page.evaluate(() =>
    [...document.querySelectorAll('#pic svg.docair-pic g.line')].map((g) => {
      const d = (g as HTMLElement).dataset;
      const text = document.querySelector(`#pic svg.docair-pic g.words[data-line="${d.id}"] text`);
      const words = text ? [...text.querySelectorAll(':scope > tspan')].map((t) => t.textContent ?? '').join(' ') : '';
      return { id: d.id as string, from: d.from as string, to: d.to as string, edges: d.edges as string, words, cls: g.getAttribute('class') ?? '' };
    }),
  );
}

/** A copy of a fixture to change, in a folder of its own. */
export function copyOf(name: string): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docair-e2e-'));
  const copy = path.join(dir, name);
  fs.cpSync(fixture(name), copy, { recursive: true });
  return copy;
}

/** `docair serve` on a folder, at a port the system picks, with the fake model. */
export interface ServedCopy {
  url: string;
  folder: string;
  stop(): Promise<void>;
}

/***
 * Start `docair serve` on a copy of a fixture, at a free port, and wait until it says where it listens.
 *
 * @id      e2e.serve
 * @part-of e2e
 * @calls   docair-cli.serve.cmd_serve "serves the copy, with the fake model"
 ***/
export async function serve(name: string): Promise<ServedCopy> {
  const folder = copyOf(name);
  const proc: ChildProcess = spawn(docair, ['serve', folder, '--bind', '127.0.0.1:0'], {
    env: { ...process.env, ...fakeAiEnv },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let said = '';
  const url = await new Promise<string>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`docair serve did not start: ${said}`)), 60_000);
    const look = (chunk: Buffer) => {
      said += chunk.toString();
      const m = said.match(/docair serve: (http:\/\/[\d.:]+)\/ shows/);
      if (m) {
        clearTimeout(timer);
        resolve(m[1]);
      }
    };
    proc.stdout!.on('data', look);
    proc.stderr!.on('data', look);
    proc.on('exit', (code) => reject(new Error(`docair serve stopped (${code}): ${said}`)));
  });
  return {
    url,
    folder,
    stop: async () => {
      if (proc.exitCode === null) {
        const gone = new Promise((r) => proc.once('exit', r));
        proc.kill('SIGINT');
        await gone;
      }
      fs.rmSync(path.dirname(folder), { recursive: true, force: true });
    },
  };
}

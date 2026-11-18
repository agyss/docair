import { expect, test } from '@playwright/test';
import { execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { docair, fakeAi, fixture, openStatic, out, select, withFakeAi } from './helpers';

const env = { ...process.env, DOCAIR_AI_ENDPOINT: `${fakeAi}/v1`, DOCAIR_AI_MODEL: 'fake-model', DOCAIR_AI_API_KEY: 'fake-key' };
const run = (...args: string[]) => execFileSync(docair, args, { env, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });

/***
 * What the words mean, the code first (docs/MEANINGS.md): words the code defines explain themselves, with where the meaning comes from; the model settles unclear words once.
 *
 * @id      e2e.meanings
 * @kind    test
 * @part-of e2e
 ***/
test.describe('what the words mean, the code first', () => {
  test.describe.configure({ mode: 'serial' });

  test.beforeAll(() => {
    execFileSync(docair, ['scan', fixture('words'), '--out', path.join(out, 'words')]);
  });

  /***
   * A word a part's docs define is underlined like a glossary term; the hover names the part, and Read more opens the doc heading that defines it.
   *
   * @part-of e2e.meanings
   * @covers  docair-core.meanings.settle "checks a word from the code becomes a term with its part and section"
   * @covers  docair-viewer.doc-page.popups "checks the hover names the part"
   * @covers  docair-viewer.doc-page.reader "checks Read more opens the doc heading"
   ***/
  test('a word the code defines explains itself and names its part', async ({ page }) => {
    await openStatic(page, 'words');
    await select(page, 'books.ledger.record');
    const entry = page.locator('#details .term[data-term="entry"]');
    await expect(entry).toHaveText('entry');
    await entry.hover();
    const pop = page.getByTestId('term-pop');
    await expect(pop).toContainText('An entry is one payment in the ledger');
    await expect(pop.getByTestId('term-from')).toHaveText('from Entry');
    await pop.getByTestId('pop-more').click();
    await expect(page.locator('#reader .flash')).toHaveText('2. Entries: one line of the ledger per payment');
    await page.getByTestId('reader-close').click();

    // "audit" is decided to be an everyday word: never underlined.
    await select(page, 'books.audit.compare');
    await expect(page.locator('#details .term[data-term="entry"]')).toHaveText('entries');
    await expect(page.locator('#details .term', { hasText: 'audit' })).toHaveCount(0);
  });

  /***
   * A glossary entry not checked against the code that defines its word shows the code's words first, and the glossary's below, marked; the list marks words from code.
   *
   * @part-of e2e.meanings
   * @covers  docair-viewer.page_glossary.glossary "checks the list marks words from the code"
   * @covers  docair-core.meanings.load "checks the decisions are read"
   ***/
  test('an unchecked glossary entry gives way to the code', async ({ page }) => {
    await openStatic(page, 'words');
    await select(page, 'books.ledger.record');
    await page.locator('#details .term[data-term="ledger"]').click();
    await expect(page.getByTestId('term-title')).toContainText('ledger');
    await expect(page.locator('#details')).toContainText('The record of all payments, newest first.');
    await expect(page.locator('#details').getByTestId('term-from')).toContainText('the glossary is not checked against it');
    await expect(page.getByTestId('term-unchecked')).toHaveText('The book of every payment ever made, kept on paper.');
    await page.getByTestId('glossary-button').click();
    await expect(page.getByTestId('glossary').locator('dt', { hasText: 'entry' }).locator('.badge')).toHaveText('ƒ');
  });

  /***
   * docair check warns about a word that names a part and is not settled (W120) and about a glossary entry not checked against the code (W121); docair glossary settles both in one edit, and a second run has nothing left to ask.
   *
   * @part-of e2e.meanings
   * @covers  docair-cli.ask.cmd_glossary "checks decisions and rewritten entries are written"
   * @covers  docair-ai.glossary.explain "checks open words and unchecked entries are asked once"
   * @covers  docair-core.edit.patch "checks the section and file edits"
   ***/
  test('docair glossary settles the open words once', async ({ request }) => {
    test.skip(!withFakeAi, 'needs the fake model');
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docair-words-'));
    const words = path.join(dir, 'words');
    fs.cpSync(fixture('words'), words, { recursive: true });
    const checked = spawnSync(docair, ['check', words], { encoding: 'utf8' });
    const before = `${checked.stdout}${checked.stderr}`;
    expect(before).toContain('W120');
    expect(before).toContain('W121');
    expect(before).toContain('0 errors, 2 warnings');

    const outText = run('glossary', words, '--apply');
    expect(outText).toContain('Settled 1 words that name parts');
    expect(outText).toContain('Checked 1 glossary entries against the code (1 rewritten to agree with it)');
    const decisions = fs.readFileSync(path.join(words, 'docs/meanings.toml'), 'utf8');
    expect(decisions).toContain('[payment]\nfrom = "books.ledger.payment"');
    expect(decisions).toContain('[ledger]\nfrom = "books.ledger"\nchecked = ');
    expect(decisions).toContain('[audit]\nterm = false');
    const glossary = fs.readFileSync(path.join(words, 'GLOSSARY.md'), 'utf8');
    expect(glossary).toContain('## ledger\n\nSee: books.ledger\n\nthe record of all payments, newest first.');
    expect(glossary).not.toContain('kept on paper');
    expect(run('check', words)).toContain('0 errors, 0 warnings');

    // One-off: the next run asks the model about no open word and no entry.
    try {
      run('glossary', words);
    } catch {
      // The fake model's terms do not occur in these docs: nothing to add is fine here.
    }
    const sent = (await (await request.get(`${fakeAi}/requests`)).json()) as any[];
    const mine = sent.filter((r) => JSON.stringify(r).includes('System `words`'));
    const last = mine.at(-1).messages[1].content as string;
    expect(last).not.toContain('Open words:');
    expect(last).not.toContain('Entries to check:');
    fs.rmSync(dir, { recursive: true, force: true });
  });
});

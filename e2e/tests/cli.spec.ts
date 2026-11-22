import { expect, test } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { docair, fakeAi, fixture, ready, serveFolder, withFakeAi } from './helpers';

const env = { ...process.env, DOCAIR_AI_ENDPOINT: `${fakeAi}/v1`, DOCAIR_AI_MODEL: 'fake-model', DOCAIR_AI_API_KEY: 'fake-key' };
const run = (...args: string[]) => execFileSync(docair, args, { env, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });

function copyShop(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docair-cli-'));
  fs.cpSync(fixture('shop'), path.join(dir, 'shop'), { recursive: true });
  return path.join(dir, 'shop');
}

/***
 * The same from the terminal: docair ask records walkthroughs and writes edits, docair apply puts them into the code.
 *
 * @id      e2e.cli
 * @kind    test
 * @part-of e2e
 ***/
test.describe('the CLI', () => {
  /***
   * docair ask records the walkthrough; a follow-up proposes an edit; docair apply writes it and the graph has the new edge.
   *
   * @part-of e2e.cli
   * @covers  docair-cli.ask.cmd_ask "checks the command records a walkthrough"
   * @covers  docair-cli.ask.cmd_apply "checks the reply's edits get written"
   * @covers  docair-core.walkthrough.save "checks the walkthrough lands as JSON"
   * @covers  docair-core.edit.resolve "checks each edit finds its place"
   ***/
  test('docair ask records a walkthrough and docair apply writes the edits', async () => {
    const shop = copyShop();
    run('ask', 'How does checkout work?', '--root', shop);
    const recorded = JSON.parse(fs.readFileSync(path.join(shop, 'docs/walkthroughs/how-does-checkout-work.json'), 'utf8'));
    expect(recorded.steps).toHaveLength(3);
    expect(JSON.stringify(recorded)).not.toContain('"path"');

    const out = run('ask', '-f', 'Listing orders also reads the orders table; that is missing.', '--root', shop);
    expect(out).toContain('✎ orders.list');
    expect(fs.readFileSync(path.join(shop, '.docair/docair.patch'), 'utf8')).toContain('+ * @reads   orders-db -> Order');
    expect(JSON.parse(fs.readFileSync(path.join(shop, '.docair/conversation.json'), 'utf8'))).toHaveLength(2);

    run('apply', shop);
    expect(fs.readFileSync(path.join(shop, 'api/src/lib.rs'), 'utf8')).toContain(' * @reads   orders-db -> Order');
    expect(run('check', shop)).toContain('9 edges, 0 errors');
    fs.rmSync(path.dirname(shop), { recursive: true, force: true });
  });

  /***
   * docair chat answers line by line, keeps the conversation, and /apply writes the edits proposed so far.
   *
   * @part-of e2e.cli
   * @covers  docair-cli.ask.cmd_chat "checks the conversation persists between questions"
   * @covers  docair-cli.ask.turn "checks each round answers and records"
   ***/
  test('docair chat keeps the conversation and /apply writes the edits', async () => {
    const shop = copyShop();
    const out = execFileSync(docair, ['chat', shop], {
      env,
      encoding: 'utf8',
      input: 'How does checkout work?\nand then?\nListing orders also reads the orders table; that is missing.\n/apply\n/quit\n',
    });
    expect(out).toContain('Checkout goes from the cart');
    expect(out).toContain('Turns so far: 1.');
    expect(out).toContain('edited    api/src/lib.rs');
    expect(fs.readFileSync(path.join(shop, 'api/src/lib.rs'), 'utf8')).toContain('@reads   orders-db -> Order');
    expect(JSON.parse(fs.readFileSync(path.join(shop, '.docair/conversation.json'), 'utf8'))).toHaveLength(3);
    fs.rmSync(path.dirname(shop), { recursive: true, force: true });
  });

  /***
   * docair word gives the lines that say nothing their words and, with --apply, writes them into the tags.
   *
   * @part-of e2e.cli
   * @covers  docair-cli.ask.cmd_word "checks silent lines receive wording"
   * @covers  docair-ai.word.word "checks the proposals become tag edits"
   * @covers  docair-core.edit.patch "checks the edits rewrite the files"
   ***/
  test('docair word writes words for the lines into the tags', async () => {
    const shop = copyShop();
    const out = run('word', shop, '--apply');
    expect(out).toContain('cart-spec.buying-stores-the-order → shop-ui.cart: checks Cart');
    expect(fs.readFileSync(path.join(shop, 'ui/tests/cart.spec.ts'), 'utf8')).toContain('@covers  shop-ui.cart "checks Cart"');
    expect(run('check', shop)).toContain('0 errors');
    const graph = JSON.parse(fs.readFileSync(path.join(shop, '.docair/docair.graph.json'), 'utf8'));
    expect(graph.edges.filter((e: any) => e.kind === 'cover').map((e: any) => e.text)).toContain('checks Cart');
    fs.rmSync(path.dirname(shop), { recursive: true, force: true });
  });

  /***
   * docair glossary adds entries for the words the docs use without explaining them; terms the docs do not use are dropped.
   *
   * @part-of e2e.cli
   * @covers  docair-cli.ask.cmd_glossary "checks new entries land in the glossary"
   * @covers  docair-ai.glossary.explain "checks terms not in the docs are dropped"
   ***/
  test('docair glossary writes new entries into the glossary', async () => {
    const shop = copyShop();
    const out = execFileSync(docair, ['glossary', shop, '--apply'], { env, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
    expect(out).toContain('Explained 2 terms: orders table, cart page.');
    const glossary = fs.readFileSync(path.join(shop, 'GLOSSARY.md'), 'utf8');
    expect(glossary).toContain('## orders table\n\nSee: orders-db\n\nThe table in Postgres that keeps every order.');
    expect(glossary).toContain('[Read more](https://example.com/cart)');
    expect(glossary).not.toContain('flux capacitor');
    expect(run('check', shop)).toContain('0 errors, 0 warnings');
    fs.rmSync(path.dirname(shop), { recursive: true, force: true });
  });

  /***
   * "I don't care about …" with --apply goes into docair.toml, so every later page leaves it out.
   *
   * @part-of e2e.cli
   * @covers  docair-cli.ask.turn "checks the view change lands in config"
   ***/
  test('a view change with --apply goes into docair.toml', async () => {
    const shop = copyShop();
    run('ask', "I don't care about the infrastructure", '--apply', '--root', shop);
    const toml = fs.readFileSync(path.join(shop, 'docair.toml'), 'utf8');
    expect(toml).toContain('[view]\nhide = ["infra"]');
    const graph = JSON.parse(fs.readFileSync(path.join(shop, '.docair/docair.graph.json'), 'utf8'));
    expect(graph.view.hide).toEqual(['infra']);
    fs.rmSync(path.dirname(shop), { recursive: true, force: true });
  });

  /***
   * docair explain: two commits of the shop, the second says what it does and changes the README; the model reviews the change (docs/EXPLAIN.md §9): the pieces and the intent go to it, it groups them into a change with why and its sources, the review is kept: while it covers every piece, a second run asks nothing and keeps it as stored, and docair serve does the same as a job.
   *
   * @part-of e2e.cli
   * @covers  docair-cli.diff.cmd_explain "checks the review is kept and the comment written"
   * @covers  docair-ai.explain.explain "checks the pieces and the intent go to the model, bound"
   * @covers  docair-server.project.explain_job "checks a served page's review runs as a job and ends with its findings"
   * @covers  docair-cli.diff.cmd_diff "checks the flows two commits change"
   ***/
  test('docair explain reviews the change and keeps the review', async ({ page, request }) => {
    test.skip(!withFakeAi, 'needs the fake model');
    const shop = copyShop();
    const git = (...args: string[]) =>
      execFileSync('git', ['-C', shop, '-c', 'user.name=docair test', '-c', 'user.email=test@example.invalid', ...args], { encoding: 'utf8' });
    git('init', '-q', '-b', 'main');
    git('add', '.');
    git('commit', '-q', '-m', 'a');
    // A new step in the checkout: the API also writes an audit entry; the README says so.
    const lib = path.join(shop, 'api', 'src', 'lib.rs');
    fs.writeFileSync(
      lib,
      fs.readFileSync(lib, 'utf8') +
        '\n/***\n * Keep an audit entry of the order.\n *\n * @id      orders.audit\n * @part-of orders-api\n * @flow    checkout 4 "the order is audited"\n ***/\npub fn audit() {}\n',
    );
    fs.appendFileSync(path.join(shop, 'README.md'), '\nEvery order is audited: the API keeps an entry of each one.\n');
    git('commit', '-q', '-am', 'Audit every order\n\nThe auditors want a trail of each order.');
    const diff = run('diff', 'HEAD~1', 'HEAD', '--root', shop);
    expect(diff).toContain('**flow:checkout**: changed; now through `orders.audit`');
    const out = run('explain', 'HEAD~1', 'HEAD', '--root', shop);
    expect(out).toContain('### docair: the review of `HEAD~1` → `HEAD`');
    expect(out).toContain('**1. Audit every order: the flow now reaches `orders.audit`.**');
    // The AI's own words, nothing cited (docair-8l6.55).
    expect(out).toContain('Why: Checkout now goes one step further, to `orders.audit`.');
    expect(out).toContain('<details><summary>Sources (2)</summary>\n\n- the commit “Audit every order”\n- `README.md`');
    expect(out).not.toContain('commit:');
    // What was said against what the code does (docair-8l6.58).
    expect(out).toContain('*Said, but the code does otherwise:*\n\n- **Said:** the commit “Audit every order”: Every order is audited.  \n  **The code:** Only checkout reaches `orders.audit`. (in `orders.audit`)');
    // The risks apart, under their own heading; a kind with nothing found has none (docair-8l6.62).
    expect(out).toContain('*Looks wrong or risky:*\n\n- Nothing tests `orders.audit`. (in `orders.audit`)');
    expect(out).not.toContain('Done, but said nowhere');
    const store = path.join(shop, '.docair', 'changes'); // the default store (docair-8l6.74)
    const [file] = fs.readdirSync(store).filter((f) => f.endsWith('.json'));
    const found = JSON.parse(fs.readFileSync(path.join(store, file), 'utf8'));
    expect(found.schema).toBe('docair.explained/2');
    expect(found.changes[0].recorded.by).toBe('model');
    expect(found.changes[0].off[0]).toEqual({ kind: 'contradicts', source: 'commit', where: 'Audit every order', says: 'Every order is audited.', code: 'Only checkout reaches `orders.audit`.', parts: ['orders.audit'] });
    expect(found.changes[0].how.map((h: any) => h.piece)).toContain('path:orders.audit');
    // What the model got: the pieces and the intent, never the whole graph.
    const sent = (await (await request.get(`${fakeAi}/requests`)).json()) as any[];
    const mine = sent.filter((r) => r.messages[0].content.startsWith('You review what changed') && r.messages[1].content.includes('orders.audit'));
    expect(mine.length).toBeGreaterThan(0);
    const input = mine.at(-1).messages[1].content;
    expect(input).toContain('path:orders.audit');
    expect(input).toContain('Audit every order');
    expect(input).toContain('The auditors want a trail of each order.');
    expect(input).toContain('README.md');
    expect(input).not.toContain('commit:');
    expect(input).not.toContain('kind_cluster');
    // A store that covers every piece is kept as stored, and nothing is asked (docair-8l6.75: no hand corrections, the store is only kept).
    found.changes[0].title = 'Kept as stored: the API keeps an audit entry of each order.';
    fs.writeFileSync(path.join(store, file), JSON.stringify(found, null, 2));
    const before = mine.length;
    const again = run('explain', 'HEAD~1', 'HEAD', '--root', shop);
    expect(again).toContain('Kept as stored');
    const sentAgain = (await (await request.get(`${fakeAi}/requests`)).json()) as any[];
    expect(sentAgain.filter((r) => r.messages[0].content.startsWith('You review what changed') && r.messages[1].content.includes('orders.audit')).length).toBe(before);
    // The same from a served page: docair serve on the repository reviews as a job (docair-8l6.11).
    const served = await serveFolder(shop, ['--changes', 'HEAD~1']);
    const html = fs.readFileSync(path.join(shop, '.docair', 'docair.html'), 'utf8');
    expect(html).toContain('Δ Changes · ');
    // POST answers a job at once; GET /api/explain/<job> says how far it is, and at the end holds the review.
    const explainJob = async (data: object) => {
      const res = await request.post(`${served.url}/api/explain`, { data });
      expect(res.status()).toBe(202);
      let job = await res.json();
      expect(job.text).toBeTruthy();
      for (let i = 0; i < 200 && job.running; i++) {
        await new Promise((r) => setTimeout(r, 100));
        job = await (await request.get(`${served.url}/api/explain/${job.job}`)).json();
      }
      expect(job.phase, job.text).toBe('done');
      return job;
    };
    const answer = (await explainJob({ a: 'HEAD~1', b: 'HEAD' })).findings;
    expect(answer.changes[0].title).toContain('Kept as stored');
    // The working tree as B, the page's own pair: the model reviews it, and the page is written again with the review.
    const version = async () => (await (await request.get(`${served.url}/api/version`)).json()).version;
    const was = await version();
    const work = await explainJob({ a: 'HEAD~1' });
    expect(work.text).toMatch(/^Reviewed: \d+ changes? in \d+ requests? · /);
    expect(work.findings.changes[0].title).toContain('now reaches');
    expect(await version()).toBeGreaterThan(was);
    // The job says how far it is with a bar, and gives out the changes as they are made (docair-8l6.65).
    expect(work.html).toContain('<progress');
    expect(work.found[0].title).toContain('now reaches');
    await page.goto(`${served.url}/#review/1`);
    await ready(page);
    // The change's whole title heads its card (docair-8l6.54).
    await expect(page.getByTestId('step-card').locator('.card-head')).toContainText('now reaches');
    // From the page: the button shows the job's progress under it, then the page reloads with the review.
    const kept = await version();
    for (const f of fs.readdirSync(store).filter((f) => f.endsWith('..work.json'))) fs.rmSync(path.join(store, f));
    // The removed store is a file change: the page is written again, without the review.
    await expect.poll(version, { timeout: 10_000 }).toBeGreaterThan(kept);
    await page.goto(served.url);
    await ready(page);
    await page.getByTestId('changes-button').click();
    const reloaded = page.waitForEvent('load', { timeout: 20_000 });
    await page.getByTestId('explain').click();
    await expect(page.getByTestId('explain-progress').locator('progress')).toBeVisible();
    await reloaded;
    await ready(page);
    await page.getByTestId('changes-button').click();
    await expect(page.getByTestId('review-list')).toContainText('now reaches');
    await served.stop();
    fs.rmSync(path.dirname(shop), { recursive: true, force: true });
  });
});

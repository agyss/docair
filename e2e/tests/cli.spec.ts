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
   * docair explain: two commits of the shop, a new step in the checkout; the model explains the changed flows, the findings are kept, a second run asks the model nothing new, and a finding a human corrected is kept as it is.
   *
   * @part-of e2e.cli
   * @covers  docair-cli.diff.cmd_explain "checks the findings are kept and the comment written"
   * @covers  docair-ai.explain.explain "checks only the changed flows go to the model, bound"
   * @covers  docair-cli.diff.cmd_diff "checks the flows two commits change"
   ***/
  test('docair explain explains the changed flows and keeps the findings', async ({ page, request }) => {
    test.skip(!withFakeAi, 'needs the fake model');
    const shop = copyShop();
    const git = (...args: string[]) =>
      execFileSync('git', ['-C', shop, '-c', 'user.name=docair test', '-c', 'user.email=test@example.invalid', ...args], { encoding: 'utf8' });
    git('init', '-q', '-b', 'main');
    git('add', '.');
    git('commit', '-q', '-m', 'a');
    // A new step in the checkout: the API also writes an audit entry.
    const lib = path.join(shop, 'api', 'src', 'lib.rs');
    fs.writeFileSync(
      lib,
      fs.readFileSync(lib, 'utf8') +
        '\n/***\n * Keep an audit entry of the order.\n *\n * @id      orders.audit\n * @part-of orders-api\n * @flow    checkout 4 "the order is audited"\n ***/\npub fn audit() {}\n',
    );
    git('commit', '-q', '-am', 'b');
    const diff = run('diff', 'HEAD~1', 'HEAD', '--root', shop);
    expect(diff).toContain('**flow:checkout**: changed; now through `orders.audit`');
    const out = run('explain', 'HEAD~1', 'HEAD', '--root', shop);
    expect(out).toContain('### docair: what `HEAD~1` → `HEAD` changes');
    expect(out).toContain('- **flow:checkout** (changed)');
    expect(out).toContain('`orders.audit` changed, so this flow goes another way.');
    const store = path.join(shop, 'docs', 'changes');
    const [file] = fs.readdirSync(store);
    const found = JSON.parse(fs.readFileSync(path.join(store, file), 'utf8'));
    expect(found.schema).toBe('docair.explained/0');
    expect(found.flows.find((f: any) => f.seed === 'flow:checkout').recorded.by).toBe('model');
    // What the model got: the changed flows only, never the whole graph.
    const sent = (await (await request.get(`${fakeAi}/requests`)).json()) as any[];
    const mine = sent.filter((r) => r.messages[0].content.startsWith('You explain changes') && r.messages[1].content.includes('orders.audit'));
    expect(mine.length).toBeGreaterThan(0);
    expect(mine.at(-1).messages[1].content).not.toContain('kind_cluster');
    // A human corrects a finding; the next run keeps it and asks about nothing new.
    const checkout = found.flows.find((f: any) => f.seed === 'flow:checkout');
    checkout.explanation = 'Audited by hand: the API keeps an audit entry of each order.';
    checkout.recorded = { by: 'human', at: '2026-11-19T00:00:00Z' };
    fs.writeFileSync(path.join(store, file), JSON.stringify(found, null, 2));
    const before = mine.length;
    const again = run('explain', 'HEAD~1', 'HEAD', '--root', shop);
    expect(again).toContain('Audited by hand');
    const sentAgain = (await (await request.get(`${fakeAi}/requests`)).json()) as any[];
    expect(sentAgain.filter((r) => r.messages[0].content.startsWith('You explain changes') && r.messages[1].content.includes('orders.audit')).length).toBe(before);
    // The same from a served page: docair serve on the repository explains through POST /api/explain.
    const served = await serveFolder(shop, ['--changes', 'HEAD~1']);
    const html = fs.readFileSync(path.join(shop, '.docair', 'docair.html'), 'utf8');
    expect(html).toContain('Δ Changes · ');
    const res = await request.post(`${served.url}/api/explain`, { data: { a: 'HEAD~1', b: 'HEAD' } });
    expect(res.status()).toBe(200);
    const answer = await res.json();
    expect(answer.flows.find((f: any) => f.seed === 'flow:checkout').explanation).toContain('Audited by hand');
    // The working tree as B, the page's own pair: the model explains it, and the page is written again with the findings.
    const version = async () => (await (await request.get(`${served.url}/api/version`)).json()).version;
    const was = await version();
    const work = await request.post(`${served.url}/api/explain`, { data: { a: 'HEAD~1' } });
    expect(work.status()).toBe(200);
    expect((await work.json()).flows.find((f: any) => f.seed === 'flow:checkout').explanation).toContain('so this flow goes another way');
    expect(await version()).toBeGreaterThan(was);
    await page.goto(`${served.url}/#changes/flow:checkout`);
    await ready(page);
    await expect(page.locator('#details')).toContainText('so this flow goes another way');
    await served.stop();
    fs.rmSync(path.dirname(shop), { recursive: true, force: true });
  });
});

import { expect, test } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { docair, fakeAi, fixture } from './helpers';

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
});

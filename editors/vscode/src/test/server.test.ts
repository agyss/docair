import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { addressOf, ServeProcess, serveArgs } from '../serve-process';

/** A program that says what `docair serve` says, then does `then`. */
function fake(then: string): { binary: string; dir: string } {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docair-vscode-'));
  const binary = path.join(dir, 'docair');
  fs.writeFileSync(binary, `#!/bin/sh\necho "docair: 3 nodes"\necho "docair serve: http://127.0.0.1:43123/ shows $2"\n${then}\n`);
  fs.chmodSync(binary, 0o755);
  return { binary, dir };
}

test('serve listens where the system picks, and says where', () => {
  assert.deepEqual(serveArgs('/work/shop'), ['serve', '/work/shop', '--bind', '127.0.0.1:0']);
  assert.equal(addressOf('docair serve: http://127.0.0.1:43123/ shows /work/shop\n'), 'http://127.0.0.1:43123');
  assert.equal(addressOf('docair: 3 nodes\n'), undefined);
});

test('a started server is where it said, and stops', async () => {
  const { binary, dir } = fake('exec sleep 30');
  const said: string[] = [];
  const server = new ServeProcess(binary, dir, (t) => said.push(t));
  try {
    assert.equal(await server.start(), 'http://127.0.0.1:43123');
    assert.equal(server.url, 'http://127.0.0.1:43123');
    assert.ok(server.running);
    assert.match(said.join(''), /3 nodes/);
  } finally {
    server.stop();
    fs.rmSync(dir, { recursive: true, force: true });
  }
  assert.ok(!server.running);
});

test('a program that is not there, or stops before it serves, says so', async () => {
  const missing = new ServeProcess('/no/such/docair', os.tmpdir(), () => {});
  await assert.rejects(missing.start(), /is not found: install docair, or set docair.path/);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docair-vscode-'));
  const binary = path.join(dir, 'docair');
  fs.writeFileSync(binary, '#!/bin/sh\necho "error: no docair.toml" >&2\nexit 1\n');
  fs.chmodSync(binary, 0o755);
  try {
    await assert.rejects(new ServeProcess(binary, dir, () => {}).start(), /stopped \(1\) before it served: error: no docair.toml/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

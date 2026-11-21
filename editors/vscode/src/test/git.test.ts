import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';
import { commitsOf, refsOf, show, versions } from '../git-history';

const SEP = '\x1f';

test('branches come before tags, and a remote HEAD is left out', () => {
  const out = [
    ['refs/tags/v1', 'v1', '3f2a9c1', 'release', '2026-11-01'],
    ['refs/heads/main', 'main', 'aa11bb2', 'the shop', '2026-11-20'],
    ['refs/remotes/origin/HEAD', 'origin', 'aa11bb2', 'the shop', '2026-11-20'],
    ['refs/remotes/origin/main', 'origin/main', 'aa11bb2', 'the shop', '2026-11-20'],
  ]
    .map((f) => f.join(SEP))
    .join('\n');
  const refs = refsOf(out + '\n');
  assert.deepEqual(
    refs.map((r) => [r.kind, r.label, r.rev]),
    [
      ['branch', 'main', 'main'],
      ['branch', 'origin/main', 'origin/main'],
      ['tag', 'v1', 'v1'],
    ],
  );
  assert.equal(refs[0].description, 'aa11bb2 · 2026-11-20 · the shop');
});

test('a commit goes by its full sha and shows its short one', () => {
  const sha = 'a'.repeat(40);
  const [c] = commitsOf(`${sha}${SEP}aaaaaaa${SEP}Audit every order${SEP}2026-11-19\n`);
  assert.deepEqual(c, { rev: sha, label: 'aaaaaaa', description: '2026-11-19 · Audit every order', kind: 'commit' });
  assert.deepEqual(commitsOf(''), []);
});

test('a real history: its branches, tags and commits, and a file at a commit', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docair-vscode-'));
  const git = (...args: string[]) =>
    execFileSync('git', ['-C', dir, '-c', 'user.name=docair test', '-c', 'user.email=test@example.invalid', ...args], { encoding: 'utf8' });
  try {
    fs.writeFileSync(path.join(dir, 'shop.rs'), 'fn buy() {}\n');
    git('init', '-q', '-b', 'main');
    git('add', '.');
    git('commit', '-q', '-m', 'buy');
    git('tag', 'v1');
    git('checkout', '-q', '-b', 'feature');
    fs.writeFileSync(path.join(dir, 'shop.rs'), 'fn buy() { audit(); }\n');
    git('commit', '-q', '-am', 'audit');
    const all = await versions(dir);
    assert.deepEqual(
      all.filter((v) => v.kind !== 'commit').map((v) => v.label).sort(),
      ['feature', 'main', 'v1'],
    );
    assert.deepEqual(
      all.filter((v) => v.kind === 'commit').map((v) => v.description.split(' · ')[1]),
      ['audit', 'buy'],
    );
    assert.equal(await show(path.join(dir, 'shop.rs'), 'main'), 'fn buy() {}\n');
    await assert.rejects(show(path.join(dir, 'shop.rs'), '--output=x'), /not a commit/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

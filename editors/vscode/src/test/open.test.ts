import assert from 'node:assert/strict';
import { test } from 'node:test';
import { asOpen, planOf } from '../open-place';

const sha = '3f2a9c1d0e4b5a6978877665544332211aabbccd';

test('both versions open as a diff, old left, new right, at the new line', () => {
  const msg = asOpen({
    type: 'docair:open',
    old: { rev: sha, path: 'api/src/lib.rs', line: 3 },
    new: { rev: null, path: 'api/src/lib.rs', line: 5 },
  });
  assert.ok(msg);
  assert.deepEqual(planOf(msg, '/work/shop'), {
    left: { file: '/work/shop/api/src/lib.rs', rev: sha },
    right: { file: '/work/shop/api/src/lib.rs', rev: null },
    line: 5,
    title: 'lib.rs (3f2a9c1d ↔ working tree)',
  });
});

test('a new or a gone part opens alone, at its version', () => {
  const gone = asOpen({ type: 'docair:open', old: { rev: sha, path: 'old.rs', line: 2 }, new: null })!;
  assert.deepEqual(planOf(gone, '/p'), { one: { file: '/p/old.rs', rev: sha }, line: 2, title: 'old.rs (3f2a9c1d)' });
  const added = asOpen({ type: 'docair:open', old: null, new: { rev: null, path: 'new.rs', line: 1 } })!;
  assert.deepEqual(planOf(added, '/p').one, { file: '/p/new.rs', rev: null });
});

test('anything that is not a place inside the project is not acted on', () => {
  const place = { rev: null, path: 'a.rs', line: 1 };
  for (const bad of [
    null,
    'docair:open',
    { type: 'docair:ready' },
    { type: 'docair:open', old: null, new: null },
    { type: 'docair:open', new: { ...place, path: '../../etc/passwd' } },
    { type: 'docair:open', new: { ...place, path: '/etc/passwd' } },
    { type: 'docair:open', new: { ...place, rev: '--output=x' } },
    { type: 'docair:open', new: { ...place, rev: 'main' } },
    { type: 'docair:open', new: { ...place, line: 0 } },
    { type: 'docair:open', new: { ...place, line: 1.5 } },
    { type: 'docair:open', new: { ...place, path: '' } },
    { type: 'docair:open', new: 'a.rs:1' },
  ]) {
    assert.equal(asOpen(bad), undefined, JSON.stringify(bad));
  }
});

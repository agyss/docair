import assert from 'node:assert/strict';
import { test } from 'node:test';
import { Api, ServedPair, pairLabel } from '../api';

const pair = (p: Partial<ServedPair>): ServedPair => ({ old: null, new: null, base: null, comparing: false, error: null, version: 1, ...p });

/** A serve that answers from a list, and keeps what it was asked. */
function fakeServe(answers: { status: number; body: unknown }[]) {
  const asked: { url: string; method?: string; body?: string }[] = [];
  const fetcher = async (url: string, init?: { method?: string; body?: string }) => {
    asked.push({ url, method: init?.method, body: init?.body });
    const a = answers.shift()!;
    return { ok: a.status < 300, status: a.status, json: async () => a.body };
  };
  return { asked, fetcher };
}

test('compare posts the pair, and the working tree is no name', async () => {
  const { asked, fetcher } = fakeServe([
    { status: 202, body: pair({ old: 'main', comparing: true }) },
    { status: 202, body: pair({ old: 'main', new: 'feature', comparing: true }) },
  ]);
  const api = new Api('http://127.0.0.1:1', fetcher);
  await api.compare('main', null);
  await api.compare('main', 'feature');
  assert.deepEqual(
    asked.map((a) => [a.url, a.method, a.body]),
    [
      ['http://127.0.0.1:1/api/changes', 'POST', '{"old":"main"}'],
      ['http://127.0.0.1:1/api/changes', 'POST', '{"old":"main","new":"feature"}'],
    ],
  );
});

test("serve's own words are the error", async () => {
  const { fetcher } = fakeServe([{ status: 400, body: { error: 'git knows no commit `nope`' } }]);
  await assert.rejects(new Api('http://x', fetcher).compare('nope', null), /git knows no commit `nope`/);
});

test('compared waits until serve is done', async () => {
  const { asked, fetcher } = fakeServe([
    { status: 200, body: pair({ old: 'main', comparing: true }) },
    { status: 200, body: pair({ old: 'main', comparing: true }) },
    { status: 200, body: pair({ old: 'main', base: 'abc', version: 3 }) },
  ]);
  const done = await new Api('http://x', fetcher).compared(1);
  assert.equal(done.version, 3);
  assert.equal(asked.length, 3);
  assert.ok(asked.every((a) => a.method === 'GET'));
});

test('a pair is called by the names the picker showed', () => {
  const sha = 'a'.repeat(40);
  assert.equal(pairLabel(pair({ old: 'main' })), 'main → the working tree');
  assert.equal(pairLabel(pair({ old: sha, new: 'feature' })), 'aaaaaaaa → feature');
  assert.equal(pairLabel(pair({ old: sha }), new Map([[sha, 'aaaaaaa']])), 'aaaaaaa → the working tree');
  assert.equal(pairLabel(pair({})), '');
});

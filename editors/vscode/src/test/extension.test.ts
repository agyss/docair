import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import Module from 'node:module';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

// The extension against a stand-in for VS Code's API, a fake docair serve, and a real git history.

const did: unknown[][] = [];
const commands = new Map<string, () => Promise<void>>();
const picks: ((labels: string[]) => string)[] = [];
let provider: { provideTextDocumentContent(uri: FakeUri): Promise<string> } | undefined;
let onMessage: ((m: unknown) => void) | undefined;
let html = '';
let folder = '';
let binary = '';

class FakeUri {
  constructor(
    readonly scheme: string,
    readonly fsPath: string,
    readonly query: string,
  ) {}
  with(c: { scheme?: string; query?: string }): FakeUri {
    return new FakeUri(c.scheme ?? this.scheme, this.fsPath, c.query ?? this.query);
  }
  toString(): string {
    return this.scheme === 'http' ? this.fsPath : `${this.scheme}:${this.fsPath}${this.query ? `?${this.query}` : ''}`;
  }
}

const vscode = {
  StatusBarAlignment: { Left: 1 },
  ProgressLocation: { Notification: 15 },
  ViewColumn: { One: 1, Beside: -2 },
  QuickPickItemKind: { Separator: -1 },
  Range: class {
    constructor(
      readonly line: number,
      readonly character: number,
    ) {}
  },
  Uri: {
    parse: (s: string) => new FakeUri('http', s, ''),
    file: (f: string) => new FakeUri('file', f, ''),
  },
  env: { asExternalUri: async (u: FakeUri) => u },
  window: {
    createOutputChannel: () => ({ append() {}, appendLine: (s: string) => did.push(['log', s]), show() {}, dispose() {} }),
    createStatusBarItem: () => ({ show() {}, dispose() {}, text: '', tooltip: '', command: '' }),
    withProgress: (_: unknown, f: () => Promise<unknown>) => f(),
    showErrorMessage: async (m: string) => void did.push(['error', m]),
    showQuickPick: async (items: { label: string; kind?: number }[], o: { placeHolder: string }) => {
      const labels = items.map((i) => (i.kind === -1 ? `— ${i.label}` : i.label));
      did.push(['pick', o.placeHolder, labels]);
      const label = picks.shift()!(labels);
      return items.find((i) => i.label === label && i.kind !== -1);
    },
    createWebviewPanel: () => ({
      webview: {
        set html(h: string) {
          html = h;
        },
        onDidReceiveMessage: (f: (m: unknown) => void) => (onMessage = f),
      },
      onDidDispose() {},
      reveal() {},
    }),
    showTextDocument: async (uri: FakeUri, o: { selection: { line: number } }) => void did.push(['show', String(uri), o.selection.line]),
  },
  commands: {
    registerCommand: (id: string, f: () => Promise<void>) => (commands.set(id, f), { dispose() {} }),
    executeCommand: async (...a: unknown[]) => void did.push(['run', ...a.map((x) => (typeof x === 'object' && x && 'selection' in x ? (x as any).selection.line : String(x)))]),
  },
  workspace: {
    get workspaceFolders() {
      return [{ uri: { fsPath: folder } }];
    },
    getConfiguration: () => ({ get: () => binary.replace(folder, '${workspaceFolder}') }),
    registerTextDocumentContentProvider: (_: string, p: typeof provider) => ((provider = p), { dispose() {} }),
  },
};

const resolve = (Module as any)._resolveFilename;
(Module as any)._resolveFilename = function (request: string, ...rest: unknown[]) {
  return request === 'vscode' ? 'vscode' : resolve.call(this, request, ...rest);
};
require.cache['vscode'] = { id: 'vscode', filename: 'vscode', loaded: true, exports: vscode } as any;

test('compare two versions, see the page, open a changed part as a diff', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'docair-vscode-'));
  folder = path.join(dir, 'shop');
  fs.mkdirSync(folder);
  const git = (...args: string[]) =>
    execFileSync('git', ['-C', folder, '-c', 'user.name=docair test', '-c', 'user.email=test@example.invalid', ...args], { encoding: 'utf8' });
  fs.writeFileSync(path.join(folder, 'docair.toml'), 'system = "shop"\n');
  fs.writeFileSync(path.join(folder, 'lib.rs'), 'fn buy() {}\n');
  git('init', '-q', '-b', 'main');
  git('add', '.');
  git('commit', '-q', '-m', 'buy');
  const sha = git('rev-parse', 'HEAD').trim();

  // A fake docair serve: its routes for the pair, at a port of its own.
  const asked: string[] = [];
  let now = { old: null as string | null, new: null as string | null, base: null, comparing: false, error: null, version: 1 };
  const serve = http.createServer((req, res) => {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      asked.push(`${req.method} ${req.url} ${body}`);
      if (req.method === 'POST') {
        const p = JSON.parse(body);
        now = { ...now, old: p.old, new: p.new ?? null, version: now.version + 1 };
      }
      res.writeHead(req.method === 'POST' ? 202 : 200, { 'content-type': 'application/json' });
      res.end(JSON.stringify(now));
    });
  });
  await new Promise<void>((r) => serve.listen(0, '127.0.0.1', r));
  const port = (serve.address() as { port: number }).port;
  binary = path.join(folder, 'docair-fake');
  fs.writeFileSync(binary, `#!/bin/sh\necho "docair serve: http://127.0.0.1:${port}/ shows $2"\nexec sleep 30\n`);
  fs.chmodSync(binary, 0o755);

  const ext = require('../extension');
  const subscriptions: { dispose(): void }[] = [];
  ext.activate({ subscriptions });
  try {
    assert.deepEqual(
      [...commands.keys()].sort(),
      ['docair.compare', 'docair.compareWithWorkingTree', 'docair.open', 'docair.restart', 'docair.stopComparing'],
    );
    picks.push(
      () => 'main',
      (labels) => {
        assert.equal(labels[0], 'the working tree', 'for the new side, the working tree first');
        return 'the working tree';
      },
    );
    await commands.get('docair.compare')!();
    assert.ok(!did.some((d) => d[0] === 'error'), JSON.stringify(did));
    const [first, second] = did.filter((d) => d[0] === 'pick');
    assert.deepEqual(first[2], ['— branches', 'main', '— commits', sha.slice(0, 7)]);
    assert.match(String(second[1]), /Compare main with/);
    assert.deepEqual(asked.filter((a) => a.startsWith('POST')), ['POST /api/changes {"old":"main"}']);
    assert.match(html, new RegExp(`src="http://127.0.0.1:${port}/\\?editor=vscode&amp;load=1"`), 'the page is in the panel, in the editor');

    // The page sends a changed part: the diff of its file, at the commit and in the working tree.
    onMessage!({ type: 'docair:open', old: { rev: sha, path: 'lib.rs', line: 1 }, new: { rev: null, path: 'lib.rs', line: 2 } });
    await new Promise((r) => setTimeout(r, 20));
    assert.deepEqual(did.find((d) => d[0] === 'run'), [
      'run',
      'vscode.diff',
      `docair-git:${path.join(folder, 'lib.rs')}?${sha}`,
      `file:${path.join(folder, 'lib.rs')}`,
      `lib.rs (${sha.slice(0, 8)} ↔ working tree)`,
      1,
    ]);
    onMessage!({ type: 'docair:open', old: null, new: { rev: '../x', path: '/etc/passwd', line: 1 } });
    await new Promise((r) => setTimeout(r, 20));
    assert.equal(did.filter((d) => d[0] === 'run' || d[0] === 'show').length, 1, 'a message that is no place opens nothing');

    // The file at the commit, read-only, from git.
    const at = vscode.Uri.file(path.join(folder, 'lib.rs')).with({ scheme: 'docair-git', query: sha });
    fs.writeFileSync(path.join(folder, 'lib.rs'), 'fn buy() { audit(); }\n');
    assert.equal(await provider!.provideTextDocumentContent(at), 'fn buy() {}\n');

    await commands.get('docair.stopComparing')!();
    assert.ok(asked.some((a) => a.startsWith('DELETE /api/changes')));
  } finally {
    ext.deactivate();
    serve.close();
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

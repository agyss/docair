import fs from 'node:fs';
import path from 'node:path';
import * as vscode from 'vscode';
import { Api, ServedPair, pairLabel } from './api';
import { show, versions, PickedVersion, WORKING_TREE } from './git-history';
import { asOpen, planOf, Target } from './open-place';
import { frameUrl, nonce, panelHtml } from './panel';
import { ServeProcess } from './serve-process';

/***
 * The extension: docair's documentation beside the code (docs/EDITOR.md §6). It starts `docair serve` for the workspace, shows the page in a panel, picks an old and a new version from the git history and has serve compare them, and opens a place the page sends in VS Code's diff editor.
 *
 * @id      docair-vscode.extension
 * @kind    module
 * @part-of docair-vscode
 ***/

/** The scheme of a file at a commit: `docair-git:/<file>?<rev>`, read-only, from `git show`. */
const SCHEME = 'docair-git';

let output: vscode.OutputChannel;
let status: vscode.StatusBarItem;
let server: ServeProcess | undefined;
let api: Api | undefined;
let root: string | undefined;
let panel: vscode.WebviewPanel | undefined;
let loads = 0;
/** The pair asked for last, so a restarted server compares it again. */
let asked: { old: string; new: string | null } | undefined;
/** The names the picker showed for revs, for the status bar. */
const names = new Map<string, string>();

/***
 * Register the commands, the status bar and the `docair-git:` documents.
 *
 * @id      docair-vscode.extension.activate
 * @part-of docair-vscode.extension
 ***/
export function activate(context: vscode.ExtensionContext): void {
  output = vscode.window.createOutputChannel('docair');
  status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 50);
  status.command = 'docair.compare';
  showStatus(undefined);
  status.show();
  const command = (id: string, f: () => Promise<void>) => vscode.commands.registerCommand(id, () => run(f));
  context.subscriptions.push(
    output,
    status,
    command('docair.open', openDocumentation),
    command('docair.compare', compareTwo),
    command('docair.compareWithWorkingTree', compareWithWorkingTree),
    command('docair.stopComparing', stopComparing),
    command('docair.restart', restart),
    vscode.workspace.registerTextDocumentContentProvider(SCHEME, {
      provideTextDocumentContent: (uri) => show(uri.fsPath, uri.query),
    }),
    { dispose: () => server?.stop() },
  );
}

export function deactivate(): void {
  server?.stop();
}

/** Run a command; what goes wrong is said, and kept in the docair output. */
async function run(f: () => Promise<void>): Promise<void> {
  try {
    await f();
  } catch (e) {
    const said = e instanceof Error ? e.message : String(e);
    output.appendLine(`docair: ${said}`);
    void vscode.window.showErrorMessage(`docair: ${said}`, 'Show the output').then((a) => a && output.show());
  }
}

/** The project's folder: the first workspace folder with a docair.toml, else the first. */
function projectRoot(): string {
  const folders = vscode.workspace.workspaceFolders ?? [];
  const found = folders.find((f) => fs.existsSync(path.join(f.uri.fsPath, 'docair.toml'))) ?? folders[0];
  if (!found) throw new Error('open a folder first');
  return found.uri.fsPath;
}

/***
 * The running `docair serve`, started when there is none: the binary from `docair.path` (`${workspaceFolder}` in it is the project's folder), the project's folder, its output in the docair output channel.
 *
 * @id      docair-vscode.extension.ensure-server
 * @part-of docair-vscode.extension
 * @calls   docair-vscode.serve-process.serve-process.start "starts docair serve"
 ***/
async function ensureServer(): Promise<Api> {
  if (server?.running && api) return api;
  root = projectRoot();
  const setting = vscode.workspace.getConfiguration('docair').get<string>('path') || 'docair';
  const binary = setting.split('${workspaceFolder}').join(root);
  const started = new ServeProcess(binary, root, (text) => output.append(text));
  server = started;
  const url = await vscode.window.withProgress(
    { location: vscode.ProgressLocation.Notification, title: 'docair: building the documentation…' },
    () => started.start(),
  );
  api = new Api(url);
  return api;
}

/***
 * The panel beside the editor, made when there is none, and the page loaded into it again.
 *
 * @id      docair-vscode.extension.show-panel
 * @part-of docair-vscode.extension
 * @calls   docair-vscode.panel.panel-html "the frame around the page"
 ***/
async function showPanel(): Promise<void> {
  const served = server?.url;
  if (!served) return;
  if (!panel) {
    panel = vscode.window.createWebviewPanel(
      'docair',
      'docair',
      { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true },
      { enableScripts: true, retainContextWhenHidden: true },
    );
    panel.onDidDispose(() => (panel = undefined));
    panel.webview.onDidReceiveMessage((m) => run(() => opened(m)));
  } else {
    panel.reveal(undefined, true);
  }
  const external = await vscode.env.asExternalUri(vscode.Uri.parse(`${served}/`));
  panel.webview.html = panelHtml(frameUrl(external.toString(true), ++loads), nonce());
}

/** docair: Show the documentation. */
async function openDocumentation(): Promise<void> {
  await ensureServer();
  await showPanel();
}

/***
 * Pick a version from the git history, grouped: the working tree when `withWorkingTree`, branches, tags, the last commits.
 *
 * @id      docair-vscode.extension.pick
 * @part-of docair-vscode.extension
 * @calls   docair-vscode.git-history.versions "reads the history"
 ***/
async function pick(placeHolder: string, withWorkingTree: boolean): Promise<PickedVersion | undefined> {
  const all = await versions(root ?? projectRoot());
  const groups: [string, PickedVersion[]][] = [
    ['', withWorkingTree ? [WORKING_TREE] : []],
    ['branches', all.filter((v) => v.kind === 'branch')],
    ['tags', all.filter((v) => v.kind === 'tag')],
    ['commits', all.filter((v) => v.kind === 'commit')],
  ];
  type Item = vscode.QuickPickItem & { version?: PickedVersion };
  const items: Item[] = [];
  for (const [title, vs] of groups) {
    if (!vs.length) continue;
    if (title) items.push({ label: title, kind: vscode.QuickPickItemKind.Separator });
    for (const v of vs) items.push({ label: v.label, description: v.description, version: v });
  }
  const chosen = await vscode.window.showQuickPick(items, { placeHolder, matchOnDescription: true });
  return chosen?.version;
}

/***
 * Have serve compare `old` with `neu` (the working tree when null), wait until it has, and show the page with its changes; the status bar names the pair.
 *
 * @id      docair-vscode.extension.compare
 * @part-of docair-vscode.extension
 * @calls   docair-vscode.api.api.compare "asks serve to compare"
 * @calls   docair-vscode.api.api.compared "waits until it has"
 ***/
async function compare(old: string, neu: string | null): Promise<void> {
  const a = await ensureServer();
  asked = { old, new: neu };
  const label = pairLabel(asked, names);
  await a.compare(old, neu);
  const done = await vscode.window.withProgress(
    { location: vscode.ProgressLocation.Notification, title: `docair: comparing ${label}…` },
    () => a.compared(),
  );
  showStatus(done);
  await showPanel();
  if (done.error) throw new Error(`the page has no changes section: ${done.error}`);
}

/** docair: Compare two versions… — old, then new. */
async function compareTwo(): Promise<void> {
  await ensureServer();
  const old = await pick('Compare from: the old version', false);
  if (!old?.rev) return;
  names.set(old.rev, old.label);
  const neu = await pick(`Compare ${old.label} with: the new version`, true);
  if (!neu) return;
  if (neu.rev) names.set(neu.rev, neu.label);
  await compare(old.rev, neu.rev);
}

/** docair: Compare the working tree with… — old only. */
async function compareWithWorkingTree(): Promise<void> {
  await ensureServer();
  const old = await pick('Compare the working tree with: the old version', false);
  if (!old?.rev) return;
  names.set(old.rev, old.label);
  await compare(old.rev, null);
}

/***
 * docair: Stop comparing — the page without its changes section.
 *
 * @id      docair-vscode.extension.stop-comparing
 * @part-of docair-vscode.extension
 * @calls   docair-vscode.api.api.stop "drops the changes section"
 ***/
async function stopComparing(): Promise<void> {
  if (!api) return;
  asked = undefined;
  showStatus(await api.stop());
  await showPanel();
}

/** docair: Restart the server — e.g. after a new docair build; the pair asked for last is compared again. */
async function restart(): Promise<void> {
  server?.stop();
  server = undefined;
  api = undefined;
  await ensureServer();
  if (asked) await compare(asked.old, asked.new);
  else await showPanel();
}

/** The status bar: the pair the page compares, or docair alone; a click compares again. */
function showStatus(p: ServedPair | undefined): void {
  const label = p ? pairLabel(p, names) : '';
  status.text = label ? `$(git-compare) docair: ${label}` : '$(git-compare) docair';
  status.tooltip = label
    ? `docair compares ${label}${p?.base ? `, from where they split (${p.base.slice(0, 8)})` : ''}. Click to compare two other versions.`
    : 'docair: compare two versions';
}

/***
 * A place the page sent (docs/EDITOR.md §5): both versions in VS Code's diff editor, old at its commit on the left, new on the right; one version alone; at its line. A file at a commit opens read-only as `docair-git:`; the working tree's is the file itself.
 *
 * @id      docair-vscode.extension.opened
 * @part-of docair-vscode.extension
 * @calls   docair-vscode.open-place.as-open "checks the message"
 * @calls   docair-vscode.open-place.plan-of "decides what opens"
 ***/
async function opened(message: unknown): Promise<void> {
  const msg = asOpen(message);
  if (!msg || !root) return;
  const plan = planOf(msg, root);
  const uri = (t: Target) => (t.rev ? vscode.Uri.file(t.file).with({ scheme: SCHEME, query: t.rev }) : vscode.Uri.file(t.file));
  const at = new vscode.Range(plan.line - 1, 0, plan.line - 1, 0);
  const options: vscode.TextDocumentShowOptions = { selection: at, viewColumn: vscode.ViewColumn.One, preview: true };
  if (plan.left && plan.right) {
    await vscode.commands.executeCommand('vscode.diff', uri(plan.left), uri(plan.right), plan.title, options);
  } else if (plan.one) {
    await vscode.window.showTextDocument(uri(plan.one), options);
  }
}

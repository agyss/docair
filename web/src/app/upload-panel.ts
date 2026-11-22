import { Component, computed, input, output, signal } from '@angular/core';
import ignore from 'ignore';
import { Progress, ProgressLine } from './progress-line';
import { PickedFile } from './scan.service';

/** Folders that only make an upload big: dependencies, build output, VCS data, provider caches. */
const SKIPPED = ['node_modules', '.git', 'target', 'dist', '.angular', '.docair', '.terraform'];

/** docair reads source text; bigger files are binaries or generated data. */
const MAX_FILE_BYTES = 2 * 1024 * 1024;

/** Files that hold secrets: never uploaded. `.env.example` is fine. */
function secret(name: string): boolean {
  return (
    (/^\.env(\..+)?$/.test(name) && name !== '.env.example') ||
    /\.tfstate(\..+)?$/.test(name) ||
    /\.tfvars(\.json)?$/.test(name)
  );
}

function useful(path: string): boolean {
  const parts = path.split('/');
  return !parts.some((part) => SKIPPED.includes(part)) && !secret(parts[parts.length - 1]);
}

/** Files whose rules leave files out of the CLI's scan (`docair_core::discover`), each for its own folder. */
const RULE_FILES = ['.gitignore', '.docairignore'];
/** git's own excludes, for the folder that holds `.git`; read, but never uploaded. */
const EXCLUDE = '.git/info/exclude';

/** One file's ignore rules, and the folder they apply to ('' for the top). */
interface Rules {
  base: string;
  rules: ignore.Ignore;
  /** git reads `.git/info/exclude` before the `.gitignore` files of the same folder. */
  first: boolean;
}

/** The folder a picked file's ignore rules apply to, or null when it holds none. */
function baseOf(path: string): string | null {
  const at = path.lastIndexOf('/');
  if (RULE_FILES.includes(path.slice(at + 1))) return path.slice(0, Math.max(at, 0));
  if (path === EXCLUDE) return '';
  if (path.endsWith(`/${EXCLUDE}`)) return path.slice(0, -EXCLUDE.length - 1);
  return null;
}

const depth = (base: string): number => (base ? base.split('/').length : 0);

/** The ignore rules among the picked files, read: folders nearer the top first, as git applies them. */
async function rulesOf(picked: PickedFile[]): Promise<Rules[]> {
  const found = picked.flatMap((p) => {
    const base = baseOf(p.path);
    return base === null ? [] : [{ base, file: p.file, first: p.path.endsWith(EXCLUDE) }];
  });
  const all = await Promise.all(found.map(async (f) => ({ base: f.base, rules: ignore().add(await f.file.text()), first: f.first })));
  return all.sort((a, b) => depth(a.base) - depth(b.base) || Number(b.first) - Number(a.first));
}

/** Whether the rules of `path`'s folders leave it out: the last rule that matches decides. */
function matched(path: string, dir: boolean, all: Rules[]): boolean {
  let out = false;
  for (const r of all) {
    const prefix = r.base ? `${r.base}/` : '';
    if (path.length <= prefix.length || !path.startsWith(prefix)) continue;
    const t = r.rules.test(path.slice(prefix.length) + (dir ? '/' : ''));
    if (t.ignored) out = true;
    else if (t.unignored) out = false;
  }
  return out;
}

/** Whether git leaves `path` out: a folder above it left out leaves it out too, whatever deeper rules say. */
function ignored(path: string, all: Rules[]): boolean {
  if (!all.length) return false;
  const parts = path.split('/');
  return parts.some((_, k) => matched(parts.slice(0, k + 1).join('/'), k + 1 < parts.length, all));
}

/** docair's own settings: the paths in them are read by the scan whatever `.gitignore` says. */
const CONFIG = 'docair.toml';
/** Paths the scan reads when `docair.toml` does not name others (`docair_core::config`). */
const DEFAULT_NAMED = ['docs/walkthroughs', 'docs/meanings.toml'];

/**
 * The paths a `docair.toml` in folder `base` names, from the top of the upload: every quoted
 * string in it, and the scan's defaults. The scan reads them as they are (the tests' reports,
 * walkthroughs, the glossary, the decisions about words, API descriptions), so they stay in an upload even
 * where `.gitignore` leaves them out, as `e2e/test-results/` does. A string that is no path, such
 * as the AI's address, names no file.
 */
function namesIn(text: string, base: string): string[] {
  const prefix = base ? `${base}/` : '';
  const strings = [...text.matchAll(/"((?:[^"\\]|\\.)*)"|'([^']*)'/g)].map((m) => m[1] ?? m[2]);
  return [...strings, ...DEFAULT_NAMED]
    .map((s) => s.replace(/^\.\//, '').replace(/\/+$/, ''))
    .filter((s) => s && !s.startsWith('/') && !s.split('/').includes('..'))
    .map((s) => prefix + s);
}

/** Whether `path` is a path `names` holds or lies under one; a folder also when it holds one. */
function named(path: string, names: string[], dir = false): boolean {
  return names.some((n) => n === path || path.startsWith(`${n}/`) || (dir && n.startsWith(`${path}/`)));
}

/** The file of git's own excludes in a dropped `.git` folder, if there is one. */
async function excludeOf(git: FileSystemDirectoryEntry): Promise<PickedFile[]> {
  try {
    const entry = await new Promise<FileSystemEntry>((resolve, reject) => git.getFile('info/exclude', {}, resolve, reject));
    const file = await new Promise<File>((resolve, reject) => (entry as FileSystemFileEntry).file(resolve, reject));
    return [{ path: entry.fullPath.replace(/^\//, ''), file }];
  } catch {
    return [];
  }
}

/** What to say when a picked file cannot be read: it changed or went away after it was picked. */
export const CHANGED = 'A file changed while it was read; pick the folder again.';

/**
 * The file's bytes as they are now, in memory: a file that changes on disk after this cannot break
 * the upload (the browser would send it as it is at send time, and the sizes would no longer agree).
 */
async function kept(f: PickedFile): Promise<PickedFile> {
  const bytes = await f.file.arrayBuffer();
  return { path: f.path, file: new File([bytes], f.file.name, { type: f.file.type, lastModified: f.file.lastModified }) };
}

const pathOf = (entry: FileSystemEntry): string => entry.fullPath.replace(/^\//, '');
const fileOf = (entry: FileSystemEntry): Promise<File> =>
  new Promise<File>((resolve, reject) => (entry as FileSystemFileEntry).file(resolve, reject));
const isRules = (entry: FileSystemEntry): boolean => entry.isFile && RULE_FILES.includes(entry.name);
const isGit = (entry: FileSystemEntry): boolean => entry.isDirectory && entry.name === '.git';

/**
 * All files under a dropped file or folder, with their paths; `found` hears of each one. A folder's
 * own ignore rules are read before what it holds (`.git/info/exclude`, then `.gitignore` and
 * `.docairignore`), and a folder that its rules or its parents' leave out is not walked at all:
 * `.beads/backup` or a second worktree costs nothing. `take` still judges every file.
 */
async function filesOf(
  entry: FileSystemEntry,
  found: () => void = () => {},
  rules: Rules[] = [],
  names: string[] = [],
): Promise<PickedFile[]> {
  if (entry.isFile) {
    const file = await fileOf(entry);
    found();
    return [{ path: pathOf(entry), file }];
  }
  if (!entry.isDirectory || !useful(entry.fullPath) || (matched(pathOf(entry), true, rules) && !named(pathOf(entry), names, true))) {
    return [];
  }
  const reader = (entry as FileSystemDirectoryEntry).createReader();
  const entries: FileSystemEntry[] = [];
  for (;;) {
    const batch = await new Promise<FileSystemEntry[]>((resolve, reject) => reader.readEntries(resolve, reject));
    if (!batch.length) break;
    entries.push(...batch);
  }
  const git = entries.find(isGit);
  const exclude = git ? await excludeOf(git as FileSystemDirectoryEntry) : [];
  const ruleFiles = await Promise.all(entries.filter(isRules).map(async (e) => ({ path: pathOf(e), file: await fileOf(e) })));
  ruleFiles.forEach(() => found());
  const base = pathOf(entry);
  const own = await Promise.all(
    [...exclude.map((f) => ({ f, first: true })), ...ruleFiles.map((f) => ({ f, first: false }))].map(async ({ f, first }) => ({
      base,
      rules: ignore().add(await f.file.text()),
      first,
    })),
  );
  const inside = own.length ? [...rules, ...own] : rules;
  const config = entries.find((e) => e.isFile && e.name === CONFIG);
  const allNames = config ? [...names, ...namesIn(await (await fileOf(config)).text(), base)] : names;
  const rest = await Promise.all(entries.filter((e) => !isGit(e) && !isRules(e)).map((e) => filesOf(e, found, inside, allNames)));
  return [...exclude, ...ruleFiles, ...rest.flat()];
}

/***
 * Lets the user pick a folder or single files, by dialog or by drag and drop,
 * and hands them to the app. Dependency and build folders are left out, and so
 * are files with secrets: `.env`, Terraform state and `.tfvars`.
 *
 * @id      docair-web.upload-panel
 * @kind    component
 * @part-of docair-web
 * @creates data.files "you pick a folder"
 * @calls   docair-web.app.generate (picked files)
 * @flow    upload 2 "the user picks a folder and clicks Generate"
 ***/
@Component({
  selector: 'app-upload-panel',
  imports: [ProgressLine],
  templateUrl: './upload-panel.html',
  styleUrl: './upload-panel.css',
})
export class UploadPanel {
  readonly busy = input(false);
  /** Where the generation is, while it runs. */
  readonly progress = input<Progress | null>(null);
  readonly generate = output<PickedFile[]>();

  /** A dropped folder being read: how many files so far. */
  protected readonly reading = signal<Progress | null>(null);
  /** Why the picked files could not be taken. */
  protected readonly problem = signal('');

  protected readonly files = signal<PickedFile[]>([]);
  protected readonly skipped = signal(0);
  protected readonly dragging = signal(false);
  protected readonly folder = computed(() => this.files()[0]?.path.split('/')[0] ?? '');
  protected readonly size = computed(() => {
    const bytes = this.files().reduce((n, f) => n + f.file.size, 0);
    return bytes > 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(1)} MB` : `${Math.ceil(bytes / 1024)} kB`;
  });

  /***
   * Take the files from the folder or file dialog.
   *
   * @id      docair-web.upload-panel.pick
   * @part-of docair-web.upload-panel
   ***/
  protected async pick(event: Event): Promise<void> {
    const input = event.target as HTMLInputElement;
    const picked = Array.from(input.files ?? []).map((file) => ({ path: file.webkitRelativePath || file.name, file }));
    input.value = '';
    await this.take(picked);
  }

  /***
   * Take what was dropped on the drop zone, walking into dropped folders.
   *
   * @id      docair-web.upload-panel.drop
   * @part-of docair-web.upload-panel
   ***/
  protected async drop(event: DragEvent): Promise<void> {
    event.preventDefault();
    this.dragging.set(false);
    const entries = Array.from(event.dataTransfer?.items ?? [])
      .map((item) => item.webkitGetAsEntry())
      .filter((entry): entry is FileSystemEntry => entry !== null);
    const since = Date.now();
    let found = 0;
    this.reading.set({ phase: 'read', done: 0, since });
    try {
      await this.take((await Promise.all(entries.map((e) => filesOf(e, () => this.reading.set({ phase: 'read', done: ++found, since }))))).flat());
    } catch {
      this.files.set([]);
      this.problem.set(CHANGED);
    } finally {
      this.reading.set(null);
    }
  }

  protected over(event: DragEvent): void {
    event.preventDefault();
    this.dragging.set(true);
  }

  /***
   * Keep the useful files, with their bytes as they are now, so the upload sends what was picked.
   * What the folders' `.gitignore`, `.docairignore` and `.git/info/exclude` leave out stays out,
   * as in the CLI's scan: an upload carries no backups, test output or second checkouts. What
   * `docair.toml` names stays in, as the scan reads it whatever git ignores: the tests' reports.
   *
   * @id      docair-web.upload-panel.take
   * @part-of docair-web.upload-panel
   ***/
  private async take(picked: PickedFile[]): Promise<void> {
    this.problem.set('');
    this.files.set([]);
    const since = Date.now();
    let read = 0;
    this.reading.set({ phase: 'read', done: 0, since });
    try {
      const rules = await rulesOf(picked);
      const configs = picked.filter((p) => (p.path === CONFIG || p.path.endsWith(`/${CONFIG}`)) && useful(p.path) && !ignored(p.path, rules));
      const names = (await Promise.all(configs.map(async (p) => namesIn(await p.file.text(), p.path.slice(0, Math.max(p.path.lastIndexOf('/'), 0)))))).flat();
      const wanted = picked.filter(
        (p) => useful(p.path) && p.file.size <= MAX_FILE_BYTES && (!ignored(p.path, rules) || named(p.path, names)),
      );
      const files = await Promise.all(
        wanted.map(async (f) => {
          const k = await kept(f);
          this.reading.set({ phase: 'read', done: ++read, since });
          return k;
        }),
      );
      this.skipped.set(picked.length - files.length);
      this.files.set(files);
    } catch {
      this.problem.set(CHANGED);
    } finally {
      this.reading.set(null);
    }
  }
}

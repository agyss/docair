import path from 'node:path';

/***
 * What the page asks the editor to open (docs/EDITOR.md §5): a place in the code at the old and at the new version, checked before anything opens, and turned into what VS Code opens.
 *
 * @id      docair-vscode.open-place
 * @kind    module
 * @part-of docair-vscode
 ***/

/** A place in the code at one version: its commit, null for the working tree; its file, as the project knows it; its line. */
export interface CodePlace {
  rev: string | null;
  path: string;
  line: number;
}

/** `{ type: 'docair:open', old, new }`, as the page posts it; a side is null where the part is not. */
export interface OpenMessage {
  type: 'docair:open';
  old: CodePlace | null;
  new: CodePlace | null;
}

/** A file to open: on disk (rev null), or at a commit. */
export interface Target {
  file: string;
  rev: string | null;
}

/** What VS Code opens: a diff of two versions, or one file; at a line (1-based); under a title. */
export interface Plan {
  left?: Target;
  right?: Target;
  one?: Target;
  line: number;
  title: string;
}

const SHA = /^[0-9a-f]{7,64}$/i;

/** A side of a message, when it is a place: a sha or null, a path inside the project, a line from 1. */
function placeOf(x: unknown): CodePlace | null | undefined {
  if (x === null || x === undefined) return null;
  if (typeof x !== 'object') return undefined;
  const { rev, path: p, line } = x as Record<string, unknown>;
  const revOk = rev === null || (typeof rev === 'string' && SHA.test(rev));
  const pathOk =
    typeof p === 'string' && p.length > 0 && !path.isAbsolute(p) && !p.split(/[\\/]/).includes('..') && !p.includes('\0');
  const lineOk = typeof line === 'number' && Number.isInteger(line) && line > 0;
  return revOk && pathOk && lineOk ? { rev: rev as string | null, path: p as string, line: line as number } : undefined;
}

/***
 * The message, when it is a place to open: its type `docair:open`, at least one side, each side a sha or null, a path inside the project (no absolute path, no `..`) and a line. Anything else is not acted on.
 *
 * @id      docair-vscode.open-place.as-open
 * @part-of docair-vscode.open-place
 ***/
export function asOpen(data: unknown): OpenMessage | undefined {
  if (!data || typeof data !== 'object' || (data as { type?: unknown }).type !== 'docair:open') return undefined;
  const o = placeOf((data as { old?: unknown }).old);
  const n = placeOf((data as { new?: unknown }).new);
  if (o === undefined || n === undefined || (!o && !n)) return undefined;
  return { type: 'docair:open', old: o, new: n };
}

const shortRev = (rev: string | null) => (rev ? rev.slice(0, 8) : 'working tree');

/***
 * What to open for a place, its files under the project's folder `root`: both sides, a diff, old on the left and new on the right, at the new line; one side, that file at its version and line.
 *
 * @id      docair-vscode.open-place.plan-of
 * @part-of docair-vscode.open-place
 ***/
export function planOf(msg: OpenMessage, root: string): Plan {
  const target = (p: CodePlace): Target => ({ file: path.join(root, p.path), rev: p.rev });
  if (msg.old && msg.new) {
    return {
      left: target(msg.old),
      right: target(msg.new),
      line: msg.new.line,
      title: `${path.basename(msg.new.path)} (${shortRev(msg.old.rev)} ↔ ${shortRev(msg.new.rev)})`,
    };
  }
  const only = (msg.new ?? msg.old)!;
  return {
    one: target(only),
    line: only.line,
    title: `${path.basename(only.path)} (${shortRev(only.rev)})`,
  };
}

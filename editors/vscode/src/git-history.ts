import { execFile } from 'node:child_process';
import path from 'node:path';

/***
 * The git history the picker offers, read with git itself (docs/EDITOR.md §6, decision docair-p2c.13): branches, then tags, then the last 50 commits; for the new side, the working tree first.
 *
 * @id      docair-vscode.git-history
 * @kind    module
 * @part-of docair-vscode
 ***/

/** One version to compare: a branch, a tag, a commit, or the working tree (no rev). */
export interface PickedVersion {
  /** What git calls it: a branch's or tag's name, a commit's sha; null for the working tree. */
  rev: string | null;
  label: string;
  description: string;
  kind: 'working tree' | 'branch' | 'tag' | 'commit';
}

/** The new side when it is no commit: the project's folder, uncommitted and untracked files included. */
export const WORKING_TREE: PickedVersion = {
  rev: null,
  label: 'the working tree',
  description: 'what is on disk now, uncommitted and untracked files included',
  kind: 'working tree',
};

const SEP = '\x1f';

/** The branches and tags, newest first, a line each: ref, short name, sha, subject, date. */
export const REFS_ARGS = [
  'for-each-ref',
  '--sort=-committerdate',
  `--format=%(refname)${SEP}%(refname:short)${SEP}%(objectname:short)${SEP}%(contents:subject)${SEP}%(committerdate:short)`,
  'refs/heads',
  'refs/remotes',
  'refs/tags',
];

/** The last 50 commits from HEAD, a line each: sha, short sha, subject, date. */
export const LOG_ARGS = ['log', '-n', '50', `--format=%H${SEP}%h${SEP}%s${SEP}%cs`];

/***
 * The branches and tags in `git for-each-ref`'s lines (REFS_ARGS): branches first, then tags, each newest first; a remote's HEAD left out, as it only names another branch.
 *
 * @id      docair-vscode.git-history.refs-of
 * @part-of docair-vscode.git-history
 ***/
export function refsOf(out: string): PickedVersion[] {
  const branches: PickedVersion[] = [];
  const tags: PickedVersion[] = [];
  for (const line of out.split('\n')) {
    const [full, short, sha, subject, date] = line.split(SEP);
    if (!full || !short || full.endsWith('/HEAD')) continue;
    const tag = full.startsWith('refs/tags/');
    const said = [sha, date, subject].filter(Boolean).join(' · ');
    (tag ? tags : branches).push({ rev: short, label: short, description: said, kind: tag ? 'tag' : 'branch' });
  }
  return [...branches, ...tags];
}

/***
 * The commits in `git log`'s lines (LOG_ARGS), by their full sha, shown by their short one.
 *
 * @id      docair-vscode.git-history.commits-of
 * @part-of docair-vscode.git-history
 ***/
export function commitsOf(out: string): PickedVersion[] {
  const commits: PickedVersion[] = [];
  for (const line of out.split('\n')) {
    const [sha, short, subject, date] = line.split(SEP);
    if (!sha || !short) continue;
    commits.push({ rev: sha, label: short, description: [date, subject].filter(Boolean).join(' · '), kind: 'commit' });
  }
  return commits;
}

/** git in a folder; its output, or an error with what git said. */
export function git(cwd: string, args: string[]): Promise<string> {
  return new Promise((resolve, reject) =>
    execFile('git', ['-C', cwd, ...args], { maxBuffer: 64 * 1024 * 1024 }, (err, stdout, stderr) =>
      err ? reject(new Error(String(stderr).trim() || err.message)) : resolve(stdout),
    ),
  );
}

/***
 * Every version the picker offers for a project folder: its branches, tags and last 50 commits.
 *
 * @id      docair-vscode.git-history.versions
 * @part-of docair-vscode.git-history
 * @calls   docair-vscode.git-history.refs-of "the branches and tags"
 * @calls   docair-vscode.git-history.commits-of "the last commits"
 ***/
export async function versions(cwd: string): Promise<PickedVersion[]> {
  const [refs, log] = await Promise.all([git(cwd, REFS_ARGS), git(cwd, LOG_ARGS)]);
  return [...refsOf(refs), ...commitsOf(log)];
}

/***
 * A file's text at a commit, for the read-only `docair-git:` documents: `git show <rev>:./<name>` in the file's folder. Only a sha or a name git knows, never an option.
 *
 * @id      docair-vscode.git-history.show
 * @part-of docair-vscode.git-history
 ***/
export function show(file: string, rev: string): Promise<string> {
  if (!rev || rev.startsWith('-')) return Promise.reject(new Error(`not a commit: ${rev}`));
  return git(path.dirname(file), ['show', `${rev}:./${path.basename(file)}`]);
}

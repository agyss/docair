# Editor: docair in VS Code, comparing any two versions

Design for the epic docair-p2c. It builds on the changes
(docs/CHANGES.md, docair-zrr) and their review (docs/EXPLAIN.md, docair-8l6),
and adds no knowledge of its own: the CLI compares, the extension only picks
and opens.

## 1. The problem

docair already says what two versions change in a system's flows:
`docair diff A B`, `docair build --changes A`, `docair serve --changes A`. Two
things are missing:

* **Choosing the versions while the page is open.** `serve` compares one
  commit A, fixed when it starts, and always with the working tree.
* **Getting from a changed part to its code.** The page links a part to its
  repository host at a commit, or to `vscode://file` for the working tree. In
  the editor, a click should open the diff of that file at that line.

The extension puts the page beside the code, lets the reader pick an old and a
new version from the git history, and opens a changed part's code in VS Code's
diff editor.

## 2. Two sides: old and new

A comparison has two sides.

| Side | What it can be | How docair reads it |
|------|----------------|---------------------|
| old | a branch, a tag or a commit | a detached git worktree in a scratch folder (as today) |
| new | a branch, a tag, a commit, or the working tree | a worktree; the working tree is the project's folder itself, uncommitted and untracked files included |

The extension runs in the devcontainer, on the same disk as the project: there
are no uploads. Both sides are built the same way, code indexer included.

**Branches are compared from where they split.** The old side is the newest
commit both sides share (`git merge-base old new`), as a pull request shows
it. `docair diff main feature` then shows what the feature adds, not main's
newer work as if the feature had removed it.

| old | new | compared from |
|-----|-----|---------------|
| `HEAD~3` | the working tree | `HEAD~3` (it is an ancestor already) |
| `main` | `feature` | where `feature` left `main` |
| `main` | the working tree, on `feature` | where `HEAD` left `main` |
| `feature` | `main`, and `main` holds `feature` | `feature` (new is ahead of old) |
| `HEAD` | `HEAD~3` (backwards) | `HEAD`: new is an ancestor of old, so the tips |
| two unrelated histories | | the tips |

The commit compared from is `base` in `changes.json`, and the summary and the
page say it: *from where `feature` left `main` (`3f2a9c1`)*.

## 3. The CLI

| Command | What changes |
|---------|--------------|
| `docair diff A [B]`, `docair explain A [B]` | compared from the merge base (§2) |
| `docair build --changes A [--to B]` | `--to B`: B is a commit, not the working tree; the page is B's documentation |
| `docair serve --changes A [--to B]` | the same, and the pair can change while it runs (§4) |

With a commit as new side, the page shows that commit's documentation. The
chat, edits and `docair.graph.json` stay the working tree's: they are about the
files one can change.

## 4. `docair serve`: the pair changes while it runs

| Route | What it does |
|-------|--------------|
| `GET /api/changes` | the pair the page compares: `{ old, new, base, comparing, error }`; `new` is null for the working tree, `old` null when the page has no changes section |
| `POST /api/changes` | `{ old, new? }`: both names are checked at once (400 for a name git does not know); 202, and the page is written again in the background; its version grows and the page reloads. 409 while a comparison runs |
| `DELETE /api/changes` | the page loses its changes section; written again |

A commit's graph is built once per process (docair-wq8), so switching back to
a pair already seen costs the comparison and the page, not the scans. With the
working tree as new side, each save writes the page again, as today.

## 5. The page in the editor

The page knows it is in the editor by `?editor` in its address. Then a place in
the code — an element with `data-open` — no longer follows its link: the page
sends a message to the frame around it.

```js
{ type: 'docair:open',
  old: { rev: '3f2a9c1…', path: 'src/shop.rs', line: 12 } | null,
  new: { rev: null,       path: 'src/shop.rs', line: 14 } | null }
```

`rev` is the commit's full sha, null for the working tree. The page writes both
sides itself, so the extension keeps no state to open the right versions.

| Where | `data-open` |
|-------|-------------|
| the changes section, each changed part | old and new: the part's place at each side; one side for a new or a gone part |
| the details panel, a part's source | new only: the version the page shows (B's commit, or the working tree) |

Without `?editor`, nothing changes: links open the repository host or
`vscode://file` as before (docair-5nv).

## 6. The extension

In `editors/vscode/`: TypeScript, compiled by `tsc`, no runtime dependencies.

| Command | Does |
|---------|------|
| docair: Show the documentation | starts `docair serve` when it does not run, opens the panel |
| docair: Compare two versions… | picks old, then new, and compares |
| docair: Compare the working tree with… | picks old; new is the working tree |
| docair: Stop comparing | the page without changes section |
| docair: Restart the server | stops and starts `docair serve`, e.g. after a new docair build |

* **The server.** One `docair serve` for the first workspace folder with a
  `docair.toml` (else the first folder), on `127.0.0.1:0`; its first line says
  the address. Its output goes to the *docair* output channel. It stops with the
  window. The binary is the setting `docair.path` (default `docair`, on PATH;
  `${workspaceFolder}` in it is the project's folder).
* **The panel.** A webview that frames the served page at
  `asExternalUri(http://127.0.0.1:<port>/?editor=vscode)`, so the port is
  forwarded out of a devcontainer. Messages count only from that origin.
* **The picker.** Branches, then tags, then the last 50 commits, each with its
  subject and date; for new, *the working tree* first. Read with `git` itself.
* **Opening a place.** Both sides: VS Code's diff editor, old at its commit on
  the left, new on the right, at the new line; the working tree's file is the
  file itself, so it can be edited there. One side: that file at its version
  and line. A file at a commit opens read-only through the extension's own
  `docair-git:` scheme (`git show <rev>:<path>`).
* **The status bar** names the pair (*docair: main → working tree*); a click
  compares again.

It does not clone, upload, draw the git log, or check doc blocks as you type.

## 7. Tests

| What | Where |
|------|-------|
| merge base, two commits, `--to` | `crates/docair-server/src/commits.rs` tests, on a scratch repository |
| the routes | `crates/docair-server/src/project.rs` tests, on the router |
| `data-open` on the page | `crates/docair-viewer/tests/page.rs` |
| the page in a frame posting `docair:open` | `e2e/tests/editor.spec.ts` |
| the extension's own logic: git output into picks, messages into what opens, serve's address; the extension itself against a stand-in for VS Code's API, a fake serve and a real git history | `editors/vscode/src/test/*.test.ts`, `node --test`; left out of docair's own documentation (`docair.toml` `exclude`), as their fakes are no part of docair |

## 8. Steps

| Step | Issue | What |
|------|-------|------|
| 1 | docair-p2c.1 | branches compared from where they split (§2) |
| 2 | docair-p2c.2 | `--to B`: a commit as new side, in build and serve (§3) |
| 3 | docair-p2c.3 | the pair changes while serve runs (§4) |
| 4 | docair-p2c.4 | `data-open`, and the page in the editor (§5) |
| 5 | docair-p2c.5 | the extension (§6) |
| 6 | docair-p2c.6 | the extension packaged, documented and in docair's own picture |

## 9. Decided

| # | Question | Decided |
|---|----------|---------|
| 1 | where a comparison starts (docair-p2c.7) | at the merge base of old and new; the tips when new is an ancestor of old or there is no common commit; no flag for tips |
| 2 | what the page shows with a commit as new side (docair-p2c.8) | that commit's documentation; serve's chat, edits and `docair.graph.json` stay the working tree's |
| 3 | how serve changes the pair (docair-p2c.9) | `GET`, `POST`, `DELETE /api/changes`; names checked at once (400), the page written in the background (202), 409 while a comparison runs |
| 4 | how the page knows it is in the editor (docair-p2c.10) | `?editor` in its address; `data-open` with both sides' revs, written by docair |
| 5 | page.js's budget (docair-p2c.11) | kept: the editor mode is three lines |
| 6 | how the extension is built (docair-p2c.12) | TypeScript and `tsc`, no bundler, no runtime dependencies, dev dependencies pinned |
| 7 | where the picker's list and old files come from (docair-p2c.13) | `git` itself; files at a commit through the extension's own `docair-git:` scheme, no other extension needed |
| 8 | which server (docair-p2c.14) | one `docair serve` per window, the first folder with a `docair.toml`, a free port, the binary from `docair.path` |
| 9 | how the page gets into the panel (docair-p2c.15) | an iframe of `asExternalUri(…)`; messages only from that origin |
| 10 | the Source Control graph's menu (docair-p2c.16) | not used: a proposed API in VS Code 1.143; commands, the picker and the status bar instead |
| 11 | how the extension is tested (docair-p2c.17) | its own logic with `node --test`; the page's side in e2e; no test inside a running VS Code |
| 12 | how it ships (docair-p2c.18) | a local `.vsix`; no Marketplace release |
| 13 | names that are everyday words (docair-p2c.19) | renamed (`ComparedSides`, `git-history`, …), not settled in the glossary |
| 14 | the extension's tests in docair's own page (docair-p2c.20) | left out (`docair.toml` `exclude`) |

## 10. Built (2026-11-21)

| Step | Where | Tests |
|------|-------|-------|
| 1 | `docair_server::commits::split_point`, used by `compare_inner`; `Changes.base` and `changes::split_from`; the page's *Compared from where …* (`page_changes.rs`) | `commits::tests::a_branch_is_compared_from_where_it_split` (a branch, the working tree on it, an ancestor, backwards, unrelated); `tests/page.rs` |
| 2 | `--to <B>` in `crates/docair-cli/src/main.rs`; `project::ComparedSides` | `project::tests::a_page_compares_two_commits` |
| 3 | `project::{changes, compare, stop_comparing}`, `ServeChanges`, `CompareOutcome`; `openapi.yaml` | `project::tests::the_page_compares_the_pair_it_is_given` |
| 4 | `doc_page::{place, data_open}`, `CodeLinks.rev_a`/`rev_b`; the source row (`page_panels.rs`); each changed part (`page_changes.rs`); `page.js` +3 lines (1,049 of 1,050) | `tests/page.rs`; `e2e/tests/editor.spec.ts` (75 of 75 e2e pass) |
| 5 | `editors/vscode/src/`: `extension.ts`, `serve-process.ts`, `api.ts`, `git-history.ts`, `open-place.ts`, `panel.ts` | 16 with `node --test`; the modules also driven against the real `docair serve` on a scratch repository |
| 6 | `npm run package` → `docair.vsix` (14 KB, 10 files); `editors/vscode/README.md` declares `docair-vscode` (`@part-of workstation`): 72 parts in docair's own page, with lines to the three routes and to `docair serve` | `docair check .`: 0 errors, 0 warnings |

Not done: a run inside a real VS Code window (decision docair-p2c.17). To try
it: `cd editors/vscode && npm ci && npm run package && code --install-extension
docair.vsix`; in this repository, set `docair.path` to
`${workspaceFolder}/target/release/docair` (docair is not on the PATH here);
then *docair: Compare the working tree with…*.

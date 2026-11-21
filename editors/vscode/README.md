# docair for VS Code

docair's documentation of a system, beside the code. Pick an old and a new
version from the git history, and docair shows what the new one changes in the
system's flows. Click a changed part, and its code opens in VS Code's diff
editor. Design: `docs/EDITOR.md` in the docair repository.

The extension only picks and opens. `docair serve` builds and compares, as on
the command line.

## Use it

| Command | Does |
|---------|------|
| **docair: Show the documentation** | starts `docair serve` for the workspace, and shows the page beside the editor |
| **docair: Compare two versions…** | pick the old version (a branch, a tag, one of the last 50 commits), then the new one (the same, or the working tree) |
| **docair: Compare the working tree with…** | pick the old version; the new one is what is on disk now, uncommitted and untracked files included |
| **docair: Stop comparing** | the page without its changes |
| **docair: Restart the server** | after a new docair build; the pair is compared again |

* A branch is compared from where it split from the other side, as a pull
  request shows it (docs/EDITOR.md §2).
* In the page, a changed part's code and a part's source open in the editor: a
  diff of both versions, old on the left, or one file at its version.
* With the working tree as the new side, each saved file writes the page again.
* The status bar names the pair; a click compares two other versions.

Setting: `docair.path`, the `docair` program (default: `docair` on the PATH).
`${workspaceFolder}` in it is the project's folder; in docair's own repository,
`${workspaceFolder}/target/release/docair` runs the docair built there.
The extension runs where the workspace is, so in a devcontainer it runs in the
container next to the code, and VS Code forwards the page's port to the window.

## Build, test, install

```bash
npm ci
npm test                  # tsc, then node --test on out/test
npm run package           # docair.vsix
code --install-extension docair.vsix
```

| File | Does |
|------|------|
| `src/extension.ts` | the commands, the panel, the picker, the status bar; opens what the page sends |
| `src/serve-process.ts` | starts `docair serve` on a free port and reads where it listens |
| `src/api.ts` | `GET`, `POST` and `DELETE /api/changes` |
| `src/git-history.ts` | the branches, tags and commits to pick from; a file at a commit |
| `src/open-place.ts` | checks what the page sends, and decides what opens |
| `src/panel.ts` | the panel's frame around the page |

```docair
docair-vscode: the VS Code extension. It runs in VS Code's extension host,
starts docair serve for the workspace, frames the page beside the editor, and
opens a changed part's code in the diff editor.

@id      docair-vscode
@kind    service
@part-of workstation
@calls   docair-cli "starts docair serve for the workspace"
```

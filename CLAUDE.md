# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:1105d646 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->


## Build & Test

Everything runs inside the devcontainer (cargo, node, bd, Docker).

```bash
cargo test --workspace                      # unit tests and the golden graphs
cargo clippy --workspace --all-targets      # no warnings
cargo fmt --all --check
cargo run -p docair-cli -- check .          # docair on itself: 0 errors, 0 warnings
cargo run -p docair-cli -- scan .           # .docair/docair.html, docair's own page
```

- Golden graphs: `crates/docair-adapters/tests/golden/*.graph.json`. A change that
  moves them says why; rewrite them with `DOCAIR_BLESS=1 cargo test -p docair-adapters`
  and review the diff.
- e2e: `cd e2e && npm run build && npm test` (builds the release binaries and the web
  app first). A run uses ports 9555 (fake AI) and 9556 (server), and rewrites
  `e2e/.out` and `e2e/test-results/results.json`, which `docair check` reads.
  Don't pass `--reporter`: it replaces that JSON report, and the file is gone.

## Architecture Overview

docair reads doc blocks, Markdown and code, and draws a whole system as one HTML
page. The contract is [docs/SPEC.md](docs/SPEC.md); the crates are in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md); designs of larger changes are in
`docs/` ([ADAPTERS.md](docs/ADAPTERS.md), [IMPORTS.md](docs/IMPORTS.md)); library
choices in [docs/LIBRARIES.md](docs/LIBRARIES.md); words in [docs/GLOSSARY.md](docs/GLOSSARY.md).

## Conventions & Patterns

- docair documents itself: new code gets docair doc blocks (`/*** … ***/` with `@id`,
  `@part-of`, `@calls`, …), and `docair check .` stays at 0 errors and 0 warnings.
- Design before code: a larger change starts as a design doc in `docs/`. SPEC.md
  describes what is built and changes with each step that lands.
- Dependencies: only versions at least two weeks old, pinned exactly when a newer one
  would be picked; the choice and why go into LIBRARIES.md.
- Docs are plain English: short sentences, tables, examples.
- Do only the step that was asked for; propose the next one instead of starting it.
- Privacy: the repository may be public. Never write Claude memory, transcripts,
  session IDs, quotes of the user or personal notes into tracked files or into beads.

## Beads in this repository

These rules come on top of the Beads block above, and win where they differ.

- Do not use `bd remember`. Preferences and lessons stay in Claude's own memory,
  outside the repository; beads hold work items only.
- Before planning, read `bd list --all` and the design docs in `docs/`: extend an
  existing epic instead of starting a parallel one.
- An epic has a design doc in `docs/` (`--spec-id`). Its steps are child issues with
  `blocked-by` dependencies, also across epics. A design choice two epics share is a
  `decision` issue, linked `related` to the steps it affects.
- Claim an issue before working on it (`bd update <id> --claim`); renew the claim
  during long work (`bd heartbeat <id>`).
- Notes say what was built: files, tests, numbers. Close with `--reason`. Work left
  over becomes a new issue with its dependencies; never `--force`.
- Never pass `--session`, and keep `CLAUDE_SESSION_ID` unset: no session IDs in beads.
- Issues are exported to `.beads/issues.jsonl` after every change (`export.auto`);
  commit that file together with the code it belongs to.

## Several sessions at once

Several Claude sessions may work in this tree at the same time, with no branches
between them.

- `ListAgents` shows them. Send a short message before you change a file another
  session works in, and when you land a step someone depends on.
- One writer per file; hand files over explicitly.
- Shared files (`crates/docair-core/src/facts.rs`, `crates/docair-core/src/lib.rs`,
  the root `Cargo.toml`, `docs/SPEC.md`): re-read right before you edit, make exact
  edits, never rewrite the whole file. Run `rustfmt` on your own files, not
  `cargo fmt` over others' work.
- Golden graphs of another session's work: send the diff, let the owner bless.
- Ask before running e2e: the ports are shared.

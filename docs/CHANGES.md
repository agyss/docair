# Changes: what two commits change in app structure and flows

Design for the epic docair-zrr. Written before the slices (docs/SLICES.md,
docair-702) were built, and updated once they were (docair-zrr.1,
2026-11-19): what epic 1 built is used as it is, and the questions of §7 are
decided (decision issues under docair-zrr).

## 1. The problem

`git diff` shows which lines changed. A reviewer wants to know what the change
does to the system: which flows it touches, which apps now talk to each
other, which request goes elsewhere, which datum is kept somewhere new. Today
nobody answers that except by reading the code.

`docair diff <A> <B>` answers it, reliably, without the AI and without a
browser. The AI explains the result afterwards (docs/EXPLAIN.md, docair-8l6).

## 2. What a change is

docair compares **flows**, not text. A flow is a slice (SLICES.md §3) named by
its ends with stable names: a route, a store, a topic, an app, a datum, a
service outside. The path between the ends is what can change.

| At B, compared with A | A change? |
|-----------------------|-----------|
| a function on the path is renamed or moved, its body the same | no |
| a function on the path is renamed and its body changed | yes, and the AI links the two (EXPLAIN.md) |
| the path gets a new step (a check, a cache) | yes |
| a request now goes to another app or route | yes |
| a datum is now kept in another store | yes |
| a Helm value sends traffic to another Service | yes |
| a doc block's words change, nothing else | no flow change; listed apart |

## 3. Changed parts

The changed parts are the union of:

1. **what the graph diff shows changed**, at A or at B: parts, lines and
   tags added, removed or changed. A removed part exists only at A.
2. **the parts the changed lines fall in.** Each part knows where it ends
   (`SourceRef.end`, docair-zrr.2): the end of its code (Rust and
   TypeScript items, from the adapters), else the end of its block; parts
   the infrastructure adapter discovers without a block have no end yet,
   and their changes come from the graph diff.

The first catches what the second cannot: a route registered in another
file, a config file (Helm values, `nginx.conf`) that reroutes a request
without touching any code part.

**Moves and renames.** Each part gets a fingerprint of its body, two hashes
(docair-zrr.29): one leaves out the names of the graph's parts and finds
moves and renames; the other keeps every word. A part that kept its id
changed its body only when both differ: the same text is no change, nor is a
caller of a part that was renamed. A part gone
at B and a part new at B with the same fingerprint are one part, moved or
renamed. docair links these itself; the AI links only what it cannot prove.

**What epic 1 gives this epic.** Every line has an id that says what it is
(`shop-ui.cart -call-> orders.create`, docair-svh), so lines are compared by
id, and a moved part's lines are matched once its new id is known. Every
slice has a seed with a stable name (`flow:checkout`, `data.order`, a part's
id; `docair_core::slice::all`), and its ends (`SliceEnd`), so flows are
compared by seed. The page draws any slice the CLI hands it, so the changes
section is two slices per changed flow, drawn by docair.

## 4. Comparing the slices

For every changed part, docair takes the slices through it at A and at B
and compares them by their ends:

| Found | Reported as |
|-------|-------------|
| the same ends, the same path (after moves) | unchanged; not shown |
| the same ends, another path | the flow changed: the steps added and removed |
| an end at A only, or at B only | a flow gone, or a new flow |

Only certain lines count. A change that rests on a line that reaches one of
several parts, or could reach a part, is marked so: in a diff, a wrong line
is a false alarm, and false alarms teach reviewers to stop reading.

## 5. Output

* **JSON** (`docair.changes/0`): the changed parts, the changed flows with
  their steps at A and B, the moves; read by docair-8l6. `working_tree` is
  true when B is the working tree, not a commit, so asking about the pair
  again leaves B out.
* **Markdown:** a short summary, one line per changed flow.
* **The HTML:** a changes section. Each changed flow as one picture of
  before and after (§8).

**Built** (docair-zrr.2 to zrr.6, zrr.8): `SourceRef.end`;
`docair_core::diff` (`fingerprint`, `fingerprints`, `diff`);
`docair_core::changes` (`changes`, `markdown`, `changed_lines`); `docair diff`
in `crates/docair-cli/src/diff.rs`. On docair, HEAD against the working tree
takes 9 s with both graphs scanned. The changes section of the page
(docair-zrr.7, `docair build --changes <A>`) is built in
`crates/docair-viewer/src/page_changes.rs`: the header's **Δ Changes · n**
opens the list of changed flows (new, gone or changed, uncertain ones marked,
the moves apart), and each flow opens at `#changes/<seed>` as one picture of
its slice before and after (§8, docair-zrr.18), drawn like any slice. Both
versions are laid out as one (docair-zrr.17), and since zrr.18 drawn once
too: `merged` adds to B's graph and picture what only A has, A's moved parts
under their ids at B; `marks_of` marks each part and line `ch-added`,
`ch-removed`, `ch-changed` or `ch-same`; `draw_one` lays it out and draws it.
`docair serve --changes <A>` serves the same page and writes it again with
each change; the comparison is shared by the CLI and the server
(`docair_server::commits`).

**The review** (docs/EXPLAIN.md §9, docair-8l6.17 to 8l6.24). The changes
section now leads with the AI's review: changes as the reader understands
them, each on its own picture; the flows below are what the review is built
from, and stay as a view of their own.

## 6. Steps

| Step | Issue | What |
|------|-------|------|
| 0 | docair-zrr.1 | once the slices are done: discuss this doc and docs/EXPLAIN.md, update both and their steps |
| 1 | docair-zrr.2 | parts know where they end: an end line on `SourceRef`, from every adapter; the golden graphs move |
| 2 | docair-zrr.3 | a fingerprint of each part's body |
| 3 | docair-zrr.4 | the graph diff in core, moves linked by fingerprint, golden tests on two graphs |
| 4 | docair-zrr.5 | `docair diff <A> <B>` builds both graphs and keeps a graph for each commit |
| 5 | docair-zrr.6 | changed parts and the slices through them, compared; JSON and Markdown |
| 6 | docair-zrr.7 | the changes section in the HTML |
| 7 | docair-zrr.8 | a fixture repository with two commits: a rename, a new step, a rerouted request, a changed Helm value |
| 8 | docair-zrr.18 | one picture of before and after per changed flow (§8) |
| 9 | docair-zrr.19, zrr.22 | the changes only: marks by kind, the list in order, and a walk over the changes, one step per change (§8) |

Step 5 also needs the slices (docair-702.1), step 6 the generated HTML
(docair-702.5).

## 7. Decided (docair-zrr.1)

| # | Question | Decided |
|---|----------|---------|
| 1 | how commit A's files are read | `git worktree add --detach` of each commit into a scratch folder, removed afterwards; an untracked file of the working tree is never read for a commit |
| 2 | can uncommitted work be B? | yes: `docair diff <A>` compares A with the working tree, `docair diff <A> <B>` two commits |
| 3 | a change in what docair reads (a new adapter, a changed `docair.toml`) | the two graphs' `generator` and `docair.toml` are compared; a difference is reported first, as *the reading changed*, so it is not taken for a change in the code |
| 4 | code indexers (SCIP) are slow; two commits double it | the graph of a commit is kept in the cache folder under the commit, docair's version and the `docair.toml` (`graphs/<key>.json`); the code index is kept by its sources already |
| 5 | how far a flow reaches from a changed part | the slices whose parts hold a changed part, from `slice::all`, compared by seed |
| 6 | (new) the changes section in the HTML | a picture per changed flow at A and at B, drawn by docair like every slice; built in the page's files (the other session's), from the JSON this epic writes |

## 8. One picture of before and after (docair-zrr.18, zrr.19, zrr.22)

**Why.** On docair's own commit ba6ddf0, against the one before it (30
changed flows, 251 parts added, 316 changed), the changes could not be
found:

| What the reader saw | Why |
|---------------------|-----|
| each flow twice, side by side | the eye compares two copies; in *ask* the change is two red boxes on the left and two green ones on the right |
| *Datum graph*: 82 parts per side, words about 5 px high | the change drowns in the flow |
| 7 of 30 flows *changed*, with no mark at all | only their parts' insides changed, and nothing draws that |

**One picture.** Each changed flow is one picture: the union both versions
are already laid out as (docair-zrr.17), drawn once.

| A part or line … | is drawn |
|------------------|----------|
| only after (at B) | filled green, thick, tagged *new* |
| only before (at A) | filled red, dashed, tagged *gone* |
| in both, its code changed (`body`, `kind`, `route` or `part_of`) | filled light amber, tagged *code*; an open box whose own block changed gets an amber outline and name (docair-zrr.23) |
| in both, only its words changed (`summary`, `tags`, `name`) | not marked; listed apart, as §2 says |
| in both, unchanged | as usual, faded (opacity 0.3) |
| moved or renamed (the same fingerprint) | once, under its name at B, not marked; its details say what it was called |

Unchanged parts stay in the picture (docair-zrr.20), so the reader sees
where in the system the change sits. A new flow is all green, a flow that is
gone all red.

Where the path differs, the lines before and the lines after leave the same
part and meet at the same part again. The picture forks and rejoins by
itself, because a shared part is drawn once:

```
            ┌ ─ ─ ► turn ─ ─ ► ask ─ ─ ─ ┐        red, dashed: only before
cmd_ask ────┤                            ├──► chat
            └────► explain ────► load ───┘        green: only after
```

**How it is built** (built, docair-zrr.18). `merged()` (`page_changes.rs`)
gives one graph and one picture; first, A's ids of moved parts are replaced
by B's (docair-zrr.21). The
drawer gets a merged graph, B's plus A's own nodes and edges, for names and
kinds, and marks with several classes at once (`ChangeMarks` in `svg.rs`):
`ch-same`, `ch-changed`, `ch-removed`, `ch-added`. The legend says what
before, after and changed inside look like.

**The changes only (docair-zrr.19, zrr.22).** A changed flow opens on its
changes, not on the whole flow: its picture is a walk over its changes
(`changes_walk`, docair-zrr.24), one step per change, each step's camera on
it at a size to read, its card (*change 2 of 5*, Back, Next) above the
flow's details; Fit shows the whole flow. A new or gone flow has no walk:
the whole picture is the change.

| A step is … | Its card says |
|-------------|---------------|
| a stretch of the path: new and gone parts joined by new or gone lines, with the shared parts it forks off and rejoins at | *Now goes through `a` and `b`; no longer through `c`, at `x`* |
| a line alone, new or gone, between two shared parts | *A new line `x` → `y`* |
| the parts of one box whose code changed | *Code changed in `a` and `b` (in `box`)* |
| an open box whose own block changed | *`browser` itself changed, not the parts drawn in it* |

A shared part is only an end, never a joint: two changes that touch one
shared part stay two steps. A gone stretch and a new stretch that share an
end are one step, the fork; a gone or new line alone counts too, so a
function put in between, or taken out, is one step with the line it
replaced (docair-zrr.26). A stretch with more than 8 new or gone parts is
told box by box; a card names at most 8 parts (docair-zrr.25). Path steps
come first, then code steps, each in reading order.

**Before and after on the card** (docair-zrr.26). A path or line step's card
also shows its route as chains of parts, from where its lines start to
where they end:

| Change | Before | After |
|--------|--------|-------|
| a function put in between | `create_order` → `orders-db` | `create_order` → `validate_order` → `orders-db` |
| taken out again | `create_order` → `validate_order` → `orders-db` | `create_order` → `orders-db` |
| a write that goes elsewhere | `create_order` → `orders-db` | `create_order` → `orders-queue` |
| a new step, no line to it | — | `audit` |

In the picture, the shared parts a step forks off and rejoins at are lit
plainly (a dark outline, their own fill), never in a change's colour; a lit
new line stays solid green, a lit gone line dashed red. A part whose tags
changed what it calls is shown by its lines, not listed as a change in its
words.

**Big changes** (docair-zrr.27, zrr.28). A picture of more than 30 parts is
too big to find its changes in, even step by step: on docair's own work of
one day (`e3d27b7` to the working tree), *Datum graph* has 182 parts and 45
changes. Such a flow gets one picture per change instead: the change's
parts, the shared parts where it forks off and rejoins, and the boxes they
are in (`change_steps::focused_on`). Everything else folds away: a box with
no change in it is drawn closed with what it holds, an unchanged part in a
box that stays counts into its *· n more*, and a line goes to the closed box
its end folded into, one per pair. Laid out again, the change sits close
together. The first change is at `#changes/<seed>`, the others at
`#changes/<seed>/2`, `/3`, …; the card's Back and Next link them. The
details link *All changes in one picture* (`/all`, `change_steps::focused`,
walked as before) and *The whole flow* (`/whole`).

| Flow (`e3d27b7` to the working tree) | The whole flow | Its first change on its own |
|--------------------------------------|----------------|-----------------------------|
| *Datum files* | 90 parts | 12 |
| *Datum page* | 117 | 17 |
| *Datum graph* | 182 | 52 |
| *docair-server.ask* | 59 | 22 |

**The list** puts the flows whose path changed first, then *same path · 4
parts changed in their code: …*, then, apart, the flows whose words or tags
alone changed. A flow's details list *Now goes through*, *No longer goes
through*, *Changed in their code* and *Only their words or tags changed*.

On docair's own commit ba6ddf0 against the one before: every changed flow
opens on its first change with its parts 24 to 47 px high (5 to 17 px
before); *ask* opens on one step, *Now goes through `load` and
`explain_with`; no longer through `turn` and `ask`*; *open* shows its one
change, the box `browser`; *Datum graph* is 50 steps of at most 10 parts.

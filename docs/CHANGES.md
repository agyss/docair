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

**Moves and renames.** Each part gets a fingerprint of its body. A part gone
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
* **The HTML:** a changes section. Each changed flow at A and at B side by
  side; the parts that stayed keep their positions (SLICES.md §5, positions
  from before).

**Built** (docair-zrr.2 to zrr.6, zrr.8): `SourceRef.end`;
`docair_core::diff` (`fingerprint`, `fingerprints`, `diff`);
`docair_core::changes` (`changes`, `markdown`, `changed_lines`); `docair diff`
in `crates/docair-cli/src/diff.rs`. On docair, HEAD against the working tree
takes 9 s with both graphs scanned. The changes section of the page
(docair-zrr.7, `docair build --changes <A>`) is built in
`crates/docair-viewer/src/page_changes.rs`: the header's **Δ Changes · n**
opens the list of changed flows (new, gone or changed, uncertain ones marked,
the moves apart), and each flow opens at `#changes/<seed>` as one picture of
its slice at A and at B side by side, drawn like any slice, what it now goes
through marked green at B and what it no longer does red at A. Both versions are
laid out as one picture (docair-zrr.17): a part both have is in the same
place on both sides, and a part only one side has leaves its room empty on
the other.
`docair serve --changes <A>` serves the same page and writes it again with
each change; the comparison is shared by the CLI and the server
(`docair_server::commits`).

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

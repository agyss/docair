# Changes: what two commits change in app structure and flows

Design for the epic docair-zrr. **Draft:** written before the slices
(docs/SLICES.md, docair-702) were built. Once they are done, this doc and the
epic's steps are discussed again and updated (docair-zrr.1) before any step
starts.

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
2. **the parts the changed lines fall in.** That needs each part to know
   where it ends: today a part has a file and its first line only
   (`SourceRef` in `crates/docair-core/src/model.rs`).

The first catches what the second cannot: a route registered in another
file, a config file (Helm values, `nginx.conf`) that reroutes a request
without touching any code part.

**Moves and renames.** Each part gets a fingerprint of its body. A part gone
at B and a part new at B with the same fingerprint are one part, moved or
renamed. docair links these itself; the AI links only what it cannot prove.

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
  their steps at A and B, the moves; read by docair-8l6.
* **Markdown:** a short summary, one line per changed flow.
* **The HTML:** a changes section. Each changed flow at A and at B side by
  side; the parts that stayed keep their positions (SLICES.md §5, positions
  from before).

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

## 7. To decide

| # | Question | First thought |
|---|----------|---------------|
| 1 | how commit A's files are read | a `git worktree` in a scratch folder, removed afterwards |
| 2 | can uncommitted work be B? | yes: B is the working tree when no commit is given |
| 3 | a change in what docair reads (a new adapter, a changed `docair.toml`) | listed apart, so it is not taken for a change in the code |
| 4 | code indexers (SCIP) are slow; two commits double it | keep the index per commit, next to the kept graph |
| 5 | how far a flow reaches from a changed part | to its ends with stable names, at most as far as its slice |

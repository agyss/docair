# Explaining changes: the AI links and explains

Design for the epic docair-8l6. Written before the slices (docs/SLICES.md,
docair-702) were built, and updated once they were (docair-zrr.1,
2026-11-19): the questions of §7 are decided (decision issues under
docair-8l6).

## 1. The problem

`docair diff` (docs/CHANGES.md) says reliably which flows changed between two
commits, and how. Two things are left that docair cannot do by rules:

* **linking** what it cannot prove is the same: a function renamed *and*
  changed, an old flow and the new one that replaced it;
* **explaining** what a change means: why the new step is there, what the
  user notices.

That is the AI's whole job here. docair does everything it can do reliably,
and the AI looks only at the parts that changed.

## 2. What the AI gets

Only the changed slices from `docair diff`, at A and at B, with their
parts' docs and signatures; never the whole graph. Today `digest`
(`crates/docair-ai/src/ask.rs`) sends every part: 1,293 on docair's own
graph. The input has a size limit; a diff larger than that is split by flow.

## 3. What the AI gives back

| Answer | Example | Checked how |
|--------|---------|-------------|
| a link | `docair-core.extract.parse_body` at A is `parse_block` at B | both parts exist, at A and at B |
| a link between flows | the flow *upload to page* at A is the one at B, now with a cache | both flows are in the diff |
| an explanation | what the change does to the flow, in a few sentences | names only parts and lines of the slices |
| a workflow from the user's side | from *drop a folder* to *the page shows*, before and after | bound like a walkthrough file (SPEC §10.3) |

**Every claim is bound against the graph.** A link to a part that does not
exist, an explanation naming a line that is not there, a step of a workflow
that does not bind: dropped, with a warning (as W109 does for walkthroughs).
Nothing the AI invents gets into the result.

Each answer is recorded with the model and the time, like a walkthrough
(`recorded.by`). A person could once correct one by hand (`recorded.by: human`);
since docair-8l6.75 every review is the AI's, and a store is only kept while it
covers every piece (§9.4).

## 4. Where the findings go

A key in `docair.toml` says where the findings are written. The user decides
what happens to them:

| Where docair runs | What happens |
|-------------------|--------------|
| on a developer's machine | the findings are written there; committed, the next diff starts from them and asks the AI only about what is new |
| in a pipeline for a pull request | the findings become a comment on the pull request; the store is dropped |

## 5. Where it runs

* `docair explain <A> <B>` works without the HTML: Markdown and JSON out.
* The HTML shows the stored findings in the changes section (CHANGES.md §5),
  and asks for more through `docair serve` (SLICES.md §8).

It reuses the docair-ai crate, the model configuration (`[ai]`, the key only
in `.env`) and the fake AI for tests. A run has a cost limit.

**Built** (docair-8l6.1 to 8l6.5, 8l6.7): `docair_ai::explain`
(`digest`, `bind`, `explain`, `comment`) and `docair explain <A> [<B>]`
(`crates/docair-cli/src/diff.rs`). The model gets each changed flow's seed,
how it changed, its changed, added and removed parts in a few words, and its
lines added and removed; the first request also the parts removed at A and
added at B, to link. Its answer is bound: a link only from a part removed at
A to one added at B, an explanation only of a changed flow and naming only
ids of A or B, a workflow step only with parts of A (before) or B (after).
The findings go to `docs/changes/<A>..<B>.json` (`docair.explained/0`, short
commit ids, `work` for the working tree); a flow already there, a human's
correction above all, is not asked about again. The fake AI answers
explain requests; e2e `cli.spec.ts` runs it on two commits of the shop.
The page (docair-8l6.6, `crates/docair-viewer/src/page_changes.rs`) shows a
flow's finding in its details in the changes section, and walks its workflow
before and after on that side's picture (`#changes/<seed>/before`, `/after`);
the list shows the links and counts what is not explained yet, with the
command, and a button when docair serve shows the page; e2e
`changes.spec.ts`.

## 6. Steps

| Step | Issue | What |
|------|-------|------|
| 1 | docair-8l6.1 | the AI's input: the changed slices only, with a size limit |
| 2 | docair-8l6.2 | linking what docair cannot prove is the same, bound against both graphs |
| 3 | docair-8l6.3 | explaining what each change does to its flow |
| 4 | docair-8l6.4 | workflows from the user's side, before and after |
| 5 | docair-8l6.5 | the store, `docair explain <A> <B>`, the comment on the pull request |
| 6 | docair-8l6.6 | the HTML shows the findings |
| 7 | docair-8l6.7 | tests with the fake AI, and the cost limit |
| 8 | docair-8l6.11 | explain as a job: progress while it runs, batches in parallel (§8) |
| 9 | docair-8l6.12 | the AI's words on each change's step, by a stable step id (§8) |
| 10 | docair-8l6.17 | the review's design (§9) |
| 11 | docair-8l6.18 | the pieces of the whole change, across every flow (§9.2) |
| 12 | docair-8l6.19 | the intent: commit messages and the changed prose (§9.2) |
| 13 | docair-8l6.20 | the review request: the AI groups the pieces into changes (§9.3) |
| 14 | docair-8l6.21 | bind and cover (§9.4) |
| 15 | docair-8l6.22 | the page: the review, each change on its own picture (§9.5) |
| 16 | docair-8l6.23 | `docair explain` and the pull request comment (§9.6) |
| 17 | docair-8l6.24 | tests (§9.7) |

Step 1 needs `docair diff`'s changed slices (docair-zrr.6), step 6 needs
`docair serve` (docair-702.6).

## 7. Decided (docair-zrr.1)

| # | Question | Decided |
|---|----------|---------|
| 1 | which model by default, and the input's size limit | the configured model (`[ai]`); everything that changed goes whole in one request, at most `[explain] max_chars` (default 1,000,000 characters); a review that needs more is an error, never split (docair-8l6.70, 8l6.79, replacing the 60,000-character requests sent in parts) |
| 2 | what a person's correction looks like | none: every review is the AI's (docair-8l6.75). A stored review is kept while it covers every piece, as stored; else it is asked again whole |
| 3 | does a workflow from the user's side need a declared start? | no: it starts at the first part outside the system (`external`) on a changed flow, else at the flow's first step |
| 4 | the language of the comment on the pull request | plain English, one line per changed flow, the explanation under it, links and workflows folded in `<details>` |
| 5 | (new) where the findings go | `[explain] store` in `docair.toml`, default `.docair/changes` (docair's own output folder, so reviews are not part of the project; docair-8l6.74, before `docs/changes`): one JSON file per pair of commits (`<A>..<B>.json`) |
| 6 | (new) the cost limit | the review is one request, and one more for the pieces left in no change; its size is `[explain] max_chars`. `[explain] max_requests` is still read, so an old docair.toml loads, and no longer used (docair-8l6.79) |

## 8. A walkthrough of each change (docair-8l6.11, 8l6.12)

**Why.** On docair's own commit ba6ddf0, *Explain them* answered after
4 min 20 s (5 requests, 27 of 30 flows explained) and showed nothing in the
meantime. Its workflows before and after play on two separate pictures, so
they cannot show where the flow now goes another way.

**The walkthrough of the changes (docair-8l6.12).** The reader wants the
changes only, not a tour of the flow. docair already walks a changed flow
change by change (CHANGES.md §8, `changes_walk`): one step per change, its
camera on it, its card worded from the parts' names. *Explain them* gives
those steps the AI's words: for each step, what used to happen and what
happens now, from the user's side. It adds no steps along the unchanged rest
of the flow, and replaces the workflows before and after.

| Rule | How |
|------|-----|
| each step has a stable id | `<seed>#path:<id>` for a stretch of the path, `<id>` its first new or gone part by id; `<seed>#line:<from>-><to>` for a line alone; `<seed>#code:<box>` for the parts of a box whose code changed; `<seed>#box:<id>` for an open box whose own block changed |
| the AI answers by id | the digest sends each step's id with its parts and lines; the answer is `{step id: words}`, and `bind` keeps only the ids docair made, so nothing invented gets through |
| every change has its step | a step the AI did not word keeps docair's words (docair-8l6.13) |
| findings stored before | a finding with workflows but no step words counts as *not explained yet* and is asked about again; a human's correction is kept (docair-8l6.15) |

To share the ids, the steps' grouping (not their cameras) moves from the
viewer to docair-core, so the digest and the page work them out alike.
Without the AI, the picture and its walk over the changes work as they are.

**Built, the AI's side** (docair-8l6.12): the digest sends each changed
flow's steps from `docair_core::change_steps` (id, kind, the parts added,
removed and whose code changed, where the stretch forks off, its lines),
and the prompt asks for `steps: {id: words}` instead of workflows.
`FlowExplained.steps` holds them; `bind` keeps words only on the flow's own
step ids and only when they name parts and lines of A or B. A stored
finding counts as explained when a human wrote it, when it words the
flow's steps, or when the flow has no steps; otherwise it is asked again and
replaced. The pull request comment lists the step words under each flow.
Workflows are gone: the model is not asked for them, and a stored one is
not read. Test:
`step_words_bind_by_docairs_step_ids` (shop: B also writes the order; one
step `data.order#path:orders-db`).

**Explain as a job (docair-8l6.11).**

| Call | Answer |
|------|--------|
| `POST /api/explain` (`a`, `b`) | at once: a job id; a second call while a job runs answers that job |
| `GET /api/explain/<job>` | the requests started, answered and left, the flows asked about, the time so far, an error, the changes made so far (`found`) and the page's view of it (`html`, docair-8l6.65); kept 10 minutes after the job ends |

* The review is one request (docair-8l6.79), its answer streamed
  (`client::chat_streamed`); then one more for the pieces left in no change.
  A chat question never waits behind it: the server allows 2 questions to
  the model at once (`ASKING_AT_ONCE`) besides the review.
* The page shows the job under its button as the server writes it: a bar
  that moves while the AI works, words such as *The AI reviews 30 pieces ·
  2 changes so far · 1:12*, then each change as soon as the streamed answer
  holds it whole (docair-8l6.65), and asks again every second. The last
  request may still place pieces into them. When the job ends, the server
  writes the page again and the page reloads (docair-8l6.16). The page's
  script stays within its line budget: the HTML comes from the server.

**Built** (docair-8l6.11, 8l6.79): `docair_ai::explain::explain_with` sends
the review in one streamed request, says when it starts and answers
(`ExplainProgress`), and binds each change as soon as the answer holds it
whole (`complete_changes`), so the changes so far go out with the progress
(`ExplainProgress.found`); the whole answer is then bound as before. An
endpoint that does not stream is read whole. A failed request is an error. The server keeps the jobs
(`ExplainJobs`, `crates/docair-server/src/project.rs`); `GET
/api/explain/<job>` works out `seconds` and `text` when asked. `docair
explain` uses the same and says each answered request on stderr. Tests:
`explaining_runs_as_a_job_and_says_how_far_it_is` (a fake model in the
test), the e2e `docair explain` test follows two jobs to their findings.

## 9. The review: changes as the reader understands them (docair-8l6.17 to 8l6.24)

**Why.** A reviewer reviews *changes*, not flows. On docair's own work of
2026-11-19 (`e3d27b7` to the working tree), the changes section listed 31
flows: the new peers Service showed up in 16 of them, and a flow like
*Datum page* held 28 steps cut by structure, not by what they mean. 699
parts changed (305 new, 18 gone, 376 edited in their code); joined by their
lines they make one blob of 247 parts and 391 single parts. docair finds the
pieces reliably, but not where one intent ends and the next begins.

### 9.1 What a change is

A **change** is one difference in what the app does: added, removed or
adjusted behaviour, with every piece that works together for it
(docair-8l6.25). The AI makes each change understandable; docair finds the
pieces and checks every claim. Version 0 needs the AI: without a model the
review says it needs one and how to set it (docair-8l6.26). The number of
changes is not capped (docair-8l6.27).

| A change says | Example |
|---------------|---------|
| *what* the app does differently, one sentence from the user's side | An uploaded page keeps working when the cluster runs two server pods. |
| *how*: its pieces in the order they work together, each in a sentence | `upload_id` names the pod that took the upload; `forward` sends a request for it there, through the new Service `docair-server-peers`. |
| *why*: the AI's own reading of the code, the commits and the docs together, never a quote or a citation (docair-8l6.55) | A user's upload lands on one pod; the others now ask that pod for it instead of answering that it is gone. |
| *off*: what the AI found off, each of one of three kinds (docair-8l6.62): *contradicts*, what a commit message, a doc or a code comment says, and which one, against what the code clearly does otherwise (docair-8l6.58); *unsaid*, what the code does that nothing mentions; *risk*, a mistake, a missed case or a risk in the code; or that it found nothing | *Said:* the commit “Audit every order”: every order is audited. *The code:* `audit` has an empty body, so it records nothing yet. |
| *what to check*: the risky part | The NetworkPolicy must let the server pods reach each other. |
| *sources*: the commit messages (by subject) and docs (by path) the AI read for it, listed apart and folded; only those there are, as they are (docair-8l6.71) | the commit “Two server pods”; `docs/DEPLOY.md` |

The commit messages and docs are claims the AI checks against the code, not
sources to cite: they go to it without ids, and the prompt asks it never to
quote or name a commit, doc, issue or ticket (decision docair-8l6.55,
replacing the cited sources of 8l6.24 and the disagreements of 8l6.30).
Only an *off* names and repeats what was said, so a reader sees both sides
of it (decision docair-8l6.59), and of the offs only a contradiction does
(docair-8l6.62). The prompt asks for clear ones: what was said and what the
code does cannot both be true. Something no commit, doc or comment mentions,
or mentions only in part, is *unsaid*; a fault the AI sees in the code is a
*risk*, whatever was said of it.

### 9.2 What the AI gets

**The pieces** (docair-core `review::pieces`). The change steps of every
changed flow (CHANGES.md §8), merged across flows by a key that does not
name the flow (`path:<first new or gone part>`, `line:<from>-><to>`,
`code:<box>`, `box:<id>`): a piece that shows up in 16 flows is one piece,
and says which flows it touches. A changed part no flow goes through
becomes a piece of its box, so nothing changed is missing. Each piece goes
in a few words: its id, its kind, its new, gone and edited parts and its
ends (id, kind, name, summary, signature), its new and gone lines, the
flows it touches; and its parts' whole texts (below).

**The intent** (docair-server `commits::intent`). The commit messages from A
to B (subject and body; for the working tree as B, the commits up to `HEAD`
and a note that the rest is not committed); the changed Markdown files, as
their added and removed lines; the doc blocks whose words alone changed,
their summary before and after. Each source has an id the AI cites:
`commit:<short sha>`, `doc:<path>`, `file:<path>`, the part's id.

**All of what changed, to read** (docair-8l6.46, decision 8l6.47; it
replaces 8l6.29, signatures and summaries only). Each new, gone and edited
part's whole text, at A and at B: its source range with the doc block,
comments and attributes right above it (docair-core `review::part_texts`),
read from the commits with `git show`, or from the working tree. A piece
carries its parts' texts as `source`: `{"<part>": {"before": …, "after":
…}}`. The files that changed but hold no part (a stylesheet, a Dockerfile, a
config) go as their changed lines, `files` (`commits::other_files`);
Markdown goes as docs, and lock files, the beads export, the review store
and binary files not at all. The model reads the texts to say what an edit
does.

Nothing is cut (docair-8l6.79): a commit message's body, a Markdown file's
changed lines, a part's whole text before and after, a changed file no part
holds, every list of a piece go whole, however many. The review is one
request, at most `[explain] max_chars` characters (default 1,000,000,
docair-8l6.70); a review that needs more is an error that says how many it
needs and how to raise the limit or review a smaller range, never split. How
much the model reasons first is `[explain] reasoning` (`low`, `medium`,
`high`; default `low`, docair-8l6.78).

Then one last request (decision docair-8l6.56, docair-8l6.69) gets every
change by number, with its title, why and what its pieces do, and the pieces
no change holds yet, with their code; it goes when a piece is left. It
answers where each left piece belongs, with what it does there, or which
changes of their own the left pieces that belong to none make (docair-8l6.63):

```json
{"place": [{"piece": "<id>", "change": 2, "words": "…"}],
 "new": [{"title": "…", "how": [{"piece": "<id>", "words": "…"}], "why": "…", "check": "…"}]}
```

A left piece is named by its id, a part it holds or a part in its box
(`explain::resolve_left`); a place that does not bind is left out and said
(`place_left`). Its new changes are bound as the review's are (§9.4), of the
left pieces only.

Every piece is a change; there are no *other edits* (docair-8l6.63, decision
docair-cum, replacing 8l6.28 and 8l6.42). A piece the AI still puts in no
change, when the last request fails or is cut short, is kept in the store's
`other` and shown as a change of its own after the AI's: titled in docair's
words for the piece (*Code changed in `total`.*), marked *in docair's words:
the AI put this piece in no change*, with no why, nothing found off and
nothing to check, since the AI checked nothing of it. The same holds for a
piece a review stored before 8l6.79 did not reach (`left`) and for one newer
than the stored review, when the change moved on after `docair explain`
(docair-8l6.67): each is a change of its own in docair's words, marked so.
A review that does not cover every piece says how many it misses and offers,
served, *✦ Review again*; a stored change none of whose pieces are there now
is left out with what it found off, and the notice says how many (docair-8l6.68).

### 9.3 What the AI gives back

```json
{"changes": [{"title": "…", "how": [{"piece": "<piece id>", "words": "…"}],
              "why": "…",
              "off": [{"kind": "contradicts | unsaid | risk",
                       "source": "commit | doc | comment | none", "where": "<the commit's subject, the doc's path, the part whose comment it is>",
                       "says": "…", "code": "<what the code seems to do>",
                       "parts": ["<the id of each part where the code does it>"]}],
              "check": "…",
              "sources": [{"source": "commit | doc", "where": "<the commit's subject, the doc's path>"}]}],
 "links": [{"a": "<id at A>", "b": "<id at B>", "why": "…"}]}
```

As many changes as there are; a piece belongs to one change at most; parts
are named by their ids in backticks. `links` stay as before (§3): a part
removed at A that is the same as one added at B.

### 9.4 Bind and cover

| Rule | How |
|------|-----|
| a change has a title and a piece | else it is left out, and said |
| a `how` entry names a piece docair made | only a piece's own id binds (docair-8l6.81, replacing 8l6.37). The prompt says so with an example of each; a name that is no piece id (a part's id, or one made up) goes back to the model once, in a repair request that lists the pieces holding that part to pick from (`misnamed`, `repair_names`); what is still no piece after it is left out, and said. A piece may be in several changes, each with its own words (docair-8l6.76, replacing 8l6.32) |
| words name parts and lines there are | a title that does not is left out with its change; a `how`, `why`, `check` or `off` that does not is left out alone. A piece id in backticks is named as its part when the piece stands for one part or is named after one (`path:orders.audit` → `orders.audit`, docair-8l6.64) |
| an *off* says where only of what is there (docair-8l6.59) | a commit by its subject, made the subject as it is; a doc by its path, made whole; a comment by its part's id; else its `where` is left out, and said. An off without `code` is left out. Only a contradiction says what was said and where: one that names nothing said is kept as a risk, and said; an *unsaid* or a *risk* keeps no source (`none`). A bare sentence is a risk. An off stored before the kinds is a contradiction where something was said, else a risk. Its `parts` keep only parts of A or B, else the parts its `code` names (docair-8l6.61) |
| every piece is a change | the prompt asks for every piece in its change, a piece part of no other change a change of its own; the last request places the pieces left or makes changes of them (§9.2); the rest is kept in `other`, each shown as a change of its own in docair's words (docair-8l6.63) |

The findings go to the store (§4): `.docair/changes/<A>..<B>.json` by default (docair-8l6.74),
schema `docair.explained/2`, with `changes`, the pieces the model left as
`other`, and `links`. A store of `/1` (with `sources` and `disagrees`,
quoting and citing) is asked again. A store of an older schema, or one whose changes and
`other` do not cover today's pieces, is asked again whole; one that covers
them is kept as stored. There are no hand corrections (docair-8l6.75).

### 9.5 The page

**Δ Changes** opens the review: the changes, in the model's order (what
matters to the user first), then the flows they touch, with how many of the
system's flows changed and all of them folded, each a link to its picture and
a changed one to its change (docair-8l6.57). A
change opens at `#review/<n>` on its own picture: its pieces' parts at A and
at B as one picture (docair-8l6.33), and one step around them, faded: the
parts one line away, what calls them and what they call, at most 12 a part
(`review::AROUND`, docair-8l6.77), marked as in CHANGES.md
§8. Its walk opens on *What and why*, headed by the change's whole title:
why, then what the AI found off, marked (*⚠ Found off*), by kind, each
under its heading, contradictions first (docair-8l6.62): *Said, but the code
does otherwise*, each as *Said* (by which commit, doc or comment) over *The
code* (docair-8l6.58); *Done, but said nowhere* and *Looks wrong or risky*,
each as *The code*; every one with the parts where the code does it as links
and their code folded (docair-8l6.61). Or *✓ Checked against the
code: nothing found off*, then what to check, framed on the
whole change; then *How · k of n*, its pieces in the order of *how*, each
card the model's words for that piece over docair's own (Before and After)
and its code (docair-8l6.43). The details below the card hold only what the
card does not (docair-8l6.54): which change of how many, from and to, the
flows it touches, who wrote it. Closing the card leaves the change, so the
two are always seen together. Without findings the review says the AI makes
it, with the command, and, served, a button.

**Down to the code** (docair-8l6.49, 8l6.82, replacing decision 8l6.51). Each
piece, on its card and in the details' *How*, and what was found off, list
*The code · n parts*: each of its new, gone and edited parts, by id (a link to
the part), with a link to its file and line: at B for a new one, at A for a
gone one, *before* and *after* for an edited one. A commit opens on its
repository host at that commit (from the `origin` remote: GitHub or GitLab),
the working tree in the editor (`vscode://file/<root>/<path>:<line>`); where a
version has no link, its file and line show alone (`doc_page::CodeLinks`,
`commits::code_links`, `page_changes::piece_code`). The page embeds no code, so
it does not grow with the change's code. A flow's change steps list the same.

### 9.6 `docair explain` and the comment

`docair explain <A> [<B>]` asks, stores and prints the review: each change
with what, how, why, what was found off by kind and what to check, then each
piece no change holds as a change of its own in docair's words, links;
`--json` the store. The pull request comment is the same in Markdown, *how*
folded.

### 9.7 Tests

Rust: the pieces of two flows that share one change are one piece; the
intent of a two-commit repository (messages, a README section, a doc
block's words); bind (unknown piece, unknown id, unknown source, a piece in
two changes) and cover (other edits); the store asked again when it does not
cover; a part's text holds its comments (`a_parts_text_holds_its_comments`);
the intent holds a changed function's text before and after and a changed
stylesheet's lines; a piece in the request carries its texts; changes that
came back twice are merged, a bad group left apart
(`changes_that_came_back_twice_are_merged`); a line diff; the page's fold
with the diff, on the card and in the details. The fake AI groups the path
pieces into one change and leaves the code edits as other edits. e2e: a shop
with two commits whose messages say what they do and a README section that
changes; the review lists the change, opens it on its picture with Before
and After and its explanation, lists other edits with the cart's code, 100
struck and 120 added, and the new step's code on its card; no findings gives
the notice.

### 9.8 Built (docair-8l6.18 to 8l6.24)

| Piece of the review | Where |
|---------------------|-------|
| the pieces, on one picture of the whole change, marked by the graph diff | `docair_core::review::pieces` |
| the intent: commit messages, changed Markdown, changed doc-block words | `docair_server::commits::intent`, `docair_core::review::words_changed`; `CommitPair.intent` |
| the request, compact (docair-8l6.34), without reasoning first (docair-8l6.36) | `docair_ai::explain::{digest, explain_with}`, `REVIEW_PROMPT`; `client::chat_with` |
| bind and cover, the store `docair.explained/2` | `docair_ai::explain::bind`; `ChangeFindings { changes, other, links }` |
| the page: the review list, `#review/<n>`, a left piece as a change, the notice | `docair_viewer::page_changes::review_view`, `docair_core::review::change_picture` |
| `docair explain` and the comment | `docair-cli` `cmd_explain`, `docair_ai::explain::comment` |

With the configured model (`zai-org/GLM-5.3-Flash`): docair's commit
`02ffe98` (21 pieces, 7,000 characters in) gives two changes that match the
commit, each with its why from the commit and CHANGES.md and a check; this
step's own work (45 pieces, 40,000 characters) gives three changes in 11 s:
the review, big flows drawn change by change, and the two fingerprint hashes,
with 28 other edits. Running it found four faults, now fixed: the model
reasoned until `max_tokens` ran out (`reasoning_effort: low`,
docair-8l6.36); the diff called 16 untouched bodies edited, because the
fingerprint left out the names of the graph's parts and the two graphs had
different names (docair-zrr.29); the model named parts where pieces belong
(docair-8l6.37); and it copied whole lines as the ids of links. Tests: the Rust tests named in §9.7; e2e
`cli.spec.ts` (*docair explain reviews the change and keeps the review*)
and `changes.spec.ts` (*a reviewed change opens on its own picture, with why
and other edits*), with the fake AI.

**Reported on the live page** (docair-8l6.38 to 8l6.40): the flows were not
to be seen below 32 other edits; the other edits were unfair, most of them
pieces of a change; a change opened on its first piece under *This change*.
On the same work (`02ffe98` to the working tree), asking for every piece
in its change left 9 other edits of 46 pieces, and docair's module rule put
5 of them with their change: 4 remain.

**All of the code, and down to it** (docair-8l6.46 to 8l6.51). The model got
each changed part's whole text: on the shop example, where it had said only
that the cart's code changed, it now says the total is 120 cents an item
instead of 100, and that the commit, which speaks of audits only, does not
say so. On `fac7aa8` to `c1364fd` the input took two requests, and two of
five changes came back twice; the merge request made them three changes, no
other edits, in three requests.

**The AI's own reading, checked** (docair-8l6.53 to 8l6.56). The user found
the card repeated below it, and the text quoting and citing its sources.
Now the details hold only what the card does not; the model gets commits and
docs as claims without ids, writes in its own words and lists what it found
off. On the shop example it writes *Auditors need a trail of each order, so
the API now records one as the final step of checkout* and finds the new
`audit` function empty and the price rise unannounced; the last request put
the cart's own block into the price change, no other edits left. On
`c1364fd` to the working tree: four changes, no repeats.

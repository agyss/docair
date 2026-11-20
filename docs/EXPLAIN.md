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
(`recorded.by`). A person can correct one by hand (`recorded.by: human`).

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
| 1 | which model by default, and the input's size limit | the configured model (`[ai]`); the input is cut at 60,000 characters per request, a diff larger than that is sent flow by flow |
| 2 | what a person's correction looks like | the findings file is edited by hand and its `recorded.by` set to `human`; docair never asks the model again about a finding a human recorded |
| 3 | does a workflow from the user's side need a declared start? | no: it starts at the first part outside the system (`external`) on a changed flow, else at the flow's first step |
| 4 | the language of the comment on the pull request | plain English, one line per changed flow, the explanation under it, links and workflows folded in `<details>` |
| 5 | (new) where the findings go | `[explain] store` in `docair.toml`, default `docs/changes`: one JSON file per pair of commits (`<A>..<B>.json`) |
| 6 | (new) the cost limit | `[explain] max_requests`, default 8 requests per run; a run that would need more stops and says which flows are left |

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
| `GET /api/explain/<job>` | the requests started, answered and left, the flows asked about, the time so far, an error; kept 10 minutes after the job ends |

* Batches go out in parallel, at most 3 at once (docair-8l6.14). With the 2
  questions to the model the server allows (`ASKING_AT_ONCE`), up to 5
  requests reach the provider at once, and a chat question never waits
  behind an explain. `docair explain` sends its batches the same way.
* The links to make go with the first batch; links are merged as each batch
  answers.
* `[explain] max_requests` counts the requests started; the flows left over
  are *not explained yet*, as today.
* The page shows the job in words the server writes, for example *The AI
  explains 30 flows in 5 requests · 2 answered · 1:12*, and asks again every
  second or two. When the job ends, the server writes the page again and the
  page reloads (docair-8l6.16). The page's script stays within its line
  budget: the words come from the server.

**Built** (docair-8l6.11): `docair_ai::explain::explain_with` sends the
batches on up to `AT_ONCE` (3) threads, says each start and answer
(`ExplainProgress`), and merges the answers in the batches' order, so the
findings do not depend on which answered first; a failed request leaves its
flows, and only when none answers is it an error. The server keeps the jobs
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
| *why*, from the commits and the docs, the sources named | commit `b0e4541`; docs/SLICES.md |
| *what to check*: the risky part | The NetworkPolicy must let the server pods reach each other. |
| where a commit message or a doc and the pieces disagree (docair-8l6.30) | The commit says the uploads are shared, but each pod still keeps its own. |

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

| Input | Cut at |
|-------|--------|
| a commit message's body | 2,000 characters |
| a Markdown file's changed lines | 2,000 characters |
| all Markdown files together | 12,000 characters |
| a part's text, one version | 4,000 characters |
| a piece's texts together | 20,000 characters, then *n more parts not shown* |
| a changed file no part holds | 2,000 characters; all together 10,000 |
| a request, everything together | 60,000 characters (docair-8l6.8) |

A review too big for one request is asked in parts, the pieces in their
order, the intent with each part. Asked in more than one, the same change
can come back from two parts, so a last small request gets every change by
number, with its title, why and what its pieces do, and answers which are one:
`{"same": [[1, 4]]}`. docair merges each group into its first change: the
pieces in order, the sources and disagreements without repeats; a group that
names no two changes, or one twice, is left apart and said
(`explain::merge_same`, docair-8l6.48, decision 8l6.50, replacing 8l6.31).

### 9.3 What the AI gives back

```json
{"changes": [{"title": "…", "how": [{"piece": "<piece id>", "words": "…"}],
              "why": "…", "sources": ["commit:b0e4541", "doc:docs/SLICES.md"],
              "check": "…", "disagrees": ["…"]}],
 "links": [{"a": "<id at A>", "b": "<id at B>", "why": "…"}]}
```

As many changes as there are; a piece belongs to one change at most; parts
are named by their ids in backticks. `links` stay as before (§3): a part
removed at A that is the same as one added at B.

### 9.4 Bind and cover

| Rule | How |
|------|-----|
| a change has a title and a piece | else it is left out, and said |
| a `how` entry names a piece docair made | a part's id, or a piece id guessed from it (`path:<part>`), binds to the piece that holds that part (docair-8l6.37); else that entry is left out; a piece already in an earlier change stays there (docair-8l6.32) |
| words name parts and lines there are | a title that does not is left out with its change; a `how`, `why`, `check` or `disagrees` that does not is left out alone |
| a source is a commit in the range or a doc docair sent | else it is left out |
| every piece belongs somewhere | the prompt asks for every piece in its change; a piece in no change goes with the change that has the most pieces in its module, when one has more there than every other (*also changed with it*, docair-8l6.41); the rest is an *other edit*, which the review lists (docair-8l6.28) |

The findings go to the store as before (§4): `docs/changes/<A>..<B>.json`,
schema `docair.explained/1`, with `changes`, the pieces the model left as
`other`, and `links`. A store of an older schema, or one whose changes and
`other` do not cover today's pieces, is asked again; a change a human wrote
(`recorded.by: human`) is kept, and its pieces are not asked about.

### 9.5 The page

**Δ Changes** opens the review: the changes, in the model's order (what
matters to the user first), then the flows they touch, with how many of the
system's flows changed, then the *Other edits*, folded (docair-8l6.42). A
change opens at `#review/<n>` on its own picture: its pieces' parts at A and
at B as one picture, nothing else (docair-8l6.33), marked as in CHANGES.md
§8. Its walk opens on *What and why*, headed by the change's title: what was
done, why with its sources, what to check, where it disagrees, framed on the
whole change; then *How · k of n*, its pieces in the order of *how*, each
card the model's words for that piece over docair's own (Before and After),
then what docair put with it (docair-8l6.43). Its details say the same in the
same order, then the flows it touches and who wrote it. Without findings the
review says the AI makes it, with the command, and, served, a button.

**Down to the code** (docair-8l6.49, decision 8l6.51). Each piece, on its
card, in the details' *How* and among the other edits, has a fold, *The code
· n parts*: each of its new, gone and edited parts, by id (a link to the
part), with its file and line and its whole text as a line diff, the lines
both versions keep, removed lines struck, added ones marked. These are the
texts the AI read (`Changed::code`, `page_changes::piece_code`), so the page
grows by the change's code, not by the code base's. A flow's change steps
have the same fold.

### 9.6 `docair explain` and the comment

`docair explain <A> [<B>]` asks, stores and prints the review: each change
with what, how, why and what to check, *Other edits*, links; `--json` the
store. The pull request comment is the same in Markdown, *how* and the other
edits folded.

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
| bind and cover, the store `docair.explained/1` | `docair_ai::explain::bind`; `ChangeFindings { changes, other, links }` |
| the page: the review list, `#review/<n>`, other edits, the notice | `docair_viewer::page_changes::review_view`, `docair_core::review::change_picture` |
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

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

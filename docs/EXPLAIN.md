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

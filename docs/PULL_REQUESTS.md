# Pull requests: docair's review on the pull request

docair already compares two commits (CHANGES.md) and has the AI review the
change (EXPLAIN.md §9). This design brings that review to a pull request: a
comment for each change, with a picture of it, and a link to walk through it
on the page.

Epic: docair-pr. A sample, built on the branch `pr-review`. The choices made
while building it are decision beads (docair-pr.8 to pr.25), to go over.

## 1. The problem

A reviewer reads a pull request on GitHub. The review docair makes is in a
page on someone's machine, or in `docair explain`'s output. Nothing of it
reaches the pull request.

On the pull request there are limits:

| Limit | What it means |
|-------|---------------|
| a comment runs no script and keeps no `style` | no SVG or HTML picture can be drawn in a comment |
| an image in a comment is fetched without a login | an artifact (login needed) does not show as an image |
| an artifact needs a login and lasts 90 days | fine for a reviewer; a link on an old pull request ends |
| a fork's pull request gets no secrets and a read-only token | no AI, and no comment from that run |

## 2. What it does

```
docair pr build <BASE> [<HEAD>]      # the review, the page, the comments, in .docair/pr
docair pr plan --existing <file>     # which comments to write, change or mark gone
```

1. **A and B.** A is the merge base of BASE and HEAD, B is HEAD's commit
   (default `HEAD`). That is the diff GitHub shows under *Files changed*.
   The base branch's tip would show what landed there since as undone.
2. **The review.** As `docair explain A B`, when an AI key is there. Without
   one (a fork), every piece is a change of its own, in docair's words.
3. **The page.** B's page with its changes section and the review
   (`docair-review.html`), one file, uploaded unzipped as the run's artifact.
4. **The comments.** One summary comment, and one comment per change in the
   review's order. Each holds the change's picture as text, why, what was
   found off, what to check, how (folded), and a link to `#review/<n>` on
   the page: the walk.
5. **The plan.** The workflow lists the pull request's comments. `docair pr
   plan` matches them to the new ones and says, for each, create, update,
   keep or gone. The workflow only carries it out with `gh api`.

**On master without the workflow (2026-11-21).** `docair pr build` and
`docair pr plan` are on master; the GitHub Action that runs them on a pull
request (§5) is not, by the user's choice: it stays on branch `pr-review`
(ce8f2df) for now. Until it comes, the commands run by hand or from any CI.

## 3. Pictures as text

A comment cannot hold a drawing, but it can hold text in a code block. The
picture is the same layout as the SVG (`layout_input::lay_out`), drawn with
box-drawing characters (`docair_viewer::text_pic`). The shop's new write, as
its comment shows it:

```text
┌─ infra host ────────────────────────────┐
│┌─ shop cluster ────────────────────────┐│
││┌─ orders-api ─────┐                   ││
│││                  Order ┌───────────┐ ││
│││ ┌──────────────┐ │     │ orders-db │ ││
│││ │ create_order ╠═╬════►│           │ ││
│││ └──────────────┘ │     └───────────┘ ││
│││                  │                   ││
││└──────────────────┘                   ││
│└───────────────────────────────────────┘│
└─────────────────────────────────────────┘
```

* **Compacted.** Every side of a part, of a box's border, of a line's
  stretch and of a line's words is a value on its axis. Values keep their
  order, and may share a cell unless something needs them apart: two things
  that overlap along one axis stay one cell apart along the other, two
  beside a part, so an arrow fits in front of it. Then each name and each
  line's words get the cells they need. What does not cross in the SVG does
  not cross here; sizes are not kept.
* **Ends.** Where a line touches a part it stays inside that side, off the
  corners.
* **Words.** A line's words keep off parts, other words and lines; they may
  lie on a box's border, as in the SVG, where words have a background.
  Words with no free place within 48 px become a note: `[1]` in the picture,
  the words under it.
* **Box names.** In the box's top border, moved right off a line that
  crosses it there.
* **Marks.** `+` new, `−` gone, `~` its code changed, before the name. New
  parts and lines are double, gone ones ASCII (`+ - :`).
* **Characters.** Only light and double box drawing and `► ◄ ▲ ▼`, all in
  WGL4: GitHub's code font falls back character by character, and a glyph of
  another width breaks the columns. Heavy, dashed and rounded box drawing
  are not in every font (Consolas), so they are not used.
* **The cut.** A comment's picture holds only the change's pieces' parts and
  the boxes around them (`docair_core::review::change_cut`), not the parts
  one line away the page brings (docair-8l6.83): a comment is about 800 px
  wide and cannot be zoomed. A change whose cut has no line gets no picture.
  Past 30 parts it is folded; a comment past 60,000 characters leaves its
  picture to the page.

Each change's cut is written as an SVG file too (`pictures/<n>.svg`, the
page's colours, light and dark), for a later host that shows colour (a
branch or docair-server).

## 4. Comments across pushes

Each push runs again, and the AI's changes have no ids: change 3 may come
back as change 2, split, or merged. So each change comment carries its
pieces in a hidden marker:

```
<!-- docair:pr {"kind":"change","pieces":["path:orders.audit","code:shop-ui"]} -->
```

The plan matches a new change to the old comment that shares the most
pieces, at least half of the larger of the two. Matched: updated in place
(or kept, when its text is the same). New: created. An old one no new change
matches is *gone*: it stays, cut to one line, its old text folded, so a
reply below it still makes sense. The summary comment is one, always updated.

Only comments with the marker are touched, and with `--author` only those of
that login.

## 5. The workflow

Not on master yet (§2): `.github/workflows/docair-pr.yml` on branch
`pr-review`, on `pull_request`:

| Step | What |
|------|------|
| check out | the head commit, the whole history (the merge base needs it) |
| tools | Rust and `rust-analyzer` (the code index, ADAPTERS.md §5.4) |
| build | docair from this repository |
| review | `docair pr build origin/<base> <head sha>`, `DOCAIR_AI_API_KEY` from the secrets |
| page | `actions/upload-artifact@v7.0.2`, `archive: false`: its `artifact-url` |
| comments | `gh api` lists them; `docair pr plan --page-url <url>`; `gh api` writes them |

A fork's pull request builds the page and uploads it, but posts nothing:
its token cannot write. A two-stage workflow (an unprivileged build hands
its graphs to a privileged job that only reads them) is the next step.

## 6. Steps

| Step | Issue | What |
|------|-------|------|
| 1 | docair-pr.1 | this design, and the decisions as beads |
| 2 | docair-pr.2 | text pictures: `docair_viewer::text_pic` |
| 3 | docair-pr.3 | a change's cut: `docair_core::review::change_cut` |
| 4 | docair-pr.4 | the comments: `docair_viewer::pr`, the review's order shared with the page |
| 5 | docair-pr.5 | `docair pr build` and `docair pr plan` |
| 6 | docair-pr.6 | the workflow (on branch `pr-review`, not on master) |
| 7 | docair-pr.7 | tests, a run on docair's own history, SPEC follows |

## 7. Built (docair-pr.2 to pr.7)

| Piece | Where |
|-------|-------|
| a picture as text | `docair_viewer::text_pic::draw` |
| a change's cut | `docair_core::review::change_cut` |
| the review's changes in the page's order | `page_changes::review_list`, lifted out of `review_view` |
| the comments and the plan | `docair_viewer::pr::{review, plan, existing_from}` |
| the review, asked and kept | `docair-cli` `diff::review`, lifted out of `cmd_explain` |
| `docair pr build`, `docair pr plan` | `docair-cli` `pr.rs` |
| the workflow | `.github/workflows/docair-pr.yml`, on branch `pr-review` only |

Tests: the text picture's compaction, its characters and every slice of the
shop drawn as text (`tests/text_pic.rs`: each name whole, arrows, WGL4 only);
the cut holds only its pieces' parts; the comments with and without the AI
(the cut as text with the new line double, the parts linked at B, what was
said against the code); a mark that cannot end early; the plan over a push
(kept, updated by its pieces, written new, gone, and a gone one coming
back), half the pieces or more; GitHub's listings; `docair pr build` on a
repository whose base branch moved on since the branch (what landed there
is no change of the pull request).

**On docair's own history** (`119f9b8` as the base, `4049697` as the head,
28 files, the adapters found by name): 31 pieces, 107,812 characters to the
model, three changes that match the commit, in 6 min 10 s with both commits
scanned by rust-analyzer and two more requests for pieces left out. The
comments are 0.8k, 25k, 6k and 41k characters; the first change's picture
has 43 parts and is folded. The workflow's last step, run against a stand-in
`gh`: the first run writes four comments; a second, with one change gone,
keeps three and marks one gone, and leaves a reviewer's comment alone.

**Not checked:** the workflow has not run on GitHub. Open there: whether the
artifact link opens the page with its script running, and the runner's time
for a release build with rust-analyzer.

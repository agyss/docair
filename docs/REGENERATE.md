# The page follows each save

Design for epic docair-live: with `docair serve` running, the page shows a
saved change within seconds. Only what the change touches is worked out,
laid out, drawn and packed again; the rest is kept from the last time.

This replaces the earlier design of the same name (docair-cof, 2026-11-18:
layouts kept in the browser), which the move of layout and drawing to Rust
superseded. What carries over: a layout is kept by a key made of what the
layout reads and the layout code's version (`docair_layout::key`), and an
open page takes the new state without starting from nothing.

Decided with the user (2026-11-21):

* The cold start does not matter; a save does, within seconds.
* Pictures in `serve` may depend on the edits made before: a picture is
  updated from its last layout, not laid out from scratch.
* `docair build --from-serve` takes serve's last layouts as its base, and
  still checks that they are up to date with the code.
* No second rust-analyzer kept running beside the editor's (about 1 GB).

The work is split between two sessions: this epic builds the page side; the
scan and the code index (§4) are steps of their own, for the session that
makes the scan faster.

## 1. Where the time goes

Measured on docair itself (2,159 parts, 6,481 lines, about 2,000 pictures),
release build, `DOCAIR_TIMING=1`, which says each step's time on stderr
(`docair_core::timing`):

| Step | Before | Now |
|------|--------|-----|
| the code index, after a change to a `.rs` file | 13.6 s | 13.6 s (§4, the scan session's) |
| the scan, with the index kept | 1.5 s | 1.5 s |
| working out every picture: 6 layers, ~260 sections twice, ~1,680 slices | 12.9 s | 4.9 s (§3) |
| … after a save, only what it touched (§2.3): a doc block's words | 12.9 s | 1.4 s, 1,531 of 2,051 pictures taken over |
| layouts, all kept (4 threads) | 0.5 s | 2 ms, from memory |
| a picture whose parts changed: Code (889 parts, 2,166 lines), one new function | 270 s from scratch, twice | 2.1 s updated (§5) |
| drawing and packing every picture (4 threads) | 1.6 s | only the pictures worked out again |
| panels, index, writing the 13 MB page | 0.85 s | 0.8 s |
| the page in the browser | a reload every 5 s at most | at once (§6) |
| **a save, from the file written to the page on disk** | about 31 s, minutes when a big picture changed | **4.5 s** (a doc block's words), **5.8 s** (a new part and line) |

A save that changes one function in docair's viewer touches about 25
pictures: the Code layer (twice), the sections of the boxes around it, the
slices through it. Laid out from scratch, those alone take minutes.

## 2. A save in `docair serve`

1. **The watcher** collects the changed paths. It rebuilds 0.4 s after the
   last change, and at most 3 s after the first while changes keep coming.
   It also rebuilds when a code index made in the background lands (§4).
2. **The graph** is made whole, every time: ids, references, guessed calls
   and the words that name parts are matched across the whole project, so a
   change in one file can change what another means. With the changed paths
   given, the scan does less (§4).
3. **The pictures** are worked out once per graph (§3), and only where the
   graph changed. A picture is worked out again when one of its parts is
   touched: a node that changed, an end of a line that changed, an end of a
   line that carries a changed datum, the box a part came into or left (a
   slice counts a box's own parts it leaves out). And when one of its
   *closed* boxes holds a part that came, went or moved, or an end of a
   changed line: a closed box stands for what it holds, an open one shows
   its parts itself. The six layers and the walks are worked out each time;
   a change to the system, the view, the flows or the walkthroughs works
   everything out again. Any other picture is taken over, its lines' graph
   indices mapped to the new graph by the lines' ids.
4. **Layouts:** each picture is kept by its address between saves: its
   layout input, its key, its layout and its packed drawing. The same key:
   kept. Another key: its last layout updated (§5). No layout before: from
   the layouts kept under their keys, or from scratch.
5. **Drawing and packing** only for the pictures whose layout or drawing
   changed; the others' packed drawings are taken as they were.
6. **The page follows** (§6).

## 3. Working out the pictures once

Each slice and each section looked the whole graph up again, and each
section worked out all six layers to find the one it belongs to. Now
`layer_view::Pictures` and `slice::Slicer` look the graph up once per page,
and the layers are worked out once for all sections. A type's own section
draws its uses of other types, which can change what a layer draws but not
what it is about: for a type, only a layer about it is taken from the
shared layers, else its layers are worked out as before. The pictures are
the same, byte for byte (the golden pictures of `tests/svg.rs` and the page
tests).

## 4. The scan and the code index (the other session's steps)

* **A scan from the changed paths:** one function in docair-core, given the
  changed paths, returns the whole graph, as `scan` does. Until it is there,
  serve calls `scan`.
* **The code index is not waited for:** after a save, the last kept index
  is taken with the edited files' lines mapped to their new places and their
  calls from the syntax; the full index is made in the background, and when
  it lands, serve rebuilds once more without a file event.
* **Comments only:** an edit that changes only comments or doc blocks keeps
  the index, its positions moved to the new lines.

## 5. A layout updated, not made again

`docair_layout::update(old input, old layout, new input, options)`:

* **Parts** are matched by id and box. A matched part keeps its place; a
  part whose input grew makes room by widening everything to its right, and
  grows by as much as its input did (a placed part can be wider than its
  input, where many lines meet it).
* **Gone parts** are left out; each box that lost one fits what it still
  holds, as a box does after a part is moved out of it (docair-702.41).
* **New parts:** a new part, or a new box with what it holds laid out on
  its own, goes below the other parts of its box, at its left. Room is made
  by a band: everything below the band's line moves down by the new part's
  height, so boxes across the line grow and nothing overlaps; a part too
  wide for its box makes room the same way to the right.
* **Lines** that stay keep their routes, moved with the bands. New lines,
  lines at a new, grown or moved part, and lines that now cross a new part
  are routed around everything else, as after a move by hand
  (docair-702.40), in one round, and their words placed. On docair's Code
  layer (889 parts, 2,166 lines), one new function: 24 lines routed again,
  2,142 keep their ways, 2.1 s (five rounds took 13.8 s, two 4.2 s; laid
  out from scratch, 270 s).
* **Not updated, laid out from scratch:** when more than a quarter of the
  parts are new or gone (and more than 8), when a part gets its first part
  or loses its last, or when the old layout does not fit the old input.

Routing a few lines around a big picture's every staying line gave the
router more tracks than its grid may hold, and widening its lattice never
got below that: the router now also merges the soft tracks further apart
once its lattice is wider than the picture, and stops past that. Pictures
that fit before take the same way, so their layouts are the same.

Over many saves a picture drifts from what a new layout would give. It is
not laid out again by itself: that would make it jump. *Lay out again*
(§7) does it on request.

## 6. The page follows

* **Waiting, not polling:** `GET /api/version?after=<n>` answers as soon as
  the version is newer than `n`, or after 25 s with the same; the page asks
  again at once. Before: every 5 s.
* **The reload keeps the view:** before it reloads, the page notes in
  `history.state` (it works in the web app's sandboxed frame, where storage
  throws) the picture, the selected part and the zoom; after it, it opens
  that picture, selects that part and zooms around it (or around the
  picture's middle), as the layout may have moved things. The address keeps
  the picture already; `?editor` stays.
* **Not patched in place:** the page is reloaded, not updated part by part:
  a page that started once has no state left over from the old graph, and
  `page.js` stays within its 1,050-line budget, which only the user raises.

## 7. Lay out again

`POST /api/relayout` with an address, or none for every picture: serve
forgets the kept layouts of that picture (or all) and writes the page again,
those pictures laid out from the layouts kept under their keys or from scratch. The
page offers it, served, as a button at the bottom right of the picture.

## 8. `docair build --from-serve`

`serve` writes its kept layouts after each rebuild to `.docair/serve/`:
`state.json` (the layout code's version, and for each picture's address its
key and its layout's file) and the layouts, each in a file named by its
content. `state.json` is replaced in one step, so a build never reads half
of it; layout files nothing names any more are removed.

`docair build --from-serve` builds as always (the graph from the files),
and for each picture:

| Serve's state for its address | The build |
|-------------------------------|-----------|
| the same key | takes serve's layout |
| another key | updates serve's layout to the code (§5) |
| none, or another layout code version | lays it out from the layouts kept under their keys, or from scratch |

It says how many pictures it took, updated and laid out, and warns when
there was no state or it could not be used. Without the flag, `docair
build` and CI lay out from scratch, as before.

## 9. Steps

| Step | Issue | What |
|------|-------|------|
| 1 | docair-live.1 | this design, the decisions as beads |
| 2 | docair-live.2 | where the time goes: `DOCAIR_TIMING` (§1) |
| 3 | docair-live.3 | the pictures worked out once per graph (§3) |
| 4 | docair-live.4 | `docair_layout::update` (§5) |
| 5 | docair-live.5 | serve keeps the page between saves (§2.3 to §2.5) |
| 6 | docair-live.6 | the watcher: changed paths, quiet and at most, the index landing (§2.1) |
| 7 | docair-live.7 | the page follows: waiting for the version, the view kept (§6) |
| 8 | docair-live.8 | lay out again (§7) |
| 9 | docair-live.9 | serve's state on disk, `docair build --from-serve` (§8) |
| 10 | docair-live.10 | tests, the numbers after, SPEC, ARCHITECTURE |
| A | docair-live.11 | a scan from the changed paths (§4), the other session |
| B | docair-live.12 | the code index not waited for (§4), the other session |
| C | docair-live.13 | comments only keep the index (§4), the other session |

## 10. Built (docair-live.2 to .10)

| Piece | Where |
|-------|-------|
| timing points, `DOCAIR_TIMING=1` | `docair_core::timing`; points in `scan`, docair-lang-rust, `doc_page` |
| the pictures worked out once per graph | `layer_view::PictureSet`, `slice::Slicer` |
| a layout updated | `docair_layout::update_layout`, `try_update_layout` (`incremental.rs`); the router's grid loop |
| the page kept between saves | `docair_viewer::page_state` (`PageState`, `ChangeReach`, `save`, `load`); `doc_page::render_page_kept`, `write_html_kept` |
| serve: the watcher, the version, lay out again, the state on disk | `serve.rs` `watch`; `project.rs` `refresh_changed`, `wake`, `lay_out_again`, `version`, `relayout`, `written` |
| the page | `page.js` `wait`, `again`, `viewBack`, the `#relayout` button (`page.html`, `page.css`); 1,050 lines |
| `docair build --from-serve` | `docair-cli` `cmd_scan` |

Tests: the update (the same input, a new part, a gone part, a grown part,
too many changes, the same every time, no overlaps); docair's Code layer
updated after one new function, every line drawn (`tests/update_big.rs`, by
hand); the kept page's pictures are a new page's after a new function, a
rename, a removal, a move and a renamed datum; serve's layouts saved, read
back and a checked base; serve written again on a change or a wake, the
version waited on, lay out again; e2e `live.spec.ts`: a save on the page
within seconds where the reader was, `docair build --from-serve`, lay out
again.

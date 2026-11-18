# Regenerate only what changed

Design for docair-cof: when a page is made again or opened again, only
what changed is done again.

**Superseded (2026-11-18), not built.** Layout and drawing move to Rust
(decision docair-7lz, epic docair-702, docs/SLICES.md): the browser lays out
nothing, and the web app only uploads and shows the HTML. So §3 and §4 are
not built. What carries over to the Rust layout: where the time goes (§1),
and what a layout key needs (§2): only what the layout reads, and the
layout code's version. Kept by such a key, a scan after an edit lays out
only the pictures that changed.

## 1. Where the time goes

Measured on docair itself, through the web app on localhost, 1440×960:

| Step | Time | Done again |
|------|------|------------|
| Upload, 280 files | 90 ms, about 5 MB | every generation |
| The server reads the files and makes the page | 0.42 s | every generation |
| The answer comes back: the page (4.5 MB, the graph inside) and the graph again (1.8 MB) | 50 ms, 6.6 MB | every generation |
| The page starts and draws its first layer | 0.87 s | every page |
| … of which laying out the first layer | 0.22 s | every page |
| Code (417 parts, 740 lines), laid out | 11.8 s | every page |
| Code, drawn from a kept layout | 0.19 s | kept within one page only, for its last six pictures |

Making the graph is quick. What takes long is laying out the pictures, and
a page does that again each time it opens: after *Apply & regenerate*,
after a new upload of the same project, after a reload.

## 2. The layout key

The page keys each layout by a hash of everything it is made from
(`layoutKey`): the parts, the lines, the hints and the canvas. A layout
depends on nothing else: no clock and no chance (ELK runs with its fixed
seed). That is why a kept layout can stand in for a new one.

Today's key holds more than the layout reads:

| In today's key | The layout reads it | Changes when |
|----------------|---------------------|--------------|
| a part's id, its box, its size (`w`, `h`, `minW`) | yes | the picture changes |
| a box's name | yes: its width is measured | a box is renamed |
| a part's name | no, only its size | a test passes or fails: `✓` becomes `✗` |
| a part's classes (closed box, test result) | no | a test result |
| a line's id, ends and words | yes | the picture changes |
| a line's `count`, `members`, `via`, `dangling`, `kind` | no | `members`: a line is added anywhere before it in the graph |
| the hints: lines against their arrow, how far each points ahead | yes | the flows change |
| the canvas, the direction asked for, the font | yes | the window, the font |

`members` are positions in the graph's list of lines, so one new line
renumbers every line after it. Measured with four pages the server made
from docair, each with one change:

| Change | Layers laid out again: today's key | the layout's fields only |
|--------|------------------------------------|--------------------------|
| a doc block's description gets a line | none | none |
| one new `@calls` | all 6 | apps, code |
| one e2e test fails | code | none |

The new key is made of what the layout reads, and of the layout code's
version: a hash of the viewer's and ELK's scripts (1.9 MB, 3 ms), since
kept layouts would outlive the page that made them. The worker gets
exactly that input, made by the same function, so a layout cannot depend
on something the key leaves out.

## 3. Kept layouts across pages

A layout is kept by its key beyond the page:

* **A page on its own** (opened from disk or served): in IndexedDB. Tested
  in Chromium on a `file://` page: Code's layout (290 kB) is written in
  5.8 ms and read back, the same, in 4.9 ms. Pages from disk share one
  store; their keys do not collide, since they are made from the pictures.
  Firefox and Safari are not tested yet. Where a browser gives no storage,
  the page works as today.
* **In the web app**, the page runs in a sandboxed frame, where storage
  throws (tested: `SecurityError`). The app keeps the layouts in its own
  IndexedDB. The page asks for a layout by its key and hands over each new
  one; the app answers, with messages like the ones they exchange now. The
  app writes a mark into the page it shows, so the page asks from its first
  layer on.
* **How many:** no limit of its own. A browser bounds what a site may
  store and clears it when the disk runs short; a write it refuses is
  skipped, and that picture is laid out as today. docair's six layers take
  320 kB, and each changed picture adds its layout (Code's: 290 kB).
* **Trust:** a layout is data: numbers, part and line ids, the name of a
  way. The page checks its shape before it draws it, and the app its size
  before it keeps it; the app takes messages from the frame only, as now.
  A bad layout can draw a wrong picture, never run code.

| Case | Today | With kept layouts |
|------|-------|-------------------|
| The page opened again, then Code | 11.8 s | 0.2 s |
| The page's first layer | 0.22 s laid out | 15 ms, drawn from its kept layout |
| *Apply & regenerate* after a description edit, then Code | 11.8 s | 0.2 s |
| *Apply & regenerate* after one new call | all six layers laid out | apps and code laid out |

## 4. The open page takes the new graph (web app)

Today *Apply & regenerate* loads a new page into the frame. It starts
again (0.87 s on docair's page) and opens on its first layer. The app gives
it back the hidden sections and the tests switch; the layer, the zoom, the
selection and the opened boxes are lost.

Instead, the app sends the new graph into the open page (`docair:graph`),
and the page takes it in place:

* **Kept:** the layer, zoom and pan, the selected part (cleared when it is
  gone), opened boxes, hidden sections, the tests switch, the walkthroughs
  replies added (where their parts still exist).
* **Drawn:** from the kept layouts where a picture's key is the same; laid
  out again where it is not.
* **What it needs:** the graph, which the answer already carries, and the
  page's texts (`docair-html`, 0.35 MB), which the app takes from the new
  page. The server's API does not change.
* **When not:** the open page's viewer must be the one that made the new
  page. docair writes the viewer's version into each page (the hash of §2),
  and the page says its own in `docair:ready`. When they differ (docair was
  updated in between), the app loads the new page, as today.

In the page, `boot()` splits in two: what is done once (Cytoscape, the
wiring, the workers) and what is done for each graph (the maps,
`classify()`, the view, the glossary and the walks, the tree and the bars,
the picture). The risk is state left over from the old graph; the test is
that a page updated in place equals a new page of the same graph.

## 5. Not in this design

* **A changed picture is laid out in full.** One new call still costs Code
  11.8 s. Keeping the boxes that did not change and routing only the lines
  near the change (as a drag does: about 0.4 s) is a later step; over many
  edits, such a picture drifts from what a new layout would give.
* **The server** makes a page in 0.42 s, keeps nothing between requests
  and runs two copies in the cluster. Keeping uploads or graphs there would
  gain little.
* **Uploading only changed files** saves about 5 MB per regeneration,
  which matters only on a slow link.
* **An answer without the page:** with §4, the app needs 2.1 MB of the
  6.6 MB answer.
* **The CLI's code index:** `docair scan .` takes 6.0 s, 5.5 s of it
  `rust-analyzer scip`. Keeping the index by a hash of the Rust files is
  the cache per file hash that ADAPTERS.md §11 plans for later.

## 6. Decided (2026-11-18)

1. **The canvas counts exactly in the key.** A regenerated page keeps the
   window, so its layouts are found again.
2. **No size limit of docair's own** (§3): the browser's bounds are enough.
3. **The open page takes the new graph (§4)**, as step 3, after kept
   layouts.

## 7. Steps (docair-cof.1, .2, .3)

1. **The key:** made of what the layout reads and the layout code's
   version; the worker gets the same input. A fixture with the three
   changes of §2, and a test of their keys.
2. **Kept layouts:** IndexedDB for a page on its own, the app's store in
   the web app. e2e: a reloaded page draws Code without
   laying it out; *Apply & regenerate* after a description edit lays out
   nothing. SPEC §3.5.
3. **In place:** `boot()` in two, `docair:graph`, the version check. e2e:
   the frame is not loaded again; the layer, zoom and selection stay; the
   result equals a new page. SPEC §10.6.

Each step is measured before and after, keeps `docair check` at 0 errors
and 0 warnings, and keeps e2e green.

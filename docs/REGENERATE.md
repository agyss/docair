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
| the code index, after a change to a `.rs` file | 13.6 s | not waited for: the page with the edited functions read by their syntax, exact 3.8–4.6 s (a body) to 7.3 s (a signature) after the save (§4.11) |
| the scan, with the index kept | 1.5 s | 1.5 s |
| working out every picture: 6 layers, ~260 sections twice, ~1,680 slices | 12.9 s | 4.9 s (§3) |
| … after a save, only what it touched (§2.3): a doc block's words | 12.9 s | 1.4 s, 1,531 of 2,051 pictures taken over |
| layouts, all kept (4 threads) | 0.5 s | 2 ms, from memory |
| a picture whose parts changed: Code (889 parts, 2,166 lines), one new function | 270 s from scratch, twice | 63 ms updated (§5); from scratch 29 s |
| drawing and packing every picture (4 threads) | 1.6 s | only the pictures worked out again |
| panels, index, writing the 13 MB page | 0.85 s | 0.8 s |
| the page in the browser | a reload every 5 s at most | at once (§6) |
| **a save, from the file written to the page on disk** | about 31 s, minutes when a big picture changed | **4.5 s** (a doc block's words), **5.8 s** (a new part and line) |

Comparing two commits (`docair diff`, `docair build --changes A --to B`, the
editor's compare) scans both at once, each with its code index (docair-live.38):
`docair diff HEAD~1 HEAD` with an empty cache, 42.3 s before, 25.4 s now (about
4 GB at the peak for those seconds); the page with its changes section, warm,
12.5 s before, 10.6 s now. Cold, the page took about 5 minutes, most of it
laying out the Code layer from scratch on one thread; with the router's coarse
cells put right (§5, docair-live.42) it takes 65 s, and docair's own page with
every layout cold 50 s.

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

## 4. The scan and the code index

After a save, every call, type and line is right within seconds, and no
rust-analyzer is kept in memory between saves. The numbers come from
measurements on a copy of docair (docair-utz); the steps are A to F (§9).

Decided with the user (2026-11-21):

* Nothing of rust-analyzer stays in memory between saves (1 GB is too much).
* Code a derive generates is drawn on the type that carries the derive (§4.6).

### 4.1 Where the time goes

| Step | Time | Peak memory |
|------|------|-------------|
| `rust-analyzer scip` after any `.rs` change, comments included | 13.7 s | 1.95 GB |
| the scan, with the index kept | 1.5 s | |
| rust-analyzer's fixed cost before any inference: loading, then name resolution of std and the dependencies | 8.5 s | |
| tree-sitter alone: certain Rust calls found, of those the index finds | 64 % | |

The fixed cost is why a one-shot run that indexes only the changed files is
still slow when it loads everything. Most of it is code our calls never
pass through: `core`'s SIMD intrinsics alone are 499,000 lines.

### 4.2 A save, step by step

| The save changes | What happens | Right after |
|------------------|--------------|-------------|
| only comments or doc blocks | the kept index is kept, its positions moved to the new lines (§4.3) | at once |
| a function's body | every other function keeps its calls from the map (§4.3); the edited functions get their calls from the syntax at once, and exactly from the short run (§4.4) | 1.9–4.1 s |
| a signature or an item | as above, and the short run also takes the functions whose text names a changed item (find usages) | a few seconds more |
| `Cargo.toml`, `Cargo.lock`, a removed or renamed file, or an item more than 100 lines name | the full index at once (§4.8) | 13.7 s |

When the short run lands, it calls `ServedProject::wake()`, and serve
rebuilds once more without a file event (§2.1).

### 4.3 The map and comments

**The map** keeps each function's resolved calls, keyed by the function
(file, owner, name) and a hash of its body's tokens without comments. A
function whose key is the same after a save keeps its calls: rust-analyzer
never lets a change inside one body change what another body means. The
one exception, a function returning `impl Trait`, is taken by the short run
like an edited function.

Tested against rust-analyzer resolving everything again: 20 body edits in
10 crates (a call line removed, a call line doubled, a comment added). Each
time, 1,635 or 1,636 of 1,636 functions came from the map, none of them
wrong. 8 signature edits: every function whose calls changed names the
edited item.

**Comments only** (step C): the kept index is kept under the code without
comments and whitespace. When only comments changed, the code's tokens are
the same one by one, so each position in the old file maps to one in the
new file.

### 4.4 The short run

A small program of docair's, `docair-index`, links rust-analyzer's crates
(`ra_ap_*`, pinned exactly) and is started per save; it exits when done, so
nothing is held between saves. For the edited functions it:

1. loads the workspace with only what is needed (§4.5), and leaves the
   function bodies of the workspace's other files out: what those files
   mean rests on their items and signatures, which stay (a body with an
   `impl` in it stays too, since an impl counts everywhere),
2. primes rust-analyzer's caches of those files' crates and what they
   depend on, on up to 8 threads (docair-live.64): name resolution done in
   parallel first, not file by file as the tokens ask for it,
3. resolves every token of the edited files and of the files find usages
   names, as rust-analyzer's static index does, but only in those files:
   each definition named with rust-analyzer's own moniker code,
4. expands their derives (§4.7),
5. checks the guard (§4.6), and
6. writes the results for the map, then exits.

Measured from cold, one edited function and then its whole file:

| Edited function | Everything loaded | Only what is needed |
|-----------------|-------------------|---------------------|
| `text_pic.rs` · `Grid::render` | 3.9 s, 1,017 MB | 3.1 s, 706 MB |
| `project.rs` (server) · `page` | 5.7 s, 1,325 MB | 4.0 s, 877 MB |
| `main.rs` (CLI) · `main` | — | 4.1 s, 887 MB |
| `model.rs` (core) · `Graph::authored` | — | 1.9 s, 465 MB |

The rest of the file costs 0.1–0.5 s more: name resolution is the floor.
Calls, paths, fields, inferred types and impls were each the same as with
everything loaded.

### 4.5 What the short run loads

rust-analyzer reads every crate's source, so a crate is left out by giving
its files empty text: no fork, only the input changes.

Kept:

* our crates, `core`, `alloc`, `std`, and the crates they depend on directly;
* then, until nothing new comes in, every crate a kept crate passes on with
  `pub use` or `pub extern crate`, by name or with `*`, and every crate its
  root file imports with `use` (`tree_sitter_rust`'s `LANGUAGE` has a type of
  `tree_sitter_language`, imported there and named in its API);
* and every crate that brings a kept crate macros: proc-macro crates (from
  `cargo metadata`), crates with `#[macro_export]`, and crates that pass on
  a proc-macro crate (`thiserror` passes on `thiserror_impl`).

Left out: every other crate (its root file empty, its other files blanked),
the crates outside the edited files' closure, and `core`'s SIMD intrinsics
(`core_arch` becomes `pub mod arch {}`). The rule is worked out from the
crate graph and `cargo metadata` in seconds, and kept in the cache under
the hash of `Cargo.lock` and the toolchain.

On docair, 143 of 263 crates are left out. Everything stayed the same:
17,735 calls, paths and fields reaching our items, 49,168 inferred types
(none unknown) and 1,218 impls, and docair's graph from such an index
(2,159 parts, 6,366 lines). Why each part of the rule is there:

| Without | What broke |
|---------|-----------|
| re-exports | `ureq` hands out `http::Response`; 311 types differ |
| crates bringing `macro_rules!` | `pulldown_cmark` makes its `Options` with `bitflags!`, `tower` its `Oneshot` with `pin_project!` |
| crates passing on a derive | `protobuf`'s error gets `Display` from `thiserror`'s derive |

A test project with dependencies one to four steps away covers what docair
lacks: a renamed re-export, a glob re-export whose `Result` shadows std's, a
`macro_rules!` macro three steps away that calls our function, a derive that
does the same, an `impl Trait` return, and *one of* through a trait two of
our types implement. All come out the same. What changes is reached only
through types: a field whose type is three steps away and passed on by no
crate, and `impl Iterator<Item = …>` of such a type. The guard sees both.

### 4.6 The guard

A crate left out can change our code only through its own types, traits,
functions and macros: an impl must involve a type or trait of its own crate,
and a trait's methods need the trait in scope, which needs a path the rule
keeps. When something is missing, the edited function shows it: an inferred
type with `{unknown}`, or a path, method call, field or macro that does not
resolve. The guard compares these signals with what the last full index
recorded for the function. When there are more, the save is redone with
everything loaded (4–6 s, about 1.3 GB).

| Left out | Functions that changed | The guard saw | Seen for nothing |
|----------|------------------------|---------------|------------------|
| one step, no re-exports (192 crates) | 37 | 37 | 35 |
| the first searched list (222 crates) | 147 | 147 | 33 |
| the rule (143 crates) | 0 | 0 | 4 |
| test project, the rule | 2 | 2 | 0 |
| test project, one step or two steps | 9 | 9 | 2 |

No function changed without the guard seeing it. 21 of docair's 1,717
functions show signals with everything loaded (`docair-layout-wasm`, built
for another target, and the bodies of `#[tokio::test]` functions), which is
why the guard compares, not counts. What it cannot see: a type that changes
without becoming unknown when nothing else in the function fails to resolve
(the test project's `impl Runner` became `impl Sized`; there the method call
failed too). The full index catches that afterwards (§4.7).

### 4.7 Derive-generated code

Today docair draws no line in or to code a derive generates. The index has
no definition and no occurrence for it, so `S.hello()` on a derived
`impl Hello for S` and the call the derive writes into it are both lost.
docair's own code has 8 such calls, serde attributes like
`#[serde(default = "yes")]`.

The short run expands each derive (rust-analyzer builds the generated code)
and resolves it. What it finds is drawn on the type that carries the derive:

| Code | Line |
|------|------|
| `#[derive(Hello)] struct S`, the generated `hello` calls `greet()` | `S → greet` |
| `#[derive(Deserialize)] struct Graph` with `#[serde(default = "yes")]` | `Graph → yes` |
| `fn c5() { S.hello() }`, `hello` generated | `c5 → S` |

rust-analyzer's built-in derives (`Clone`, `Debug`, `PartialEq`: 51 of
docair's 255 derive attributes) have no generated code; their calls are not
drawn.

### 4.8 The full index

`docair build`, `docair check`, the comparisons, and serve when idle make the
full index. `docair-index` writes it too, with everything loaded, as
`rust-analyzer scip` does: the same 139,053 occurrences on docair, the same
graph, and the derive lines of §4.7 on top, so `build` and `serve` draw the
same. When serve has been idle for a while it makes the full index and
compares it with the map: a difference replaces the map and keeps one more
crate in the rule.

### 4.9 Not covered

* Calls in the built-in derives (§4.7).
* A type that changes without becoming unknown, as in §4.6, until the full
  index.
* Uploads run no indexer: the server ignores `run_indexers`, so uploads keep
  the calls the syntax makes certain.
* `docair-index` is tied to the `ra_ap_*` version it links: it uses the
  toolchain's proc-macro server, tested with 1.100. A toolchain it cannot
  work with makes docair fall back to `rust-analyzer scip`.

### 4.10 Built (steps A to F)

| Piece | Where |
|-------|-------|
| each Rust file's read kept by its path and text, in the process | `docair-lang-rust` `read_files` |
| the key from the code's tokens without comments; positions moved to new texts; the map (unchanged functions kept, gaps for the syntax); the names a change touched | `docair-lang-rust` `kept` (`code_text`, `map_file`, `changed_names`, `index_for`) |
| per line, what the index holds and what the syntax reads | `docair-lang-rust` `calls_of`, `reached`, `uses_of` with `Index::covers_line` |
| the index file by file: moved, dropped, a short run's word, gaps | `docair-scip` `Index` (`remap_file`, `drop_file`, `replace_file`, `set_gaps`) |
| the indexer found (`docair-index`, else rust-analyzer); kept indexes with their texts and signals; the project's last index | `docair-scip` `RustIndexer`, `make_index`, `kept_index`, `last_key` |
| short runs and the idle full index in the background; the word compared with the full index | `docair-scip` `background`; `docair-lang-rust` `kept::compare_overlays` |
| serve's own project indexed in the background; a landed index wakes the watcher | `Options::background_index`; `docair-server` `ProjectSession::background`, `ServedProject::set_wake` |
| `docair-index full` and `files`: the load, what is left out, other files' bodies left out, the index file by file, the guard, the derive lines | `crates/docair-index` (`load`, `rule`, `moniker`, `write`, `guard`) |
| a short run's names resolved first on up to 8 threads | `docair-index` `prime::caches` |
| a background full index made only of the texts it was asked for | `docair-scip` `background::still` |

Decisions taken while building: docair-live.54 to .62, and .65 (docair-live.64, names resolved first).

### 4.11 The numbers after

Measured on a scratch copy of docair (115 Rust files, `docair-index` and
its rust-analyzer crates among them), release build, `docair serve` from the
file written; with names resolved first on 8 threads (docair-live.64), and
before that:

| Save | The page | The short run | The page again, exact | before | Its graph against a full index of the same files |
|------|----------|---------------|-----------------------|--------|-------------------------------------------------|
| a statement in a body (`text_pic.rs`) | 3.8–4.6 s | 1 file, 2.7–4.0 s, 158 crates left out, the guard kept | with the first page, 3.8–4.6 s | 7.6 s | the same (2,364 parts, 6,930 lines) |
| a comment line (`model.rs`) | 3.0 s | none: the index is kept | — | 3.2 s | the same |
| a signature (`docair-scip`'s `text_hash`) | 3.3 s | 6 files (the edited one and the five that name it), 4.3 s, the guard kept | 7.3 s | 10.1 s | the same |

The first short run after serve starts is the slower one (4.0 s: files not
yet in the system's cache). The full index once serve is idle found nothing
the short runs said differently.

`docair-index` on its own, peak memory of the process:

| Run | Time | Peak | before |
|-----|------|------|--------|
| `files`, `model.rs` | 2.9 s | 825 MB | 3.4 s, 710 MB |
| `files`, `kept.rs` | 2.6 s | 948 MB | 3.8 s, 819 MB |
| `files`, `text_pic.rs` | 2.6 s | 979 MB | 4.1 s, 872 MB |
| `files`, `project.rs` (server) | 3.3 s | 1,158 MB | 5.7 s, 1,075 MB |
| `files`, `main.rs` (CLI) | 3.6 s | 1,166 MB | 6.1 s, 1,091 MB |
| `full` | 17.2 s | 2.40 GB | |
| `rust-analyzer scip`, for comparison | 15.0 s | 2.46 GB | |

Resolving names on 8 threads first: 40–45 % less time for 7–16 % more
memory, the caches it fills (import maps and symbols with the def maps)
making the files' tokens quick to resolve. Def maps alone first gained less
(3.1–4.6 s), more threads nothing. The full index's time: load with every
crate's names resolved on every core 3.5 s, rust-analyzer's static index
9.1 s, derive lines 1.8 s, signals 0.5 s; resolving in derive code only the
names our functions have saved 0.5 s but lost lines, so it is not done.

The short runs' documents are the full index's, occurrence by occurrence.
The full index has every occurrence `rust-analyzer scip` writes, and on the
docair copy without `docair-index` its graph is the same in all 6,366 lines,
with 10 derive lines added.

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
  layer (895 parts, 2,176 lines), one new function: 5 lines routed again,
  2,171 keep their ways, 63 ms; laid out from scratch, 29 s. Before the
  coarse cells were put right (below): 24 lines, 2.1 s, and 270 s from
  scratch (five rounds took 13.8 s, two 4.2 s).
* **Not updated, laid out from scratch:** when more than a quarter of the
  parts are new or gone (and more than 8), when a part gets its first part
  or loses its last, or when the old layout does not fit the old input.

Routing a few lines around a big picture's every staying line gave the
router more tracks than its grid may hold, and widening its lattice never
got below that: the router now also merges the soft tracks further apart
once its lattice is wider than the picture, and stops past that. Pictures
that fit before take the same way, so their layouts are the same.

**The coarse cells (docair-live.42).** A picture of more than 60,000 grid
nodes is routed first from cell to cell of 32 px, then exactly near that way
(SPEC §5). A track edge was counted only at the first cell border it
crosses. Where two tracks lie more than a cell apart (on the Code layer, 13
gaps in x and 43 in y, its lattice 60.8 px wide), the cells between had no
capacity and stood as a wall across the whole picture: no coarse way
crossed it. 895 of the Code layer's 2,173 lines found none and were searched
on the whole grid within their detour, 50.7 ms each against 0.3 ms: 98 % of
the routing time. An edge now counts at every border it crosses, for its
capacity and its use. Measured on the Code layer, one placement, five
rounds:

| | as before | now |
|---|---|---|
| time | 234 s | 28.5 s |
| lines without a coarse way (round 1) | 895 | 0 |
| price | 928 M | 91 M |
| crossings | 262,346 | 66,923 |
| length on top of another line | 755,224 px | 220,573 px |

The rounds help again: 170 M after the first, 91 M after the fifth. Before,
each round made the Code layer worse (276 M after the first, 928 M after the
fifth), lines forced around the walls. Pictures below 60,000 nodes are not
routed through cells and keep their layouts.

Routing the lines in parallel was tried on a copy and not built
(docair-live.43): lines in batches of 64 on 8 threads, each against the lines
placed before its batch, then the lines that met a line of their own batch
routed again one by one. The repair costs nearly a whole round: the dear
searches are the long lines, and those are the ones that meet. Repairing in
batches again puts lines on top of each other, round after round. With the
cells put right it takes 7.0 s for a price of 133 M, between the router's
first round (5.3 s, 170 M) and its fifth (28.5 s, 91 M).

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
| 11 | docair-live.44 | lay out again from the command line: `docair build --from-serve --lay-out-again [ADDRESS...]`; the page's button calls the same function (§7) |
| A | docair-live.11 | a scan from the changed paths, and the map of each function's calls (§4.3) |
| B | docair-live.12 | the short run: `docair-index` resolves the edited functions after a save (§4.4) |
| C | docair-live.13 | an edit to comments or doc blocks keeps the code index (§4.3) |
| D | docair-live.45 | the short run loads only what is needed, with the guard (§4.5, §4.6) |
| E | docair-live.47 | lines for code a derive generates, drawn on the type that carries it (§4.7) |
| F | docair-live.46 | `docair-index` writes the full index; serve compares it with the map when idle (§4.8) |

Steps A to F are the scan session's. Their order: C (on its own), then A, B,
D and F, and E last, since `build` needs F for the derive lines. The choices
taken alone are decisions docair-live.48 to .52.

## 10. Built (docair-live.2 to .10)

| Piece | Where |
|-------|-------|
| timing points, `DOCAIR_TIMING=1` | `docair_core::timing`; points in `scan`, docair-lang-rust, `doc_page` |
| the pictures worked out once per graph | `layer_view::PictureSet`, `slice::Slicer` |
| a layout updated | `docair_layout::update_layout`, `try_update_layout` (`incremental.rs`); the router's grid loop |
| the coarse cells | `router.rs` `cell_capacity`, `Grid::occupy` |
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

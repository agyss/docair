# Slices: one part of the system, worked out and drawn by docair

Design for the epic docair-702 and the decision docair-7lz. Not code yet:
this says what changes, why, and in which steps.

## 1. The problem

A reader rarely wants all the code of all the apps. They want one part of
it: how to get from this function to the ones it uses, where a datum goes,
what a request passes. That part often runs across many modules and apps.

Today docair offers either everything or a camera that moves over everything:

| On docair's own page, 1440×960 | Today |
|--------------------------------|-------|
| the Code layer | 433 parts, 907 lines, first layout 12 s |
| following `reply` (a datum) | 5.7 s before its first step, played on the whole Code layer |
| the page | 4.7 MB, of which 2.1 MB are Cytoscape and elkjs |
| the browser code | viewer.js, about 5,350 lines |

Speed is not the whole problem. The knowledge of which parts a datum passes,
or what a layer draws, lives in the browser (`dataTour`, `computeView` in
viewer.js). The CLI, the AI and the tests in Rust cannot use it. Comparing
two commits (docair-zrr) needs it without a browser.

## 2. Who does what

The knowledge is what the CLI builds, makes obvious and links. Everything
else is a thin wrapper around it (decision docair-7lz).

| Part | What it does | What it does not do |
|------|--------------|---------------------|
| **the CLI** | builds the graph, the slices and the views; lays out and draws every picture; writes the HTML; answers the AI's requests | — |
| **the HTML** | shows what the CLI wrote, fully usable on its own: every view and filter is baked in; a thin script lets the reader move around | decide what is drawn, lay out, draw |
| **the AI** | works through the CLI (`docair ask`, …) or, from the HTML, through `docair serve` | need the HTML |
| **the web app** | uploads a folder and shows the HTML | anything else |

docair does everything it can do reliably. The AI only links and explains.

## 3. A slice

A **slice** is one part of the system: it starts from a **seed**, grows by
fixed rules, and ends at parts with stable names.

| Seed | Grows along | Ends at |
|------|-------------|---------|
| a datum (`data.question`) | the rules of Follow today (SPEC §3.6): from where it is made, along the lines that carry it there and back, inside other lines for at most 6 hops | where it is kept or leaves: a store, a file, the browser, a service outside |
| a part (a function, a type, a module) | what it calls and what calls it, one hop each way, and the data on those lines | the parts one hop away |
| a walk (a scenario, a walkthrough) | its steps, as today's walk projection | its first and last step |
| a route (`POST /api/ask`) | from its callers through the gateways to its handler, then what the handler calls | the caller, and the stores and services the handler reaches |

**Ends with stable names.** A function can be renamed or moved; a route, a
store, a topic, an app, a datum, a service outside rarely are. A slice names
its ends by them, so it means the same thing tomorrow, and two commits can
be compared by it (docair-zrr).

**Only what docair knows.** A slice holds certain lines. A line that reaches
one of several parts, or could reach a part (step 7 of ADAPTERS.md), is
marked as such and never extends the slice on its own. Each line that
carries a datum says how docair knows:

| How | Today | Reliable |
|-----|-------|----------|
| `named` | the line's words name the datum (its name, the end of its id, a `@carried-as` word) | sometimes: a word can be a coincidence |
| `typed` | the code says it: what one call returns, the next one takes (docair-4wk.19) | yes |

**What it covers.** A slice counts the lines that carry its datum, and how
many of them it reaches from where the datum is made. `docair check` warns
when a datum's slice misses some (a new warning code, next to W109).

**The boxes.** A slice opens only the boxes its parts are in, down to the
parts. The other parts of an open box are left out and counted on it
(`docair-server · 13 more`), as the Code layer does today.

`docair slice <seed>` prints a slice as JSON:

```json
{
  "schema": "docair.slice/0",
  "seed": { "kind": "data", "id": "data.question" },
  "ends": ["docair-web", "ai-endpoint"],
  "parts": ["docair-web.ConversationService.ask", "…"],
  "lines": [
    { "edge": 812, "from": "…", "to": "…", "carries": "there", "how": "typed", "sure": "certain" }
  ],
  "boxes": { "docair-web": { "open": true, "more": 14 } },
  "covers": { "lines": 23, "reached": 19 }
}
```

The fields are a sketch; step 1 fixes them.

## 4. The documentation the CLI writes

Every picture a reader can reach is worked out, laid out and drawn by the
CLI, and written into the HTML. Nothing is laid out in the browser.

| Pictures | On docair today |
|----------|-----------------|
| the layers (Overview … Code) | 6 |
| a section for each box, at its layer | 158 boxes |
| a slice for each datum | 9 |
| a slice for each walk | 7 scenarios, 1 walkthrough |
| a slice for each part with a line | 858 parts |

**Moving around.** Opening a box is a link to the box's section; scrolling
into a box follows the same link. The breadcrumb, the tree, the search, a
part's details, a walk's step and a word's popup all link to pictures. Each
picture has its own `#` address, so the browser's Back works and a link can
be sent.

**Filters, baked in.** `[view] hide` in `docair.toml` is applied by the CLI.
Hiding a part in the browser is a reader's convenience: the script hides it,
and its place stays empty; the picture is not laid out again.

**Details, tree, glossary, diagnostics.** Rendered as HTML by the CLI, as the
docs are today (pulldown-cmark). The CLI links the words the glossary
explains; the script only opens their popups.

**Walks.** A walk is one picture. Each step is a set of parts and lines to
light up, and a box for the camera, both worked out by the CLI. The script
changes the classes and moves the camera; a token runs along the line's own
path in the SVG.

## 5. Layout in Rust

Today each box is laid out on its own, from the inside out: ELK places a
box's parts, knowing on which side each line leaves; then docair's own router
(`gridLines`) draws every line at right angles on tracks, five rounds
(SPEC §3.5). The same shape moves to Rust:

* **Text.** The words in the pictures are set in one embedded monospace font.
  A word's width is its number of characters times one advance (two for a
  wide East Asian character): exact, with no shaping and no font library.
  The page uses the same font, so the browser draws what Rust measured. The
  marks of the kinds (▣, ƒ, ◇) are SVG shapes, not characters.
* **Placement.** One box at a time, its parts in layers, the sides of its
  lines known. Candidates, checked under the two-weeks rule in step 3:
  `dagre` (a Rust port of dagre.js, with nested boxes), `rust-sugiyama`,
  `bra0-layout`, or our own layered placement for one box. Chosen by
  crossings, bends and time on docair's own sections and slices, and recorded
  in LIBRARIES.md.
* **Lines.** `gridLines` is ours; it is ported to Rust as it is, with its
  prices.
* **The same input, the same picture.** No clock, no chance, no time limit
  that changes a result. Golden tests and docair-zrr depend on it.
* **Kept layouts.** The CLI keeps each picture's layout under a key made of
  what the layout reads and of the layout code's version. docs/REGENERATE.md
  §2 measured why: a key that holds more (a part's name, a test result, a
  line's place in the list) lays out every layer again after one new call.
* **Positions from before.** Placement can start from the positions of an
  earlier layout, so that two commits' pictures look alike (docair-zrr).

Goal, measured in step 3: a slice of up to 50 parts in 100 ms or less; all
pictures of docair's documentation in under 10 s.

## 6. Drawing in Rust

Each picture is one SVG:

* boxes with their names, parts with the shape and colour of their kind (as
  `makeStyle` does today), lines with their arrows and words, data as chips;
* colours as CSS variables (`--k-*`, `--e-*`, as in viewer.css), so one SVG
  serves light and dark;
* every part and line carries its id (`data-id`), for the script;
* lines that reach one of several parts, or could, drawn as such
  (docair-4wk.12); the data kind gets its colours (docair-54p).

Golden SVGs of the fixtures are kept like the golden graphs: a change that
moves them says why, `DOCAIR_BLESS=1` rewrites them.

## 7. The thin script, and its limit

| The script may | The script may not |
|----------------|--------------------|
| pan and zoom a picture | decide what a picture holds |
| follow links between pictures | place a part or route a line |
| show the tip, details or popup the CLI wrote | write any words of its own |
| filter the search index the CLI wrote | measure text |
| step through a walk: classes, camera, tokens | lay out anything |
| unpack a picture before it is shown | |
| send questions to `docair serve` and show the answer | |

Its budget is 1,000 lines, checked by a test.

## 8. The AI in the HTML: `docair serve`

`docair serve` runs the CLI as a web server. It takes over docair-server's
routes (`/api/scan`, `/api/ask` and the others) and serves the documentation.

* The chat, apply and preview panels move from the web app into the HTML.
  The CLI writes them; the script sends and shows.
* An answer that brings a walkthrough, or an applied edit, makes the server
  write the documentation again. Kept layouts make that quick, and the page
  shows the pictures that changed.
* Writing again starts with a scan: 6.0 s on docair, 5.5 s of it the Rust
  code index (rust-analyzer's SCIP). Keeping the index per hash of the files
  it reads (ADAPTERS.md §11) makes it quick; that is an issue of its own.
* The HTML finds the server at its own address when the server serves it.
  Opened as a file, it asks for the server's address. Without a server it
  works fully, minus the AI, and says so where the AI would be.
* The web app keeps the upload, the progress and the frame that shows the
  HTML. The messages between page and app shrink to `ready`.

## 9. Each feature, and what happens to it

| Today (viewer.js unless said) | ~Lines | After |
|-------------------------------|--------|-------|
| layers and the view model (`classify`, `RULES`, `computeView`) | 420 | Rust, step 2 |
| opening boxes, zooming in and out (`drill`) | 150 | links between sections |
| placement (ELK, `topdownPlace`) | 520 | Rust, step 3 |
| layout workers, kept layouts | 210 | gone; the CLI keeps layouts |
| the router (`gridLines`) | 730 | Rust, step 3 |
| moving a part by hand, lines routed again | 180 | gone |
| render pipeline, line words, bundling | 320 | Rust, steps 4 and 5 |
| selection, pick, camera | 150 | script; the camera boxes from the CLI |
| details and tips | 260 | the CLI writes them; script shows |
| tree and search | 60 | the CLI writes them; script filters |
| header, legend, layout direction | 110 | the CLI writes them; the direction is the CLI's choice |
| walks and their replay | 1,020 | the CLI works out the steps; script plays them |
| messages to the web app | 70 | `ready` only |
| sections and the tests switch | 230 | baked in (§4, §12) |
| glossary, popups, the doc reader | 370 | the CLI links and writes; script opens |
| the data list and Follow (`dataTour`) | 320 | slices, step 1 |
| text measured with a canvas | — | Rust, one font |
| Cytoscape and elkjs | 2.1 MB | gone |
| chat, apply, previews (web app) | — | into the HTML, step 6 |

## 10. Tests

* **Rust:** golden slices (JSON) and golden SVGs on the fixtures; the numbers
  of today's layer script (crossings, detours, time) as a Rust test on
  docair's own documentation.
* **e2e:** the 59 tests drive the HTML through its DOM and the SVG's
  `data-id`s, not `window.docair`. Tests of moving a part by hand and of the
  layout worker go; `ask.spec.ts` runs against the HTML and `docair serve`.

## 11. Open issues and docs this touches

Proposed; nothing is closed or changed before the user agrees.

| Issue or doc | Proposed |
|--------------|----------|
| docair-4wk.22, the Code layer's 12 s in the browser | superseded by step 3; its numbers are the baseline |
| docair-cof, layouts kept in the browser | closed, superseded by this epic; REGENERATE.md §1 and §2 (what a layout key holds) carry over to step 3 |
| docair-uwu, the layout worker in Firefox and Safari | obsolete once step 5 lands |
| docair-kil, viewer.js counted as two parts | obsolete once viewer.js goes |
| docair-4wk.12, lines that reach one of several parts | part of step 4 |
| docair-54p, the data kind's colours | part of step 4 |
| docair-4wk.15, walks use request lines | part of step 1 |
| docair-mpx, the code as a dependency matrix | a picture the CLI could draw too; decided on its own |
| BUNDLES.md | one line for the many lines between two boxes carries over to the Rust router and drawing (steps 3 and 4) for big sections, such as docair-core's; decided there by the numbers. Its alternative, routing in parallel in browser workers, is gone |

## 12. Steps

| Step | Issue | What |
|------|-------|------|
| 1 | docair-702.1 | the slice model in core, `docair slice <seed>`, golden tests |
| 2 | docair-702.2 | layers and views in core, the same pictures as today |
| 3 | docair-702.3 | layout in Rust: one font, placement, the router, kept layouts |
| 4 | docair-702.4 | drawing in Rust: SVG for each picture |
| 5 | docair-702.5 | the generated HTML: every picture baked in, the thin script; viewer.js, Cytoscape and elkjs go |
| 6 | docair-702.6 | `docair serve`, the AI in the HTML; the web app only uploads and shows |
| 7 | docair-702.7 | e2e against the new output; SPEC, ARCHITECTURE and LIBRARIES follow |

Steps 1 and 2 can run side by side; each later step needs the one before.

## 13. To decide

| # | Question | Proposed |
|---|----------|----------|
| 1 | one HTML file, or a folder of pages? | one file. The pictures are packed (gzip, base64) and unpacked when shown. Measured in step 5; a folder only if the file passes 15 MB |
| 2 | a layout library, or our own placement? | decided in step 3 by the numbers (§5) |
| 3 | which font? | one monospace font under the OFL for the pictures; the panels keep the page's font |
| 4 | how a walk steps | one picture per walk; a step lights parts and moves the camera, nothing is laid out per step |
| 5 | which parts get a slice of their own? | every part with a line |
| 6 | the tests switch | pictures that hold tests are baked twice, with and without them |
| 7 | how far a part's slice reaches | one hop each way; more through the links in it |

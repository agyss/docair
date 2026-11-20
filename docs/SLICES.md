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
many of them it reaches from where the datum is made. `docair check
--coverage` prints it for every datum. It is no warning: a stretch of its own
is often right, for a datum that comes in from outside or that two parts make.

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

Step 1 (docair-702.1) built it; SPEC §3.7 has the fields and an example.

## 4. The documentation the CLI writes

Every picture a reader can reach is worked out, laid out and drawn by the
CLI, and written into the HTML. Nothing is laid out in the browser.

| Pictures | On docair today |
|----------|-----------------|
| the layers (Overview … Code) | 6 |
| a section for each box, at its layer | 158 boxes |
| a slice for each datum | 9 |
| a slice for each walk | 7 scenarios, 1 walkthrough |
| a slice for each part with a line that holds no parts | 859 parts (a box has its section) |

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
* **A slice keeps its words' size** (docair-702.27). A layer or a section
  shrinks what each box holds to a budget (top-down); a slice is laid out
  without one (`layout_input::options_for`), so the picture grows, the reader
  pans, and Follow's first step reads at 8.8 px or more. On docair's own
  data.graph the smallest word went from 3.2 to 11 units.
* **Kept layouts.** The CLI keeps each picture's layout under a key made of
  what the layout reads and of the layout code's version. docs/REGENERATE.md
  §2 measured why: a key that holds more (a part's name, a test result, a
  line's place in the list) lays out every layer again after one new call.
* **Positions from before.** Placement can start from the positions of an
  earlier layout, so that two commits' pictures look alike (docair-zrr).

Goal, measured in step 3: a slice of up to 50 parts in 100 ms or less; all
pictures of docair's documentation in under 10 s.

**Measured in step 3** (docair's own graph, 2026-11-18, release build):

| | Result | Goal |
|-|--------|------|
| a slice of up to 50 parts | 60 ms at most | 100 ms ✓ |
| the slowest slice (data.graph, 102 parts, 108 lines) | 149 ms | |
| all 1 017 slices | 1.5 s | |
| all 1 023 pictures, laid out | 11.8 s, of which Code 10.2 s | 10 s ✗ |
| all 1 023 pictures, from the kept layouts | 0.12 s | |

**Placement against dagre** (docair-702.9). 45 pictures of docair: its 24
data, route, scenario and walkthrough slices, the 16 part slices with the
most lines, and the five smaller layers. Both placements routed by docair's
router, crossings and bends counted on the routed lines.

| | Crossings | Bends | Area |
|-|-----------|-------|------|
| docair-layout after step 3 | 1 205 | 2 532 | 34 Mpx² |
| a part with many lines as wide as they need | 583 | 1 953 | 35 Mpx² |
| … and lines leaving a box as nodes in a layer before or after it | 552 | 1 940 | 35 Mpx² |
| … and the direction chosen by the price of the routed lines | 398 | 1 645 | 37 Mpx² |
| … and dagre's median positions and alternating ties | **332** | **1 621** | 36 Mpx² |
| dagre 0.1.2, same part sizes | 647 | 1 782 | 140 Mpx² |
| dagre 0.1.2, parts as wide as their lines need | 286 | 1 372 | 141 Mpx² |

Tried and left out: network simplex layers (883 → 888 crossings; the median
moves of single parts already find nearly the shortest layers in boxes this
small), keeping the last of equally good orders (more crossings), crossings
weighted by line weight (no change). With the same part sizes, dagre has
14 % fewer crossings, but its pictures are four times as large, its nested
boxes overlap one another, and it leaves out lines that end at a box; per
picture it does better on 12, worse on 10, the same on 23.

What misses the time goal is the whole Code layer, which a reader rarely
wants; bundles (BUNDLES.md) or fewer rounds for big pictures would cut it,
decided when step 5 shows how often it is drawn.

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
| move a part the reader drags, and draw the lines docair-layout routed again for it | |

Its budget is 1,050 lines, checked by a test (1,000 until docair-702.37: showing an explanation's progress needed the room).

**Moving a part by hand** (docair-9qv). The page carries docair-layout's move
of a part built to WebAssembly (`crates/docair-layout-wasm`, about 90 kB
gzipped). The crate's build script builds it for wasm32 whenever docair is
built, so it is always the code as it is and nothing built is kept in the
repository; a build needs the wasm32 target (docair-702.42). While the reader holds a part,
each time it has moved a whole track (8 px), and when they drop it, the
script hands the module the picture's layout input, its layout and how far
the part moved; the module shifts the part and what it holds, grows the boxes
around it, and shrinks a box where the part was what held its side out, its
side following the part by as much (docair-702.41); it routes again the lines
that touch the part, and those inside a box that shrank, around every part and
around the lines that stay, which count as used tracks so a moved line keeps
off them (docair-702.40); it places their words, and answers with each line ready to draw (`d` and the arrow, as
the SVG writes them). While held, the lines follow and the words wait; on
the drop they take their place. The router is the one `docair build` lays
out with; the script only sets attributes. A move lasts until the page is
loaded again.

## 8. The AI in the HTML: `docair serve`

`docair serve [root] [--bind 127.0.0.1:8765] [--app <dir>]` runs the CLI as a
web server for one project (docair-702.6.2, built). The server is
docair-server's router with the project in its settings, so every route has
one implementation: the docair-server program (uploads, in the cluster) and
`docair serve` (a project, on a developer's machine) are two ways to start
it. It writes the project's documentation when it starts, and again:

* when the project's files change (it looks every second at names, sizes and
  times, outside `.docair`, `target`, `node_modules` and hidden folders);
* after an answer that brings a walkthrough (recorded in the walkthroughs
  folder, as `docair ask` does);
* after edits are applied.

Writing again starts with a scan: on docair 0.7 s when the code index is kept
(docair-rc8), 7 s when the code changed and rust-analyzer runs again; kept
layouts make the pictures quick.

| Route | Request | Answer |
|-------|---------|--------|
| `GET /` | | the documentation (`docair.html` in the output folder), never cached |
| `GET /<path>` | | any other file in the output folder (`docair.graph.json`, …) |
| `GET /api/version` | | `{ "version": 3 }`, one more each time the documentation is written again |
| `GET /api/ai` | | `{ "enabled": true, "model": "…" }` (docair-server's) |
| `POST /api/ask` | `{ "question": "…", "history": [{ "question", "answer" }], "context": "part id" }`, the last two optional | `{ "reply": Reply, "regenerated": bool, "version": n, "patch": "unified diff", "pending": n, "html": "…" }`: `patch` holds the waiting edits and, for a view change, `docair.toml` as it will be written; `html` is the reply as the page's chat shows it |
| `POST /api/word` | `{ "scope": "id", "limit": n, "all": bool }`, all optional | the same as `/api/ask`; the words wait to be applied |
| `POST /api/glossary` | `{ "limit": n }`, optional | the same; the entries wait to be applied |
| `POST /api/apply` | | `{ "edited": ["api/src/lib.rs", "docair.toml"], "regenerated": true, "version": n }`; 409 when nothing waits |
| `POST /api/scan` | an upload (docair-server's) | docair-server's `ScanResult` |
| `GET /app/…` | | the web app, with `--app <dir>` |

A request that brings a `graph` (what the web app sends today) is about that
graph and answers the plain `Reply`, as docair-server always did; the
description is `crates/docair-server/openapi.yaml`.

`Reply` is docair-ai's: `answer`, and when there are any, `walkthrough`,
`view`, `changes`, `wording`, `glossary`, `problems`, then `usage`,
`seconds` and `requests`. The AI routes work on the server's own graph, so
the page sends only its question. Errors are `{ "error": "…" }`: 503 when AI
is off, 429 when two questions are already with the model, 400 for a
question that is empty or longer than 500 characters, 502 when the model
fails.

**In the HTML** (docair-702.6.3, with step 5):

* the chat, apply and preview panels move from the web app into the HTML,
  written by the CLI, sent and shown by the script;
* the page reloads, keeping its `#` address, when its own `/api/ask` or
  `/api/apply` answers `regenerated: true`, and it polls `/api/version` every
  5 s while it is visible, for documentation written again from elsewhere;
* the HTML finds the server at its own address when the server serves it,
  else in `<meta name="docair-serve" content="…">`; without a server it works
  fully, minus the AI, and says so where the AI would be.

**The web app** (docair-702.6.4) keeps the upload, the progress, and the
frame that shows the HTML; the messages between page and app shrink to
`ready`. The AI works in an uploaded page too: docair-server keeps each
upload as a project (`KeptUploads`, at most 8, dropped after an hour unused)
and serves it at `/api/projects/<id>/` with the routes above, open to any
origin for the sandboxed frame. The frame shows the page by that address, so
an applied edit reloads it from the server. (First decided otherwise,
docair-702.10; the user asked for the AI in uploads the same day.)

**More than one server pod** (docair-702.32). An upload is kept in the pod
that took it, so each pod forwards a request for an upload it does not keep
to the pod that does:

| Piece | How |
|-------|-----|
| the id | `<pod>-<random>`: the pod's address in hex (its IP from the downward API, `DOCAIR_POD_IP`, and the port of `DOCAIR_PEERS`, which every pod listens on), then 32 random hex digits. Without those two settings (one pod, `docair serve`), the id is the random part alone, as before |
| forwarding | a request for `/api/projects/<id>/…` the pod does not keep goes to the pod in the id: same method, path, query, body, `content-type` and `accept`; its answer comes back as it is, with CORS added |
| only to server pods | the pod forwards only to an address that `DOCAIR_PEERS` (a headless Service, `docair-server-peers:8080`) resolves to, so a made-up id cannot send the server anywhere else |
| no loops | a forwarded request carries `x-docair-forwarded: 1` and is never forwarded again |
| a pod that is gone | its uploads are gone too: the answer is the 404 *no longer kept*, a page or JSON as before |
| the cluster | the chart sets `DOCAIR_POD_IP` in the Deployment and `DOCAIR_PEERS` in the ConfigMap, adds the headless Service, and the NetworkPolicy lets server pods reach each other on 8080; `server.replicas` is 2 again |

Shared storage instead (a volume both pods write) would need a
ReadWriteMany provisioner, which kind does not have, and every pod would
still scan the upload into its own memory. Routing by the id in front of the
pods cannot work: the id is made by whichever pod took the upload.

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

* **Rust:** golden slices and pictures (JSON) on the fixtures, golden SVGs
  (step 4); the layout's own tests (docair-layout); the router once checked
  line for line against viewer.js (docair-702.3.4); the server's routes
  (docair-server).
* **e2e:** the tests drive the HTML through its DOM, its `#` addresses
  (`#layer/<key>`, `#box/<id>`, `#slice/<seed>`, `#part/<id>`) and the SVG's
  `data-id`, `data-from`, `data-to` and `data-edges`, not `window.docair`.
  The AI tests run against `docair serve` with the fake AI.

**The 59 tests today, and what happens to each** (docair-702.7.1): 32 stay
as they are, or see the same thing in the new DOM; 20 change, because the
behaviour changes; 7 go, because what they test goes.

| File | Test | | In the new output |
|------|------|-|-------------------|
| apis | descriptions meet the code | stays | the handler's details show what the description says |
| apis | disagreements are warnings | stays | the diagnostics list, written by the CLI |
| ask | a question becomes a walkthrough that plays on the chart | changes | the chat panel in the HTML asks `docair serve`; the page reloads at the new version, and the walkthrough is `#slice/walkthrough:<id>` |
| ask | a follow-up carries the conversation | changes | the same panel; the fake AI's requests carry the earlier turn |
| ask | I don't care about … (hide) | changes | the view change waits; `POST /api/apply` writes `[view] hide`, and the part is gone from the documentation |
| ask | a correction becomes an edit, applied to the code and the page | changes | the panel shows the patch; apply edits the file, and the page shows the new line |
| ask | Word the lines shows words at once and writes them into the code | changes | the proposed words are listed in the panel, not drawn before they are applied (nothing is laid out in the page); applied, the documentation shows them |
| ask | Explain the terms underlines them at once and writes the glossary | changes | the same: entries listed, applied, then underlined in the documentation |
| ask | the selected part goes along as context | stays | the selected part's id is `context` of `/api/ask` |
| cli | docair ask records a walkthrough and docair apply writes the edits | stays | unchanged: it tests the CLI |
| cli | docair chat keeps the conversation and /apply writes the edits | stays | unchanged |
| cli | docair word writes words for the lines into the tags | stays | unchanged |
| cli | docair glossary writes new entries into the glossary | stays | unchanged |
| cli | a view change with --apply goes into docair.toml | stays | unchanged |
| data | the shop front end is drawn in the browser, where it runs | stays | at `#layer/apps`, the front end's `data-id` lies inside the browser's box |
| data | the order is data: made by the cart, followed along its way | changes | Follow opens `#slice/data.order`; its steps come from the CLI |
| data | data that a function makes and returns is followed back to its caller, and on | changes | `#slice/data.summary` on e2e/fixtures/answer has the step back |
| generate | a picked folder becomes a page | stays | the frame shows the HTML the server wrote |
| generate | each step of a generation shows how far it is | changes | the upload in per cent and the scan in seconds stay; "the page draws itself" goes, as the page draws nothing |
| generate | secret files are never uploaded | stays | unchanged |
| generate | New upload goes back to the upload page | stays | unchanged |
| glossary | hovering a term explains it, Read more opens the doc | stays | the popup shows the entry the CLI wrote |
| glossary | a term opens its entry; the glossary lists them all | stays | the same |
| infra | Terraform, Helm and manifests become one picture | stays | `#layer/infra`, `#layer/k8s` and `#layer/network` hold the expected parts |
| layout | a function picked at Apps comes with its box opened, details first | changes | `#part/<id>` shows the section of its box, with its details |
| layout | a function the layer has no box for comes at Apps | changes | `#part/<id>` goes to the part's best picture |
| layout | a layer comes back as it was laid out, without a new layout | goes | the page lays out nothing; kept layouts are the CLI's (layout_store) |
| layout | the worker lays out inside the web app too | goes | there is no layout worker |
| levels | each layer shows what its question is about | stays | each `#layer/<key>` holds the parts its question is about |
| levels | a request through the network is one line between the apps | stays | one line whose `data-edges` are the request's hops, with its via words |
| levels | a step says what each call does and what comes back | stays | the step card the CLI wrote |
| levels | an answer comes back after the work it waits for | stays | the order of the walk's messages |
| levels | every step stays readable in a small window | changes | each step's camera box comes from the CLI; at a small window the step's words are still drawn at 0.8 or more |
| levels | every line has words | stays | every line in the SVG has its words |
| levels | zooming into a box opens it | changes | scrolling into a box follows its link to `#box/<id>` |
| levels | parts found only in the code wait until their box is opened | stays | the box says `· N more`; its section `#box/<id>` shows them |
| levels | the page says what it draws while it lays out | goes | nothing is laid out in the page |
| levels | a walk keeps one layer, and a step opens into its box | changes | a walk is one picture; a step links to its box's section |
| levels | a layer without any part of the walk is off while it plays | goes | a walk is its own picture, not played on a layer |
| levels | the legend shows from the start | stays | the legend the CLI wrote |
| levels | the details before anything is selected | stays | the same |
| levels | lines follow a part that is moved | comes back | docair's router, built to WebAssembly, routes the lines of a part dragged in the page; they end on its border at right angles and cross no part (docair-9qv, §7) |
| levels | every line is drawn at right angles and ends on its part | stays | the SVG's line paths: right angles, ends on a part's border (also a Rust test) |
| levels | a walk's lines point ahead | changes | a walk's slice is laid out with its lines pointing ahead: the CLI gives the layout that hint (viewer.js `flowHints`, walked lines; not yet in layout_input) |
| levels | top-down draws its own lines, off the parts and off each other | stays | the SVG's lines cross no part (also measured in Rust) |
| levels | top-down routes the lines again after a part is moved | goes | lines follow a part that is moved checks the lines routed again |
| meanings | a word the code defines explains itself and names its part | stays | the popup |
| meanings | an unchecked glossary entry gives way to the code | stays | the popup |
| meanings | docair glossary settles the open words once | stays | unchanged: it tests the CLI |
| pick | a part picked by a link comes close and stands out | changes | `#part/<id>`: the camera box from the CLI, the part marked |
| pick | a part clicked on the canvas is selected where it is | stays | the script selects it and shows its details; the camera stays |
| sections | the eye hides a section and its chip shows it again | changes | the script hides it (its place stays empty, nothing is laid out again) |
| sections | Hide this section in the details | changes | the same |
| sections | showing a part inside a hidden section hides its siblings instead | changes | the same, in the script |
| sections | Show only this keeps one part and the groups around it | goes | `#box/<id>` and `#part/<id>` show one part's surroundings; the reader's hiding stays for sections |
| sections | docair.toml [view] show opens a part inside a hidden section | stays | written by the CLI |
| sections | docair.toml [view] hide is the default | stays | written by the CLI |
| tests-layer | tests show with their results and can be hidden | changes | the switch shows the pictures with or without tests (SLICES.md §13, choice 6) |
| tests-layer | a part lists the tests that cover it | stays | its details |

Done (docair-702.7.2, docair-702.7.3): every spec reads the page's DOM, its
addresses and `docair.graph.json`; `ask.spec.ts` starts a `docair serve` of
its own for each test, on a copy of the shop, at a port the system picks.
Two tests moved instead of going: the order of a walk's answers is tested in
Rust (`fixtures.rs`), and the right angles and ends of every line on every
layer of shop and platform are in `page.spec.ts`. The choices made on the way
are decision issues docair-702.10 to docair-702.16.

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
| docair-mpx, the code as a dependency matrix | dropped: not needed |
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

## 13. Decisions

| # | Question | Decided |
|---|----------|---------|
| 1 | one HTML file, or a folder of pages? | one file (2026-11-18). The pictures are packed (gzip, base64) and unpacked when shown; a folder only if the file passes 15 MB |
| 2 | a layout library, or our own placement? | our own, by the numbers (step 3, docair-702.9; §5) |
| 3 | which font? | one monospace font under the OFL for the pictures, 0.6 em a character (step 3); the panels keep the page's font |
| 4 | how a walk steps | one picture per walk (2026-11-18); a step lights parts and moves the camera, nothing is laid out per step |
| 5 | which parts get a slice of their own? | every part with a line that holds no parts (step 1): a box's lines across its border reach 215 parts (`browser`), so a box has its section instead |
| 6 | the tests switch | pictures that hold tests are baked twice, with and without them (2026-11-18) |
| 7 | how far a part's slice reaches | one hop each way (step 1); more through the links in it |

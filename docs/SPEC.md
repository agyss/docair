# docair specification (v0)

This document is the foundation of docair. It defines:

1. **how the app works**: the pipeline from source files to the picture (§1),
2. **what each part does**: every input construct, tag, kind and output (§2–§9),
3. **how a use case runs**: scenarios walked through step by step: who calls
   whom, what data goes along, what comes back, and when a file is read or
   written (§7),
4. **what the user sees**: for each construct that has a visual form, a
   *viewer mock* shows how the viewer (frontend) renders the data
   that the backend has just produced,
5. **how to ask**: a conversation in plain words with an AI model, whose
   replies walk through the graph, change what is shown, and propose edits
   to the doc blocks in the code (§10),
6. **what to leave out**: sections that are hidden from the picture (§11),
7. **what is tested**: tests as nodes, linked to what they cover, with the
   result of their last run (§12).

Viewer mocks are illustrations, not pixel specs. They fix **what** is shown
and **where**. Exact styling is left to the viewer implementation.

All examples use one running system, `shop`:

| Part            | Tech      | Role |
|-----------------|-----------|------|
| `shop-ui`       | Angular   | checkout page, calls the API |
| `orders-api`    | Rust/axum | stores orders, publishes events |
| `orders-db`     | Postgres  | managed database (no code) |
| `order.created` | topic     | async event |
| `mailer`        | Rust      | consumes the event, sends a mail |
| k8s chart       | Helm      | deploys `shop-ui`, `orders-api`, `mailer` |

---

## 1. How docair works

```
 sources ──► 1 discover ──► 2 extract ──► 3 enrich ──► 4 build ──► 5 resolve ──► 6 check ──► 7 emit ──► viewer
             files          RawBlocks     + symbols    Graph       edges          diagnostics  graph.json
                                          + inferred   (nodes)     bound          exit code    mermaid
```

Each stage has one job. A stage only consumes the output of the stage before it.

| # | Stage        | Input → Output | What it does |
|---|--------------|----------------|--------------|
| 1 | **discover** | project root → list of files | Walks the root, honours `.gitignore` and `docair.toml` include/exclude, and picks a *language spec* for each file by extension (§2). |
| 2 | **extract**  | file → `RawBlock[]` | Generic and language-agnostic. Finds doc blocks by comment syntax, strips the line prefixes, splits each block into description + tags (§2.2), and records the code line the block attaches to. Also reads sibling Markdown (§5). |
| 3 | **enrich**   | `RawBlock[]` → `RawBlock[]` | Language adapters (§6) add the *symbol* and *kind*, plus tags that the code already implies (`@http` from an axum route). Adapters may also *discover* nodes that have no block at all (k8s objects). Inferred values never overwrite authored ones. |
| 4 | **build**    | blocks → nodes | Each block becomes one node (§3). Assigns ids, applies `@part-of` defaults, and merges Markdown docs into nodes. |
| 5 | **resolve**  | nodes + tag refs → edges, scenarios | Turns every data-flow tag into an edge by resolving its `<ref>` (§4.3). Creates implicit topic and sub-part nodes. Orders the `@flow` steps of each scenario and derives the messages between them: calls, returns, reads, writes, events (§7). |
| 6 | **check**    | graph → diagnostics | Unresolved refs, duplicate ids, unknown tags and kinds, block/code mismatches, broken flows (§8). |
| 7 | **emit**     | graph → files | Writes `docair.graph.json` (§9), the integration API for other tools, plus optional Mermaid, and the page (§3.5). |

> **Viewer mock: the whole app.** The viewer is a single page that loads
> `docair.graph.json`. Every later mock is one part of this screen.
>
> ```text
> ┌─ docair · shop ─────── view: [Graph] Sequence ── layer: [Code] Runtime ── ⚠ 2 ─┐
> │ ┌─ tree ──────────┐ ┌─ graph ──────────────────────────────┐ ┌─ details ─────┐ │
> │ │ ▾ shop          │ │                                      │ │ orders.create │ │
> │ │  ▸ shop-ui      │ │  ╭ shop-ui ╮   POST /api/orders      │ │ function      │ │
> │ │  ▾ orders-api   │ │  │ ▣ cart  │──req: CreateOrder──►┐   │ │ src/orders.rs │ │
> │ │     ƒ create  ◄─┼─┼─ ╰─────────╯                     │   │ │ :12           │ │
> │ │  ▸ mailer       │ │        ╭ orders-api ──────────╮  │   │ │───────────────│ │
> │ │  ◯ orders-db    │ │        │ [POST] ƒ create  ◄───┼──┘   │ │ Persist a new │ │
> │ │  ▱ order.created│ │        ╰──────┬────────┬──────╯      │ │ order and ... │ │
> │ └─────────────────┘ │      writes ▼        ▼ emits         │ │ [Docs][Tags]  │ │
> │                     │      ◯ orders-db   ▱ order.created   │ │ [Source]      │ │
> │                     └──────────────────────────────────────┘ └───────────────┘ │
> │ ▶ walk me through: [Check out a cart ▾]  ◀ ▶  ●──◉──○  2/3 The API stores …    │
> └────────────────────────────────────────────────────────────────────────────────┘
> ```
>
> | Region      | Shows | Backed by |
> |-------------|-------|-----------|
> | header      | system name, layer switch Overview … Code (§3.4), view switch Graph / Sequence (§7.4), diagnostics count | `graph.system`, `diagnostics` |
> | tree        | `@part-of` hierarchy, click to focus | `nodes[].part_of` |
> | graph       | nodes as kind shapes, edges as arrows | `nodes`, `edges` |
> | details     | on first open, the scenario list: "walk me through …" (§7.4); then the selected node, or the current step of a walkthrough | `flows`, one `node` |
> | walkthrough bar | scenario picker, Back / Next, step dots, the step's title | `flows` |
>
> Every picture is laid out and drawn by the CLI (§3.5): every box on its own,
> scaled to fit its size budget, and docair's own router draws the lines at
> right angles and puts their words on them. The direction is chosen by the
> CLI: top-to-bottom, unless left-to-right fits the canvas 1.1 times better.
> On first open, the page shows the Apps layer (§3.4). The page lays out
> nothing: each picture is ready the moment it is shown.

### 1.1 Configuration: `docair.toml`

Optional, at the project root. Without it, docair scans the root and names the
system after the directory.

```toml
system       = "shop"                 # id of the top-level system node
include      = ["apps/**", "charts/**"]
exclude      = ["**/node_modules/**", "target/**"]
strict       = true                   # unresolved refs are errors (§8)
walkthroughs = "docs/walkthroughs"    # recorded walkthroughs (§10.3), this is the default
meanings     = "docs/meanings.toml"   # the decisions about words (§13.5), this is the default

[view]                                # what the page shows by default (§11)
hide = ["terraform"]                  # sections left out; the viewer can show them again
tests = true                          # show test nodes (§12); false hides them

[tests]                               # test results to show on the page (§12.3)
results = ["e2e/test-results/results.json"]

[import]                              # API descriptions to read (§6.2)
apis = ["contracts/*.json"]           # JSON read besides *openapi*.json, *swagger*.json, *asyncapi*.json

[[import.api]]                        # one description: its service, its base path, routes it leaves out
file    = "api/openapi.yaml"
part_of = "orders-api"
base    = "/api"
ignore  = ["GET /healthz"]

[checks]                              # checks that are off by default (§8)
unrequested_routes = true             # W118: a route nothing in the project requests

[code]                                # calls between functions (docs/ADAPTERS.md §5.4)
run_indexers = true                   # run rust-analyzer and scip-typescript when installed, and read their indexes
scip = ["ci/rust.scip"]               # or read index files built elsewhere (CI), inside the root

[explain]                             # docair explain (docs/EXPLAIN.md)
store        = "docs/changes"         # where the findings about two commits are kept
max_requests = 8                      # the most requests to the model per run

[ai]                                  # the model that answers `docair ask` (§10.1)
endpoint    = "https://api.together.xyz/v1"
model       = "zai-org/GLM-5.3-Flash"
api_key_env = "DOCAIR_AI_API_KEY"     # the default; the key itself is never in this file
```

`run_indexers` builds the code (rust-analyzer runs build scripts and
procedural macros), so the server ignores it for an upload: an uploaded
project is scanned without running anything. The index it makes is kept in
the cache folder (`$DOCAIR_CACHE`, else `~/.cache/docair`) under a key of
what rust-analyzer reads: the `.rs` files and `Cargo.toml` files with their
text, `Cargo.lock` and rust-analyzer's version. A scan of the same code takes
the kept index instead of running the indexer: on docair, 0.7 s instead of
7.2 s. The six newest indexes are kept. Every path `docair.toml` names
(`scip`, `[tests] results`, `meanings`, `walkthroughs`, and the docs a
glossary links to) must be a file or folder inside the root: not an absolute
path, `..` out of it or a link out of it (W110, W111, W113).

### 1.2 CLI

| Command          | Runs stages | Result |
|------------------|-------------|--------|
| `docair build` (also `docair scan`) | 1–7 | builds the documentation on demand: writes `docair.graph.json` and `docair.html` to `--out` (default `.docair/`); exits 0 with warnings, 1 with errors (outputs are written anyway); `--changes <A>` adds the page's changes section: what the working tree changes since commit A (docs/CHANGES.md), with the findings `docair explain` stored for that pair |
| `docair tidy [root] [--apply]` | 1–6, tidy | lists the hand-written tags that only repeat what the code says (docs/ADAPTERS.md §6.1) by file, each with the code that says the same, and writes `docair.patch` to `--out`; `--apply` writes the edits into the files. `docair build` and `docair check` end with one line while any repeat: *4 tags repeat the code: run `docair tidy`* |
| `docair diff <A> [<B>] [--root .] [--json]` | 1–6 twice, diff | what two commits change in the system's flows (docs/CHANGES.md), or a commit and the working tree: each commit read from a detached git worktree and its graph kept in the cache folder (`graphs/`); the changed parts (the graph diff's, moves found by the body's fingerprint, and the parts the changed lines fall in) and every scenario, walkthrough, datum and route slice through them compared at A and at B; `changes.json` (`docair.changes/0`) to `--out`, a Markdown summary on stdout |
| `docair explain <A> [<B>] [--root .] [--json]` | 1–6 twice, diff, AI | the review of what `docair diff` finds (docs/EXPLAIN.md §9): docair's pieces of the whole change (`docair_core::review::pieces`) with each changed part's whole text before and after, the changed files no part holds, the commit messages and the changed docs (as claims, without ids) go to the model, which groups the pieces into changes, one difference in what the app does each, with what, how, why in its own words, what it found off (each a contradiction, what a commit message, a doc or a code comment says, and which one, against what the code clearly does otherwise; or what the code does that nothing mentions; or a risk in the code), and what to check; every claim bound against both graphs and the pieces, every piece a change, one the AI puts in none shown as a change of its own in docair's words; kept in `[explain] store` (default `docs/changes/<A>..<B>.json`, `docair.explained/2`), asked again only when its changes and the pieces it put in none no longer cover the pieces, never about a human's change; at most `[explain] max_requests` (default 8) requests of at most 60,000 characters, 3 at once, and one more that merges the changes that came back twice and places the pieces left or makes changes of them; progress on stderr; the review as a pull request comment on stdout |
| `docair check`   | 1–6         | prints diagnostics; exits non-zero on errors (CI gate) |
| `docair mermaid` | 1–5, 7      | prints a Mermaid diagram to stdout |
| `docair ask "<question>" [--follow-up] [--apply]` | 1–7 + AI | asks the configured model and prints the reply: answer, walkthrough (recorded in `walkthroughs/`), view changes, proposed edits (written to `.docair/docair.patch`). `--follow-up` continues the last conversation; `--apply` writes the edits into the files and the view changes into `docair.toml` (§10) |
| `docair word [--scope id] [--all] [--apply]` | 1–7 + AI | the model writes a few words for each line that says nothing yet (§10.8); `--all` also for lines that show only a type; they become edits of the tags, `--apply` writes them |
| `docair glossary [--limit n] [--apply]` | 1–7 + AI | the model finds words of the docs that the glossary does not explain yet and writes entries for them (§13.4) |
| `docair chat` | 1–7 + AI | the same as a conversation in the terminal: one question per line, each edit offered for applying (§10.6) |
| `docair apply [patch]` | none | applies `.docair/docair.patch` (or the given patch) to the files (§10.6) |
| `docair ai [--check]` | none   | prints the AI settings and where the key comes from; `--check` sends one short request (§10.1) |
| `docair slice [seed] [--all]` | 1–6 | prints a slice as JSON: one part of the system, from a seed to its ends (§3.7); without a seed it lists the seeds, `--all` prints every slice. `check --coverage` also says, for each datum, how many of its lines its slice reaches |
| `docair layer [layer] [--open id] [--tests bool] [--all]` | 1–6 | prints what a layer draws as JSON (§3.4): each part with the box it stands in, open or closed, and each line with the lines of the graph it stands for and the parts it passes; without a layer it lists the layers the project offers |
| `docair serve [root] [--bind addr] [--app dir] [--changes A]` | 1–7, watch | serves the project's documentation at `/` and writes it again when its files change; `--changes A` gives the page its changes section, as `docair build --changes A` does (the system's file events, acted on once they are quiet for 0.4 s; outputs and what a scan skips do not count), after an answer with a walkthrough, and after applied edits; `/api/ask`, `/api/word`, `/api/glossary`, `/api/apply` on the server's own graph, `/api/version` to poll, uploads as docair-server (docs/SLICES.md §8) |

---

### 1.3 The web app: generating a page

docair-web turns a picked or dropped folder into a page. It leaves out
dependencies, build output, secrets and files over 2 MB, and says at each
step where the generation is, so that a long one never looks stuck:

| Step | The user sees |
|------|---------------|
| a dropped folder is read | *Reading the folder · 1200 files so far* |
| 1 of 3: the files go up | a bar, *Uploading 286 files · 3.3 MB of 7.8 MB · 42%* |
| 2 of 3: docair reads them on the server | a moving bar, *docair reads the 286 files on the server · 6 s*; after 10 s also *big projects take a while* |
| 3 of 3: the page comes back | a bar, *Receiving the page · 1.0 MB of 5.7 MB · 18%* |

The page comes laid out and drawn by docair, so it shows at once. The app
frames it under a slim bar (what docair found, the diagnostics, *New upload*,
*Open in a new tab*, *Download*); the page tells the app only when its first
picture shows (`docair:ready`), and the frame gets `data-ready`.

docair-server keeps each upload as a project and serves its page at
`/api/projects/<id>/`, the `page` of the scan's answer. The frame shows the
page from that address, so the page's AI asks the server about the upload.
An applied edit changes the server's copy, and the frame reloads with it.
*⤓ Patch* saves the edit for the user's own files. *Open in a new tab* opens
the same address. *Download* saves the page as it is now.

The server keeps at most 8 uploads. A ninth pushes out the one used longest
ago. An upload not used for an hour is dropped, with its folder. A restart
or a deploy drops them all. An upload lives only in the server pod that
took it. Its id names that pod, and another pod passes the upload's
requests on to it (docs/SLICES.md §8), so the chart runs two server pods.
An upload no longer kept answers 404:

| Who asks | What they get |
|----------|---------------|
| the frame, a new tab | a short page: *This upload is no longer kept. Upload the folder again to see its page and ask the AI about it.* |
| the page's own calls | JSON: `{"error": "this upload is no longer kept; upload the folder again"}` |

Before the upload, the picked files are read into memory (*Reading the
folder · 286 files so far*), so a file that changes on disk meanwhile cannot
break it. A file that cannot be read says *A file changed while it was
read; pick the folder again.*

## 2. Doc blocks (stage 2: extract)

A doc block is a comment that opens with a triple-star marker. Ordinary comments
are ignored.

| Comment family | Languages (v0)                 | Open      | Close     | Line prefix stripped |
|----------------|--------------------------------|-----------|-----------|----------------------|
| C-style        | Rust, TypeScript/JS, Helm tpl  | `/***`    | `***/`    | leading `*`          |
| Hash           | YAML / Kubernetes, TOML, shell, Terraform (`.tf`), Dockerfile (by name: `Dockerfile`, `*.Dockerfile`), nginx (`.conf`) | `#***`    | `#***`    | leading `#`          |
| Markup         | HTML (Angular templates)       | `<!--***` | `***-->`  | none                 |

Helm templates may also use `{{/*** ... ***/}}`, so the block is removed
from the rendered output.

These rows are the **language registry**. Adding a language with a known comment
family means adding one row. No core change is needed.

An opener counts only when it is the **first text on its line**, so a marker
inside a string literal (`let open = "/***";`) is not a block. A marker
followed by one more star (`/*******`) is a banner comment, not an opener.

### 2.1 Attachment

A block **attaches to the code that follows it**: the next non-blank line,
skipping attributes and decorators such as `#[derive]` or `@Injectable()`
(including multi-line ones like `@Component({ ... })`) and ordinary line comments.
The adapter (§6) turns that code into a *symbol* (`fn create_order`,
`class OrdersService`, `kind: Deployment / name: orders-api`).
A block followed by no code (end of file, or another block) is a *free block*.
It needs an explicit `@id`.

```rust
/***
 * Persist a new order and publish `order.created`.
 *
 * Validates stock first; rejects with 409 if an item is sold out.
 *
 * @id       orders.create
 * @kind     function
 * @part-of  orders-api
 * @http     POST /api/orders
 * @in       req: CreateOrder
 * @out      order: Order
 * @writes   orders-db.orders (Order)
 * @emits    topic:order.created (OrderCreated)
 * @flow     checkout 2 "The API stores the order"
 ***/
pub async fn create_order(/* ... */) { /* ... */ }
```

### 2.2 Body grammar

```
block       := description? tag*
description := free text up to the first tag. Its first line is the summary.
tag         := "@" name WS value
value       := rest of the line + any following non-tag, non-blank lines (joined with spaces)
```

Extract output (one per block, before enrichment):

```
RawBlock {
  file, line,             // where the block starts
  summary, description,   // first line / rest of the free text
  tags: [(name, value, line)],
  following: { line, text }   // the attached code line, or none
}
```

Unknown tags are kept in the model with a warning, so tags can be extended
without breaking older versions of docair.

> **Viewer mock: details panel.** The selected node shows the block's text.
> The summary is the title. The description is the body.
>
> ```text
> ┌─ details ───────────────────────────────┐
> │ ƒ orders.create            function     │
> │ Persist a new order and publish         │
> │ `order.created`.                        │
> │                                         │
> │ Validates stock first; rejects with 409 │
> │ if an item is sold out.                 │
> │ ─────────────────────────────────────── │
> │ [Docs] [Tags] [Source]                  │
> │ src/orders.rs:12 · fn create_order      │
> └─────────────────────────────────────────┘
> ```

---

## 3. Nodes (stage 4: build)

### 3.1 Identity and structure tags

| Tag         | Value                    | What it does |
|-------------|--------------------------|--------------|
| `@id`       | `node-id`                | Stable id. Ids are lowercase `[a-z0-9_-]` segments joined with `.`. If omitted, the adapter derives one from the module path (Rust: `<crate>.<module…>[.<type>].<symbol>`, e.g. `docair-core.extract.parse_body`), else it is `<file-stem>.<symbol>`. The viewer uses it as the URL anchor (`#orders.create`), so links stay stable across scans. Ids that docair creates itself are namespaced: `topic:…`, `k8s:…`, `unresolved:…`. |
| `@kind`     | see §3.2                 | What the node is. Adapters can infer it (Angular `@Component` → `component`). It picks the shape and colour. |
| `@part-of`  | `node-id`                | Parent node. Builds the tree: function → module → service → system. Default, first match wins: the enclosing documented item (impl, trait, inline module), the adapter's module path (`docair-core.extract`, then `docair-core`), the nearest enclosing directory whose name is a node id, the system. |
| `@doc`      | `path.md[#anchor]`       | Explicit link to Markdown docs (see §5 for the implicit rule). |
| `@deployed-as` | `k8s:<kind>/<name>`   | Links a code-level node to its Kubernetes workload: where the code runs (§3.4). |
| `@runs-in`  | `<ref>`                  | The code comes from its `@part-of` place (a container that serves it, an image) but runs in `<ref>` (the browser, a phone, a worker). The node is drawn where it runs, with its parts; its details say *runs in* and *code from* (§3.6). |
| `@carried-as` | `word, word`           | Data nodes only: other words payloads use for this datum (`CreateOrder, Order`), besides its name (§3.6). |
| `@path`     | `glob`                   | Stores only, repeatable: where the store is on disk, relative to the project root (`docs/walkthroughs/*.json`, `.env`, `data/**`). Code that writes a fitting path out (`fs::read_to_string(".env")`, `fs::read_dir("docs/walkthroughs")`) gets a certain `@reads`/`@writes` of the store, with no block on the function (docs/ADAPTERS.md §5.7). `*` is any part of a segment, `**` any number of segments; a path that is the folder of a glob fits it too. |
| `@defines`  | `word, word`             | The block's part defines these words (docs/MEANINGS.md §10): the same as a decision `from = <its id>` in the decisions file, which wins where both speak. |

Rules:
* Two nodes with the same id are an **error**. The exception is Markdown, which
  only attaches docs to an existing id and never creates a duplicate (§5).
* `@part-of` must not form a cycle (error).
* Nodes without a parent are children of the system node.

> **Viewer mock: `@part-of` (collapse / expand).** A parent is drawn as a
> frame around its children. Collapsing the frame turns it into one node.
> Child edges are then re-attached to the frame and bundled: one edge per
> direction, thicker the more edges it stands for, labelled with what they carry.
>
> ```text
>  expanded                                 collapsed
>  ╭ orders-api ─────────────────╮
>  │ ƒ create   ƒ get   ƒ cancel │          ╭ orders-api (3) ╮══writes══► ◯ orders-db
>  ╰───┬──────────▲────────┬─────╯          ╰────────────────╯◄──reads───
>      ▼ writes   │ reads  ▼ writes
>      ◯ orders-db
> ```

### 3.2 Kinds

The kind controls the viewer's shape and colour. Unknown kinds are allowed and
get a warning. They are drawn as a plain box.

| Kind        | Meaning | Shape (mock) |
|-------------|---------|--------------|
| `system`    | the whole application, root of the tree | `╔═ shop ═╗` double frame |
| `service`   | deployable code unit (API, SPA, worker, CLI) | `╭ orders-api ╮` rounded frame |
| `module`    | code grouping that is not deployable on its own (crate, package, namespace, Angular module) | `▤ extract` |
| `component` | class-like building block: UI component, Rust struct/impl/trait, TS class | `▣ cart` |
| `function`  | function / method / handler | `ƒ create` |
| `endpoint`  | standalone HTTP route without a function node | `[POST /api/orders]` pill |
| `store`     | database, table, bucket, cache | `◯ orders-db` cylinder |
| `topic`     | message topic / queue | `▱ order.created` parallelogram |
| `external`  | third-party system outside the repo | `┆ payments ┆` dashed box |
| `job`       | scheduled or one-shot task | `⏱ nightly-report` |
| `workload`  | k8s Deployment/StatefulSet/Job | `⬡ orders-api` hexagon |
| `config`    | ConfigMap, Secret ref, config file | `⚙ orders-config` |
| `host`      | a machine or runtime that runs things: a workstation, a Docker engine, a k8s node | `◫ workstation` |
| `cluster`   | a Kubernetes cluster | `⎈ kind` |
| `namespace` | a Kubernetes namespace | `⬚ docair` dashed frame |
| `network`   | a network or address range: a Docker network, the pod or service CIDR | `⇌ kind-net` ellipse |
| `gateway`   | where traffic is routed or answered by software the project only configures: an ingress controller, an Ingress, a published port, a web server such as nginx | `⇶ traefik` |
| `container` | a container image as it runs: what a Dockerfile builds | `▦ docair-server` double frame |
| `release`   | an installed package: a Helm release, grouping everything one installation created | `⎔ docair release` |
| `test`      | a test case, or a suite of tests (§12) | `✓ a picked folder becomes a page` |
| `data`      | what users and the code make and carry: a question, uploaded files, a graph, an order (§3.6) | `◇ question` (in the Data list, not a box) |
| `api`       | an API description, an OpenAPI or AsyncAPI file, inside the service it describes; it holds the routes no code serves (§6.2) | `⎘ openapi.yaml` a page with a folded corner, rose |

**One node per thing that exists at runtime.** Its recipe is its source, not a
second node: a Dockerfile documents its container, a Helm chart (`Chart.yaml`)
documents its release, a template documents the object it renders. Whatever
brings the thing into existence (a Terraform resource, a build step) links to
it with `@creates` (§4.1). So *chart*, *`helm_release`* and *release* are
three roles of one installation, not three copies: the chart is the source of
the release, `helm_release.docair` creates it, and the Deployments, Services
and Ingress live inside it.

### 3.3 Node model

What stage 4 produces for each node:

```
Node {
  id, kind, name,           // name = symbol or last id segment
  summary, description,
  part_of: id?,
  source: { file, line, end? }?,  // none for Markdown-only nodes; end: the part's last line, its code's or its block's (docair-zrr.2)
  symbol: string?,          // "fn create_order"
  http: { method, path }?,
  deployed_as: ref?,
  docs: [ { file, anchor?, markdown } ],
  tags: [ { name, value, origin } ],       // all tags, including unknown ones
  origin: authored | inferred | discovered | placeholder
          // block / adapter-filled / adapter-created / stand-in for an unresolved ref
}
```

> **Viewer mock: origin.** Inferred values carry a small `ⓘ` with the reason.
> Discovered nodes (found from code alone) get a lighter outline.
>
> ```text
> [Tags]
>   @kind     function
>   @http     POST /api/orders     ⓘ inferred from #[post("/api/orders")]
>   @part-of  orders-api           ⓘ inferred from module path
>   @owner    team-checkout        ⚠ unknown tag
> ```

### 3.4 Layers: one concern at a time

A system has more than one story: the machines it runs on, the way a request
gets in, what is installed in the cluster, the programs and how they talk, the
code inside a program. Drawn all at once they are a tangle, so the viewer shows
one **layer** at a time. Each layer answers one question and draws the parts
that question is about; the rest is left out, or stands as context where a line
joins it to them.

| Layer | Answers | Draws | Leaves out |
|-------|---------|-------|------------|
| Overview | What are the big pieces, and how do they talk? | the system's direct parts, closed | nothing, it folds |
| Infrastructure | Which machines, networks and clusters are there, and what provisions them? | hosts, networks, clusters, the cluster's nodes, gateways outside the namespaces; namespaces as closed boxes; Terraform with its resources | what is inside the namespaces, the programs |
| Network | How does a request get in, and where does it go? | gateways (published ports, ingress controllers, Ingress), Services, networks, the cluster's nodes; Deployments as closed boxes; the frames around them | Terraform, ConfigMaps and Secrets, the code |
| Kubernetes | What is installed in the cluster, and how is it configured? | everything in the cluster down to the containers; the programs in them as closed boxes | the machines around the cluster, the code |
| Apps | Which programs are there, and how do they talk to each other? | each program and library as one box (a library of several crates shows them), stores, topics, clients and services outside, Terraform as one box, test suites | the machines and the cluster, whose boxes are **dissolved** (the programs stand side by side); gateways, Services and the other cluster objects |
| Code | How does a program work inside? | the Apps layer with every program and library open, down to their modules. Parts docair only found in the code (no doc block about them) wait until their box is zoomed into: a box of code (a program, a module, a type) holding only such parts, with no doc block about anything inside them, stands closed, as a module of functions only does; in a box that also holds documented parts, such parts without a line are left out, and the box says how many (`docair-server · 13 more`). A part with a line stays. Zooming into the box, a walk that passes the part, or picking it in the tree or the search opens the box | as Apps |

What a part is, for the layers: a **program** is a `service` or `job` that is
not inside another program and not declared in Terraform; a **library** is a
`module` that is not inside a program (an `api`, an OpenAPI or AsyncAPI
description, counts as a module throughout: it holds the endpoints it
declares as a module holds its functions); **Terraform's own parts** are the nodes
declared in `.tf` files that are not machines, networks, gateways, endpoints,
stores or outside services (its resources and the run itself).

**Context.** A part outside a layer that a line joins to one of the layer's
parts is drawn too, folded into the outermost box around it that holds none of
the layer's parts: the browser that calls the published port, at the
Infrastructure and Network layers; Terraform, which installs the releases, at
the Kubernetes layer. The Network layer takes context only from requests and
data (`call`, `route`, `select`, `data`).

**Lines through left-out parts.** A request or a datum that passes parts a
layer leaves out is drawn as one line between the drawn parts it joins, and
says what it passes. docair follows the line from a drawn part along lines of
the same family (a request: `call`, `route`, and `select` from a Service to
its pods; data: `data`), at most 8 hops:

* at a route fork it takes the route whose path is the longest prefix of the
  request's own (`POST /api/ask` takes `HTTP /api`, not `HTTP /`);
* a request that arrives at a dissolved box (a Deployment) ends at the
  programs drawn inside it;
* the line says the request it carries (the hop that names a route, else the
  first), and **via** names the gateways it passes (else every part):
  `docair-web → docair-server: GET /api/ai · POST /api/ask +4 · via host-port › traefik › Ingress docair`;
* at the Network layer, a request that leaves through a part the layer leaves
  out (a library) and ends at a service outside brings that service into the
  picture: `Deployment docair-server → ai-endpoint: POST /chat/completions · via docair-lib`.

The tip of a line lists every part it passes and every line it stands for.

**Which layers, and which first.** Only the layers a graph has something for
are offered: a graph of plain code gets Overview, Apps, Code; docair's own
repository gets all six. On first open, the viewer shows the Apps layer when it
is readable with the whole picture on the screen (its smallest words at zoom
0.65 or more), else the first other layer that is, else the Overview. Selecting a part that the layer leaves out, or draws only as a closed box
around it (in the tree, by search, by a link), opens the box the part is in:
on this layer when it draws a box around the part, else on Apps, else the
selection goes to the first layer that is about the part. The breadcrumb then
says where it is (*Apps › ask*), and scrolling out closes the box again. The
part's details show at once, before the picture is drawn again.

| Picked on docair's own page | before | now |
|-----------------------------|--------|-----|
| `docair-ai.ask.digest` at Network | all of Code laid out, 14 s | Apps with `docair-ai.ask` opened, 1 s |
| `docair-ai.ask.digest` at Apps | the closed box `docair-ai` selected | `docair-ai.ask` opened, 0.5 s |

Tests are a switch over every layer (§12.4).

**Worked out by the CLI.** `docair layer <layer>` prints what a layer draws,
with `--open` for boxes the reader opened: each part with the drawn box it
stands in (`parent`), whether it is drawn `open` or `closed` (a closed box says
how many parts it `holds`, an open box of code how many found-only parts it
leaves out, `more`), and each line with the lines of the graph it stands for
(`members`) and the left-out parts they pass (`via`). On docair's own graph
and on every fixture it draws the same parts and lines as the viewer, which
still works them out itself until docs/SLICES.md step 5.

> **Viewer mock: one request at two layers.**
>
> ```text
>  Layer  Overview  Infrastructure  [Network]  Kubernetes  Apps  Code
>
>  ☁ browser ──GET / · POST /api/*──► ⇶ host-port ──TCP to :30080──► ◫ kind-node
>                                                                       │ NodePort 30080 → :8000
>     ╭ docair namespace ────────────────────────────────╮               ▼
>     │ ⇶ Ingress docair ◄──match host and path─────────────────── ⇶ traefik
>     │   ├─HTTP /api─► ⊙ Service docair-server ─► ⬡ Deployment docair-server (15) ──POST /chat/completions──► ☁ ai-endpoint
>     │   └─HTTP /────► ⊙ Service docair-web ────► ⬡ Deployment docair-web (2)
>     ╰──────────────────────────────────────────────────╯
>
>  Layer  Overview  Infrastructure  Network  Kubernetes  [Apps]  Code
>
>  ╭ browser ────────╮  POST /api/scan · POST /api/ask +4          ▣ docair-server (12) ──► ▤ docair-core
>  │ ▣ docair-web    │─────────────────────────────────────────►        │
>  ╰─────────────────╯  via host-port › traefik › Ingress docair         ▼
>                                                                  ◯ upload-dir
> ```

### 3.5 Zooming in and out

`@part-of` makes one tree from the outside in: a host contains a Docker
engine, which contains a network, which contains the cluster, its namespaces,
their Deployments, the containers, the apps, their modules and functions. A
layer draws the part of that tree it is about; zooming moves inside it.

> **The browser's layout, until docs/SLICES.md step 5.** From here down to
> *Laid out by the CLI*, this section records how the viewer laid out and drew
> the pictures in the browser, with ELK and Cytoscape.js, and what was measured
> on the way. Those parts are gone: the CLI lays out and draws every picture
> now, and the page shows them (*Laid out by the CLI*, *Drawn by the CLI*, *The
> documentation page*, below). The rules for what a picture holds, the
> top-down sizing and the router carried over; the numbers here are the
> baseline the Rust layout was checked against.

**Zoom in (scroll).** Scrolling into a closed box opens it where it is; the
rest of the layer stays as it was. Keep scrolling to open the boxes inside it.
A breadcrumb (`Apps › docair-lib › docair-core`) shows the boxes opened;
clicking the layer's name closes them all. Double-clicking a box opens or
closes it.

**Zoom out.** Scrolling out past the whole picture closes the box opened last,
then goes up a layer: Code → Apps, any other layer → Overview. `-` does the
same; `+` goes to the next layer in the header.

**Pick a part.** A part picked outside the canvas (a link in the details, a
row in the tree, a diagnostic, a chip in a walk's step, a word's popup, the
page's `#id`) comes close. The camera puts it in the middle, its words at the
size they are drawn for, and never smaller than readable: zoom × the scale the
part is drawn at stays at 0.8 or more, as for a walk's step (§10.4). Its box
stays in view when the box fits at a readable zoom. A part too large to read
whole is shown whole, and a reader who is closer already keeps their zoom.

| Picked on docair's own page | before | now |
|-----------------------------|--------|-----|
| `docair-core.build` at Code, drawn at scale 0.49 | zoom 1, words 5.9 px | zoom 2.04, words 12 px |
| `docair-server` at Code, a box 1 381 px wide at zoom 1 | zoom 1, runs off the 830 px canvas | zoom 0.51, all of it in view |

The selected part has a halo in the accent colour, which swells twice when
the camera stops (not with reduced motion); a search hit only gets the accent
border. The search, a walk's step and a followed datum never dim the selected
part. A click on the canvas selects the part where it is: the camera does not
move.

* An edge into or out of a collapsed group stands for all the edges it
  bundles: drawn once per pair and direction, thicker the more edges it stands
  for, and labelled with what they carry (`POST /api/scan · POST /api/ask +4`,
  `scan() · write_json()`, `NodePort 30080 → :8000`).
* On the chart, a label has at most 48 characters, broken into lines of about
  28 so a long label makes the picture taller, not wider; the tip has every
  word.
* **Lines at right angles.** A line is drawn exactly on its route: only
  horizontal and vertical pieces, both ends pinned where the router ends it
  (left to itself, Cytoscape aims from a part's centre at the first bend,
  which slants the first and last piece and tilts the arrow). Every box is
  the layout's box: its name sits in the band kept at its top, and the lines
  end on its border.
* **The picture reads from where things start.** ELK, laying out a box,
  breaks the cycles of a graph by reversing the lines that matter least for
  the direction. docair tells it which: the requests of a client (a person or
  program outside that calls in: the browser, the developer) point ahead,
  lines back into a client (the app it is served) give way, and an answer
  that goes back to the part that called (a `data` line, or a read of a store
  the reader also writes) is laid out along the call; its arrow still points
  back. The call may come from anywhere in the box the answer goes to (the
  CLI calls docair-core; docair-core's check hands its diagnostics to the
  CLI's printer). During a walk, the lines it follows to parts it has not
  been to point ahead too. So the Apps layer reads from the browser over the
  server and the CLI to the libraries and the files, and a walk's lines run
  forward, not around the picture.
* **Beside the page, and once.** The layout runs in a worker beside the
  page (ELK and docair's router, read from the page's own scripts), so the
  page keeps answering while a large picture is laid out: on docair's code
  layer (414 parts, 862 lines, 23 s) no frame waits longer than 20 ms, where
  the page stood still for up to 6 s before. A newer picture (another layer,
  a box opened) stops the layout under way. The last six pictures keep their
  layout: going back to one draws it at once (docair's code layer again:
  0.44 s instead of a new layout). A part moved by hand is not kept; the
  picture comes back as it was laid out. The placements a picture is chosen
  from (the two directions, then the ways below) do not depend on each
  other, so two workers lay them out side by side, each taking the next when
  it is free; the page chooses, and one worker draws the winner's lines. The
  same picture: docair's code layer 14.2 s → 12.0 s, the page ready in 0.66 s
  instead of 1.05 s. Where a browser starts no worker, the page lays out
  itself and draws the same picture.
* **Top-down** (after Kasperowski & von Hanxleden). Every box is laid out on
  its own (inside out), then what it holds is scaled to the box's budget of
  800 px, never below 70 % per box: a box with a lot inside does not blow the
  picture up; the reader zooms in to read it. A slice is not scaled
  (docair-702.27): its words keep their size, the picture grows, and a walk
  through it shows each step at 0.8 to 1.3 px per unit, so every word reads at
  8.8 px or more (docair's data.graph went from words of 3.2 to 11 units).
  docair sizes the boxes, as
  elkjs lacks ELK's size approximators and would give every box one size.
  Lines between boxes count, for each box's layout, as lines between the
  parts that hold their ends, laid out the way things happen (as above).
  docair's own router draws the lines: every line takes the cheapest way
  given all the others, from any side of a part; then all lines are routed
  again until none finds a cheaper way. Crossings cost in every picture,
  however many lines it has, and nothing is loaded from the web. The words
  sit on their line where they cover no part, no other words, no box's name
  and no other line, as near the arrow as that allows; where every spot on
  and beside the line covers something, the one that covers least.

  **Where the parts go.** Laid out one by one, a box does not know where its
  lines go outside it, and most crossings come from that. On docair's code
  layer the lines, drawn straight from part to part, cross 2 123 times; in
  one ELK layout across all boxes (as docair placed them before top-down),
  1 259 times. The lines between parts of the code cross about as often in
  both (274 and 214); the lines of the tests cross twice as often among
  themselves (1 395 and 668), because the tests stand in the order of their
  files, not of what they test. So the picture is laid out as it is, in the
  direction that fits the canvas best, and from that placement each box is
  told where its lines go, in four ways:

  | Way | What a box is told |
  |-----|--------------------|
  | a port per line | every line that leaves the box gets a port on the side of the box that faces the line's other end; ELK pulls the line's part to that side |
  | ports in order | one port for all lines of one part that leave by one side; the ports of a side stand in the order of where their lines go |
  | a port per line, parts in order | ports as in the first way, but only for boxes whose parts have lines among them (parts without are packed in rows; ports would string them up in one). The parts of every box are given to ELK in the order of where their lines go: ELK keeps that order where its own crossing count does not say otherwise, and packs in it |
  | ports and parts in order | ports as in the second way, for the same boxes, and the parts in order |

  The placement whose lines cost least wins. Up to 60 lines, each of the five
  is routed (20 ms) and the picture's price decides: the length of its lines,
  60 px for each bend, 400 px for each crossing. With more lines, the lines
  drawn straight from part to part are priced (their length, 400 px a
  crossing); on docair's code layer that ranks the placements as routing them
  does. Each way wins somewhere: Infrastructure takes "a port per line",
  Network and Apps "ports in order", Kubernetes "a port per line, parts in
  order", Code "ports and parts in order", the Overview stays as it is. The
  two ways with a port per line are left out when a box would get more than
  40 ports: on docair's code layer docair-core got 124, ELK took 10 s for that
  box alone, and those ways never won (Code: 33 s → 14 s, the same picture;
  the other layers have at most 24 ports to a box). A way
  is done once: done again from its own result, it makes the picture worse
  more often than better (all boxes move at once, each after where the others
  were). All boxes of one height go to ELK in one call, as the children of a
  frame that lays each out on its own: a call costs 3 ms on top of the layout,
  and a box with ports has to be such a child (elkjs fails on ports of the
  graph it lays out).

  **How the lines are drawn.** Lines run on tracks: beside every
  part at its clearance (8 px, times the scale the part is drawn at), through
  the middle of every line's end, and on a lattice 8 px apart in between. A
  line may leave a part anywhere on any side. Each line takes the cheapest
  way given all the others, counted in px of line:

  | What | Costs |
  |------|-------|
  | a bend | 60 px |
  | crossing a line | 400 px |
  | running on another line, or less than 3 px beside it | 1 200 px for each piece, and 6 px per px: a line never rides on another to save a crossing |
  | running less than 6 px beside another line | 1.5 px per px |
  | ending where another line ends | 800 px |
  | the border of a box that holds neither end | 150 px |
  | along a box's border, across a box's name, right beside a part | 1, 3 and 0.25 px per px |

  A line keeps to the points that lie on a way at most 200 px plus 30 %
  longer than the direct one: no line goes around the picture to save a
  crossing. Then every line is
  routed again, given the others as they now are, at most five times or until
  none finds a cheaper way. Wires on chips are routed like this (McMurchie &
  Ebeling, PathFinder). Counting a crossing takes one look-up, so crossings
  cost in every picture. In a picture with more than 60 000 grid nodes, a
  line is first routed roughly, from cell to cell of 32 px, and then exactly
  within two cells around that way (global and detailed routing): the 227
  lines of docair's code layer take 1.0 s instead of 2.9 s. Dearer crossings
  mean more bends: at 150 px a crossing and 100 px a bend, the four layers
  below have 8, 3, 13 and 903 crossings and 0.7, 0.6, 0.6 and 2.7 bends per
  line.

  Measured on docair's own layers (crossings · lines with a detour of 300 px
  or more · bends per line), against the three ways the viewer offered
  besides top-down until it replaced them: ELK's layout and lines; ELK's,
  with the lines it drew worst routed again by libavoid ("any side"); and
  ELK's layout with libavoid's lines:

  | Layer | Top-down | ELK | ELK, any side | libavoid |
  |-------|----------|-----|---------------|----------|
  | Infrastructure | 5 · 2 · 1.2 | 5 · 1 · 2.6 | 5 · 1 · 2.3 | 6 · 0 · 1.4 |
  | Kubernetes | 3 · 0 · 0.7 | 1 · 1 · 2.0 | 3 · 1 · 1.6 | 2 · 0 · 1.6 |
  | Apps | 9 · 0 · 0.9 | 12 · 2 · 2.6 | 12 · 1 · 2.4 | 11 · 0 · 1.4 |
  | Code | 791 · 32 · 3.4 | 872 · 39 · 4.3 | 1 070 · 17 · 3.2 | 1 272 · 0 · 1.4 |

  On the code layer, top-down's lines run within 6 px of each other for
  3 388 px (ELK 21 332, libavoid 63 304) and on top of each other for 78 px
  (ELK 3 262, libavoid 20 650); on the three smaller layers, 10 px and none.
  A line goes through a box that holds neither end 41 times (ELK 0, libavoid
  37), and 88 pieces of line run within 6 px of a part (libavoid 20): all
  of them in boxes drawn smaller, where the clearance shrinks with the box.
  Top-down takes 0.3 to 0.6 s on the smaller layers and 1.9 s on the code
  layer (ELK took 0.1 and 0.5 s, libavoid 0.1 and 1.1 s): five placements
  instead of one.

  Before, top-down had one placement and libavoid's lines: 9, 4, 9 and 3 089
  crossings on these layers, 1.7 to 1.9 bends per line, and on the code layer
  163 656 px of line within 6 px of another and 75 647 px on top of another,
  in 3.3 s. What was tried on the way: libavoid's crossing penalty on all
  lines of the code layer brings the crossings to 844, but takes 3.5 minutes;
  libavoid box by box (a line enters a box through a pin on its border) is
  fast, but its lines have 6 bends instead of 2; keeping each line within
  240 px of its shortest way, instead of limiting its detour, leaves 2 091
  crossings where the limit leaves 1 640.

  **Compact.** Top-down places the parts more compactly than ELK did: the
  whole picture fits the screen at zoom 0.71–0.93 instead of 0.53–0.67 (Code
  0.19 instead of 0.06), and connected parts are closer (90 % of them within
  484 px on the Apps layer, against 819; on the code layer 1 976 against
  5 563). Most of that comes from laying out each box on its own: one ELK
  layout across all boxes stretches a box over every layer its parts fall
  into. ELK's own options (compaction, wrapping, packing, other node placers)
  change ELK's picture by 6 % at most, and merging lines makes them run on
  top of each other.
* A part **moved by hand** takes its lines along, and only those. While it
  is held, the router routes again the lines of the part, of what is inside
  it and of the boxes around it (they grow or shrink with it), and the lines
  the part now stands on; every other line keeps its way and its words, and
  a line pushed aside comes back once the part has passed (incremental
  routing, as libavoid does it after Wybrow, Marriott & Stuckey). A line
  between two parts moved together moves along with them. The moved part is
  routed as if it had moved by whole tracks (8 px): it then stands to the
  tracks around it as before, so its lines keep their shape instead of
  flickering between near-equal ways (a short jog pays off at some offsets
  from the tracks and not at others), and they are routed only when it has
  moved a whole track; their ends are stretched onto where it stands. A
  moved line keeps the sides it leaves and enters its parts by unless
  another saves 150 px, and the lines are routed in the order they had when
  the move began. Their words wait while the part is held and find their
  place once it is put down, as do the words the part now covers. A click
  moves nothing. On the smaller layers a move is routed in under 15 ms a
  frame, and the lines stand at the drop as they stood a moment before; on
  the code layer, where routing a part with many lines takes up to 150 ms,
  they are routed once while the part is held and to the end once it is put
  down. Starting a line from where it was a moment ago, or making its old way
  cheaper, made the lines jump more, not less: the jog back to the old way
  pays off now and then.
* The details of a group list what it **talks to**: every part outside the
  group that something inside it calls, reads, writes or exchanges data with.
* An edge between a group and one of its own members cannot be drawn as an
  arrow; `docair check` reports it (W108) and the viewer leaves it out.

**Laid out by the CLI** (crate `docair-layout`, docs/SLICES.md §5). `docair
layer <layer> --layout` and `docair slice <seed> --layout` print where each
part goes, each line's points and where its words stand.

* The words are set in one monospace font: a word's width is its number of
  characters times 0.6 em (two for a wide East Asian character).
* Each box is laid out on its own in layers by docair's own placement (no
  ELK): cycles broken by priority, layers that keep lines short, sweeps and
  swaps against crossings, positions that keep lines straight. The boxes are
  sized and placed inside out as above (budget 800 px, floor 0.7; a slice
  without a budget, `layout_input::options_for`).
* A part with many lines is as wide as they need to meet it side by side: a
  track and a half for each pair of lines (across the layers when they run
  right).
* The four ways tell each box where its lines go in the first placement. A
  line that leaves the box before its first layer or after its last becomes
  a node of its own there, with a bend point in each layer between, so the
  box keeps a way open for it and its crossings count; one that leaves by a
  side across the layers pulls its part that way.
* Positions across the layers are the weighted median of a part's
  neighbours (in line with one of them); ties in the sweeps against
  crossings go left first, and right first every other two sweeps.
* The cheapest placement wins: a picture of up to 60 lines is routed in each
  way and in both directions (the other direction only when it fits the
  canvas at least 0.8 times as well), side by side; a bigger one is priced
  by its lines drawn straight.
* The router is the viewer's own, ported: on the same placement it draws the
  same lines at the same price (checked on six pictures of docair's own page
  and on its Code layer, all 1 040 lines).
* Each layout is kept under a key of what it reads (the parts' boxes and
  sizes, the lines' ends, hints and words' sizes, the options) and of the
  layout code itself; ids and names are not in it.

| docair's own graph, 2026-11-18 | Laid out | Taken from the kept layouts |
|--------------------------------|----------|-----------------------------|
| all 1 023 pictures (6 layers, 1 017 slices) | 11.8 s, 13 855 crossings | 0.12 s |
| the 1 017 slices | 1.5 s, 149 ms at most; about 370 crossings in all | |
| the Code layer (511 parts, 1 092 lines) | 10.2 s, 13 440 crossings, 7.3 bends a line | |

**Drawn by the CLI** (docs/SLICES.md §6). `docair layer <layer> --svg` and
`docair slice <seed> --svg` print the picture as an SVG, laid out and drawn
by docair.

* The SVG holds classes, not colours: `assets/pictures.css` gives each class
  the page's colours (`--k-*`, `--e-*`, `--t-*` in viewer.css), so one picture
  serves light and dark. A file of its own (`--svg`) carries them inside.
* Each part is drawn in the shape and colour of its kind, and no two kinds
  look alike (a cluster a heptagon, a function a rounded box, a service a box
  with a cut corner, a datum a pill, …): a test in the colour of its result,
  a closed box with a double outline, an open box with its name in the band at
  its top. A part declared in Terraform (`.tf`), and a box that holds only
  such parts, carries Terraform's mark at its top left.
* Lines run at right angles, rounded where they turn, with an arrow; the
  line style tells the kinds of one family apart. A bundle is wider the more
  lines it stands for; a line to a ref docair could not resolve is red and
  dashed, one that only may reach its part (`could`) dashed.
* A call through a trait or an interface says how many parts it may reach:
  `calls one of 5` (docs/ADAPTERS.md §5.5).
* A line's words take at most two lines of 28 columns, the second cut with
  `…` (docair-702.34; the whole words are in the line's tip and details).
  The layout puts them on or beside their line where they cover no part, no
  other words and no other line, else where they cover least, each other
  line under them costing the same; they stand on an opaque tag with a thin
  edge, so lines under them do not show through. Words that still sit on
  another line are crowded (`data-z`): the page shows them once the zoom makes
  them 9 px or more, and always those of the selected part's lines, a picked
  line and a walk's step (docair-702.35); a file of its own shows every word.
* A line's words stand where the layout put them. The data it carries are
  chips in them, found the way docair finds them (§3.6): a datum's name, the
  last part of its id or a `@carried-as` word, as a whole word in any case.
  Each chip carries its datum's id (`data-data`).
* Every part and line carries its id (`data-id`; a line also `data-from`,
  `data-to` and the ids of the graph's lines it stands for, one per line of
  `data-edges`), for the page's
  script.
* Golden pictures: every layer of each e2e fixture and every slice of the
  shop, in `crates/docair-viewer/tests/golden`; `DOCAIR_BLESS=1` rewrites them.

**The documentation page** (docs/SLICES.md §4). `docair scan` (and `docair
serve`, and the server for an upload) writes every picture a reader can reach
into `docair.html`:

* each layer; a section for each box (the box at its layer, one level down,
  with what its lines join to folded into closed boxes); a slice for each
  seed. A layer or a section that holds tests is drawn twice, with them and
  without them.
* Each picture is laid out (layouts kept in `$DOCAIR_CACHE/layouts`, else in
  the user's cache folder, next to the kept code index), drawn as above and
  packed (raw deflate, then base64). The browser unpacks a picture when it is
  shown, with its own `DecompressionStream`; the page needs no library.
* Every picture has an address: `#layer/<key>`, `#box/<id>`,
  `#slice/<seed>` (a walk's seed is `flow:<name>` or `walkthrough:<id>`).
  `#part/<id>` shows a part where it is when the picture on screen draws it,
  else in its home: its box's section, else a layer that draws it (Apps
  first), else its own slice, and a note says where (`in orders-api · Apps`).
  Back goes to the picture before.
* The script pans (drag) and zooms (wheel; a picture is shown at most 1.25
  times its size; F fits it). A click on a box, closed or open, opens its
  section (what it holds, and only that) with the box's details; so does
  zooming into a box until it fills the canvas; zooming out far past the picture goes to the
  picture above it in the crumbs. A click selects a part (`.sel`, its lines
  `.hl`) and shows its details; the camera stays. A click on a line or its
  words selects the line: its two ends, and the tips of the graph's lines it
  stands for. A click on a datum's chip opens the datum's page. The arrow keys
  step through a walk. When the canvas changes size, as when the chat opens,
  a fitted picture fits again; else it keeps its middle and its scale.
* A part dragged in the page is moved by hand (above): docair's router, built
  to WebAssembly (`docair-layout-wasm`), routes its lines again in the
  browser, from the picture's layout input and layout that the page packs for
  it: while the part is held, each time it has moved a whole track (8 px),
  and once more where it is put down, when their words follow. A move lasts
  until the page reloads.
* The panels are the CLI's, as the viewer wrote them: each part's details
  (Overview, Docs, Tags; what a box talks to, the tests that cover a part,
  its lines in and out, the flows it is in, every other part a link
  `#part/<id>`), the tips of parts and lines (a line's: the first 8 lines of
  the graph it stands for), the tree, the legend, the diagnostics and the
  tests switch. Details, tips and search words are packed like the pictures.
  The legend draws each kind and line as the pictures do; each row names what
  it explains, and the page shows only the rows for what the picture on screen
  holds. What the forms say (dashed, double, thick, …) folds away.
* The glossary and the data are the CLI's too (§13, §3.6): in the panels the
  first mention of each term is marked (`.term`), a code span that names a
  part links to it (`.node-ref`), and the data a line's words name are chips
  (`.data-ref`); each has the popup the CLI wrote. The glossary's list and
  entries, the data's list, each datum's page (its details) and the docs that
  Read more opens are in the page; Follow goes to the datum's slice
  `#slice/data.<id>`, and the bar above the picture leads back.
* A walk is one picture (docs/SLICES.md §13, choice 4): a scenario, a
  walkthrough and a datum's way are each their slice, laid out with their
  lines pointing ahead (a line to a part the walk has not been to yet: 20; one
  back to an earlier part: 0). For each step the CLI writes the parts and lines
  it lights, the box the camera shows (at 0.8 to 1.3 times the picture's
  size), the tokens that run along its lines (a hollow one for an answer) and
  its card: what happens (§7.3), what it has built up, Back, Next and Whole
  step. The script plays them; *▶ Walkthroughs (n)* next to Tests opens a
  panel that lists the walks (docair-7h5), and the details start with them.
  The side panels' widths are dragged at their inner edge, kept per reader;
  a double-click puts one back (docair-jjk).
* The AI is a panel in the page (docs/SLICES.md §8; ✦ Ask). It talks to
  `docair serve`: the page's own address when the server serves it, else
  `<meta name="docair-serve">`; without a server it says so, and everything
  else works. A question goes to `/api/ask` with the conversation and what it
  is about as typed refs: the selected part, and what the reader added with
  ✦ Ask about this in a part's details, a datum's page, a term's entry or a
  line's details (`part:<id>`, `data:<id>`, `term:<term>`, and a line's
  `edge:<id>` for each graph line it stands for).
  The chat shows them as chips above the question; × takes one away. On a
  wide screen the open chat takes its own room beside the details. Word the
  lines and Explain the terms go to
  `/api/word` and `/api/glossary`. The server writes each reply as HTML
  (`page_ai::reply_html`): the answer, a walkthrough to play at
  `#slice/walkthrough:<id>`, words, terms, a view change, edits, and the edits
  waiting to be applied with their diff, Apply and a patch to save. A view
  change hides and shows at once, as the reader's own hiding, with Undo. A reply
  that wrote the documentation again, and Apply, reload the page where it
  was; the conversation is kept for the tab. The page also reloads when the
  documentation is written again from elsewhere (it asks `/api/version` every
  5 s while it is shown).
* Changes (docs/CHANGES.md, `docair build --changes <A>`): the header's
  **Δ Changes · n** opens the list of the flows the working tree changes since
  A (those whose path changed first, then those whose parts changed only in
  their code, then apart those whose words alone changed), and each opens at
  `#changes/<seed>` as one still picture (nothing in it is moved by hand) of
  its slice before and after (docs/CHANGES.md §8): what only the flow after
  has is filled green and tagged *new*, what only the flow before had filled
  red, dashed and tagged *gone*, a part whose code changed light amber and
  tagged *code*, the rest faded, a part that only moved or was renamed once
  under its new name. It opens on its changes only, as a walk with one step
  per change (a stretch of the path where it forks off and rejoins, or the
  parts of one box whose code changed), each step's camera on its change at a
  size to read and its card above the flow's details, which list what it now
  and no longer goes through, the parts changed in their code and those whose
  words alone changed. A flow of more than 30 parts gets one picture per
  change instead (`#changes/<seed>`, `/2`, …, linked by Back and Next): the
  change, where it forks off and rejoins, and the boxes they are in, the rest
  folded into closed boxes; its details link all changes in one picture
  (`/all`) and the whole flow (`/whole`). The list leads with the review
  (docs/EXPLAIN.md §9), when `docair explain` stored one: the AI's changes,
  each one difference in what the app does, at `#review/<n>` on a picture of
  its pieces only, opening on *What and why* under its whole title (the AI's
  why, what it found off by kind: *Said, but the code does otherwise*, each
  *Said* over *The code*, then *Done, but said nowhere* and *Looks wrong or
  risky*, each *The code*, all with the parts where the code does it linked
  and their code folded, or that it found
  nothing, what to check), then *How
  · k of n*, its pieces in the order they work together, each card the
  model's words over docair's; its details hold only what the card does not:
  which change, the flows it touches, who wrote it;
  after the changes the flows, with how many of the system's flows changed
  and all of them folded, each to its picture,
  the model's links. There are no other edits: a piece the AI puts in no
  change is a change of its own after the AI's, in docair's words and marked
  so. Every piece, and every change step of a flow, folds out its code,
  *The code · n parts*: each new, gone and edited part's file and line and its
  whole text before and after, comments with it, as a line diff. Without a review the
  list says the AI writes it, with the command, and served with `docair serve
  --changes <A>` a button that asks for it (`POST /api/explain`) and then says
  how far the job is (`GET /api/explain/<job>`, its `text`, every 1.5 s)
  until the page is written again.
* Tidy (docs/ADAPTERS.md §6.1): the CLI counts the tags the code repeats
  (`docair_core::tidy::repeated`) and writes the header's **✂ Tidy · n**
  (**✂ Tidy ✓** when none), a banner under the header that a reader can
  close until the count changes, the panel that lists them by file and block,
  ticked, with the code that says the same, and a mark on each in the Tags
  tab. Served by `docair serve`, the page shows the patch, and Apply (or
  Tidy this tag) posts the ticked tags to `/api/tidy` and reloads; opened as
  a file, it offers the command `docair tidy --apply`. The Tags tab knows the
  tags `docair check` knows.
* The reader's hiding is the script's: the eye in the tree or "Hide this
  section" leaves a part out, with its lines and their words, and its place
  stays empty; a chip above the picture brings it back. Showing a part inside
  a hidden section brings the section back with only that part in it. The
  search filters the tree and marks the parts it finds (`.hit`, the rest
  `.dim`). What is hidden, and whether tests are shown, is remembered by the
  browser.

| docair's own documentation, 2026-11-18 | |
|----------------------------------------|-|
| addresses | 1 266: 6 layers, 184 sections, 1 076 slices |
| pictures drawn | 1 317, of which 51 twice (tests) |
| the page | 6.3 MB with the panels and walks (pictures 3.8 MB; details, tips, search words, glossary, data, docs and walks 1.5 MB; the tree 0.6 MB), nothing loaded from elsewhere; the viewer's page: 4.7 MB |
| `docair scan .` with the page, layouts and code index kept | 2.4 s (0.67 s with the old page, which laid out in the browser) |
| … with no layout kept | 24.5 s |
| the page ready in the browser; a section shown after its link | about 150 ms (the old page: seconds of layout); 45 ms |

### 3.6 Code and data

The boxes of the chart are **code and the places it runs**: the working
parts. **Data** is what users and that code make and carry: the question you
type, the files you upload, the graph a scan builds, the reply of a model.
docair keeps the two apart.

* **Where code runs.** A web app's files come from a container, but the app
  runs in the browser, and everything it makes is made there. `@runs-in
  browser` draws it in the browser (with all its parts), and keeps *code
  from* as a fact in its details. A walkthrough of a request therefore starts
  in the browser, where the request is made.
* **Data nodes** (`@kind data`, usually fenced blocks in a `DATA.md`) say
  what a datum is. The part that makes it says so: `@creates data.question
  "you type it"`. A line **carries** a datum when its words name it (its
  name, the end of its id, or a `@carried-as` word): docair finds that
  itself. It carries it **there** when what it sends names it
  (`edge.carries`), and **back** when a call's answer names it
  (`edge.carries_back`); a call can do both.
* In the viewer, data is not a box among the parts. **◇ Data** in the header
  lists every datum with who makes it and how many lines carry it. In step
  cards and edge lists, data names are teal chips, apart from the bold names
  of the parts. **Follow** walks the datum's way as a walkthrough: where it is
  made, then every line that carries it, in the order it travels. The lines
  that carry it stay lit while it plays, and the walk's own parts and lines
  never fade. The walk's words mark the datum with ◇ (*ask.digest makes ◇
  digest*), and its first card says what it covers (*Carried by 23 lines,
  all in this walk; 4 of them start stretches of their own, not reached from
  where it is made.*) and lists the lines the layer does not draw.
  * It goes on along a line that carries it there, and back along a call
    that carries it back (*summarize answers ask: brings summary back*). A
    call that carries it both ways is walked there, and later back; never
    back and then there.
  * Where no such line goes on, it travels inside other lines, by the
    shortest way of at most 6 lines: along the arrows (a request through
    the network, *carries it inside …*), back along calls (an answer,
    *brings it back inside …*), or back and then on (an answer its caller
    hands on). Only a part that made the datum, or got it back as an
    answer, hands it back to its callers.
  * A line that carries it but that the walk cannot reach from where it is
    made starts a stretch of its own, from a line no other such line leads
    to, and says *Not reached from where … is made*. No line is left out.
  * Each step is the one line the datum travels, there or back: a data walk
    keeps no call stack, so answers that do not carry it are left out.
* The AI gets both: where each part runs, and which data each line carries,
  and walks a request by following its data from where it is made.

> **Viewer mock: following the question.**
>
> ```text
> ┌─ ◇ Follow question ──────── step 4 of 13 ┐
> │ ConversationService.ask calls             │
> │ AskService.ask                            │
> │ → sends ◇question, ◇graph, history ·      │
> │   gets ◇Reply back                        │
> └───────────────────────────────────────────┘
> ```

### 3.7 Slices: one part of the system

A reader rarely wants the whole system at once. A **slice** is one part of
it: it starts from a **seed**, grows by fixed rules, and ends at parts that
rarely change their names. docair works slices out from the graph alone, the
same way every time; `docair slice <seed>` prints one as JSON
(docs/SLICES.md says where this goes next).

| Seed | Named as | The slice holds | Its ends |
|------|----------|-----------------|----------|
| a datum | its id (`data.question`) | its way as Follow walks it (§3.6): each step one line, there or back, inside other lines where no line names it, and the stretches its way cannot reach | where it is made, and the parts it reaches that pass it on to nobody |
| a route | the id of the part that serves it | the part's own lines, and the way requests come in: back along calls, routes and selections, at most 8 hops | where the requests start, and the parts it reaches |
| a scenario | `flow:<name>` | its steps and their paths (§7) | its first and last step |
| a walkthrough | `walkthrough:<id>` | its steps and their bound lines (§10.3) | its first and last step |
| any other part | its id | the lines across its border, one hop each way | the parts at their other end, tests left out |

`docair slice` without a seed lists the seeds: every datum, route, scenario
and walkthrough, and every part with a line that holds no parts of its own
(a box is shown whole; `docair slice <box>` still works).

* **Lines that could go elsewhere.** A line whose target only could be the
  one drawn (`reach: could`) never carries a datum on inside other lines,
  and never brings a request in. Every line keeps its `reach` in the slice.
* **How docair knows a line carries the datum** (`how`): `typed` when its
  words come from the code (the callee's signature, an API description),
  `named` when an author's words name it.
* **What a datum's slice covers** (`covers`): the lines that carry it, how
  many of them its way reaches before the first stretch of its own, and how
  many such stretches there are. `docair check --coverage` prints it for
  every datum.
* **Boxes.** Every box around a part of the slice, with how many of its own
  parts the slice leaves out (`more`).
* **Stable ends.** An end is `stable` unless it is a function, a component, a
  module inside a program or below a library's crates, a config or a test.
  An end that is not names the nearest part around it that is (`within`):
  `docair-ai.client.chat` is within `docair-ai`. The slice goes no further
  than its end; `within` only names it.

On docair's own graph (2026-11-18), the nine data take the same steps as the
viewer's Follow does. A datum's slice holds 25 parts in the middle (57 for
the graph, the most); a part's slice 4, 90 % of them 11 or fewer, 49 at most.

```json
{
  "schema": "docair.slice/0",
  "seed": { "kind": "data", "id": "data.summary" },
  "ends": [{ "id": "app.summarize", "stable": false, "within": "app" }, { "id": "model", "stable": true }, { "id": "mail", "stable": true }],
  "parts": ["app.summarize", "app.ask", "model", "app.report", "mail"],
  "steps": [
    { "nodes": ["app.summarize"], "made": true },
    { "nodes": ["app.ask"], "lines": [1], "way": "back", "inside": true },
    { "nodes": ["model"], "lines": [2], "way": "there" },
    { "nodes": ["mail"], "lines": [4], "way": "there", "apart": true }
  ],
  "lines": [{ "edge": 1, "from": "app.ask", "to": "app.summarize", "kind": "call" }, "…"],
  "boxes": [{ "id": "app", "more": 0 }],
  "data": ["data.summary"],
  "covers": { "lines": 2, "reached": 1, "apart": 1 }
}
```

(e2e/fixtures/answer: the summary is made, brought back inside the answer
to `app.ask`, sent on to the model; mailing the report is a stretch of its
own.)

**A step's messages** (`docair slice <seed> --messages`): each step of a
slice in words, for its step card. A datum's slice has one message a step,
the line it travels, there or back. A scenario's or walkthrough's slice keeps
one call stack across its steps (§7.3), so an answer comes back after the
work it waits for. Each message has its `kind` (`call`, `return`, `read`,
…), its `edge`, `who` (`["cart", "calls", "orders.create"]`), `why` (the
line's words), `carry` (`sends CreateOrder · gets Order back`), the `token`'s
word, and what the receiving end `has` afterwards. On docair's 17 slices with
steps they were the same as the old viewer's step cards.

---

## 4. Edges (stage 5: resolve)

### 4.1 Data-flow tags

Every data-flow tag names a ref (§4.3) and can say what travels along, with
one rule for all of them:

> `(…)` is what this node **sends**, `-> …` is what it **receives**.

And, right after the ref, `"…"` says **what happens** along the line, in a
few words from this node's side: `@calls docair-core.scan "scans the uploaded
folder" (folder) -> Graph`. The viewer writes it on the line; the payload goes
into the line's tooltip and the details. `docair word` (§10.8) lets the model
write these words.

| Tag          | Value                         | Edge created | What travels |
|--------------|-------------------------------|--------------|--------------|
| `@calls`     | `<ref> [(args)] [-> result]`  | `this → ref` (`call`) | the arguments go with the call, the result comes back |
| `@reads`     | `<ref> [-> data]`             | `store → this` (`read`) | the data read |
| `@writes`    | `<ref> [(data)]`              | `this → store` (`write`) | the data written |
| `@emits`     | `topic:<name> [(event)]`      | `this → topic` (`emit`) | the event published |
| `@consumes`  | `topic:<name> [-> event]`     | `topic → this` (`consume`) | the event received |
| `@creates`   | `<ref> [(how)]`               | `this → ref` (`create`) | how it is created: `helm install …`, `docker run …` |
| `@covers`    | `<ref> [(what)]`              | `this → ref` (`cover`) | what the test checks (§12) |
| `@routes`    | `<ref> [(how)]`               | `this → ref` (`route`) | where a gateway sends requests: `HTTP /api, port 80` (§6.1) |
| `@selects`   | `<ref> [(how)]`               | `this → ref` (`select`) | which workload a Service sends to: `port 8080` (§6.1) |
| `@in`        | `name: Type [from <ref>]`     | `ref → this` (`data`) | `name: Type` |
| `@out`       | `name: Type [to <ref>]`       | `this → ref` (`data`) | `name: Type` |
| `@http`      | `METHOD /path/{param}`        | none; makes this node a target for `http:` refs | — |

A call edge points from caller to callee. Every other edge points to where the
data ends up.

`@creates` is not data flow: it says who brings something into existence
when the system is provisioned (Terraform creates the cluster and installs
the releases), and is drawn as a dashed arrow.

**Annotate the call, not the plumbing.** Most data is handed over by passing it
to a call or by returning it from one. Write that on `@calls`; the walkthrough
(§7.3) then shows every hand-over and every return, in order:

```ts
/***
 * The cart page.
 * @calls http:POST /api/orders (CreateOrder) -> Order
 ***/
```

`@in … from` and `@out … to` are for data that arrives without a modelled
call: a framework callback, a queue, a file drop. Without `from`/`to` they only
document the node's inputs and outputs (details panel). Two edges between the
same nodes in the same direction are merged, and their payloads are joined.

> **Viewer mock: edge styles.** Calls are thin and labelled `(args) → result`.
> Reads and writes always end at a store node, a file or a table (§4.3), so
> reading a file never looks like handing data over.
>
> ```text
>  ▣ cart ──(CreateOrder) → Order──► ƒ create        @calls      what goes in → what comes back
>  ƒ create ──(Order)──────────────► ◯ orders        @writes     into the table orders-db.orders
>  ◯ orders ──Order[]──────────────► ƒ list          @reads      out of that table
>  ƒ create ┈┈(OrderCreated)┈┈┈┈┈┈► ▱ order.created  @emits      async, dotted
>  ▱ order.created ┈┈OrderCreated┈┈► ƒ send-mail     @consumes
>  ▣ form ══address: Address═══════► ƒ checkout      @in … from  data without a call, thick
>  [POST] ƒ create                                   @http       badge, no edge
> ```
>
> Hovering an edge shows the source tag, its file and line, and whether it was
> authored or inferred.

### 4.2 Edge model

```
Edge {
  from: id, to: id,
  kind: data | call | read | write | emit | consume | route | select | create | cover,
  text: string?,            // what happens, the tag's "…": "scans the uploaded folder"
  payload: string?,         // what travels along the edge: "CreateOrder", "Order[]"
  reply: string?,           // calls only: what comes back, "Order"
  id: string,               // unique and stable across scans: "shop-ui.cart -call-> orders.create";
                            // ` "<label>"` when two lines share that, `#2` only for true twins
  evidence: {              // where payload and reply come from when the tag does not say them
    source: "code" | "api", // the callee's signature, or an API description (§6.2)
    file?: string,          // the callee's file, or the description: "api/openapi.yaml"
    line?: number,          // the callee's line
    rule?: "signature" | "described"
  }?,
  via: string?,             // "http:POST /api/orders" when resolved through http
  tag: string,              // the tag that created it, e.g. "@calls http:POST /api/orders (CreateOrder) -> Order"
  source: { file, line },   // where that tag is
  origin: authored | inferred
}
```

`route` and `select` edges are produced only by the k8s adapter (Ingress → Service,
Service → workload).

**Colours and styles.** A line's colour says its family, its style tells the
members apart, and its arrow says the way:

| Family | Kinds | Colour (light / dark) | Style |
|--------|-------|-----------------------|-------|
| requests | `call`; `route`, `select` | neutral grey `#6d6e68` / `#9c9c95` | solid; dashed |
| data handed over | `data` | blue `#2a78d6` / `#3987e5` | solid, thicker |
| storage | `write`, `read`; `create` | amber `#c98500` | solid; dashed |
| events | `emit`, `consume` | magenta `#d55181` | dotted |
| tests | `cover` | light grey `#a3a49c` / `#6b6c66` | dotted |

Three hues are as many as a picture can keep apart in every pair, for every
reader: blue, amber and magenta pass the colour-vision checks in both themes
(no pair closer than ΔE 13 for colour-blind readers, 19 for everyone else),
so any two lines that meet can be told apart. The walk's red-orange and the
data's teal are not line colours: they stay free for highlighting a walk and
a datum's way. Requests, the most lines by far, are neutral; tests, which only
say what is covered, are the quietest.

**Every line has words.** A line without them leaves the reader guessing what
happens. The viewer writes, in this order:

1. the tag's `"…"`: what happens (`scans the uploaded folder`);
2. else the payload and reply: from the tag, or, for a call that names
   neither, from the callee's signature (§6): its parameter names and its
   return type without wrappers, `(root, adapters, opts) → Graph`. Such an
   edge has `evidence.source` `code`, and its tooltip says so. When an API
   description declares the route or the topic, its words come first
   (`(CreateOrder) → Order`); the edge has `evidence` with `source` `api` and the description's `file`, and the tooltip says
   "from api/openapi.yaml" (§6.2);
3. else the verb of its kind: `calls`, `reads`, `writes`, `creates`, `tests`, ….

A bundle of lines between collapsed groups says what its lines carry; a bundle
of tests says how many (`3 tests`).

### 4.3 References

A `<ref>` names something by one of these forms:

| Form                  | Resolves to |
|-----------------------|-------------|
| `orders.create`       | the node with that id |
| `tf:helm_release.docair` | the node of that Terraform resource, by its address (§6.1) |
| `k8s:Service/docair-web` | the node of that Kubernetes object, by its kind and name (§6.1) |
| `http:POST /api/orders` | the node declaring `@http POST /api/orders`: same method, and the path fits the route for certain (`{id}` ≡ `:id` ≡ `<int:id>` ≡ `${id}`; query, scheme and host ignored). This links an Angular `HttpClient` call to a Rust handler. |
| `topic:order.created` | a topic node, created implicitly the first time it is referenced |
| `k8s:deployment/orders-api` | a Kubernetes object produced by the k8s adapter |
| `orders-db.orders`    | a part of a node: an implicit child node `orders-db.orders` (named `orders`, same kind as its parent) inside `orders-db`, so every table or file that is read or written has its own node |

**Requests from code.** A request an adapter reads (`@requests POST /api/scan
-> ScanResult`, inferred) meets what serves the same route after resolve,
the same way. When calls, routes and selections already lead from the client
to the handler (through the host port, Ingress, Service and router), the
request only confirms them. Otherwise it becomes a call line `via
http:POST /api/scan`. Two servers give `W116`, none `W117` (§8).

Resolution order for a bare ref: exact id, then the longest id prefix
(sub-part form). A sub-part of code (a system, service, module, component,
function, job, workload or test) gives `W122`: the code part has no such
part, so the ref is most likely stale. A ref that doesn't resolve fails `docair check` (it is a
warning when run with `--lenient`, see §8). In the graph it becomes a
*dangling edge* to a placeholder node, so nothing is silently dropped.

> **Viewer mock: dangling edge.**
>
> ```text
>  ƒ create ──calls──► ┆? inventory.reserve ┆   ⚠ unresolved ref (src/orders.rs:19)
> ```

---

## 5. Markdown next to code (stage 2 reads, stage 4 merges)

* `src/orders.rs` is documented by `src/orders.md`, and
  `orders.service.ts` by `orders.service.md` (same stem, sibling file).
  The text outside anchored sections attaches to the node the code file stands
  for: the item that declares it (Rust `mod orders;`), else the first node in
  the file that is not inside another node of the same file.
* A directory's `README.md` documents the directory as a group node: the node
  whose id is the directory's name, else the first node the README declares in
  a `docair` block, else a new `module` node named after the directory. The
  README at the root documents the system.
* A heading carrying an id anchor attaches the section below it (up to the next
  heading of the same or higher level) to that node:

  ```md
  ## Creating an order {#orders.create}
  Long-form explanation, diagrams, edge cases ...
  ```

* Nodes that have no code (external systems, managed databases) can be
  declared in a fenced `docair` block inside any Markdown file. The block uses
  the same tag grammar as §2.2 and must carry `@id`:

  ````md
  ```docair
  @id    payments-provider
  @kind  external
  @calls http:POST /api/payments/webhook
  ```
  ````

> **Viewer mock: Docs tab.** The attached Markdown is rendered, including
> Mermaid. Every source file is listed, and each one links back to its file.
>
> ```text
> ┌─ details · orders.create ──────────────┐
> │ [Docs] Tags  Source                    │
> │ ── from src/orders.md#orders.create ── │
> │ Creating an order                      │
> │ Long-form explanation, diagrams, edge  │
> │ cases ...                              │
> └────────────────────────────────────────┘
> ```

---

## 6. Code analysis (stage 3: enrich)

> **Changing.** docair is moving to *code first*: the code says what exists
> and what talks to what, and blocks explain ([ADAPTERS.md](ADAPTERS.md)).
> This section describes what is built today and changes with each step of
> that plan.

Adapters fill in what the code already tells us, so authors don't repeat it.
An adapter has two jobs:

* `enrich`: complete a block from its attached code (symbol, kind, inferred tags).
* `discover`: create nodes from code that has no block (k8s objects,
  undocumented routes). These nodes have `origin: discovered`.

| Adapter    | Infers |
|------------|--------|
| Rust       | every item is a node, with or without a block (`origin: discovered`), except impl blocks (their methods sit inside the type), unit tests (`#[test]`, `#[cfg(test)]`, `tests/`) and items inside function bodies; symbol + kind (`fn`, `struct`, `impl`); the `///` docs as summary and description when no block says them; the signature of a function (parameter names, return type without `Result`/`Option`/`Json`…) for the words on calls (§4.2); rules (`crates/docair-lang-rust/rules/`): actix route attributes and axum `.route("/x", get(handler))` → the routes a handler serves (`@http`), and the axum router → `@calls` to each handler with the route as payload; module path → default `@part-of`; calls between functions: from rust-analyzer's index (`[code]`), where a call through a trait reaches one of its implementations (`reach: one-of`, one line to each), else the calls the syntax makes certain (`self.save()`, `load()` in the same file, `config::load()`, `Self::new()`); data between calls in one function: `let x = f(…); g(&x)` → `@in x: T from f` on g (inferred; docs/ADAPTERS.md §5.6); rules `std.env` (`env::var("X")`, also through a closure or function that passes the name on) → a read of the variable, `std.fs-read`/`std.fs-write` → `@reads file:`/`@writes file:`, a request through `ureq`/`reqwest` (code index) → `@calls net:`; `file:` and `net:` bind to the store or external system the block, or a caller's block, names (docs/ADAPTERS.md §5.7) |
| TypeScript / JavaScript | every file is a module; every class, function, method (also arrow-function fields), constant, interface, type and enum at module or class level is a node, with or without a block, and a documented top-level call is named after what it calls; JSDoc (`/** … */`) as summary and description when no block says them; `@Component`/`@Injectable`/… → why a class is a component; the signature of a function or method from its declaration line (parameter names, return type without `Promise`/`Observable`); `test('…')`, `test.describe('…')` → a test named by its title (§12); rules (`crates/docair-lang-ts/rules/`): Angular `this.http.post<Reply>(url, …)` in a class whose `http` is an HttpClient, and `fetch(url, { method })` → `@requests POST /api/scan -> ScanResult` on the method (inferred; lines to what serves them in §4.3); calls the syntax makes certain: `this.save()` reaches the class's method, `load()` the function of that name in the file; across files: a call on an injected service (`this.scanner.scan()`), an Angular output to the parent's handler its template binds (`(asked)="ask($event)"`), `postMessage({ type })` to the function a `message` listener calls for that type, `bootstrapApplication(App)` to `App` (docs/ADAPTERS.md §5.4); rules `node.env` (`process.env.X`) → a read of the variable, `node.fs-read`/`node.fs-write` → `@reads file:`/`@writes file:` (docs/ADAPTERS.md §5.7) |

### 6.1 Infrastructure: how things are provisioned and routed

Terraform, Helm charts and Kubernetes manifests say exactly what creates what
and where requests go, so docair reads them even when they carry no doc
block. The infra adapter reads the whole project at once, because references
cross files:

| Code | Becomes |
|------|---------|
| `resource "T" "N" { … }` | a node `tf.T.N` (kind `config`) in the Terraform module of its directory (a `job` node) |
| a reference `T.N.attr` in a resource, `depends_on = [T.N]` | `@in attr from tf:T.N`, `@in runs after from tf:T.N`: what has to exist first |
| a `helm` or `kubernetes` provider that points at a cluster resource | `@in cluster access from tf:<cluster>` on every Helm and Kubernetes resource |
| `helm_release` | `@creates k8s.release.<name> (helm install: chart <chart> <version>)`, the release node in `k8s.ns.<namespace>` |
| `kind_cluster`, `google_container_cluster`, `aws_eks_cluster`, `azurerm_kubernetes_cluster`, … | `@creates k8s.cluster.<name>`; namespaces go inside it |
| `kubernetes_namespace` | `@creates k8s.ns.<name>` |
| a `helm_release` whose `chart` is a directory of the project (also through `local.x`) | the chart's objects are inside that release |
| an object in a manifest or chart template | a node `k8s.<deploy/svc/ingress/cm/…>.<name>` with its kind |
| an Ingress rule (`path`, `service.name`, `port.number`) | `@routes k8s:Service/<name> (HTTP <host><path>, port <n>)` |
| an Ingress's `ingressClassName: c` when a release `c` exists | `@in requests (ingress class c) from k8s.release.c`: where requests come in |
| a Service `selector` (also through `{{ include "…" "name" }}`) | `@selects k8s:<Workload>/<name> (port <targetPort>)` for each workload whose labels match |

**Declared wins.** A resource or object that has a doc block keeps it; the
adapter only adds the inferred tags the block does not state (`@creates` only
when the block has none). A node the adapter finds whose id is already
declared yields to the declaration. Everything inferred is marked: nodes
`origin: discovered`, tags `origin: inferred` with the reason.

Until the language adapters exist, the generic adapter (`docair-lang-generic`) reads what a single line tells:
class members and arrow functions in TypeScript (→ `function`), `resource`
blocks and attributes in Terraform (`namespace = "docair"` → *namespace
docair*), and for Kubernetes manifests (plain or Helm templates) the
object's `kind:` and `metadata.name` (*Service docair-web*, kind `endpoint`).
| Kubernetes / Helm | Deployments, StatefulSets, Services, Ingresses, ConfigMaps → `workload`/`config` nodes; selector matching → Service→Pod edges (`select`); Ingress rules → `http:` routing (`route`); a container's `env` (`value`, `configMapKeyRef`, `secretKeyRef`) and `envFrom` → the variables a workload, ConfigMap or Secret sets, which the code's reads of them meet (`@reads k8s.cm.x (NAME)`, docs/ADAPTERS.md §5.7); `{{ .Values.x }}` in a template from the chart's `values.yaml`; Terraform `extra_port_mappings` in `node`, `nodePort` in a release's values and its IngressClass → the way into the cluster: host port → node → ingress controller → Ingress (docs/ADAPTERS.md §7.1) |

If a block and the code disagree, the block wins and `docair check` warns about
the mismatch.

> **Viewer mock: mismatch.** The node shows a warning dot. The Tags tab shows
> both values.
>
> ```text
>   @http   POST /api/orders          (block, used)
>           POST /api/order           ⚠ code says this (src/orders.rs:24)
> ```

### 6.2 API descriptions: OpenAPI and AsyncAPI

An OpenAPI or AsyncAPI file says which routes a service answers and which
topics an application sends to and receives from, with what travels there.
docair reads these files (`docair-lang-apis`) and matches what they declare
with the code; the code still decides what exists. Design: [IMPORTS.md](IMPORTS.md).

**Files.** YAML with a top-level `openapi:`, `swagger:` or `asyncapi:` key;
JSON named `*openapi*.json`, `*swagger*.json` or `*asyncapi*.json`, or listed
in `[import] apis` (§1.1). OpenAPI 3.0, 3.1 and 3.2, Swagger 2.0, AsyncAPI
2.x and 3.x. A file that cannot be read, or a part that is not read (a path
item in another file, 3.2's `query`), is a W113.

| In the description | Becomes |
|--------------------|---------|
| the file | a node of kind `api` (id from its path, `api.openapi`), inside the service it describes: `[[import.api]] part_of`, else `x-docair-part-of`, else the closest service or job declared in its directory or above |
| an OpenAPI operation | a route: base path (the path of `servers[0].url`, or 2.0 `basePath`, or `[[import.api]] base`) + path, with what the request sends (the body's type, else the parameter names) and what comes back (the first 2xx schema) |
| … that a handler of the service serves (same method, same route with parameter names aside, base path optional) | nothing new: the handler gets the operation as a doc, and the inferred tags `described` (`POST /api/orders (CreateOrder) -> Order`) and `operation` (the `operationId`) |
| … that no handler serves, but a function named like its `operationId` | the same, plus the description's route as an inferred `@http`, so W103 shows that the routes differ |
| … that nothing in the service serves | an `endpoint` node inside the description, which `http:` refs resolve to |
| an AsyncAPI channel | a `topic` node `topic:<address>` at the top, with the channel's text: the node `@emits topic:…` resolves to |
| a send or receive (2.x: `subscribe` sends, `publish` receives) | a line between the description and the topic, carrying the message; the same line from code inside the service wins, and the description's line goes |

**Words.** A call that reaches a described route through `http:`, and a line
to or from a described topic, take the description's words when their tag
says nothing of what travels; the edge has `evidence` from the description (§4.2).

**Small pictures.** A description whose every route merged and every line
yielded goes; one documented with a doc block stays. Inside an `external`
part, endpoints nothing calls are dropped, so a vendored description adds
only the operations the code uses.

**Disagreements** are warnings (§8): W114, the description declares what no
code in its service has; W115, code in a described service has what its
descriptions leave out (`[[import.api]] ignore` lists routes left out on
purpose); W103, a route renamed on one side.

---

## 7. Scenarios: walk me through (stage 5 derives, viewer replays)

A **scenario** is a use case told step by step: *what happens when the shopper
clicks "Buy"?* The viewer lists the scenarios first, because they are the
easiest way into a system you don't know. It walks through each one: who calls
whom, what data goes along, what comes back, and when a file, a table or a
topic is touched.

### 7.1 Declaring a scenario

| Tag         | Value    | Meaning |
|-------------|----------|---------|
| `@scenario` | `<name>` | This block describes scenario `<name>`. Its summary is the title, its description says what the reader learns. |
| `@actor`    | text     | Who or what starts it: `shopper`, `developer`, `CI`, `cron`. |
| `@trigger`  | text     | What starts it: `clicks "Buy" on the cart page`, `docair scan .`, `every night at 02:00`. |

A scenario block is a free block, usually a fenced `docair` block in a README.
It describes the scenario, not a node, so it needs no `@id`.

````md
```docair
Check out a cart.

Follow one order from the click on "Buy" to the confirmation mail.

@scenario checkout
@actor    shopper
@trigger  clicks "Buy" on the cart page
```
````

### 7.2 Steps

| Tag     | Value                         | Meaning |
|---------|-------------------------------|---------|
| `@flow` | `<scenario> <step> ["label"]` | This node is step `<step>` of the scenario. The label is the step's title in the walkthrough: one short sentence from the reader's side ("The API stores the order"). |

* Steps are sorted by number. Numbers don't have to be contiguous
  (`10, 20, 30` leaves room to insert steps later).
* Nodes with the **same** step number run in parallel.
* A node may be in several scenarios, and more than once in the same one.
* Implicit nodes such as topics carry no block, so they cannot be steps. A
  scenario passes *through* them.
* A scenario without a `@scenario` block is still listed, under its name.

### 7.3 From steps to messages

Steps say *where* the scenario is. **Messages** say *what happens*. Stage 5
derives them from the edges and keeps a call stack, like a debugger does:

1. The first step starts with a `trigger` message from the actor.
2. For every next step node N:
   * the search starts from the latest earlier step that leads to N along
     the arrows, from its nodes and the nodes its path went through (the
     stack), at most 6 hops; against the arrows at most 3 hops back. A longer
     detour would tell a wrong story, so it is W104 instead;
   * if a path of **calls** leads from a node on the stack to N, every node
     above that one **returns** first (the reply of the call that put it on
     the stack), then the calls run along the path (their arguments), and the
     nodes go on the stack;
   * else, if a path of other edges leads from the previous step to N, each
     edge becomes a message (`emit`, `consume`, `data`, `read`, `write`).
     Anything that is not a call ends the chain: the stack returns, and N
     starts a new one;
   * else the scenario is broken here (W104), and N starts a new stack.
3. Then N's own work: one `read`, `write` or `emit` message for each of N's
   `@reads`, `@writes` and `@emits`.
4. After the last step, everything that was called returns. Nodes started by
   a trigger or an event return to nobody.

**One stack for the whole walk.** The stack is not emptied at the end of a
step. A call's answer goes back only when the walk leaves the callee: the next
message starts from a node lower on the stack or from one not on it, the same
call is named again, or the walk ends. Answers then go back innermost first.
A call whose callee still sends something later in the walk stays open even
when the walk moves on, so a pipeline whose stages hand data to each other
keeps its orchestrator open until its last call (`docair-core.scan` answers
`cmd_scan` after `walkthrough.load`, not after the first stage). When step 2 calls the
server and step 3 is the server's call to the model, the server answers in
step 3, after the model has answered, never in step 2 before the model is even
asked. The viewer derives the messages once per walk, so a projected or
opened step shows the same messages its inner steps have.

This keeps the two things the picture must never mix apart: **reading a file**
is a `read` message from a store node, **handing data over** is the
`(args)` of a `call` or the result of a `return`.

```
Message { kind: trigger | call | return | read | write | emit | consume | data,
          from: id, to: id, payload: string?, edge: index? }
```

Example `checkout`. `cart` has `@calls http:POST /api/orders (CreateOrder) -> Order`:

| Step | Messages |
|------|----------|
| 1 "The shopper clicks Buy" | `trigger` shopper → cart: clicks "Buy" on the cart page |
| 2 "The API stores the order" | `call` cart → orders.create (CreateOrder) via `POST /api/orders` · `write` orders.create → orders-db.orders (Order) · `emit` orders.create → order.created (OrderCreated) |
| 3 "The mailer sends the confirmation" | `return` orders.create → cart: Order · `consume` order.created → mailer.send-confirmation: OrderCreated |

### 7.4 Viewer: walk me through

> **Viewer mock: the scenario list.** This is the first thing the details panel
> shows. Each entry starts a walkthrough.
>
> ```text
> ┌─ walk me through ─────────────────────────┐
> │ ▶ Check out a cart                3 steps │
> │   shopper · clicks "Buy" on the cart page │
> │   Follow one order from the click on      │
> │   "Buy" to the confirmation mail.         │
> │                                           │
> │ ▶ Send the nightly report         4 steps │
> │   cron · every night at 02:00             │
> │                                           │
> │ ▶ Roll out a new orders-api       5 steps │
> │   developer · helm upgrade                │
> └───────────────────────────────────────────┘
> ```

> **Viewer mock: one step.** The card tells the step in words, one line per
> message. The graph fades everything that is not in the scenario and moves a
> token along each message: forward along a call, back along the same edge
> for its return, out of the file node for a read.
>
> ```text
> ┌─ Check out a cart ─────────── step 2 of 3 ┐
> │ The API stores the order                  │
> │                                           │
> │ → cart calls orders.create                │
> │     POST /api/orders (CreateOrder)        │
> │ ✎ orders.create writes orders-db.orders   │
> │     (Order)                               │
> │ ⇢ orders.create emits order.created       │
> │     (OrderCreated)                        │
> │                                           │
> │ ƒ orders.create · src/orders.rs:12        │
> │ Persist a new order and publish           │
> │ `order.created`.                          │
> │                        ◀ Back    Next ▶   │
> └───────────────────────────────────────────┘
> ```

> **Viewer mock: the sequence** (`view: Sequence` in the header). The whole
> scenario over time, top to bottom. Lifelines follow the grouping of the
> graph: a collapsed group is one lifeline, so the first view shows how the big
> parts talk to each other. Expanding a group splits its lifeline. `──▶` is a
> call or I/O, `◀╌╌` a return, `┈┈▶` async. The current step is highlighted,
> and clicking a message jumps to its step.
>
> ```text
>  shopper         shop-ui                           orders-api          orders-db   order.created       mailer
>     │               │                                   │                  │             │                │
>  1  ├─ clicks Buy ─▶│                                   │                  │             │                │
>  2  │               ├─ POST /api/orders (CreateOrder) ─▶│                  │             │                │
>     │               │                                   ├─ write (Order) ─▶│             │                │
>     │               │                                   ├┈┈┈┈┈ emit (OrderCreated) ┈┈┈┈┈▶│                │
>  3  │               │◀╌╌╌╌╌╌╌╌╌╌╌╌╌ Order ╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤                  │             │                │
>     │               │                                   │                  │             ├┈ OrderCreated ▶│
>     │               │                                   │                  │             │                │
> ```

### 7.5 Model

```
Flow {
  name, title?, description?, actor?, trigger?,       // from the @scenario block
  steps: [ { step, nodes: [id], label?, path: [edge-index], messages: [Message] } ]
}
```

---

## 8. Diagnostics (stage 6: check)

| Code   | Severity (default / `--lenient`) | Raised when |
|--------|----------------------------------|-------------|
| `E001` | error / warning | a ref doesn't resolve (§4.3) |
| `E002` | error / error   | duplicate `@id` |
| `E003` | error / error   | `@part-of` cycle |
| `E004` | error / error   | free block without `@id` (except a `@scenario` block), or malformed id |
| `E005` | error / error   | doc block is never closed |
| `E006` | error / error   | malformed tag value (`@http` without a path, `@flow` without a step number) |
| `W101` | warning         | unknown tag |
| `W102` | warning         | unknown kind |
| `W103` | warning         | block and code disagree (§6) |
| `W104` | warning         | broken scenario: two consecutive steps are not connected (§7.3) |
| `W105` | warning         | Markdown anchor `{#id}` points to no node, or a sibling `.md` has no node to attach to |
| `W106` | warning         | text after a blank line that follows the tags (ignored) |
| `W107` | warning         | a `@scenario` block names a scenario that has no `@flow` steps |
| `W108` | warning         | an edge between a group and one of its own members (it cannot be drawn as an arrow; containment already says it) |
| `W109` | warning         | a recorded walkthrough names a node or an edge that is not in the graph (the ref is left out; a step with nothing left is dropped) |
| `W110` | warning         | a file in `walkthroughs/` is not a walkthrough: invalid JSON or another `schema`, or the folder is no folder inside the root |
| `W111` | warning         | a test result names a test that has no `test` node, or a results file cannot be read or is no file inside the root (§12.3) |
| `W112` | warning         | a glossary term is defined twice, has no definition, or `See:` names no node (§13) |
| `W113` | warning         | a file cannot be read, such as an API description that is not valid YAML or JSON, a part of a description is not read (§6.2), or a source file tree-sitter can't parse, or a `[code] scip` path is no file inside the root |
| `W114` | warning         | an API description declares a route, or a send or receive, that no code in its service has (§6.2) |
| `W115` | warning         | code in a described service has a route, or a send or receive, that its descriptions leave out, and `[[import.api]] ignore` does not list it (§6.2) |
| `W116` | warning         | two parts serve a request the code sends: both get a line (§4.3) |
| `W117` | warning         | nothing in the project serves a request the code sends, such as a route renamed on one side (§4.3); a request to another origin (`https://…`) is left alone |
| `W118` | warning         | nothing in the project requests a route; off unless `[checks] unrequested_routes = true`, because public APIs and probes are requested from outside |
| `W119` | warning         | `[code] run_indexers` ran rust-analyzer or scip-typescript and it failed: the calls come from the syntax only (§6) |
| `W120` | warning         | a word names a part (a module, a component, a data) and at least two texts use it, but nothing settles what it means: no summary that defines it, no decision (§13.5); only in a project with a glossary or a decisions file |
| `W121` | warning         | a word's settling is out of date: a decision's `from` names no node, or the glossary entry of a word the code defines is not checked against the code's current text (§13.5) |
| `W122` | warning         | a ref names a part of a code part by the sub-part form (`docair-core.infra.discover` when there is no such node), so it binds to the whole of `docair-core`: a stale ref after a move or rename; sub-parts are for stores, topics and other parts that hold named things (§4.3) |

Each diagnostic has `{ code, severity, message, file, line, node? }`.
`docair check` exits non-zero if any error remains.

> **Viewer mock: diagnostics drawer** (opened from `⚠ 2` in the header). Each
> entry selects its node and links to the source.
>
> ```text
> ┌─ diagnostics ──────────────────────────────────────────────┐
> │ ✖ E001 unresolved ref `inventory.reserve`  src/orders.rs:19 │
> │ ⚠ W101 unknown tag `@owner`                 src/orders.rs:15 │
> └────────────────────────────────────────────────────────────┘
> ```

---

## 9. Output: `docair.graph.json` (stage 7: emit)

The only contract between the backend and the viewer (and any other tool).
Versioned. A change that breaks this format bumps `schema`.

```json
{
  "schema": "docair.graph/0",
  "system": "shop",
  "generator": "docair 0.1.0",
  "nodes": [
    { "id": "orders.create", "kind": "function", "name": "create_order",
      "summary": "Persist a new order and publish `order.created`.",
      "part_of": "orders-api",
      "source": { "file": "apps/orders-api/src/orders.rs", "line": 12 },
      "http": { "method": "POST", "path": "/api/orders" },
      "docs": [], "tags": [], "origin": "authored" }
  ],
  "edges": [
    { "from": "shop-ui.cart", "to": "orders.create", "kind": "call",
      "payload": "CreateOrder", "reply": "Order", "via": "http:POST /api/orders",
      "tag": "@calls http:POST /api/orders (CreateOrder) -> Order",
      "source": { "file": "apps/shop-ui/src/app/cart.component.ts", "line": 8 },
      "origin": "authored" }
  ],
  "flows": [
    { "name": "checkout", "title": "Check out a cart",
      "description": "Follow one order from the click on \"Buy\" to the confirmation mail.",
      "actor": "shopper", "trigger": "clicks \"Buy\" on the cart page",
      "steps": [
        { "step": 2, "nodes": ["orders.create"], "label": "The API stores the order", "path": [0],
          "messages": [
            { "kind": "call",  "from": "shop-ui.cart",  "to": "orders.create",    "payload": "CreateOrder", "edge": 0 },
            { "kind": "write", "from": "orders.create", "to": "orders-db.orders", "payload": "Order",       "edge": 1 }
          ] }
      ] }
  ],
  "walkthroughs": [],
  "view": { "hide": ["terraform"], "tests": true },
  "diagnostics": []
}
```

`walkthroughs` holds the recorded walkthroughs, bound to this graph (§10.3).
`view` is the default of what is shown (§11). A test node carries its last
result in `test` (§12). `glossary` holds every explained word, from the
glossary, the code or a doc heading, `library` the docs they link to, and
`meanings` the decisions about words and what is still to settle (§13).

The viewer reads only this file. It never parses source code, so any
backend change that keeps this schema needs no viewer change.

`docair.html` embeds the same JSON in a `<script type="application/json">`
element, plus the Markdown pre-rendered to HTML (with raw HTML escaped), the
viewer script and its libraries. It is one self-contained file and makes no
network requests. Libraries and versions: [LIBRARIES.md](LIBRARIES.md).

---

## 10. Ask: a conversation with an AI model

Scenarios (§7) are written by hand, next to the code. Many questions have no
scenario yet: *how does Terraform bring the cluster up? what happens to an
upload?* `docair ask` sends the question and the graph to an AI model. The
model answers with a **reply** (§10.7). Its main part is usually a
**walkthrough**: steps in the order things happen, each one naming the nodes
and edges it is about, and saying in words what happens and why ("Terraform
creates the kind cluster first, because every other resource needs its
kubeconfig"). docair checks the answer against the graph, **records** it as a
JSON file, and the viewer **replays** it on the picture like a scenario.

Asking is a **conversation**: a follow-up ("and what happens if the upload is
too big?") is asked with the earlier turns, so the model knows what *it*
refers to. A reply can also **change the view** ("I don't care about
Terraform" hides it, §11) and **propose edits** to the doc blocks when the
picture is wrong or incomplete ("the server also reads the Secret", "document
the undocumented functions in docair-core"). Edits are shown as a diff and
written into the code only when the user applies them (§10.6).

```
question ─► docair ask ─► scan (1–6) ─► digest ─► AI endpoint ─► JSON ─► bind (§10.3) ─► walkthroughs/<id>.json
                                                    ▲               │ invalid?              │
                                                    └── one repair ◄┘                       ▼
                                                                    every later scan: graph.walkthroughs ─► viewer replays
```

### 10.1 Configuration

The model is reached over the **OpenAI-compatible chat-completions API**
(`POST {endpoint}/chat/completions`, bearer key). Together, OpenAI, a local
vLLM or Ollama, and others speak it.

| Setting | `docair.toml` `[ai]` | Environment (wins) | Example |
|---------|----------------------|--------------------|---------|
| endpoint: the API base URL | `endpoint` | `DOCAIR_AI_ENDPOINT` | `https://api.together.xyz/v1` |
| model id, as the endpoint names it | `model` | `DOCAIR_AI_MODEL` | `zai-org/GLM-5.3-Flash` |
| key | never | the variable named by `api_key_env` (default `DOCAIR_AI_API_KEY`) | `tgp_v1_…` |
| max tokens of the answer (reasoning included) | `max_tokens` | `DOCAIR_AI_MAX_TOKENS` | `32000` (default) |

* The key is never read from `docair.toml`, because that file is committed.
  docair looks it up in the process environment, then in a `.env` file at the
  project root (`KEY=value` lines, gitignored; `.env.example` shows the
  format). The key never appears in output files, logs or error messages:
  `docair ai` only says *where* it was found.
* Without an endpoint and model, AI is off: `docair ask` says so and exits 2,
  and the web app hides the Ask box. Nothing else changes.
* In the cluster, Terraform passes the three settings to the Helm release
  (`ai_endpoint`, `ai_model`, and `ai_api_key`, which defaults to
  `DOCAIR_AI_API_KEY` from the repository's `.env`): endpoint and model go into
  the ConfigMap, the key into the Secret `docair-ai`, and the server reads
  them from its environment. Terraform's state holds the key too, so it is
  gitignored like `.env`.
* The web app never uploads `.env`, `.env.*`, Terraform state or `.tfvars`
  files, so a key in the project does not travel with an upload.

### 10.2 Asking

1. **Digest.** docair runs stages 1–6 and sends the graph without the source
   code: for each node its id, kind, name, parent, summary, description
   (shortened), file and line, HTTP route; for each edge its ends, kind,
   label and payload; for each scenario its steps. Source code never leaves
   the machine; summaries, descriptions and file paths do.
2. **Prompt.** The system prompt fixes the rules: use only ids from the
   graph; walk in the order things happen and say why that order; name the
   concrete things (resources, files, ports, payloads); 3 to 12 steps; answer
   in the language of the question; say so when the graph cannot answer. The
   reply format is the walkthrough JSON of §10.3, without `recorded`. The
   request asks for JSON output (`response_format: json_object`).
3. **Bind.** The answer is parsed and bound to the graph (§10.3). If it is
   not valid JSON, or names node ids that do not exist, docair sends **one**
   repair request with the problems listed. What is still wrong after that is
   left out, with a warning.
4. **Record.** The walkthrough is written to `walkthroughs/<id>.json`; the id
   is the slug of the question, so asking the same question again replaces
   the file. `recorded` notes when, which model and which endpoint.

```text
$ docair ask "How does terraform provision the cluster?"
Provisioning the kind cluster                       zai-org/GLM-5.3-Flash · 18 s
  Terraform builds a one-node kind cluster, loads the images and installs two charts.

1 Terraform creates the kind cluster first
  Every other resource needs the cluster's kubeconfig, so tf.kind_cluster runs
  first: it starts the node container kindest/node and publishes port 30080.
  · tf.kind_cluster → kind
2 …
recorded  docs/walkthroughs/how-does-terraform-provision-the-cluster.json
page      .docair/docair.html
```

### 10.3 The walkthrough file

```json
{
  "schema": "docair.walkthrough/0",
  "id": "how-does-terraform-provision-the-cluster",
  "question": "How does terraform provision the cluster?",
  "title": "Provisioning the kind cluster",
  "summary": "Terraform builds a one-node kind cluster, loads the images and installs two charts.",
  "recorded": { "at": "2026-11-14T23:10:00Z", "by": "ai",
                "model": "zai-org/GLM-5.3-Flash", "endpoint": "https://api.together.xyz/v1" },
  "steps": [
    { "title": "Terraform creates the kind cluster first",
      "text": "Every other resource needs the cluster's kubeconfig, so …",
      "nodes": ["tf.kind_cluster", "kind"],
      "edges": [{ "from": "tf.kind_cluster", "to": "kind" }] }
  ]
}
```

| Field | Meaning |
|-------|---------|
| `title`, `summary` | the answer in one line and in one or two sentences |
| `steps[].title` | one sentence from the reader's side, like a `@flow` label |
| `steps[].text` | what happens in this step and why: order, dependencies, what is passed, created, read or written |
| `steps[].nodes` | the nodes the step is about, most important first; the camera goes to them |
| `steps[].edges` | the connections that carry the step, by their ends |
| `recorded.by` | `ai`, or `human` for a walkthrough written or corrected by hand |

The files are plain JSON on purpose: they are reviewed in pull requests,
corrected by hand, and replayed by every later page. **Binding** happens on
every scan, against the current graph:

* a node id that is not in the graph is left out (W109);
* an edge ref binds to every edge `from → to`, else to every edge `to → from`;
  without one it is left out (W109). Bound edges are written into the
  graph's copy of the step as `path` (edge indices, like a flow step's path).
  The file keeps the refs, never indices;
* a step with no node and no edge left is dropped.

So a walkthrough that has gone stale after a refactoring shows up in
`docair check`, like a broken scenario does.

### 10.4 Walks in the page

The header's *▶ Walkthroughs (n)*, next to Tests, opens a panel that lists
the scenarios and walkthroughs, and the details panel starts with them
(*Walk me through*); the walk on screen is marked in both (docair-7h5). A walk is one picture: its slice (§3.7), laid
out with its lines pointing ahead. A scenario, a walkthrough and a datum's
way (§3.6) play the same:

* each step lights its parts and the lines it goes along; the steps before it
  stay marked, everything else fades;
* the step's **messages** play one by one (§7.3): before each one moves, its
  line in the step card lights up (**What happens**: "→ ask calls chat: … ·
  sends question + digest · gets reply JSON back"), and a token runs along
  the line's own path with the words of what travels. A call's answer goes
  back along the same line as a hollow token ("↩ chat answers ask with reply
  JSON") once the calls it made in turn have answered, even when those come in
  later steps (one stack for the walk). **Now it has** lists what the step
  built up: the answers received, the data read, written or created. Clicking
  a line in the card shows that hop in the picture;
* the camera goes to the step's box, which the CLI worked out from the layout:
  the step's parts and lines with some room, never smaller than readable
  (zoom 0.8) and never closer than 1.3. **Whole step** fits the box without
  that limit;
* the details panel shows the **step card** on top: the walk's title,
  *asked:* for a walkthrough, *step k of n*, the step in words, its parts as
  links, What happens, **◀ Back** and **Next ▶** (the arrow keys too).
  Clicking a part shows its details under the card;
* a datum's walk starts with what carries it: *Carried by 2 lines, all in this
  walk*, or how many of them start a stretch of their own;
* closing the walk (×) goes back to the picture before.

The viewer played a walk on a layer, opened steps into their boxes and let the
reader pick the layer; a walk is its own picture now, so those are gone.

`✦` marks a walkthrough written by a model, so it is never mistaken for an
authored scenario.

### 10.5 Asking from the page: `docair serve`

The AI is asked through the CLI: `docair ask` in a terminal, or `docair
serve` from the page (docs/SLICES.md §8, `crates/docair-server/openapi.yaml`).
The server works on its own graph of the project, so the page sends only its
question; the key stays with the server.

| Route | Request → Answer |
|-------|------------------|
| `GET /api/ai` | → `{ enabled, model }`. Never the key, never the endpoint |
| `POST /api/ask` | `{ question, history?, context? }` → `{ reply, html, regenerated, version, patch, pending }`; `context` is what the question points at, one part's id or typed refs (`part:<id>`, `data:<id>`, `term:<word>`, `line:<k>@<address>`, `edges:<i>,<j>`), each given to the model in words, and one that names nothing named in `problems`: the reply (§10.7), written as HTML for the page too; a walkthrough in it is recorded and the documentation written again (`regenerated`); its edits wait (`patch`, `pending`). 400 for an empty question or one over 500 characters, 503 when AI is off, 429 when two questions are already with the model, 502 when the endpoint fails |
| `POST /api/word` | `{ scope?, limit?, all? }` → the same, with `wording` (§10.8) |
| `POST /api/glossary` | `{ limit? }` → the same, with the terms explained (§13.4) |
| `POST /api/apply` | → `{ edited, regenerated, version }`: the waiting edits written into the project's files, the view change into `docair.toml`, the documentation written again; 409 when nothing waits |
| `GET /api/tidy` | → `{ tags, patch }`: the tags the code repeats (`node`, `tag`, `line`, `code_says`, `code_file`, `code_line`, `reason`) and the diff that removes them all |
| `POST /api/tidy` | `{ tags?: [{ node, tag }] }` → `{ edited, regenerated, version }`: the picked tags removed (all when none are named), the documentation written again; 409 when no tag repeats |
| `POST /api/explain` | `{ a, b? }` → 202 and an `ExplainJob` at once (docs/EXPLAIN.md §8): the job explains what the served project changes since commit `a` (or between `a` and `b`), the flows without stored findings sent to the model at most 3 requests at once and stored as `docair explain` stores them; a page served with `--changes` is written again with them. A second call while a job runs answers that job (the same commits) or 409 (others); 409 for a project without git history (an upload), 503 when AI is off |
| `GET /api/explain/<job>` | `ExplainJob`: `job`, `a`, `b`, `running`, `phase` (`comparing`, `asking`, `done`, `failed`), `flows`, `requests`, `started`, `answered`, `left`, `asking` (the flows the model works on now), `seconds`, `error`, `text` (what the page shows, e.g. *The AI explains 30 flows in 5 requests · 2 answered · 1:12*), and at the end `findings` and the documentation's new `version`; kept 10 minutes after it ends, then 404 |

An uploaded page asks the same routes under `/api/projects/<id>/` (§1.3):
the server keeps the upload as a project, and answers any origin, since the
web app's frame is sandboxed. A page with no server to ask (a downloaded
file opened from disk) works fully, minus the AI, and says so where the AI
would be.

### 10.6 Feeding edits back to the code

An **edit** changes one doc block. It names its node, and docair resolves
*where* the edit goes from the graph:

| The node … | Mode | The edit … |
|------------|------|------------|
| has an authored block | `edit` | adds tag lines after the block's last tag (aligned like its neighbours), removes tag lines, or replaces the summary |
| was discovered or inferred from code, without a block | `insert` | writes a new block above the node's code line, in the file's comment syntax |
| does not exist yet (needs `@id` and `@kind` in `add`) | `append` | appends a fenced `docair` block to a Markdown file (`file`, default `docs/docair.md`) |

```json
{ "node": "docair-server.config", "mode": "edit",
  "file": "crates/docair-server/src/lib.rs", "line": 41,
  "add": ["@reads k8s.secret.docair-ai -> DOCAIR_AI_API_KEY"], "remove": [],
  "why": "The server reads the key from the Secret docair-ai." }
```

Only known tags (§3–§4) are accepted; an edit to a node that does not exist,
without `@kind`, is dropped. Every edit is checked twice. When the reply
arrives, a ref that names no node goes back to the model in the repair
request (§10.2). When the edit is applied, the new file must still parse (§2)
and every ref it adds must resolve; an edit that fails either is left out
and reported, never written.

* **CLI.** `docair ask` writes the edits as a unified diff to
  `.docair/docair.patch` (paths relative to the root, so `git apply` takes
  it). `--apply` or `docair apply` writes them into the files; then the next
  scan shows the change.
* **The page** (with `docair serve`). Each proposed edit is listed under the
  reply with its reason. **Review diff** shows the waiting edits as the
  server wrote them; **Apply & regenerate** writes them into the project's
  files (`POST /api/apply`), the documentation is written again and the page
  reloads where it was; **⤓ Patch** saves `docair.patch`.

### 10.7 The reply

```json
{
  "answer": "One to three sentences that answer the question directly.",
  "walkthrough": { "title": "…", "summary": "…", "steps": [ … ] },
  "view": { "hide": ["terraform"], "show": [] },
  "changes": [ { "node": "…", "add": ["@calls …"], "remove": [], "why": "…" } ]
}
```

Every part but `answer` is optional. The model is told when to use which:
a *how does … work / what happens when …* question gets a walkthrough; *I
don't care about … / show me only …* gets a view change; *this is wrong /
… is missing / document …* gets edits. The request carries the last turns of
the conversation (question and answer, at most 8) and, in the web app, the
selected node as context.

> **Page mock: the conversation, ✦ Ask in the page.** A panel at the right;
> the page stays usable while the model writes.
>
> ```text
> ┌─ ✦ Ask docair ─────────────── fake-model × ┐
> │                     How does provisioning   │
> │                                    work? ◀──┤ you
> │ One terraform apply creates the cluster,    │
> │ loads the images and installs two charts.   │
> │ ✦ Provisioning   [▶ Play · 10 steps]        │
> │                                             │
> │                  I don't care about terraform│
> │ Hide terraform: written into docair.toml    │
> │ when applied.                               │
> │       The server also reads the AI Secret   │
> │ Right, the graph misses that edge.          │
> │   ✎ docair-server.config  lib.rs:41         │
> │     + @reads k8s.secret.docair-ai           │
> │   2 edits wait to be applied                │
> │   [Review diff] [Apply & regenerate] [⤓]    │
> │─────────────────────────────────────────────│
> │ [✦ Word the lines] [✦ Explain the terms]    │
> │ [Ask, or tell docair what to change…  ] [➤] │
> └─────────────────────────────────────────────┘
> ```

A walkthrough plays at `#slice/walkthrough:<id>` once the documentation is
written again; a view change and edits wait until they are applied.

### 10.8 Words for the lines

The code says what a call passes and returns (§4.2), but not *why*. The
model can: it reads both ends of each line that says nothing yet (no `"…"`,
no authored payload), with their kinds, names and summaries, and writes 2 to
7 words from the source's side, starting with a verb: "scans the uploaded
folder", "reads the upload limits", "checks that a folder becomes a page".

* Calls come first, tests last; 40 lines go into one request; `--scope` (or
  the selected part in the page) limits it to lines that touch one part.
  `--all` also words lines that show only a type (`Order`): the phrase goes
  on the line, the type stays in the tooltip and the details.
* The words are listed in the reply; the pictures show them once they are
  applied, as the page lays out nothing itself.
* Each line's words become an edit of the tag that made the line: the
  `"…"` goes in right after the ref, on the same line (§10.6), so a review
  sees one changed line per word. Applying them makes them part of the docs.

```diff
- * @calls   docair-core.walkthrough.bind
+ * @calls   docair-core.walkthrough.bind "checks the reply against the graph"
```

---

## 11. Sections: what to leave out

Not everyone cares about everything: a backend developer may not want the
Terraform resources, a reviewer of the cluster may not want the Rust
functions. Any node can be **hidden**: it disappears with everything inside
it, and so do the edges that touch it. Hiding is a view setting, never a
change to the graph: `docair check` and the JSON are unchanged.

* `docair.toml` `[view] hide = [ids]` leaves those parts out of the pictures
  the CLI draws; `[view] show = [ids]` brings a part inside a hidden section
  back, as below.
* In the page, each row of the tree has an eye: click to hide or show that
  section. The page hides it where it is: its place in the picture stays
  empty, with its lines and their words, and nothing is laid out again. Hidden
  sections are listed above the picture as chips; a chip shows its section
  again, **Show all** shows everything.
* **A part inside a hidden section can be shown.** Its eye in the tree (or
  *Show this section* in its details) brings the hidden section back with
  only that part in it: from the hidden section down to the part, each group
  is shown, and its other children are hidden instead. Showing a sub-folder
  is the same as hiding its siblings, and that is what the chips then say.
* The node details have **Hide this section**, or **Show this section** when
  it is hidden.
* The choice is remembered per system in the browser.
* A reply of the model can propose to hide and show sections (§10.7); applied,
  they go into `[view]` in `docair.toml`.

> **Viewer mock: `infra` hidden, then its `cluster` shown again.** The
> Terraform resource beside the cluster is hidden instead.
>
> ```text
> ┌─ tree ─────────────┐ ┌─ graph ──────────────────────────────────────┐
> │ ▾ infra         👁 │ │ hidden: [kind_cluster.shop ×]                 │
> │   ▸ cluster     👁 │ │   ╭ infra ───────────────────╮                │
> │     kind_cluster ⊘ │ │   │ ╭ cluster ─────────────╮ │                │
> │                    │ │   │ │ orders-api  orders-db │ │                │
> └────────────────────┘ └───╰─╰──────────────────────╯─╯────────────────┘
> ```

> **Viewer mock: hidden sections.**
>
> ```text
> ┌─ tree ──────────┐ ┌─ graph ────────────────────────────────────────┐
> │ ▾ docair     👁 │ │ hidden: [terraform ×] [docair-lib ×]  Show all │
> │  ▸ browser   👁 │ │                                                │
> │  ▸ docair-lib ⊘ │ │   ╭ browser ╮        ╭ workstation ─────────╮  │
> │  ▾ workstation👁│ │   │ app     │──────► │ docker  ▸ kind …     │  │
> │   ▸ terraform ⊘ │ │   ╰─────────╯        ╰──────────────────────╯  │
> └─────────────────┘ └────────────────────────────────────────────────┘
> ```

---

## 12. Tests: what is covered, and did it pass

Tests are part of how a system works: they say which journeys are known to
work. docair shows them as nodes, linked to what they cover, with the result
of their last run. They can be hidden like any section.

### 12.1 Declaring a test

| Tag | Value | Meaning |
|-----|-------|---------|
| `@kind test` | | the block describes a test case, or a suite when other tests are `@part-of` it |
| `@covers` | `<ref> ["what"]` | this test exercises `<ref>`; one edge of kind `cover` per tag |

```ts
/***
 * Generate the docs: pick a folder, the page appears.
 *
 * @kind    test
 * @part-of e2e.web
 * @covers  docair-web.upload-panel
 * @covers  docair-server.scan_upload
 ***/
test('a picked folder becomes a page', async ({ page }) => { … });
```

For `test('…')`, `it('…')` and `describe('…')` lines, the adapter takes the
name from the string, so the node is named like the test in the report.

### 12.2 At which layer a test shows

A test is shown where it is placed (`@part-of`, usually its suite) and its
`cover` edges lead to what it covers. A suite is part of the Apps layer: one
box whose bundled edges say which programs its tests cover ("e2e · 9 tests →
docair-web"). At the Code layer, or opened, each test leads to the component
or function it exercises. At the Infrastructure, Network and Kubernetes layers
a suite shows as context where its tests cover parts of the layer. A unit test
placed in its module shows only when that module is open. So an end-to-end
test is visible from far away, a unit test only close up. This is the "level
intended".

The details of every node list **Tested by**: the tests that cover it or
anything inside it, with their results.

### 12.3 Results

`[tests] results` names test reports to read on every scan. docair reads the
Playwright JSON report (`--reporter=json`). A result binds to the test node
in the same file whose name equals the test's title; one without a node is
W111. A bound test node carries:

```json
"test": { "status": "passed", "duration_ms": 1830, "at": "2026-11-15T08:00:00Z" }
```

`status` is `passed`, `failed`, `flaky` or `skipped`. The viewer draws the
test with its result (✓ green, ✗ red, ~ flaky, – skipped or no result), and
the header shows the totals.

### 12.4 Showing and hiding tests

The header's **Tests** button shows or hides every test node and `cover`
edge (`[view] tests` is the default). With tests hidden, the picture is the
system alone; with tests shown, coverage is part of it.

> **Viewer mock: tests shown, at the Apps layer.**
>
> ```text
> ┌─ … ── Tests ✓ 9 ✗ 1 ─┐
> │  ╭ e2e ─────────────────────────╮        ╭ docair-web ───────────╮
> │  │ ✓ a picked folder becomes …  │┈┈┈┈┈┈┈►│ ▣ UploadPanel          │
> │  │ ✓ hiding a section …         │┈┈┈┈┐   ╰───────────────────────╯
> │  │ ✗ a follow-up keeps context  │┈┈┐ └┈┈►╭ docair-viewer.page ───╮
> │  ╰──────────────────────────────╯  └┈┈┈┈►│ ƒ sections  ƒ walk     │
> │                                          ╰───────────────────────╯
> ```

---

## 13. Glossary: what the words mean

Descriptions use words a newcomer may not know: *diagnostics*, *digest*,
*Ingress*, *NodePort*. The glossary says what they mean, and the page
explains them where they are used.

The glossary is not the only place that says what a word means: the code
does too, and the code comes first (§13.5, [MEANINGS.md](MEANINGS.md)).

### 13.1 The glossary file

Any file named `GLOSSARY.md` (and the files listed in `glossary = [...]` in
docair.toml) is a glossary. It defines words, not nodes. Each `##` (or `###`)
heading is a term:

```md
## Diagnostics

Also: diagnostic
See: docair-core.check.check

What `docair check` reports about the docs: errors such as a ref that names
nothing, and warnings such as a broken scenario.

Every code is listed in [SPEC §8](SPEC.md#8-diagnostics-stage-6-check).
```

| Line | Meaning |
|------|---------|
| `## <term> [{#id}]` | the term; its id is the slug of the term unless given |
| `Also: a, b` | other spellings: plurals are found anyway, so these are synonyms, codes, abbreviations |
| `See: <id>, <id>` | the nodes where the term is implemented or used |
| the rest | the definition; its **first paragraph** is the short version a hover shows |

A term with a capital letter (`Ingress`, `Secret`) is found only as written;
others in any case. W112 when a term is defined twice, has no definition, or
`See:` names no node.

### 13.2 In the viewer

* The first mention of each term in the details panel, a step card, the
  glossary itself and an opened doc is underlined. **Hovering** shows the
  short definition, the nodes to see, and *Read more*; **clicking** opens the
  whole entry in the details panel, with the parts whose texts use the term.
* A code span that names a node (`docair-core.scan`, `ScanService`) links to
  that node; hovering shows its summary.
* **Glossary** in the header lists every term, with a filter. A word the
  code defines is marked `ƒ`, one a doc heading defines `§`.
* The hover and the whole entry say where the meaning comes from: *from
  `Entry`* (the part, a link to it), *in DESIGN.md §2*, or *in the glossary*.
  A glossary entry not checked against the code that defines its word shows
  the code's words first and its own below, under *In the glossary, not
  checked against the code*.

> **Viewer mock: hovering a term in the details.**
>
> ```text
> │ ƒ check                                      │
> │ Graph-wide rules and the severity policy:     │
> │ turns problems into ┈diagnostics┈.            │
> │                   ┌───────────────────────────────────────┐
> │                   │ Diagnostics                            │
> │                   │ What `docair check` reports about the  │
> │                   │ docs: errors and warnings.             │
> │                   │ see check · print_diagnostics          │
> │                   │ In the glossary · Read more            │
> │                   └───────────────────────────────────────┘
> ```

### 13.3 Read more: docs in the page

A link in a definition to a Markdown file of the project (`SPEC.md#8-…`)
puts that file into the page (`library` in the graph), so *Read more* opens
it inside the page, at the heading, even offline. Headings get the anchors
GitHub gives them. Links to the web open in a new tab.

### 13.4 Terms the model explains

`docair glossary` (and **✦ Explain the terms** in the web app) sends the
summaries and descriptions to the model with the terms already defined. It
picks the words a newcomer would not know, defines each in one to three
sentences (for a term of this project: what it is here; for a known
technology: what it is in general and how this project uses it, with a link
to its official docs), and names the nodes to see. A term that does not occur
in the texts is dropped.

The entries are shown at once, marked ✦, and become one edit that appends
them to the glossary (`docs/GLOSSARY.md` when there is none yet): review
the diff, apply it, or download the patch (§10.6).

The same request settles the words that name parts (§13.5): for each open
word the model says whether it is a term, and which of its parts defines it;
it may word that part's text better for a newcomer, never changing its
meaning, and that becomes a glossary entry. It compares each unchecked
glossary entry with the code, and rewrites the entry where they disagree: the
code is right. Open words go 20 to a request, entries 10, at most six
requests a run; a request whose answer is cut off leaves its words open. The edit also writes the decisions file, so a second run on
an unchanged project asks about no word again.

### 13.5 The code first: where a word's meaning comes from

[MEANINGS.md](MEANINGS.md) explains why. Three sources, best first:

| Source | Which words | Example |
|--------|-------------|---------|
| the docs of a part of kind `module`, `component` or `data` | its name, split into words (`LibraryDoc` → *library doc*); names with `-` or `.` are names of things | `Entry`: "An entry is one payment in the ledger" |
| a glossary entry | its term and `Also:` spellings | *Ingress* |
| a doc heading `Word: the rest` | one to three words before the colon | `## 2. Entries: one line of the ledger per payment` |

A part **defines its word by itself** when it is the only such part with
that name and its summary says so (`Entries: …`, `An entry is …`); in every
other case a **decision** says so. A word that names parts, is used in at
least two texts and is settled neither way is **open** (W120). A doc heading
alone never makes a word a term: it adds *Read more*, and it defines a word a
decision makes a term without naming a part.

When the code defines a word that also has a glossary entry, the entry is
shown only once it is **checked**: compared with the part's text at its
current fingerprint (FNV-1a of the summary and description). Until then the
page shows the code's words, and W121 asks for the comparison.

**The decisions file** (`docs/meanings.toml`, or `meanings` in docair.toml)
has one table per word:

```toml
[ledger]
from = "books.ledger"        # the part whose docs define it
checked = "ace0ee32"         # the glossary entry agrees with this text of it
why = "the module that keeps the payments"

[audit]
term = false                 # an everyday word here, never underlined
```

`docair glossary` writes it (§13.4); people review and edit it like code. A
decision is opened again when `from` names no node or `checked` no longer
matches (W121). `term = false` stays until a person changes it.

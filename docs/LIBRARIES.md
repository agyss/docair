# Libraries

Decision record for docair's dependencies (2026-11-14). The choices are verified
by the proof of concept in this repository: `docair scan .` run on docair itself.

## What drives the choice

* The output is **one HTML file**: offline, no CDN, fit to attach to a CI run or a mail.
* The core stays **language-agnostic** ([SPEC §2](SPEC.md)): it knows comment
  syntax and the tag grammar only. Knowledge about code lives in adapters.
* The picture needs **nested groups** (`@part-of`) with edges crossing them,
  data flowing in one direction, a shape per kind, and **animated flows**.
* The tool is Rust. The viewer needs no Node build step.

## Backend (Rust)

| Concern | Choice | Why | Not chosen |
|---------|--------|-----|------------|
| File walk | `ignore` 0.4 + `globset` 0.4 | ripgrep's walker: `.gitignore`, a custom `.docairignore`, fast | `walkdir` (no ignore rules) |
| Code parsing (adapters) | `tree-sitter` 0.27 + grammar crates: `tree-sitter-rust` 0.24 now; `-typescript`, `-html` / `tree-sitter-angular`, `-yaml` next | one API for every language, tolerant of broken code, exact rows for attaching blocks, grammars for all v0 languages | `syn` (Rust only, fails on broken code); `oxc` / `swc` (TS only; keep `oxc` in mind if the TS adapter needs type resolution) |
| Pattern inference | `ast-grep-core` + `ast-grep-config` `=0.51.1` (crate `docair-rules`, 2026-11-17) | framework rules as data: structural patterns like `this.http.post($URL, $$$)` or `#[$METHOD($PATH)]` on top of the same tree-sitter 0.27, in ast-grep's documented YAML rule format, so rules can be tried in its playground ([ADAPTERS.md](ADAPTERS.md) §5.2). 0.51.2 and 0.51.3 were under two weeks old | a hand-written tree walk per pattern; tree-sitter queries (`.scm`), which are harder to read and write |
| Code graph (cross-references, call graph) | **none in the core**; SCIP as an optional input later | blocks are the source of truth. Precise call edges can come from indexers (`rust-analyzer scip`, `scip-typescript`) through the `scip` crate | `stack-graphs` (archived by GitHub in Sep 2025), `tree-sitter-graph` (no release since Dec 2024), `code2graph` (beta, created Jun 2026), CodeQL (license), Joern (JVM) |
| Graph algorithms | `petgraph` 0.8 | shortest paths between flow steps (A*), directed and undirected views | a hand-written BFS |
| Markdown | `pulldown-cmark` 0.13 | CommonMark with source offsets for sections; its heading-attribute extension parses `{#node-id}` natively; also renders the HTML for the viewer | `comrak` (GFM, heavier) |
| Config, JSON | `toml` 1.1, `serde`, `serde_json` | `docair.toml`, `docair.graph.json` | — |
| CLI | `clap` 4 (derive) | the standard choice | `argh`, `lexopt` |
| YAML and JSON with line numbers | `serde-saphyr` =2.0.0: API descriptions ([IMPORTS.md](IMPORTS.md)) are read into a tree whose every node keeps its line (`Spanned`), and JSON parses as YAML 1.2; ast-grep-config uses the same crate for rules. Kubernetes manifests are still read line by line, because Helm templates are not YAML | `serde_yaml` is deprecated and archived (Mar 2024); `saphyr`'s own tree has lines too, but would be a second YAML stack next to ast-grep's; 2.1.1 was younger than two weeks and 2.1.0 is yanked | `serde_yaml`, `serde_yml` (both deprecated), `saphyr` |
| Pretty diagnostics (later) | `miette` or `ariadne` | source snippets in `docair check` output | — |

## Viewer (inlined into `docair.html`)

| Concern | Choice | Size | License |
|---------|--------|------|---------|
| Drawing, interaction, animation | Cytoscape.js 3.34.4 | 436 KB | MIT |
| Layout inside each box | elkjs 0.12.1 (ELK layered) | 1.6 MB | EPL-2.0 |
| Glue | `crates/docair-viewer/assets/viewer.js`: no framework, no build step | ~35 KB | — |

**Cytoscape.js** has compound nodes built in, which gives the `@part-of`
frames. It also covers styles per kind (barrel, rhomboid, hexagon, …), tap,
double-tap and hover events, an animation API for the flow token, and a
headless mode for tests. It needs no framework or bundler.

**ELK** computes a layered (Sugiyama) layout. The viewer lays out every box on
its own with it, sizes the boxes itself and draws the lines with its own router,
at right angles and around the parts (top-down, SPEC §3.5); Cytoscape draws
those routes directly (`round-segments`). The page loads nothing from the web.

**Own adapter instead of `cytoscape-elk`**: the plugin pins `elkjs ^0.9.3` and
lays out the whole graph in one run; the viewer lays it out box by box.

| Not chosen | Why |
|------------|-----|
| Mermaid | a static diagram: no API for selection, collapse or animation. Still good as a later text output (`docair mermaid`) |
| Graphviz in WASM (`@hpcc-js/wasm-graphviz`, `@viz-js/viz`) | strong layered layout with clusters, but the output is SVG, so all interaction would be hand-made; about 1 MB of WASM |
| React Flow / xyflow | needs React and a bundler; has no layout of its own (it would use ELK anyway) |
| Sigma.js + graphology | WebGL for very large graphs, but no compound nodes |
| vis-network | clusters instead of real compound nodes; weaker layered layout |
| AntV G6 5 | capable (combos, many layouts), but larger, with a heavier API than this needs |
| D3 | low-level: layout, routing and interaction would all be ours to write |
| maxGraph, JointJS | diagram editors; far more than a viewer needs |
| `@dagrejs/dagre`, `cytoscape-dagre` | weaker than ELK on compound graphs; no orthogonal routing |

**Licensing.** elkjs is EPL-2.0 (weak copyleft, file level). Shipping it
unmodified inside the generated page is fine. The page carries a notice that
names the license and the source repository, and
`crates/docair-viewer/vendor/` holds the full license texts.

## Web app and cluster

| Concern | Choice | Why | Not chosen |
|---------|--------|-----|------------|
| HTTP server | `axum` 0.8 on `tokio` 1.53, `tower-http` 0.7 (`ServeDir`) | tokio-native and small, multipart uploads built in; `ServeDir` lets the server also serve the built frontend for runs without Kubernetes | actix-web (own runtime, macro-heavy), warp (little activity) |
| Scratch folders | `tempfile` 3.27 | one folder per upload, deleted when the request ends: the server keeps no state | — |
| Frontend | Angular 22 (standalone components, signals, zoneless) | Angular is one of docair's target stacks, so the app is also a real Angular codebase for docair to read; HttpClient reports upload progress | React/Vite (not a target stack), plain JS |
| Frontend image | `nginxinc/nginx-unprivileged` 1.30 | static files as a non-root user, SPA fallback | Caddy |
| Server image | `gcr.io/distroless/cc-debian13:nonroot`, built on `rust:1.99-slim-trixie` | no shell or package manager, glibc for the Rust binary | alpine (musl), scratch (needs a static binary) |
| Cluster | kind | Kubernetes in Docker: one container per node, runs inside the devcontainer with Docker-in-Docker | minikube (heavier), k3d (k3s instead of upstream Kubernetes) |
| Provisioning | Terraform 1.16, `tehcyx/kind` 0.13, `hashicorp/helm` 3.3 | one `terraform apply` from nothing to a running app, `destroy` to clean up, a plan to read first | shell scripts (no state, no destroy); OpenTofu works too |
| Ingress controller | Traefik 3.7 (chart 42.2.0) on NodePort 30080 | maintained, reads standard Ingress objects | ingress-nginx (retired in March 2026); a Gateway API setup can come later |
| Getting images into kind | `docker save` piped into `ctr images import` in the node | no registry needed; this is what `kind load` does | a local registry |
| Checks without Docker | `helm lint`, `kubeconform` 0.8, `terraform validate` | validate charts and Terraform where no cluster can run | — |

## Versions and supply chain

* The JS libraries are vendored in `crates/docair-viewer/vendor/` with their
  sha256 (see that README), so builds and generated pages need no network.
* Every pinned version had been released for at least two weeks. Cytoscape
  3.35.0 and cytoscape-elk 2.4.0 were four days old and were skipped;
  `tree-sitter` is pinned to 0.27.1 (not 0.27.2) and `toml` to 1.1.8.

## Proof of concept, verified on docair itself

`docair scan .` writes `.docair/docair.graph.json` and `.docair/docair.html`.
docair's own sources are documented in docair format in every comment family:
Rust, JavaScript, TOML (`Cargo.toml`, `docair.toml`) and HTML. Markdown is used in
all three ways: README group docs, a sibling file (`resolve.md`) with an
anchored section, and fenced declarations for nodes without code (stores, the browser).

| Check | Result |
|-------|--------|
| `cargo test` | 20 tests pass |
| `cargo clippy --all-targets` | no warnings |
| `docair check .` | 53 nodes, 43 edges, 3 flows; 0 errors, 0 warnings |
| scan time (release build) | 14 ms |
| `docair.html` | 2.2 MB, a single file, makes no network requests |
| headless Chromium (Playwright), light and dark | 35/35 checks: boots in about 300 ms including layout; every node and edge drawn; no overlaps; children inside their groups; edges follow ELK's routes exactly; select, details, docs, tags; double-click collapse with edge bundling; flow stepping and token animation to the last step; search; diagnostics drawer; 400 px width; no console errors |
| multi-language fixture (Rust, TS, YAML, Markdown) | an actix route attribute is inferred as `@http`, and TS `@calls http:POST /api/orders` resolves to the Rust handler; E001, W101 and W103 are reported; `check` exits 1, `--lenient` exits 0 |

What dogfooding found:

1. **All edges were invisible at first.** ELK ends routes exactly on the node
   border, which Cytoscape counts as inside the node. Route ends are now pushed
   4 px outward.
2. **The flow token could hang**: the animation of a removed token never
   resolves. Each animation step now races a timeout.
3. **Return values create cycles.** `check → cmd_scan` made ELK draw long loops
   around the whole core. Annotating the data's consumer instead
   (`write_json @in graph from check`) is more accurate and lays out cleanly.
   Advice for authors: annotate where data is used, not where a call returns.
4. **Pipelines are long.** Fully expanded, docair's graph is 6:1 wide and
   unreadable when fitted. The viewer now picks the layout direction that fits
   the screen and collapses nested groups on first open.
5. **The spec had gaps** that the code had to decide. They are now in SPEC.md:
   openers only at the start of a line, the `module` kind, adapter-derived ids,
   the `@part-of` default chain, the `placeholder` origin, E005/E006/W106, the
   README and sibling-file rules, and a corrected flow example.

Not in the PoC yet: the TypeScript/Angular and Kubernetes adapters, the
Runtime layer switch, Mermaid output, `docair serve`, and edge-label placement.
Labels can overlap on short edges; ELK can place them if it is given label sizes.

## Sources

* stack-graphs archived: [github/stack-graphs](https://github.com/github/stack-graphs), [ecosyste.ms record](https://issues.ecosyste.ms/hosts/GitHub/repositories/github%2Fstack-graphs)
* [code2graph README](https://docs.rs/crate/code2graph/0.0.0-beta.17/source/README.md)
* [SCIP](https://github.com/sourcegraph/scip)
* [serde_yaml deprecation thread](https://users.rust-lang.org/t/serde-yaml-deprecation-alternatives/108868)
* ELK: [hierarchyHandling](https://eclipse.dev/elk/reference/options/org-eclipse-elk-hierarchyHandling.html), [JSON format](https://eclipse.dev/elk/documentation/tooldevelopers/graphdatastructure/jsonformat.html)
* Versions and licenses: crates.io and the npm registry, queried on 2026-11-14

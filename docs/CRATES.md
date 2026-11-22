# The crates

Epic docair-df1. docair's crates follow one rule: **programs on top of
libraries, and no program depends on another**. Adapters read one format
each and report what they found; the core links it all.

## 1. Why

Before 0.1, three things went against that:

* The CLI depended on `docair-server` for things that have nothing to do with
  HTTP: comparing two commits (`commits.rs`, used by `docair diff`) and the
  project session (`project.rs`: scan, ask, pending edits, apply, lay out
  again, used by `docair ask` and `docair serve`).
* `docair-infra` read Terraform, Helm charts, Kubernetes manifests and
  Dockerfiles in one crate of 2,248 lines, and linked them itself; three
  crates of 41 to 64 lines (`docair-lang-terraform`, `-helm`, `-k8s`) only
  registered it under three names.
* The adapter crates were called `docair-lang-*`, although they are adapters
  (the trait is `Adapter`, the design ADAPTERS.md), and Terraform or a
  Dockerfile is not a language. `docair-lang-generic`, the guess for
  languages no adapter knows, was a crate of 219 lines of its own.
* The AI, a core feature of docair (asking, wording the lines, the glossary,
  explaining changes), was a crate of its own beside the core,
  `docair-ai`, together with the HTTP client that talks to the model.

## 2. The crates after

| Layer | Crate | What |
|-------|-------|------|
| programs | `docair-cli` | the `docair` command |
| | `docair-server` | the upload server for the web app: a `main.rs` on `docair-http` |
| | `docair-index` | rust-analyzer as a library, run as a process of its own (heavy to build) |
| page and service | `docair-http` | the HTTP routes: a project (`docair serve`) and uploads (`docair-server`) |
| | `docair-viewer` | the HTML page, the pictures as SVG |
| | `docair-layout` | placement and the router; knows nothing of docair's graph |
| | `docair-layout-wasm` | moving a part by hand, built for the page |
| project | `docair-project` | a project session (scan, ask, pending edits, apply, lay out again, the kept page), two commits compared, and the client that sends to the model over HTTP |
| core | `docair-core` | model, scan, linking (also across infrastructure formats, §3), checks, layers, the guess for unknown languages, and the AI: prompts, the context from the graph, the answers read and checked (§4) |
| adapters | `docair-adapter-rust`, `-ts`, `-apis` | code and API descriptions |
| | `docair-adapter-terraform`, `-helm`, `-k8s`, `-docker` | infrastructure, one format each |
| | `docair-adapters` | the list of all adapters |
| | `docair-rules`, `docair-scip` | ast-grep rules, SCIP indexes, for the code adapters |

19 crates, as before, but each with one job and none a mere registration.
Dependencies point down only: a program uses any library; `docair-http` uses
`docair-project`, `docair-viewer`; adapters use only `docair-core` (and
`docair-rules`, `docair-scip`).

## 3. Infrastructure adapters report, the core links

Today `docair-infra` keeps an index of the whole project (`Index`: the
Terraform resources, releases, charts and their values, objects, Dockerfiles,
image builds, crate mains) and draws the lines between formats itself.
After the split each adapter reads its own files only, and reports what it
found as facts; what links formats are rendezvous points, as HTTP routes,
topics and environment variables already are (`facts::Meet`):

| Meets at | One side | Other side |
|----------|----------|-----------|
| a chart directory | Terraform's `helm_release` installs it | Helm: the chart and its objects |
| an image | Terraform's `docker build -t` (or a manifest's `image:`) | Docker: the Dockerfile that builds it |
| a Rust crate | Docker: `cargo build -p <crate>` | Rust: the crate's `main` |
| a Service's selector | Kubernetes: a Service | Kubernetes: the Deployment whose labels match |
| an Ingress backend | Kubernetes: an Ingress | Kubernetes: the Service |
| a cluster | Terraform: the resource that creates it | Helm and Kubernetes: what runs in it |

Values a chart fills into its templates (`.Values`) stay inside the Helm
adapter: chart and templates are the same format. The goldens of the
fixtures `platform`, `shop` and `tools` must come out the same; that is the
test of the split.

## 4. The AI in the core, its client outside

The AI's logic is a core feature and goes into `docair-core` as the module
`ai` (from `docair-ai`: ask, word, glossary, explain, the context, the
settings). Sending to the model does not: the core says what a model is
(messages in, text out, as a trait), and `docair-project` gives the client
that sends over HTTP (`ureq`, with TLS). So the core, and every adapter on
it, builds without a network stack, and tests and e2e put a fake model in.

## 5. Steps

| Step | Issue | What |
|------|-------|------|
| 1 | docair-df1.1 | `docair-lang-*` renamed `docair-adapter-*`; ids, docs, glossary, goldens |
| 2 | docair-df1.2 | `docair-project` from `docair-server` (`project.rs`, `commits.rs`); the CLI no longer depends on the server |
| 3 | docair-df1.3 | `docair-http` from `docair-server` (the routes); `docair-server` a `main.rs`; `docair serve` on `docair-http` |
| 4 | docair-df1.4 | the generic guess into `docair-core`; `docair-lang-generic` goes |
| 5 | docair-df1.5 | the AI's logic into `docair-core` behind a model trait, the HTTP client into `docair-project`; `docair-ai` goes (§4) |
| 6 | docair-df1.6 | the rendezvous points of §3 in the core |
| 7 | docair-df1.7 | `docair-adapter-terraform`, `-helm`, `-k8s`, `-docker` from `docair-infra`, each on §3's points; `docair-infra` goes |
| 8 | docair-df1.8 | the numbers, SPEC and ARCHITECTURE; all tests and e2e; the 0.1 commit made again and pushed |

Order: 1 first; 2 then 3 and 5; 4 after 1; 6 after 1, then 7; 8 last.
Decisions docair-df1.9 to .16.

Each step leaves every test and e2e passing.

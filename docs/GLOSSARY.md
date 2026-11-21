# Glossary

What the words of docair's docs mean. Each term is explained where the page
uses it: hover it for the short version, click it for the whole entry.

## doc block

Also: doc blocks, block
See: docair-core.extract.extract

A comment that docair reads: it opens with `/***`, `#***` or `<!--***`
(whatever the language) and holds a summary, a description and tags.

The comment syntaxes and the body grammar are in [SPEC §2](SPEC.md#2-doc-blocks-stage-2-extract).

## tag

Also: tags
See: docair-core.extract.parse_body

A line in a doc block that starts with `@`, such as `@calls`, `@reads` or
`@part-of`: it says what the part is and how data flows to and from it.

Every tag is listed in [SPEC §3.1](SPEC.md#31-identity-and-structure-tags) and [SPEC §4.1](SPEC.md#41-data-flow-tags).

## graph

See: docair-core.scan, graph-json

Everything docair knows about a system: the nodes (its parts), the edges
between them, the scenarios, the walkthroughs and the diagnostics.

`docair build` writes it as `docair.graph.json`, the contract between the
backend and the viewer: [SPEC §9](SPEC.md#9-output-docairgraphjson-stage-7-emit).

## node

Also: nodes
See: docair-core.build.build

One part of the system on the chart: a host, a cluster, a workload, a
service, a function, a store. Each doc block becomes one node; `@part-of`
puts it inside another.

Node kinds and the node model: [SPEC §3](SPEC.md#3-nodes-stage-4-build).

## edge

Also: edges
See: docair-core.resolve.resolve

An arrow between two nodes, made by a data-flow tag: a call, a read, a
write, an event, a test that covers a part. Its words say what happens along
it.

How edges are made and worded: [SPEC §4.2](SPEC.md#42-edge-model).

## ref

Also: refs
See: docair-core.resolve.resolve

How a tag names another node: by its id (`docair-core.scan`), by a route
(`http:POST /api/scan`), by a topic (`topic:order.created`), or by a part of a
node (`orders-db.orders`). A ref that names nothing is the error E001.

The forms of a ref: [SPEC §4.3](SPEC.md#43-references).

## payload

See: docair-core.resolve.resolve

What travels along an edge: `(…)` in a tag is what the node sends, `-> …`
what it receives. When a call names neither, docair takes them from the
called function's signature.

[SPEC §4.1](SPEC.md#41-data-flow-tags)

## stage

Also: stages
See: docair-core.scan

One step of the pipeline from source files to the page: discover, extract,
enrich, build, resolve, check, emit. Each stage only uses what the stage
before it produced.

[SPEC §1](SPEC.md#1-how-docair-works)

## adapter

Also: adapters
See: docair-core.adapter.collect, docair-core.merge.merge, docair-adapters.all

The part of docair that knows one language. It reads the code and reports
facts: which symbols exist (name, kind, signature), which routes they serve,
what they use. The core attaches each doc block to the symbol it documents.
Rust has one (tree-sitter), infrastructure has one (Terraform, Helm,
Kubernetes); other languages get the generic adapter, which reads the line
after a block.

[ADAPTERS](ADAPTERS.md)

## API description

Also: API descriptions, OpenAPI, AsyncAPI, Swagger
See: docair-lang-apis, docair-core.declare.settle

A file that says which routes a service answers, or which topics an
application sends to and receives from, with what travels there: OpenAPI
(Swagger is its older name) for HTTP, AsyncAPI for messages. docair reads it
next to the code: each operation merges into the handler that serves its
route, callers' lines get its words, and where the description and the code
disagree, `docair check` warns.

[IMPORTS](IMPORTS.md)

## diagnostics

Also: diagnostic
See: docair-core.check.check, docair-cli.print_diagnostics

What `docair check` reports about the docs: errors (codes `E…`), such as a ref
that names nothing, and warnings (`W…`), such as a broken scenario. With
errors, `docair check` fails, so CI can gate on it.

Every code: [SPEC §8](SPEC.md#8-diagnostics-stage-6-check).

## layer

Also: layers
See: docair-core.layer_view.layers, docair-core.layer_view.picture

One concern of the system at a time: the overview, the infrastructure, the
network, Kubernetes, the apps or the code. A layer draws the parts its question
is about, shows what a line joins to them as context, and draws a request
through the parts it leaves out as one line that says what it passes ("via
host-port › traefik › Ingress docair"). Zooming into a box opens it where it
is; zooming out past the picture closes it, then goes up a layer.

[SPEC §3.4](SPEC.md#34-layers-one-concern-at-a-time)

## scenario

Also: scenarios, flow, flows
See: docair-core.resolve.flow_paths

A use case told step by step, declared with `@flow <name> <step> "label"`
on the parts it passes through. The viewer replays it like a walkthrough.

[SPEC §7](SPEC.md#7-scenarios-walk-me-through-stage-5-derives-viewer-replays)

## walkthrough

Also: walkthroughs
See: docair-ai.ask.ask, walkthrough-files

A question answered by the AI model as steps through the chart, each one
naming its parts and saying what happens and why. It is recorded as JSON in
`docs/walkthroughs/` and replayed by every later page.

[SPEC §10.3](SPEC.md#103-the-walkthrough-file)

## digest

See: docair-ai.ask.digest

The graph shortened for the AI model: ids, kinds, names, summaries,
descriptions and edges, without any source code.

[SPEC §10.2](SPEC.md#102-asking)

## patch

Also: patches
See: docair-core.edit.patch

The edits the AI model proposes for the doc blocks, as a unified diff
(`.docair/docair.patch`). `git apply` takes it, `docair apply` writes it, and
Apply in the page (with `docair serve`) writes it and the documentation again.

[SPEC §10.6](SPEC.md#106-feeding-edits-back-to-the-code)

## glossary

See: docair-core.glossary.parse, docair-viewer.page_glossary.glossary

This list: what the words of the docs mean. Any `GLOSSARY.md` is one;
`docair glossary` lets the model add the terms that are missing.

[SPEC §13](SPEC.md#13-glossary-what-the-words-mean)

## kind cluster

Also: kind node
See: kind, tf.kind_cluster

kind is Kubernetes IN Docker: a tool that runs a whole Kubernetes cluster as Docker
containers on one machine. docair's cluster is one kind node,
`docair-control-plane`, created by Terraform. [kind docs](https://kind.sigs.k8s.io/)

## Terraform

See: terraform, tf.kind_cluster

A tool that creates infrastructure from a description of what should exist.
Here, one `terraform apply` creates the kind cluster, builds and loads the
images and installs both Helm charts. [Terraform docs](https://developer.hashicorp.com/terraform/docs)

## Helm

Also: chart, charts
See: k8s.release.docair, k8s.release.traefik

The package manager of Kubernetes: a chart is a set of templates for
Kubernetes objects, and installing it makes a release. docair's chart is in
`deploy/helm/docair`. [Helm docs](https://helm.sh/docs/)

## Traefik

See: k8s.traefik

The ingress controller in the cluster: it takes every HTTP request that comes
in through the published port and routes it by the Ingress rules. [Traefik docs](https://doc.traefik.io/traefik/)

## Ingress

See: k8s.ingress.docair

A Kubernetes object with routing rules for HTTP: here `/api` goes to the
docair-server Service and everything else to docair-web. [Kubernetes: Ingress](https://kubernetes.io/docs/concepts/services-networking/ingress/)

## NodePort

See: kind-node, host-port

A Service type that opens the same port on every node of the cluster. Traefik
listens on NodePort 30080, which kind publishes on 127.0.0.1:8080. [Kubernetes: NodePort](https://kubernetes.io/docs/concepts/services-networking/service/#type-nodeport)

## NetworkPolicy

See: k8s.netpol.docair

A Kubernetes rule for which pods may talk to which. docair's policy lets only
the Traefik namespace reach the apps, and only on port 8080. [Kubernetes: Network Policies](https://kubernetes.io/docs/concepts/services-networking/network-policies/)

## tree-sitter

See: docair-lang-rust.rust-adapter

A parser library that turns source code into a syntax tree, fast and without
compiling. The Rust adapter uses it to find functions, their signatures and
routes. [tree-sitter docs](https://tree-sitter.github.io/tree-sitter/)

## ELK

See: docair-layout

The Eclipse Layout Kernel, which placed the parts inside each box of the chart
in the browser until docs/SLICES.md step 5. docair lays out its pictures
itself now, in the CLI (docair-layout).
[elkjs](https://github.com/kieler/elkjs)

## docair.toml

See: docair-core.config, docair-core.config.load

The configuration file of a scanned project, at its root. It sets the system id, include/exclude globs, strictness, test reports, glossary files and view settings such as hidden sections.

## docair.graph.json

See: docair-core.emit, docair-core.emit.write_json

The JSON file a scan writes: the whole graph with its nodes, edges and diagnostics. It is the integration API that the viewer and other tools read.

## docair.html

See: docair-viewer.write_html, docair-viewer.render_html, html-file

The self-contained HTML page docair produces: every picture laid out and drawn as SVG by the CLI, the panels around them and a thin script, all inlined, so it works offline and can be mailed or archived.

## Cytoscape.js

Also: Cytoscape
See: docair-viewer, docair-viewer.svg

A JavaScript library for drawing graphs and interacting with them, which drew the chart in the browser until docs/SLICES.md step 5. The CLI draws every picture as SVG now. [Read more](https://js.cytoscape.org/)

## Playwright

See: e2e, docair-core.test_results, test-reports

A tool that drives a web app in a real browser for end-to-end tests. docair's e2e tests run on it, and a scan reads its JSON reports to put results on the test nodes. [Read more](https://playwright.dev/)

## Angular

See: docair-web, container.docair-web

A framework for single-page web applications. The docair-web app is written in Angular and built to static files that nginx serves. [Read more](https://angular.dev/)

## nginx

See: nginx, container.docair-web, k8s.deploy.docair-web

A web server for static files. In the docair-web container it serves the built Angular app on port 8080; unknown paths fall back to index.html so deep links work. [Read more](https://nginx.org/en/docs/)

## actix

See: docair-lang-rust.rust-adapter, docair-rules

A Rust web framework. A rule of the Rust adapter (`rules/actix.route.yaml`) reads its route attributes, such as `#[post("/api/orders")]`, as the routes a handler serves. [Read more](https://actix.rs/)

## pod

See: k8s.deploy.docair-server, k8s.deploy.docair-web, upload-dir

The smallest unit Kubernetes runs: one or more containers that share an IP address and are scheduled together. The docair server and web app each run as pods. [Read more](https://kubernetes.io/docs/concepts/workloads/pods/)

## Deployment

See: k8s.deploy.docair-server, k8s.deploy.docair-web

A Kubernetes object that keeps a set of identical pods running and replaces them when they fail. The docair chart installs two, for docair-server and docair-web. [Read more](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/)

## rendezvous point

See: docair-core.facts.meet, docair-core.facts.serve, docair-core.request.link

Where two sides meet without knowing each other: a route that a handler serves, an API description declares, or a client requests. Adapters report each side as a fact, and the core links what requests a point with the code that serves it.

## ast-grep

See: docair-rules, docair-lang-rust.rules, docair-lang-ts.rules

A tool that finds code by patterns written like the code itself, built on tree-sitter grammars. docair's framework rules are ast-grep patterns plus the facts a match means. [Read more](https://ast-grep.github.io/)

## decision

See: docair-core.meanings.decision

What was decided about one word of the docs: which part's docs define it, or that the word is ordinary. The decisions are kept in the meanings file, which `docair glossary` writes and every scan reads.

## event

See: docair-lang-apis.model.event

A message that an AsyncAPI description says the described application sends to, or receives from, a channel.

## family

See: docair-core.lang.family

A family of languages whose comments are written the same way; its markers say how doc blocks are written in its languages.

## fit

See: docair-core.route.fit

How well a request's path matches a route.

## inferred

See: docair-core.facts.inferred

A value an adapter read from the code, kept together with the reason it was read; inferred tags never replace authored ones.

## item

See: docair-lang-rust.item

One thing tree-sitter finds in a Rust file; every item gets an id, and its parent comes from `@part-of` or the item that encloses it.

## marker

See: docair-core.lang.marker

The comment symbols that fence a doc block in a source file: an opening marker, a closing marker, and the line prefix stripped from content lines.

## meet

See: docair-core.facts.meet

A rendezvous point where two sides meet without knowing each other: doc blocks meet the facts, requests the code sends meet the routes that serve them, and API descriptions meet the handlers and topics they describe.

## merge

See: docair-core.merge

The core joins the facts the adapters collected with the docs: each doc block is attached to the symbol it documents, and declared routes and description endpoints merge into the handlers that serve them.

## route

See: docair-core.route.route

A path pattern that code serves or that an API description declares; docair splits routes and requested paths into segments, so it can judge how well a request fits a route.

## rule

See: docair-rules.rule

A loaded rule: its matcher recognises a framework's code pattern, and a match tells docair what that code is.

## rule file

See: docair-rules.rule-file

A YAML file holding the rules for one grammar; docair loads and checks each rule file, and its `language` names the grammar it targets.

## serve

See: docair-core.facts.serve

A symbol serves a rendezvous point — a place where requests arrive, such as a route — when it is the code that runs there, e.g. the handler of a route.

## tag value

See: docair-core.resolve.tag-value

The full content of a data-flow tag: the ref it points at, what happens on the way, what the tag's node sends and what it receives.

## view

See: docair-core.model.view

What the page shows when it opens: which of its sections are hidden or shown.

## view change

See: docair-ai.ask.view-change

The change an AI reply asks for in the view: which sections of the page to hide and show.

## review

Also: reviews
See: docair-ai.explain.change-findings, docair-core.review, docair-ai.explain.explain

What `docair explain` writes and the changes section shows first: the
changes between two versions, one difference in what the app does each:
the AI's own reading of docair's pieces and their code, with the commit
messages and the changed docs checked against it, and what it found off. See [EXPLAIN §9](EXPLAIN.md#9-the-review-changes-as-the-reader-understands-them-docair-8l6.17-to-8l6.24).

## found off

See: docair-ai.explain.found-off

What the AI found wrong in a change, of one of three kinds. A
*contradiction*: what a commit message, a doc or a code comment says, and
which one (*Said*), against what the code clearly does otherwise (*The
code*); a doc left out of date is one. *Unsaid*: what the code does that no
commit, doc or comment mentions. A *risk*: a mistake, a missed case or a
risk in the code. See [EXPLAIN §9.1](EXPLAIN.md#91-what-a-change-is).

## piece

Also: pieces
See: docair-core.review.piece

One bit of a change that docair found: a stretch of new and gone parts and
lines, a line alone, the parts of one box whose code changed, or an open box
whose own block changed. A piece that shows up in many flows is one piece;
the AI groups pieces into changes.

## intent

See: docair-core.review.intent

What was meant, beside what changed: the commit messages between two
versions, the changed Markdown docs, and the doc blocks whose words
changed. The AI reads it to say why a change was made. With it goes all
of what changed, to read: each changed part's whole text before and
after, comments with it, and the changed files that hold no part.

## slice

Also: slices
See: docair-core.slice, docair-core.slice.slice-struct

One part of the system, followed from a seed to its ends: from a datum, a
part, a route or a walk, along fixed rules, to parts with stable names (a
store, a route, the browser). The page draws each slice as one picture. In
the code, the module `slice` finds slices, and `struct Slice` holds one: its
seed and ends, its parts in the order it meets them, its steps, lines and
boxes, and the data its lines carry. See [SLICES §3](SLICES.md#3-a-slice).

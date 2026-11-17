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

`docair scan` writes it as `docair.graph.json`, the contract between the
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
See: docair-viewer.show-layer, docair-viewer.layer-view

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
(`.docair/docair.patch`). `git apply` takes it, `docair apply` writes it, the
web app applies it and generates the page again.

[SPEC §10.6](SPEC.md#106-feeding-edits-back-to-the-code)

## glossary

See: docair-core.glossary.parse, docair-viewer.glossary

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

See: docair-viewer.top-down

The Eclipse Layout Kernel: it places the parts inside each box of the chart,
one box at a time, with room for the words of their lines. docair sizes the
boxes and draws the lines itself.
[elkjs](https://github.com/kieler/elkjs)

## docair.toml

See: docair-core.config, docair-core.config.load

The configuration file of a scanned project, at its root. It sets the system id, include/exclude globs, strictness, test reports, glossary files and view settings such as hidden sections.

## docair.graph.json

See: docair-core.emit, docair-core.emit.write_json

The JSON file a scan writes: the whole graph with its nodes, edges and diagnostics. It is the integration API that the viewer and other tools read.

## docair.html

See: docair-viewer.write_html, docair-viewer.render_html, html-file

The self-contained HTML page docair produces: graph, rendered Markdown, viewer code and libraries all inlined, so it works offline and can be mailed or archived.

## Cytoscape.js

Also: Cytoscape
See: docair-viewer, docair-viewer.page

A JavaScript library for drawing graphs and interacting with them. The docair page inlines it to draw the chart and handle selecting, zooming and hiding. [Read more](https://js.cytoscape.org/)

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

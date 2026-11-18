# Architecture

```
            ┌──────────────── sources ────────────────┐
            │ *.rs  *.ts  *.html  *.yaml  *.md  charts│
            └───────────────┬─────────────────────────┘
                            │ walk + language registry (by extension)
                            ▼
  ┌───────────────────────────────────────────────────────────┐
  │ docair-core :: extract   (generic, comment-syntax driven) │
  │   RawBlock { file, line, description, tags, following }   │
  └───────────────┬───────────────────────────────────────────┘
                  │ adapters enrich (symbol, kind, inferred tags)
                  ▼
  ┌───────────────────────────────────────────────────────────┐
  │ docair-lang-*  (rust, typescript/angular, k8s/helm, ...)  │
  │   trait Adapter { fn enrich(file, &mut [RawBlock]) }      │
  │   + fn discover(&self, src) -> Vec<RawBlock>  (no-comment │
  │     nodes found from code alone, e.g. k8s objects)        │
  └───────────────┬───────────────────────────────────────────┘
                  ▼
  ┌───────────────────────────────────────────────────────────┐
  │ docair-core :: graph                                      │
  │   blocks + sibling .md  →  Graph { nodes, edges, flows }  │
  │   ref resolution (id / http: / topic: / k8s:)             │
  │   diagnostics (unresolved refs, unknown tags, mismatches) │
  └───────────────┬───────────────────────────────────────────┘
                  ▼
  ┌───────────────────────────────────────────────────────────┐
  │ outputs                                                   │
  │   docair.graph.json  (stable schema, the integration API) │
  │   mermaid            (quick look, renders on GitHub)      │
  │   viewer             (interactive HTML: layers code ↔ k8s,│
  │                       flow simulation / animation)        │
  └───────────────────────────────────────────────────────────┘
```

## Principles

1. **Language-agnostic core.** The core only knows comment syntaxes and the tag
   grammar in [SPEC.md](SPEC.md). Everything language-specific lives in an
   adapter. Adding a language means adding a `LanguageSpec` entry, plus an
   adapter if the language needs code inference.
2. **Code is the source of truth; blocks explain.** Adapters say what exists
   and what talks to what. Blocks add why, the words on the lines, scenarios,
   and what code can't show. Where a block contradicts the code, the code wins
   and `docair check` warns. Design and plan: [ADAPTERS.md](ADAPTERS.md).
3. **One graph, many layers.** Code nodes (functions, components) and runtime
   nodes (k8s workloads) live in one graph, linked by `@deployed-as` and
   `@part-of`. The viewer switches layers and keeps the edges.
4. **Simulation is data.** A flow is an ordered list of steps over resolved
   edges. The viewer replays it, and later versions can add timing, replicas
   and failure injection from k8s data (replica counts, probes, resource limits).
5. **Dogfooding.** docair's own source is documented in the docair format.

## Crates

| Crate          | Purpose | State |
|----------------|---------|-------|
| `docair-core`  | model, language registry, block extraction, tag parsing, graph building, ref resolution, checks | PoC |
| `docair-adapters` | the adapter registry used by the CLI and the server ([ADAPTERS.md](ADAPTERS.md)) | PoC |
| `docair-lang-rust` | Rust adapter (tree-sitter): symbol, kind, module-path ids, parents, actix routes | PoC |
| `docair-lang-infra` | Terraform, Helm charts, Kubernetes manifests, read as a whole | PoC |
| `docair-lang-generic` | keyword guesses from the line after a block, for languages without an adapter | PoC |
| `docair-lang-apis` | OpenAPI and AsyncAPI descriptions: the routes and topics they declare, matched with the code by the core (`declare`); where they disagree, `docair check` warns ([IMPORTS.md](IMPORTS.md)) | PoC |
| `docair-lang-ts` | TypeScript/JavaScript adapter (tree-sitter): declarations, Playwright tests, HttpClient and fetch requests | PoC |
| `docair-scip` | reads SCIP code indexes (rust-analyzer) into calls between functions; runs the installed indexer | PoC |
| `docair-viewer`| one self-contained HTML page: Cytoscape.js + ELK, levels, drill-down, scenarios | PoC |
| `docair-cli`   | `docair scan`, `docair check` (later: `docair mermaid`, `docair serve`) | PoC |
| `docair-server`| HTTP backend: `POST /api/scan` turns an uploaded folder into a docair page | PoC |

Around the crates:

| Directory | Purpose |
|-----------|---------|
| `web/` | docair-web, the Angular frontend: upload a folder, show the generated page |
| `docker/` | the images: docair-server (distroless) and docair-web (nginx) |
| `deploy/helm/docair/` | the chart: two Deployments, two Services, Ingress, ConfigMap, NetworkPolicy |
| `infra/terraform/` | a kind cluster, the images, Traefik and the chart, from one `terraform apply` |

The system is documented in its own format, from the machine down to the
code: `docair scan .` draws the workstation, Docker, the kind network and
cluster, the namespaces and their objects, the containers, the apps and their
functions, as one tree you can zoom through.

Libraries and why: [LIBRARIES.md](LIBRARIES.md).

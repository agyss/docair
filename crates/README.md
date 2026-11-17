# crates

| Crate | Role |
|-------|------|
| `docair-core` | the pipeline: discover, extract, collect facts, merge, build, resolve, check |
| `docair-adapters` | the adapter registry, in the order that decides who says what a symbol is |
| `docair-lang-rust` | the Rust adapter (tree-sitter) |
| `docair-lang-infra` | the infrastructure adapter: Terraform, Helm charts, Kubernetes manifests |
| `docair-lang-generic` | the generic adapter: guesses from the line after a block, for code no other adapter knows |
| `docair-viewer` | renders a graph into one self-contained HTML page |
| `docair-ai` | asks an AI model about the graph: answers, walkthroughs, view changes, doc-block edits |
| `docair-cli` | the `docair` binary for your terminal and CI |
| `docair-server` | the HTTP backend of the docair web app |

```docair
docair-lib: the Rust library with the whole pipeline. It is linked into both binaries, docair-cli on the workstation and docair-server in the cluster, and runs in-process in each.

@id      docair-lib
@kind    module
@part-of docair
```

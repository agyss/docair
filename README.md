# docair

docair reads doc blocks, Markdown and code, and draws how a whole system works:
from the machine and the Kubernetes cluster down to single functions, in one
self-contained HTML page. This repository is documented in docair's own format,
so `docair build .` draws docair itself.

```
                 ┌──────────────────────────── workstation (Docker) ─────────────────────────────┐
browser ─► 127.0.0.1:8080 ─► kind node :30080 ─► Traefik ─► Ingress ─┬─ /    ─► docair-web (nginx + Angular)
                 │                                                   └─ /api ─► docair-server (Rust) ─► docair-lib
                 │  terraform apply: kind cluster, images, Traefik, Helm chart                         │
                 └────────────────────────────────────────────────────────────────────────────────────┘
```

| Part | What it is |
|------|------------|
| [crates/](crates/) | the Rust workspace: the pipeline library, the CLI, the HTTP server |
| [web/](web/) | docair-web, the Angular frontend: upload a folder, get its docair page |
| [docker/](docker/) | the two images: docair-server (distroless) and docair-web (nginx) |
| [deploy/helm/docair/](deploy/helm/docair/) | the Helm chart: Deployments, Services, Ingress, ConfigMap, NetworkPolicy |
| [infra/](infra/) | Terraform: the kind cluster, the images, Traefik and the chart |
| [e2e/](e2e/) | the end-to-end tests (Playwright), shown on docair's own page next to what they cover |
| [docs/](docs/) | [SPEC](docs/SPEC.md) (the annotation format and the viewer), [ARCHITECTURE](docs/ARCHITECTURE.md), [LIBRARIES](docs/LIBRARIES.md) |

## Run it

**In a kind cluster** (needs Docker; the devcontainer has Docker-in-Docker and Terraform):

```bash
terraform -chdir=infra/terraform init
terraform -chdir=infra/terraform apply      # ~5 min the first time: images build, cluster starts
# open http://localhost:8080, pick a project folder, click "Generate docair page"
terraform -chdir=infra/terraform destroy
```

**Without Kubernetes**, the server can serve the built frontend itself:

```bash
(cd web && npm ci && npm run build)
DOCAIR_BIND=127.0.0.1:8081 DOCAIR_STATIC_DIR=web/dist/docair-web/browser cargo run -p docair-server
# open http://localhost:8081
```

**From the terminal or CI**:

```bash
cargo run -p docair-cli -- check .          # diagnostics, non-zero exit on errors
cargo run -p docair-cli -- build .           # .docair/docair.graph.json and .docair/docair.html
```

**Clarify with AI** (docs/SPEC.md §10): ask how something works, say what you
don't care about, or tell docair what is wrong in the docs.

```bash
cp .env.example .env                        # put your key into DOCAIR_AI_API_KEY (gitignored)
cargo run -p docair-cli -- ai --check       # endpoint and model come from [ai] in docair.toml
cargo run -p docair-cli -- ask "How does terraform provision the cluster?"
cargo run -p docair-cli -- ask -f "and what happens when an image changes?"   # a follow-up
cargo run -p docair-cli -- ask -f "the server also reads the AI Secret; that is missing"
cargo run -p docair-cli -- apply            # writes the proposed doc-block edits into the files
cargo run -p docair-cli -- chat             # the same as a conversation; /apply, /quit
```

Walkthroughs are recorded in `docs/walkthroughs/` and replayed by every page
from then on. Edits go to `.docair/docair.patch` (`git apply` takes it) until
you apply them; "I don't care about terraform" becomes `[view] hide` in
docair.toml. In the web app, the **Ask docair** panel does the same: replies
play on the chart, hide sections with Undo, and edits are reviewed as a diff,
then applied and the page regenerated.

**The e2e tests** (e2e/README.md): `cd e2e && npm ci && npm run build && npm test`.
They run against a fake AI endpoint, so they are repeatable; afterwards
`docair build .` shows every test on the page, next to what it covers, with its
result.

## Look around

Open `.docair/docair.html` after `docair build .`. The header's **Layer** switch
shows one concern at a time (docs/SPEC.md §3.4):

| Layer | Answers |
|-------|---------|
| Overview | the big pieces: the browser, the developer, the workstation, the docair library |
| Infrastructure | the workstation, Docker, the kind cluster and its node, and what Terraform creates |
| Network | how a request gets in: the published port, the node, Traefik, the Ingress, the Services |
| Kubernetes | what the charts install: namespaces, releases, Deployments, containers, ConfigMap, Secret |
| Apps | each program and library and how they talk; a request through the network is one line, `via host-port › traefik › Ingress docair` |
| Code | the modules inside the programs; zoom into one (or walk through it) for its functions |

* **Scroll into a box** to open it where it is, and keep going; the breadcrumb
  shows the boxes opened. **Scroll out past the picture** to close them, then
  to go up a layer.
* **Walk me through** (bottom bar, or the list in the details panel) replays a
  use case step by step: the `@flow` scenarios `open`, `upload`, `provision`,
  `scan`, `check` and `ask`, and the recorded walkthroughs (✦). A walk plays
  at the layer that is about it (`open` at the network, hop by hop;
  `provision` at the infrastructure; `ask` at the apps), from part to part, so
  the picture stays put; **Open** walks a step inside its box, or the hops it
  passes at their own layer, **◂ Back** returns, and the card's layer select
  shows the walk at any layer. Each step is framed at a readable size, and the
  card tells what happens: each call with why and what it sends, each answer
  going back (a hollow token), and what the step has at its end. Closing the
  walk restores your layer.
* **Hide what you don't care about**: the eye next to each part in the tree,
  or *Hide this section* in its details. Hidden parts are chips above the
  picture; click one to bring it back. Inside a hidden section, a part's eye
  shows just that part (its siblings are hidden instead); *Show only this*
  keeps one part and the groups around it. `[view] hide` and `[view] show` in
  docair.toml set the default.
* **Words that explain themselves**: terms of [docs/GLOSSARY.md](docs/GLOSSARY.md)
  are underlined where the docs use them. Hover for the short definition,
  click for the whole entry; *Read more* opens the linked section of the SPEC
  inside the page. **Glossary** in the header lists every term. `docair
  glossary` (or *✦ Explain the terms* in the web app) lets the model add the
  terms that are missing.
* **Code and data**: parts are drawn where they run (`@runs-in browser` for a
  web app a container serves); **◇ Data** lists what users and the code make
  and carry (docs/DATA.md), and *Follow* walks a datum's way, e.g. your
  question from the chat panel through the network to the model.
* **Infrastructure from its code**: Terraform resources, Helm releases and
  Kubernetes manifests become parts even without doc blocks; the page shows
  what Terraform creates (cluster, namespaces, releases), what each release
  contains, and how requests are routed (Ingress → Service → Deployment).
* **API descriptions** ([docs/IMPORTS.md](docs/IMPORTS.md)): OpenAPI and
  AsyncAPI files next to the code merge into the handlers they describe;
  callers' lines carry their types ("· from api/openapi.yaml"), routes no code
  serves stay inside the description (⎘), and `docair check` warns where the
  description and the code disagree (W114, W115). docair-server's own
  routes are described in `crates/docair-server/openapi.yaml`.
* **Words on the lines**: every line says what happens along it; `docair word`
  (or *✦ Word the lines*) lets the model write the words.
* **Tests** (header button) shows or hides the e2e tests: each one is linked
  to what it covers and coloured by its last result; every part lists the
  tests that cover it under *Tested by*.

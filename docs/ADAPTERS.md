# Adapters: code first

Status: **agreed 2026-11-17**, being built step by step (§11; beads epic
`docair-4wk`). SPEC sections change as each step lands; until then SPEC
describes what is built.

docair used to draw what the doc blocks say. This design turns that around:
**the code says what exists and what talks to what; the blocks explain it.**
Adapters become the strongest part of docair. Doc blocks are for what code
cannot say: why, what it means, the words on the lines, the scenarios, and the
parts that have no code (external systems, data, people).

---

## 1. Where we are

Measured on docair's own repository (`docair scan .`, 2026-11-17):

| | |
|---|---|
| Edges in the graph | 281 |
| Inferred from code | 5, all from the infra adapter (2 Service → Deployment, 3 Terraform refs) |
| Hand-written calls between functions inside Rust and TypeScript | about 100 |
| Hand-written test coverage (`@covers`) | 88 |
| Rust functions / Rust doc blocks | about 340 / 95 |

How the code is built today, and what holds it back:

| Today | Consequence |
|-------|-------------|
| Everything flows through `RawBlock`. Adapters fill in blocks that exist, or make fake blocks for discovered code; code facts become string tags (`Tag::inferred("http", "POST /x")`). | Code facts are second-class: parsed from strings, merged by string. |
| One adapter per file: the first that `handles` it. | Frameworks can't add up: Rust + axum + sqlx on one file is impossible. |
| The Rust adapter only looks at items that have a block. | Code without a block is invisible. |
| No TypeScript adapter. TS gets keyword guesses from `adapter::fallback` in core, which also knows Terraform, Kubernetes and Playwright. | Core is no longer language-agnostic. |
| The infra adapter needs the whole project, so `lib.rs` special-cases it. | The trait can't express project-wide analysis. |
| A discovered node with the same id as a block is dropped (`build.rs`). | When both exist, the block's version wins and the code's facts are lost. |
| W103 checks only `@http`; a tree-sitter parse failure is also reported as W103. | Blocks can go stale without a warning. |

---

## 2. Principles

1. **Code is the source of truth** for what exists, where it is, and what it
   calls, serves, reads, writes, sends and receives. Blocks explain, and add
   what code cannot show.
2. **Adapters produce facts, never graph.** One typed fact model. Only the core
   turns facts and blocks into nodes and edges.
3. **Languages meet at rendezvous points**: an HTTP route, a topic, a store, a
   Kubernetes object, a Terraform address, an environment variable. No adapter
   knows another adapter.
4. **Every fact has evidence** (file, line, rule) and a reach: certain, one of,
   or could (§5.5). Nothing is
   invented: when an adapter can't tell, it leaves the fact out and says so in
   a diagnostic.
5. **Extending is adding data.** A framework on a known language is one rule
   file and its fixture. A new language is one crate.
6. **Every rule proves itself** with a fixture. docair's own repository is the
   regression benchmark.

---

## 3. The pipeline

```
 files ─► 1 discover ─► 2 parse ──────► 3 recognize ─► 4 link ─────────► 5 merge ────► 6 build+resolve ─► 7 check ─► 8 emit
                        per file,       rules on the   per language,     facts + doc    nodes, edges,     diagnostics  graph.json
                        in parallel:    syntax tree:   whole project:    blocks: blocks flows, data                    docair.html
                        symbols, spans  facts          names, prefixes   annotate code
                                                            │
                        2b extract doc blocks ──────────────┘  (unchanged: comment syntax, tag grammar, Markdown)
```

| # | Stage | Who | What it does |
|---|-------|-----|--------------|
| 1 | discover | core | unchanged |
| 2 | parse | language adapter | parses each file once (tree-sitter) and lists its **symbols**: name, kind, span, parent, signature, native doc comment (`///`, `/** */`) |
| 2b | extract | core | unchanged: doc blocks, tags, Markdown |
| 3 | recognize | rules | framework rules match patterns in the syntax tree and emit **facts**: this function serves `POST /api/scan`; this method requests `POST /api/scan` |
| 4 | link | language adapter | resolves names across files (imports, `use` paths, `this.field`), composes route prefixes (axum `nest`), turns call sites into calls between symbols; with a SCIP index (§5.4), the language's own tooling resolves every call |
| 5 | merge | core | attaches each block to its symbol and applies the precedence rules (§6) |
| 6 | build, resolve | core | symbols become nodes, relations become edges; rendezvous points are matched across languages (§7) |
| 7 | check | core | today's rules plus the new code-vs-block checks (§9) |
| 8 | emit | core | unchanged schema, plus evidence on edges (optional fields, no schema bump) |

---

## 4. Facts: the contract between adapters and core

Rust sketch; names may still change.

```rust
pub struct Facts {
    pub symbols: Vec<Symbol>,
    pub relations: Vec<Relation>,
    pub serves: Vec<Serve>,
}

/// Something that exists in the code.
pub struct Symbol {
    pub key: SymKey,                  // "rust:docair-server::scan_upload"; stable, qualified, per language
    pub kind: CodeKind,               // function, method, type, module, package, component, test, …
    pub name: String,                 // scan_upload
    pub display: String,              // fn scan_upload
    pub parent: Option<SymKey>,       // containment: package › module › type › function
    pub span: Span,                   // file, line of the item itself (after attributes and decorators), end line
    pub signature: Option<Signature>,
    pub docs: Option<String>,         // native doc comment
    pub evidence: Evidence,
}

/// A symbol uses something: a call, a request, a read, a write, an event.
pub struct Relation {
    pub from: SymKey,
    pub to: Vec<Target>,              // one target, or the candidates (§5.5)
    pub reach: Reach,
    pub kind: EdgeKind,               // the existing kinds: call, data, read, write, emit, consume, …
    pub payload: Option<String>,      // what travels: CreateOrder
    pub reply: Option<String>,        // what comes back: Order
    pub evidence: Evidence,
}

/// A symbol is the thing a rendezvous point leads to: it serves a route, consumes a topic, …
pub struct Serve { pub by: SymKey, pub at: Meet, pub evidence: Evidence }

pub enum Target {
    Symbol(SymKey),                   // linked by the language adapter
    Meet(Meet),                       // matched by the core, across languages
}

/// Rendezvous points: where two sides meet without knowing each other.
pub enum Meet {
    Http { method: String, path: PathPattern },
    Topic(String),
    Store { name: String, part: Option<String> },
    K8s { kind: String, name: String },
    Tf(String),
    Env(String),
}

/// What a line can say about where a call goes (§5.5).
pub enum Reach {
    Certain,  // exactly `to[0]`: the syntax says it, names were followed (mod, use,
              // self, inject), or a compiler-grade index says it (§5.4)
    OneOf,    // exactly one of `to`, decided at runtime: a trait or interface method
              // and its implementations
    Could,    // any of `to`, or something outside the project: a value of unknown
              // type, a path known only in part, a glob import
}

/// Where a fact comes from: the rule, and the places it was proven from, in order
/// (e.g. the call `config::load(root)` at lib.rs:99, then `mod config;` at lib.rs:21).
pub struct Evidence { pub rule: String, pub at: Vec<SourceRef> }
```

Why this shape:

* **Adapters never touch the graph.** They can't break ids, parents or merging
  for other languages, and the core can be tested without any adapter.
* **`Meet` is a small, closed vocabulary in the core.** Matching stays sound in
  one place. A new kind of rendezvous is a core change, and that is rare.
* **`Reach` keeps "nothing is invented" true.** A line says exactly what is
  known: *calls*, *calls one of*, or *could call*. A could line is a true
  statement, not a failure; a scenario or a block says what happens (§5.5).
  Only a certain line can confirm or contradict a block.

**Built so far (step 2, 2026-11-17):** `crates/docair-core/src/facts.rs` has
`Symbol`, `Serve { by, at: Meet }` with `Meet::Http`, `Relation` (its `verb`
named like the tags, `Target::Ref` for node ids and refs such as
`tf:helm_release.x`) and `Facts`. The trait is `docair_core::adapter::Adapter`
with `file` (one file) and `project` (the whole project, for references
across files). `merge.rs` attaches blocks to symbols and still hands build and
resolve the blocks they read before. `Reach` and candidate targets come with
step 7; more kinds of `Meet` with step 6 and the imports (beads epic
`docair-58a`, which also adds a `Declare` fact for API descriptions).

---

## 5. Adapters and rules

An adapter has two layers.

### 5.1 Language adapter: one crate per language

Knows the language: parsing, symbols, modules and imports, call sites, types
of fields where the syntax says them. Written in Rust.

```rust
pub trait LanguageAdapter: Send + Sync {
    fn id(&self) -> &'static str;                         // "rust"
    fn handles(&self, file: &SourceFile) -> bool;
    /// One file: parse, list symbols, run the rules. Pure, so files run in parallel.
    fn file(&self, file: &SourceFile, rules: &RuleSet) -> FileFacts;
    /// All files of this language: resolve names across files, compose prefixes.
    fn link(&self, files: Vec<FileFacts>, project: &Project) -> Facts;
    /// What this adapter sees completely (§9.3). Default: nothing.
    fn complete(&self) -> &[Completeness] { &[] }
}
```

The infra adapter fits the same trait: `file` reads Terraform blocks and
Kubernetes objects, `link` follows references across files. That removes the
special case in `lib.rs`.

### 5.2 Rules: one file per framework pattern

Knows a framework: axum routes, Angular `HttpClient`, `fetch`, sqlx queries,
Kafka producers, Playwright tests. Rules are **data**: an
[ast-grep](https://ast-grep.github.io/) pattern (the same YAML rule language as
the `sg` tool, so patterns can be tried in its playground) plus a `facts:`
section that says what a match means.

ast-grep-core and ast-grep-config 0.51 use tree-sitter 0.27, the version docair
already uses, and serde-saphyr for YAML, which LIBRARIES.md already chose.

```yaml
# crates/docair-lang-rust/rules/axum.route.yaml
id: rust.axum.route
language: rust
rule:
  pattern: $ROUTER.route($PATH, $METHOD($HANDLER))
constraints:
  METHOD: { regex: "^(get|post|put|patch|delete|head|options)$" }
facts:
  - serves: http
    method: $METHOD
    path: $PATH
    by: $HANDLER            # a name; the Rust adapter links it to the fn item
```

```yaml
# crates/docair-lang-ts/rules/angular.http-client.yaml
id: ts.angular.http-client
language: typescript
rule:
  any:
    - pattern: this.$CLIENT.$METHOD<$REPLY>($URL, $$$)
    - pattern: this.$CLIENT.$METHOD($URL, $$$)
constraints:
  METHOD: { regex: "^(get|post|put|patch|delete|head)$" }
where:
  CLIENT: { type: HttpClient }   # asked of the TS adapter: is this field an HttpClient?
facts:
  - requests: http
    method: $METHOD
    path: $URL
    reply: $REPLY
    from: enclosing              # the symbol the match is in
```

`where:` is docair's one addition to ast-grep: it asks the language adapter
about what a pattern can't see, such as a field's type or the enclosing
symbol.

When a pattern isn't enough, a rule can be Rust code behind the same
interface. Examples: axum's `get(a).post(b)` method chains, `nest("/api", …)`
prefixes carried through router variables, Helm templating. That is the escape
hatch, not the default.

**Built so far (step 3, 2026-11-17):** crate `docair-rules` (ast-grep
`=0.51.1`). A rule file has `id`, `language`, ast-grep's `rule`,
`constraints`, `utils` and `transform`, and `facts`. A fact is `serves: http`
with `method` and `path` templates (`$NAME`; a string literal gives its
content; the method is upper-cased) and `by`: `match` (the matched node),
`next` (the next sibling of another kind: the item an attribute belongs to) or
`parent`. A rule fails to load when its language is wrong, it says no facts,
or a template names a variable the rule doesn't define. The first rule is
`crates/docair-lang-rust/rules/actix.route.yaml`, which replaced the
hand-written `route_of`.

**Added in step 4:** `by: enclosing` (the symbol the match is written in) and
`by: $NAME` (the function a captured name names: in the same file, else the
only one of that name in the crate; no fact when that's unclear); a `calls:
$NAME` fact with a `payload` template; ast-grep `transform` variables in
templates (e.g. `convert … toCase: upperCase`); a fact whose variables a
match left unbound is skipped. Rule `axum.route.yaml`: `.route("/x",
get(a))`, also `get(a).post(b)` chains, gives the routes the handlers serve
and the router's calls to them. In an `any`, list the longest pattern first:
ast-grep keeps the first pattern that fits and doesn't try the next one when
a constraint then fails. Not yet: axum `nest` prefixes (beads, a follow-up of
step 4); `requests` and `where:` come in step 5.

### 5.3 Registry

One crate, `docair-adapters`, lists every language adapter with its built-in
rules. The CLI and the server both use `docair_adapters::all()`, so they can't
drift apart. Later, a project can add its own rules through `[rules] dirs` in
docair.toml (for a team's in-house HTTP wrapper, for example).

### 5.4 Who calls whom: one index format for every language

Following a call to the function it reaches needs each language's own rules:
Rust's modules, traits and macros; TypeScript's imports, classes and types.
docair does not re-implement them per language. It reads
[SCIP](https://github.com/scip-code/scip), an index format that each
language's own compiler-grade tooling writes: `rust-analyzer scip` for Rust,
`scip-typescript` for TypeScript and JavaScript, and indexers for most other
major languages. One reader in docair (`docair-scip`) turns any of them into
call relations that are certain, or one of a trait's implementations.

Checked on docair itself (2026-11-17, rust-analyzer 1.100, 4.5 s for the
workspace):

| Call | tree-sitter alone | SCIP index |
|------|-------------------|------------|
| `config::load(root)`, core `lib.rs:99` | certain, through `mod config;` | `docair-core config/load()` |
| `parser.parse(…)`, lang-rust `lib.rs:43` | type unknown; a name match picks `glossary::parse` (wrong) | tree-sitter's `Parser::parse` (outside the project, not drawn) |
| `adapter.enrich(…)`, `adapter.rs:43` | `&dyn Adapter`: which one is unknown | the trait method `Adapter::enrich`; its implementations come from the index too |

Division of work:

| Source | Gives | Needs |
|--------|-------|-------|
| tree-sitter adapter + rules | symbols, spans, framework facts (routes, requests), calls the syntax makes certain | nothing: works on any folder, also an upload that cannot build |
| SCIP index | every call and reference, with types | the project's toolchain: Cargo and its dependencies, Node and `node_modules` |

`docair scan` runs the indexers it finds installed, or reads index files named
in docair.toml (built in CI, for example). Without an index, the syntax still
shows many types: Rust declares them on parameters and fields, and
`let mut parser = Parser::new()` with `use tree_sitter::Parser` makes
`parser.parse(…)` a certain call outside the project. Calls whose target the
syntax can't pin down become could lines (§5.5).

### 5.5 Could and does: scenarios pick

Code shows what **could** happen. Scenarios and blocks say what **does**
happen.

| Reach | When | Drawn as | Example in docair |
|-------|------|----------|-------------------|
| certain | one target | a call | `config::load(root)` |
| one of | a trait or interface method: one of its implementations runs | *calls one of 2*, a fork to each | `adapter.enrich(…)` → `RustAdapter::enrich` or `InfraAdapter::enrich` |
| could | the type is unknown (an upload without an index, `any`, `obj[name]()`, plain JavaScript), or a path is known only in part | *could call*, dashed, to each candidate | ``page.request.get(`${fakeAi}/requests`)`` → every route ending in `/requests`, here fake-ai's |

**Scenarios pick.** When a `@flow` step follows a step at the caller, the
path search (SPEC §7.3) already takes the line that leads to it. During the
walk that line is drawn as a call, and the other candidates fade. docair's
`scan` scenario does this today: step 6 is `docair-core.adapter.enrich`, step 7
("tree-sitter parses Rust") is `RustAdapter::enrich`.

**Blocks pick for good.** `@calls` naming one candidate turns that candidate's
line into a call everywhere. Such a tag is a clarification, never a repeated
tag.

**Candidates must fit what is known**, or a could line would be false:

* A method call (`x.parse()`) reaches only methods, never free functions.
* Names that standard-library types also have are left out. docair has 167
  `.is_empty()` calls; the index shows that one reaches docair's own
  `ViewChange::is_empty` and 166 are Rust's. Matched by name, all 167 would be
  could lines to it. Without an index, that one true call is drawn as
  nothing; with an index, it is certain.
* A path known in part matches only routes that fit every known segment.
* A call with more candidates than `[code] max_candidates` (default 5) draws
  no lines; the details panel lists them instead.

**Not could lines:**
* A function handed over to a framework (`post(scan_upload)`) is certain
  through the framework's rule.
* A request to a host that only configuration names
  (`format!("{}/chat/completions", cfg.endpoint)` in `docair-ai`) is a certain
  request to an unknown host. The block says which system that is. That is
  what blocks are for.

---

## 6. Merging code and blocks

A block attaches to the symbol whose span starts at the block's following line.
That is today's attachment rule (SPEC §2.1), now done once in the core for
every language. A block with no symbol (a free block, Markdown, a `docair`
fence) declares a node, as today.

| What | Code says | Block says | Result |
|------|-----------|------------|--------|
| that it exists, where | symbol, span | — | code |
| id | from the qualified path | `@id` | block: a stable name for links and anchors |
| kind | from the syntax and rules | `@kind` | block, if given: a clarification |
| parent | containment | `@part-of` | block, if given: regrouping is a clarification |
| summary, description | native doc comment | block text | block, else the native doc comment |
| signature | parameters, return type | — | code |
| route | `serves http …` | `@http` | **code**; if they differ: stale block (W103) |
| edges | relations | `@calls`, `@in`, `@reads`, … | union; for the same pair: one edge. **The code's facts on it always show** (`runs after` from a Terraform `depends_on`); the block's words add to them; words that already say what the code says show once (`(POST /api/scan: files) → ScanResult` already says `POST /api/scan`). A call's payload in the block (`(folder, Rust adapter, lenient)`) words the same parameters as the code's signature (`(root, adapters, opts)`), so it is used; the signature fills in when the block says nothing |
| words on a line | — | `"…"` on the tag | block |
| scenarios | — | `@flow` | block |

Four outcomes for each authored edge tag:

| Outcome | When | What docair does |
|---------|------|------------------|
| **confirmed** | code has the same edge | one edge; the tag adds its words and its wording of what travels. If it adds nothing, it is a **repeated tag** (§6.1) |
| **stale** | code contradicts it (another route), or the adapter is complete for that relation (§9.3) and finds no such edge | warning (W103), also in strict mode; code wins |
| **adds** | code can't see it (dynamic dispatch, an external system, data) | the block's edge, `origin: authored` |
| **picks** | code has it as one of several candidates (one of, could) | the candidate's line becomes a call everywhere; a clarification, never repeated |

### 6.1 Tidy: removing repeated tags

**Decided (2026-11-17):** docair offers tidying in the CLI, and very visibly
in the UI.

A tag is **repeated** when a `Certain` fact says the same and the tag adds
nothing: no words (`"…"`), and no wording of what travels that differs from
the code. Only a certain line makes a tag repeated. Tidy never touches `@id`,
`@kind`, `@part-of`, `@flow` or the block's text.

Every scan finds the repeated tags, without AI, and puts them into the graph
as proposed edits (`tidy`: the same `Change` model as the AI's edits, SPEC
§10.6), each with the code that says the same.

| Where | What it shows |
|-------|---------------|
| header | a **✂ Tidy · 58** button next to the diagnostics, in the accent colour while something can be tidied; **✂ Tidy ✓** when nothing repeats |
| on load | a banner under the header while repeated tags exist: "58 tags repeat what the code says. **Review and tidy**". Closing it hides it for this viewer until the count changes |
| tidy panel | the repeated tags by file and block, each with the code that says the same (`.route("/api/scan", post(scan_upload))`, lib.rs:104) and a checkbox, all ticked; the diff of the result |
| details, Tags tab | a repeated tag is marked **✂ the code says this**, linked to that code, with *Tidy this tag* |
| in the web app | **Apply** writes the edits through `/api/patch` and regenerates the page, like the AI's edits |
| page opened as a file | **Copy command** (`docair tidy --apply`) and **Download patch** |
| CLI | `docair tidy` lists them and writes `.docair/docair.patch`; `--apply` writes them into the files. `docair scan` and `docair check` end with one line: "58 tags repeat the code: run `docair tidy`" |

---

## 7. Linking across languages

The core matches `Meet`s: a request for `POST /api/scan` meets the symbol that
serves `POST /api/scan`.

* **Path algebra lives in the core**: segments, parameters (`{id}`, `:id`,
  `${id}`), and prefixes composed from routers and mounts. Adapters hand over
  strings plus the prefixes they know.
* **Unknown prefixes**, such as `${environment.apiUrl}/orders`, match by the
  known tail: could lines to every route that fits (§5.5).
* **Ambiguity is reported, not resolved by luck.** Two services serving the
  same route without infrastructure that tells them apart is a warning.
* **Through the network.** A browser request to `/api/scan` goes to the page's
  origin and reaches the handler through the published port, Traefik, the
  Ingress and the Service. The infra adapter already knows that path (Ingress
  `@routes`, Service `@selects`). The edge goes client → handler with
  `via http:POST /api/scan`, and the Network layer keeps drawing the hops. How
  exactly those two fit together is settled in step 6 of the plan.

---

## 8. What becomes a node

**Decided (2026-11-17): everything, for now.** Every symbol an adapter finds
becomes a node. On the Code layer, a module of functions only stands closed
until it is opened (SPEC §3.4), so the picture starts small.

**Built (step 4):** a symbol says when it is drawn with `Draw`: `Documented`
(only with a block), `Always` (the block's node if one documents it, else a
node of its own) or `Alone` (always its own node; what a Terraform resource
creates). The Rust adapter draws every item `Always`, except impl blocks
(their methods sit inside the type), unit tests and items inside function
bodies. On docair itself that took the graph from 277 to 758 nodes.

**Decided (2026-11-17): fold them.** Measured on docair's page, the new
parts doubled the Code picture's area (176 → 408 parts) but added only 8
lines, because calls between functions only come with step 7. So a box that
holds only discovered parts stands closed until it is opened, like a module
of functions only (SPEC §3.4). The parts stay in the graph, the tree and the
search; opening the box shows them. Built in the viewer.

If it gets too crowded, this comes up again. What was looked at:

| Rule | Verdict |
|------|---------|
| serves or requests a rendezvous point; entry points; has a doc block | good |
| called from another module, except leaves (call no other drawn function, touch nothing outside) | good; needs the calls (step 7) |
| touches files, network, environment, processes | good |
| `pub` / `export` | no: says who may call, not who does. 53 of 75 `pub` functions are used only in their own crate, and the `POST /api/scan` handler is not `pub` |
| called from many places | no: ranks utilities first. `slug` has 32 call sites, `docair_core::scan` 2 |

---

## 9. Reliability

### 9.1 Fixtures

Every rule has a fixture next to it. The expected facts are written inline
after the code, in the language's comment syntax:

```rust
// crates/docair-lang-rust/rules/axum.route.fixture.rs
let app = Router::new()
    .route("/api/scan", post(scan_upload))
    .route("/healthz", get(health));
//> serves http POST /api/scan by fn scan_upload
//> serves http GET /healthz by fn health
```

One test runner loads every rule file and checks it against its fixture. A
rule without a fixture fails `cargo test`. Facts print as sorted lines, so a
change shows as a readable diff.

### 9.2 Golden graphs

`docair.graph.json` of docair itself and of the fixtures `shop` and `platform`
are snapshot tests. A refactor that changes the graph must say why in the
snapshot diff.

### 9.3 Completeness

An adapter may declare a relation **complete** for a scope. For example: "every
axum route in a crate is seen", or "every direct call between free functions of
one crate is seen". It declares that only when its fixtures prove it. Only then
can a missing code edge make a block stale. Without such a claim, docair
confirms but never contradicts.

### 9.4 Diagnostics

| Code | Change |
|------|--------|
| `W103` | widened: any block/code conflict (route, a missing edge under completeness), not only `@http`; a warning also in strict mode |
| new | a request that nothing serves: `POST /api/x` from `scan-service.scan`, while some adapter serves HTTP in this project |
| new | a route that nothing requests (off by default: public APIs are requested from outside) |
| new | ambiguous rendezvous: two symbols serve the same route |
| new | a file that could not be parsed (today wrongly W103) |
| new | a rule that failed to load or match, with its id |

### 9.5 Measured on ourselves

`docair check --coverage` prints, per adapter: facts found, authored edges
confirmed, stale, and added. The numbers in §1 are the baseline. Every step
below says which number it moves.

---

## 10. Crates

```
crates/
  docair-core           model, facts, merge, build, resolve, check, emit (no language knowledge)
  docair-rules          the rule format (ast-grep + facts), the matcher, the fixture runner
  docair-lang-rust      symbols, modules, use paths, calls; rules/: actix, axum, reqwest, sqlx, …
  docair-lang-ts        TS/JS: symbols, classes, decorators, imports, calls; rules/: angular, http-client, fetch, playwright, …
  docair-lang-infra     Terraform, Helm, Kubernetes (today's core/src/infra.rs)
  docair-lang-generic   keyword guesses for languages without an adapter (today's adapter::fallback)
  docair-scip           reads SCIP indexes (any language) into call relations; runs the installed indexers
  docair-adapters       the registry: all()
```

---

## 11. Plan

Each step is small, keeps every test green, and ends with something measurable.

| # | Step | Done when |
|---|------|-----------|
| 1 | Agree on this proposal; update SPEC §1/§6/§8 and ARCHITECTURE | the docs agree with each other |
| 2 | Facts model and merge in core; port the Rust and infra adapters onto it; move `fallback` to `docair-lang-generic`; registry crate. **No new inference.** | the golden graphs of docair, `shop` and `platform` are unchanged |
| 3 | `docair-rules`: rule format, matcher, fixture runner; actix attributes become a rule | the actix rule is data plus a fixture; a rule without a fixture fails the tests |
| 4 | Rust: symbols without blocks, native `///` docs, axum routes (`nest`, method chains) | docair-server's 7 routes come from code; removing the router's 7 `@calls` lines changes nothing in the graph |
| 5 | TypeScript adapter (tree-sitter-typescript): symbols, Angular decorators, `inject()`, `HttpClient` and `fetch` rules, Playwright tests moved out of core | docair-web's requests come from code |
| 6 | Cross-language HTTP matching, the new diagnostics, the network path | web → server lines are inferred end to end in docair itself; renaming a route on one side warns |
| 7 | Calls: `docair-scip` reads rust-analyzer and scip-typescript indexes; without an index, only calls the syntax makes certain | most of the ~100 hand-written internal calls come from code (`--coverage`); `parser.parse` never links to `glossary::parse` (test) |
| 8 | Tidy (§6.1): CLI, header button, banner, panel, Tags tab, apply in the web app; run it on docair itself | docair's blocks hold explanations and what code can't show |
| later | a cache per file hash; project rule dirs; more frameworks; more SCIP languages | |

---

## 12. Open questions

1. ~~Conflicts~~ **Decided (2026-11-17):** a stale block is a warning, also in
   strict mode.
2. ~~Default granularity~~ **Decided (2026-11-17):** draw everything for now
   (§8).
3. ~~Guessed facts~~ **Decided (2026-11-17):** there are no guesses. A line
   is certain, *one of*, or *could*; scenarios and blocks pick (§5.5). Could
   lines are drawn, dashed, like everything else (§8).
4. ~~docair's own blocks~~ **Decided (2026-11-17):** tidy, from the CLI and
   very visibly from the UI (§6.1).

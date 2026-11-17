# Imports: API descriptions

Status: **built 2026-11-17** (beads epic `docair-58a`). SPEC §6.2 describes
what docair does; this file explains why it is built this way.

An OpenAPI or AsyncAPI file says which routes a service answers and which
topics an application sends to and receives from, with what travels there.
docair reads these files as one more adapter. The code still decides what
exists ([ADAPTERS.md](ADAPTERS.md)); a description adds the types and the
words, and where the two disagree, docair warns.

---

## 1. What docair reads

| Files | Found by |
|-------|----------|
| YAML | a top-level `openapi:`, `swagger:` or `asyncapi:` key, looked for without parsing (Helm templates are not YAML and are never parsed) |
| JSON | the name: `*openapi*.json`, `*swagger*.json`, `*asyncapi*.json` (case ignored), or `[import] apis` in docair.toml; other JSON files, such as `package-lock.json`, stay unread |

Versions: OpenAPI 3.0, 3.1 and 3.2, Swagger 2.0, AsyncAPI 2.x and 3.x. The
files are parsed with serde-saphyr into a tree whose every node keeps its
line, so each fact points at its place in the file. JSON parses as YAML 1.2.

**OpenAPI.** Each operation is a route the service answers:

| In the description | Read as |
|--------------------|---------|
| path and method | the route, with the base path in front: the path of `servers[0].url` (`{var}` takes its default, a relative URL is fine, none means `/`); servers on a path or an operation override it; Swagger 2.0 uses `basePath`; `[[import.api]] base` overrides all of them |
| request body (`$ref` → its name, `Order[]` for arrays, an inline schema's `title`; 2.0: the `in: body` parameter) | what the request sends; without a body, the names of the path and query parameters |
| the first 2xx response's schema | what comes back |
| `operationId`, `summary`, `description` | the operation's name and text |

**AsyncAPI.** Each send or receive is a line between the application and a
topic. In 3.x the operation's `action` says `send` or `receive` and its
channel's `address` is the topic (the channel's key when there is no
address). In 2.x the channel's key is the topic, and the verbs are named from
the other side: `subscribe` lists what the application **sends**, `publish`
what it **receives**. The message's `name`, else its `title`, else its
payload's `$ref` name is what travels.

Not read, with a W113 at the place: a path item that is a `$ref` into another
file, OpenAPI 3.2's `query` method and `additionalOperations`, an AsyncAPI
operation whose channel is in another file.

## 2. Facts

A description **declares** a rendezvous point; it does not serve it. A
handler and its description are not two servers of one route.

```rust
pub struct Declare {
    pub by: String,               // the description's symbol
    pub at: Meet,                 // Meet::Http or Meet::Topic
    pub payload: Option<String>,  // CreateOrder; for topics, the message
    pub reply: Option<String>,    // Order
    pub name: Option<String>,     // the operationId
    pub role: Role,               // Serves, Sends, Receives
    pub summary: Option<String>,
    pub description: Option<String>,
    pub evidence: Evidence,
}
```

The description file itself is a symbol of kind `api`. What an AsyncAPI
application sends and receives is also a relation of that symbol: its lines
to the topics.

## 3. Meeting the code

`crates/docair-core/src/declare.rs`, three steps in `scan()`:

1. **Blocks** (before build). Each declared route becomes an endpoint inside
   its description (`<description>.<operationId>`); each channel becomes the
   topic `topic:<address>` at the top, the id `@emits topic:…` already
   resolves to, with the channel's text.
2. **Settle** (after build).
   * The description goes into its service: `[[import.api]] part_of`, else
     `x-docair-part-of` in the file, else the closest service or job declared
     in the description's directory or above it (`api/openapi.yaml` next to
     `api/Cargo.toml`).
   * An endpoint merges into a handler of that service that serves the same
     route, parameter names aside (`{id}` is `{orderId}`), with or without the
     base path. The handler gets the operation as a doc, and the inferred tags
     `described` (`POST /api/orders (CreateOrder) -> Order`) and `operation`.
   * An endpoint whose `operationId` names a function of the service merges
     into it although the routes differ: the description's route is added as
     an inferred `@http`, so W103 shows both.
   * A description's line to a topic yields to the same line from code inside
     the service: the code's line is the one drawn.
   * A description whose every route merged and every line yielded goes; one
     an author documented with a doc block stays.
3. **Words and pruning** (after resolve). A call that reaches a described
   route through `http:` and says nothing of what it sends takes the
   description's request and answer; a line to or from a topic takes its
   message. Such an edge carries `from_api` (the file), and the page says
   "· from api/openapi.yaml". Inside an `external` part, endpoints nothing
   calls are dropped, so a vendored description with hundreds of operations
   adds only the ones the code uses.

No new node fields: what a description says lives in tags, docs and
`Edge.from_api`, which the viewer already shows.

## 4. Warnings

| Code | When |
|------|------|
| `W113` | a description cannot be read, or a part of it is not read |
| `W114` | the description declares a route, or a send or receive, that no code in its service has; only for services that have such code (never for `external` parts). A description whose routes are served in another service says where, and to set `part_of` |
| `W115` | code in a described service has a route, or a send or receive, that its descriptions leave out; `[[import.api]] ignore` lists routes left out on purpose |
| `W103` | a route renamed on one side: the `operationId` names the function, the routes differ |

## 5. Configuration

```toml
[import]
apis = ["contracts/*.json"]     # more JSON files to read, besides the default names

[[import.api]]
file    = "api/openapi.yaml"    # one description
part_of = "orders-api"          # the service it describes
base    = "/api"                # instead of the servers' path
ignore  = ["GET /healthz"]      # routes left out on purpose (W115)
```

## 6. Tests

| Where | What |
|-------|------|
| `crates/docair-lang-apis` | the parser (OpenAPI 2.0 to 3.2, AsyncAPI 2.6 and 3.0 give the same events), and `tests/declares.rs`: the fixture's declarations as sorted lines |
| `crates/docair-core/src/declare.rs`, `route.rs` | merging, placement, warnings, words; route algebra |
| `crates/docair-adapters/tests/fixtures.rs` | the golden graph of `e2e/fixtures/apis`, and its planted drift: exactly one W103, two W114, one W115 |
| `crates/docair-server/openapi.yaml` | dogfooding: docair-server's seven routes, merged into its handlers |
| `e2e/tests/apis.spec.ts` | the page of the fixture: descriptions drawn as `api` parts, the caller's line in the description's words, the warnings |

## 7. Not yet

* Clients generated from a description (openapi-generator names its methods
  after the `operationId`): a TypeScript adapter could follow calls through them.
* `$ref`s into other files, beyond taking the name.
* Exports (Mermaid, LikeC4, Backstage, Structurizr): planned, not started.

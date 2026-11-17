# Ref resolution

Every data-flow tag (`@in … from`, `@out … to`, `@calls`, `@reads`, `@writes`,
`@emits`, `@consumes`) holds a ref. Refs are resolved in this order:

1. `http:METHOD /path`: the node that declares a matching `@http`.
2. An exact node id.
3. `topic:<name>`: the topic node, created on first use.
4. The sub-part form `store.table`: the longest id prefix that exists. The
   rest becomes the edge label (`orders-db.orders` → edge to `orders-db`
   labelled `orders`).
5. Anything else becomes a placeholder node `unresolved:<ref>` plus `E001`,
   so a typo shows up in the picture instead of disappearing.

Two edges between the same two nodes in the same direction are merged into
one, and their labels are joined.

## Matching HTTP refs {#docair-core.resolve.match_http}

Paths match **by position**. A segment written as `{id}` or `:id` on either
side matches any segment, so the Angular call
`this.http.get('/api/orders/' + id)`, inferred as `GET /api/orders/:id`,
reaches the Rust handler declared as `GET /api/orders/{id}`. Query strings
are ignored, and so are leading and trailing slashes.

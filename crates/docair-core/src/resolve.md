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
one. The code's words stay on the line, an author's words add to them, and
words that say the same show once.

## Matching HTTP refs

A called path meets a declared route through the route module
(`docair-core.route`): the method must be the same and the path must fit
the route **for certain**. A segment written as `{id}`, `:id`, `<int:id>`
or `${id}` is a value and matches any segment, so `GET /api/orders/42`
reaches a handler declared as `GET /api/orders/{id}`. Query strings,
fragments, a scheme and a host are ignored, and so are empty segments. A
path whose start the code computes (`${base}/orders`) only *may* fit, so it
gets no line. Requests the code sends are matched the same way, after
resolve (`docair-core.request.link`).

//! Requests meet what serves them (docs/ADAPTERS.md §7).
//!
//! An adapter reports the HTTP requests code sends (`Request` facts, shown as
//! `@requests`); handlers and API descriptions say which routes they serve
//! (`@http`). This pass connects the two after resolve.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::model::{Diagnostic, Edge, EdgeKind, Node, Origin, SourceRef, Tag};
use crate::resolve::split_payload;
use crate::route::{Fit, Route};

/// The tag a request shows as on the node that sends it.
pub const REQUESTS: &str = "requests";

/// How far lines of calls, routes and selections are followed to see whether the graph already
/// routes a request (client → host port → … → Ingress → Service → router → handler).
const MAX_HOPS: usize = 8;

/// A request read back from its `@requests` tag: `POST /api/scan (form) -> ScanResult`.
struct Asked<'a> {
    from: &'a Node,
    tag: &'a Tag,
    method: String,
    path: String,
    payload: Option<String>,
    reply: Option<String>,
}

fn asked(node: &Node) -> Vec<Asked<'_>> {
    node.tags_named(REQUESTS)
        .filter_map(|tag| {
            let v = split_payload(&tag.value);
            let (method, path) = v.target.trim().split_once(char::is_whitespace)?;
            Some(Asked {
                from: node,
                tag,
                method: method.to_uppercase(),
                path: path.trim().to_string(),
                payload: v.sends,
                reply: v.receives,
            })
        })
        .collect()
}

/// Whether a request leaves for another origin (`https://api.example.com/x`): nothing in the project serves it.
fn elsewhere(path: &str) -> bool {
    path.starts_with("http://") || path.starts_with("https://") || path.starts_with("//")
}

/// The nodes lines of calls, routes and selections lead to from `from`, at most `MAX_HOPS` away.
fn routed_from<'a>(from: &'a str, out: &HashMap<&'a str, Vec<&'a str>>) -> HashSet<&'a str> {
    let mut seen = HashSet::from([from]);
    let mut queue = VecDeque::from([(from, 0)]);
    while let Some((at, hops)) = queue.pop_front() {
        if hops == MAX_HOPS {
            continue;
        }
        for &next in out.get(at).into_iter().flatten() {
            if seen.insert(next) {
                queue.push_back((next, hops + 1));
            }
        }
    }
    seen
}

fn label(payload: &Option<String>, reply: &Option<String>) -> Option<String> {
    match (payload, reply) {
        (Some(p), Some(r)) => Some(format!("({p}) → {r}")),
        (Some(p), None) => Some(format!("({p})")),
        (None, Some(r)) => Some(format!("→ {r}")),
        (None, None) => None,
    }
}

/***
 * Give every request the code sends a line to what serves it, unless the graph already routes it there.
 *
 * A route serves a request when the method is the same and the path fits it
 * for certain (`route::Route::fit`): a handler's `@http`, or an operation an
 * API description declares. When lines of calls, routes and selections
 * already lead from the client to the handler (through the network: host
 * port, Ingress, Service, router), the request only confirms them. Otherwise
 * the request becomes a call line, `via http:POST /api/scan`, like a
 * hand-written `@calls http:` ref. W116 when two parts serve the same request,
 * W117 when nothing in the project does (a request to another origin is left
 * alone); a request that only may fit (a path the code computes) gets
 * neither a line nor a warning yet.
 *
 * @in  graph: Node[] + Edge[] from docair-core.resolve.resolve
 * @out graph: Node[] + Edge[]
 ***/
pub fn link(nodes: &[Node], edges: &mut Vec<Edge>, diags: &mut Vec<Diagnostic>) {
    let servers: Vec<(&Node, Route)> = nodes
        .iter()
        .filter_map(|n| n.http.as_ref().map(|h| (n, Route::parse(&h.path))))
        .collect();
    let mut out: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in edges.iter() {
        if matches!(e.kind, EdgeKind::Call | EdgeKind::Route | EdgeKind::Select) {
            out.entry(e.from.as_str()).or_default().push(e.to.as_str());
        }
    }
    let mut new_edges = Vec::new();
    for node in nodes {
        let requests = asked(node);
        if requests.is_empty() {
            continue;
        }
        let routed = routed_from(&node.id, &out);
        let file = node.source.as_ref().map_or("", |s| s.file.as_str());
        for r in requests {
            if elsewhere(&r.path) {
                continue;
            }
            let wanted = Route::parse(&r.path);
            let fits: Vec<(&Node, Fit)> = servers
                .iter()
                .filter(|(n, _)| {
                    n.http
                        .as_ref()
                        .is_some_and(|h| h.method.eq_ignore_ascii_case(&r.method))
                })
                .map(|(n, route)| (*n, wanted.fit(route)))
                .filter(|(_, fit)| *fit != Fit::No)
                .collect();
            let certain: Vec<&Node> = fits
                .iter()
                .filter(|(_, fit)| *fit == Fit::Certain)
                .map(|(n, _)| *n)
                .collect();
            let at = |code: &str, msg: String| {
                Diagnostic::new(code, msg)
                    .at(file, r.tag.line)
                    .node(&r.from.id)
            };
            match certain.as_slice() {
                [] if fits.is_empty() => diags.push(at(
                    "W117",
                    format!(
                        "request `{} {}` from `{}`: nothing in the project serves it",
                        r.method, r.path, r.from.id
                    ),
                )),
                [] => {}
                [_, _, ..] => diags.push(at(
                    "W116",
                    format!(
                        "request `{} {}` from `{}`: {} parts serve it: {}",
                        r.method,
                        r.path,
                        r.from.id,
                        certain.len(),
                        certain
                            .iter()
                            .map(|n| format!("`{}`", n.id))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                )),
                [_] => {}
            }
            for server in certain {
                if routed.contains(server.id.as_str()) {
                    continue;
                }
                new_edges.push(Edge {
                    id: String::new(),
                    from: r.from.id.clone(),
                    to: server.id.clone(),
                    kind: EdgeKind::Call,
                    label: label(&r.payload, &r.reply),
                    via: Some(format!("http:{} {}", r.method, r.path)),
                    payload: r.payload.clone(),
                    reply: r.reply.clone(),
                    text: None,
                    carries: vec![],
                    carries_back: vec![],
                    evidence: None,
                    reach: None,
                    tag: format!("@{REQUESTS} {}", r.tag.value),
                    source: SourceRef {
                        file: file.to_string(),
                        line: r.tag.line,
                        end: None,
                    },
                    origin: Origin::Inferred,
                });
            }
        }
    }
    edges.extend(new_edges);
}

/***
 * W118: a route that nothing in the project requests (off by default: `[checks] unrequested_routes = true`).
 *
 * A route counts as requested when a request fits it, or a line with
 * `via http:` or a line of calls, routes and selections reaches it. Public
 * APIs and probes are requested from outside, so this is for projects whose
 * clients are all in the project.
 ***/
pub fn unrequested(nodes: &[Node], edges: &[Edge], diags: &mut Vec<Diagnostic>) {
    let reached: HashSet<&str> = edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Call | EdgeKind::Route | EdgeKind::Select))
        .map(|e| e.to.as_str())
        .collect();
    let asked: Vec<(String, Route)> = nodes
        .iter()
        .flat_map(asked)
        .map(|r| (r.method, Route::parse(&r.path)))
        .collect();
    for n in nodes {
        let Some(h) = &n.http else { continue };
        let route = Route::parse(&h.path);
        let requested = reached.contains(n.id.as_str())
            || asked
                .iter()
                .any(|(m, r)| m.eq_ignore_ascii_case(&h.method) && r.fit(&route) != Fit::No);
        if !requested {
            let (file, line) = n
                .source
                .as_ref()
                .map_or(("", 0), |s| (s.file.as_str(), s.line));
            diags.push(
                Diagnostic::new(
                    "W118",
                    format!(
                        "route `{} {}` of `{}`: nothing in the project requests it",
                        h.method, h.path, n.id
                    ),
                )
                .at(file, line)
                .node(&n.id),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Http;

    fn node(id: &str, http: Option<(&str, &str)>, requests: &[&str]) -> Node {
        Node {
            id: id.into(),
            kind: "function".into(),
            name: id.into(),
            summary: String::new(),
            description: String::new(),
            part_of: None,
            source: Some(SourceRef {
                file: format!("{id}.ts"),
                line: 1,
                end: None,
            }),
            symbol: None,
            http: http.map(|(m, p)| Http {
                method: m.into(),
                path: p.into(),
            }),
            deployed_as: None,
            docs: vec![],
            tags: requests
                .iter()
                .map(|r| Tag::inferred(REQUESTS, r, 3, "this.http.post(…)"))
                .collect(),
            origin: Origin::Discovered,
            test: None,
            signature: None,
            runs_in: None,
            code_from: None,
        }
    }

    fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
        Edge {
            id: String::new(),
            from: from.into(),
            to: to.into(),
            kind,
            label: None,
            via: None,
            payload: None,
            reply: None,
            text: None,
            carries: vec![],
            carries_back: vec![],
            evidence: None,
            reach: None,
            tag: String::new(),
            source: SourceRef {
                file: String::new(),
                line: 0,
                end: None,
            },
            origin: Origin::Authored,
        }
    }

    #[test]
    fn a_request_becomes_a_line_to_what_serves_it() {
        let nodes = vec![
            node("web.scan", None, &["POST /api/scan -> ScanResult"]),
            node("server.scan_upload", Some(("POST", "/api/scan")), &[]),
        ];
        let (mut edges, mut d) = (vec![], vec![]);
        link(&nodes, &mut edges, &mut d);
        assert!(d.is_empty(), "{d:?}");
        let [e] = &edges[..] else { panic!("{edges:?}") };
        assert_eq!(
            (e.from.as_str(), e.to.as_str()),
            ("web.scan", "server.scan_upload")
        );
        assert_eq!(e.via.as_deref(), Some("http:POST /api/scan"));
        assert_eq!(e.label.as_deref(), Some("→ ScanResult"));
        assert_eq!(e.origin, Origin::Inferred);
    }

    #[test]
    fn a_request_the_graph_already_routes_is_only_confirmed() {
        let nodes = vec![
            node("web.scan", None, &["POST /api/scan"]),
            node("host-port", None, &[]),
            node("ingress", None, &[]),
            node("server.router", None, &[]),
            node("server.scan_upload", Some(("POST", "/api/scan")), &[]),
        ];
        let mut edges = vec![
            edge("web.scan", "host-port", EdgeKind::Call),
            edge("host-port", "ingress", EdgeKind::Call),
            edge("ingress", "server.router", EdgeKind::Route),
            edge("server.router", "server.scan_upload", EdgeKind::Call),
        ];
        let mut d = vec![];
        link(&nodes, &mut edges, &mut d);
        assert_eq!(edges.len(), 4, "no new line");
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn renamed_routes_ambiguity_and_other_origins() {
        let nodes = vec![
            node(
                "web.client",
                None,
                &[
                    "POST /api/scna",
                    "GET /api/orders/42",
                    "GET https://api.example.com/x",
                    "GET ${base}/api/items",
                ],
            ),
            node("a.list", Some(("GET", "/api/orders/{id}")), &[]),
            node("b.list", Some(("GET", "/api/orders/:orderId")), &[]),
            node("c.items", Some(("GET", "/api/items")), &[]),
        ];
        let (mut edges, mut d) = (vec![], vec![]);
        link(&nodes, &mut edges, &mut d);
        let codes: Vec<_> = d.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, ["W117", "W116"], "{d:?}");
        assert!(d[0].message.contains("POST /api/scna"));
        // Two lines for the request two parts serve; none for the other origin or the computed path.
        assert_eq!(edges.len(), 2);
        let mut d = vec![];
        unrequested(&nodes, &edges, &mut d);
        assert!(d.is_empty(), "every route is requested: {d:?}");
        let lonely = vec![node("x.health", Some(("GET", "/healthz")), &[])];
        unrequested(&lonely, &[], &mut d);
        assert_eq!(d[0].code, "W118");
    }
}

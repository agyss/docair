//! How much of what authors wrote the code confirms (docs/ADAPTERS.md §9.5).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::model::{EdgeKind, Graph, Node, Origin, Tag};
use crate::request::REQUESTS;
use crate::resolve::{resolve_ref, split_payload};
use crate::route::{Fit, Route};

/// The tags that make lines, as authors write them; `@covers` is intent, not code, so it is left out.
const LINES: &[&str] = &[
    "calls", "in", "out", "reads", "writes", "emits", "consumes", "creates", "routes", "selects",
    "uses",
];

/// How far a chain of lines is followed for a shortcut or a request.
const MAX_HOPS: usize = 8;

/// For one kind of line, what authors wrote and how much of it the code says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Count {
    /// Hand-written lines (signatures like `@out reply: Reply` are no lines and left out).
    pub authored: usize,
    /// The code says the same line.
    pub confirmed: usize,
    /// Calls: the code goes there from inside the part (its methods) or through other functions (the author wrote a shortcut).
    /// Data (`@in … from`): the value came through other makers, from the part's caller, or from what a maker called.
    /// Reads and writes: a function the part calls reads or writes it.
    pub via_helpers: usize,
    /// Calls: a request the code sends goes this way (the network path of step 6).
    pub by_request: usize,
    /// Lines only the code knows.
    pub code_only: usize,
}

impl Count {
    /// Hand-written lines the code doesn't account for.
    pub fn missing(&self) -> usize {
        self.authored - self.confirmed - self.via_helpers - self.by_request
    }
}

/// What a line tag points at: `docair-core.config.load` of `docair-core.config.load "reads it"`, the ref after `from`/`to`.
/// `None` for `@in`/`@out` without `from`/`to`: a signature, no line.
fn target(tag: &Tag) -> Option<String> {
    let value = match tag.name.as_str() {
        "in" => tag.value.rsplit_once(" from ")?.1,
        "out" => tag.value.rsplit_once(" to ")?.1,
        _ => tag.value.as_str(),
    };
    Some(split_payload(value).target.trim().to_string())
}

/// The parts the lines in `out` lead to from any of `from`, at most `MAX_HOPS` away.
fn reachable<'a>(from: &[&'a str], out: &HashMap<&'a str, Vec<&'a str>>) -> HashSet<&'a str> {
    let mut seen: HashSet<&str> = from.iter().copied().collect();
    let mut queue: VecDeque<(&str, usize)> = from.iter().map(|f| (*f, 0)).collect();
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

/// The routes a part's `@requests` tags ask for: (method, path).
fn requests(n: &Node) -> Vec<(String, Route)> {
    n.tags_named(REQUESTS)
        .filter_map(|t| {
            let v = split_payload(&t.value);
            let (m, p) = v.target.trim().split_once(char::is_whitespace)?;
            Some((m.to_uppercase(), Route::parse(p.trim())))
        })
        .collect()
}

/***
 * Count, per kind of line, how much of what authors wrote the code accounts for, and the lines only the code knows.
 *
 * A hand-written line is *confirmed* when the same part has an inferred tag
 * of the same kind to the same target (for a call, also a route or a
 * selection). A hand-written call is *via helpers*
 * when inferred calls lead from the part, or a part inside it, to the target
 * (through other functions or not), and *by a request* when the part sends a request (`@requests`)
 * and lines of calls, routes and selections lead from the target to what
 * serves it. `@in`/`@out` without `from`/`to` are signatures and not counted.
 ***/
pub fn coverage(graph: &Graph) -> BTreeMap<String, Count> {
    let by_id: HashMap<&str, &Node> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let index: HashMap<String, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.clone(), i))
        .collect();
    // A target as resolve finds it: `tf:helm_release.traefik` is the node `tf.traefik`.
    let node_of = |target: String| match resolve_ref(&target, &graph.nodes, &index) {
        Some(r) => r.id,
        None => target,
    };
    // What the code says, from the inferred tags: an edge merges the code's line with the author's.
    let mut calls: HashMap<&str, Vec<String>> = HashMap::new();
    let mut callers: HashMap<String, Vec<&str>> = HashMap::new();
    let mut makers: HashMap<String, Vec<String>> = HashMap::new();
    for n in &graph.nodes {
        for t in n.tags.iter().filter(|t| t.origin == Origin::Inferred) {
            let Some(to) = target(t).map(&node_of) else {
                continue;
            };
            match t.name.as_str() {
                "calls" => {
                    callers.entry(to.clone()).or_default().push(n.id.as_str());
                    calls.entry(n.id.as_str()).or_default().push(to);
                }
                // A data line goes from the maker to the taker.
                "in" => makers.entry(n.id.clone()).or_default().push(to),
                "out" => makers.entry(to).or_default().push(n.id.clone()),
                _ => {}
            }
        }
    }
    let calls: HashMap<&str, Vec<&str>> = calls
        .iter()
        .map(|(k, v)| (*k, v.iter().map(String::as_str).collect()))
        .collect();
    let makers: HashMap<&str, Vec<&str>> = makers
        .iter()
        .map(|(k, v)| (k.as_str(), v.iter().map(String::as_str).collect()))
        .collect();
    let callers: HashMap<&str, Vec<&str>> = callers
        .iter()
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();
    let mut ways: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &graph.edges {
        if matches!(e.kind, EdgeKind::Call | EdgeKind::Route | EdgeKind::Select) {
            ways.entry(e.from.as_str()).or_default().push(e.to.as_str());
        }
    }
    let serves = |id: &str, asked: &[(String, Route)]| {
        by_id
            .get(id)
            .and_then(|n| n.http.as_ref())
            .is_some_and(|h| {
                asked.iter().any(|(m, r)| {
                    m.eq_ignore_ascii_case(&h.method)
                        && r.fit(&Route::parse(&h.path)) == Fit::Certain
                })
            })
    };
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for n in &graph.nodes {
        if let Some(p) = &n.part_of {
            children.entry(p.as_str()).or_default().push(n.id.as_str());
        }
    }
    // A part and the parts inside it: a component's lines may start in its methods.
    let inside = |id: &'_ str| -> Vec<&str> {
        let Some(&start) = by_id.get_key_value(id).map(|(k, _)| k) else {
            return Vec::new();
        };
        let mut all = vec![start];
        let mut at = 0;
        while at < all.len() {
            all.extend(children.get(all[at]).into_iter().flatten());
            at += 1;
        }
        all
    };
    // Data: through other makers, from the part's callers (over several calls), or from what a maker called.
    let data_via = |inside: &[&str], to: &str| {
        let through = reachable(inside, &makers);
        through.contains(to)
            || reachable(inside, &callers).contains(to)
            || through
                .iter()
                .any(|m| calls.get(m).is_some_and(|c| c.contains(&to)))
    };
    // What the code says each part reads and writes, to find it behind the part's calls.
    let touched: HashMap<&str, HashSet<(String, String)>> = graph
        .nodes
        .iter()
        .map(|n| {
            let lines = n
                .tags
                .iter()
                .filter(|t| {
                    t.origin == Origin::Inferred && matches!(t.name.as_str(), "reads" | "writes")
                })
                .filter_map(|t| Some((t.name.clone(), node_of(target(t)?))))
                .collect();
            (n.id.as_str(), lines)
        })
        .collect();
    let mut out: BTreeMap<String, Count> = BTreeMap::new();
    for n in &graph.nodes {
        let of = |origin: Origin| -> HashSet<(String, String)> {
            n.tags
                .iter()
                .filter(|t| t.origin == origin && LINES.contains(&t.name.as_str()))
                .filter_map(|t| Some((t.name.clone(), node_of(target(t)?))))
                .collect()
        };
        let authored = of(Origin::Authored);
        let inferred = of(Origin::Inferred);
        let asked = requests(n);
        for (verb, to) in &authored {
            let c = out.entry(verb.clone()).or_default();
            c.authored += 1;
            // A hand-written call is what the code draws as a route or a selection too (an Ingress to its Service).
            let same = |v: &str| inferred.contains(&(v.to_string(), to.clone()));
            if same(verb) || (verb == "calls" && (same("routes") || same("selects"))) {
                c.confirmed += 1;
            } else if (verb == "calls" && reachable(&inside(&n.id), &calls).contains(to.as_str()))
                || (verb == "in" && data_via(&inside(&n.id), to))
                || (matches!(verb.as_str(), "reads" | "writes")
                    && reachable(&inside(&n.id), &calls).iter().any(|m| {
                        touched
                            .get(m)
                            .is_some_and(|l| l.contains(&(verb.clone(), to.clone())))
                    }))
            {
                c.via_helpers += 1;
            } else if verb == "calls"
                && !asked.is_empty()
                && reachable(&[to.as_str()], &ways)
                    .iter()
                    .any(|id| serves(id, &asked))
            {
                c.by_request += 1;
            }
        }
        for (verb, to) in &inferred {
            if !authored.contains(&(verb.clone(), to.clone())) {
                out.entry(verb.clone()).or_default().code_only += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Edge, Http, SCHEMA, SourceRef, View};

    fn node(id: &str, tags: Vec<Tag>, http: Option<(&str, &str)>) -> Node {
        let mut n: Node = serde_json::from_value(serde_json::json!({
            "id": id, "kind": "function", "name": id, "summary": "", "description": "",
            "docs": [], "tags": [], "origin": "authored"
        }))
        .unwrap();
        n.tags = tags;
        n.http = http.map(|(m, p)| Http {
            method: m.into(),
            path: p.into(),
        });
        n
    }

    fn edge(from: &str, to: &str, kind: EdgeKind, origin: Origin) -> Edge {
        let mut e: Edge = serde_json::from_value(serde_json::json!({
            "from": from, "to": to, "kind": "call", "tag": "", "origin": "inferred",
            "source": {"file": "", "line": 0}
        }))
        .unwrap();
        e.kind = kind;
        e.origin = origin;
        e.source = SourceRef {
            file: String::new(),
            line: 0,
            end: None,
        };
        e
    }

    #[test]
    fn what_authors_wrote_and_how_the_code_accounts_for_it() {
        let a = node(
            "a",
            vec![
                Tag::authored("calls", "b \"does it\"", 2),
                Tag::authored("calls", "c (x) -> y", 3),
                Tag::authored("calls", "z", 4),
                Tag::authored("in", "x from d", 5),
                Tag::authored("out", "reply: Reply", 6),
                Tag::authored("calls", "svc", 7),
                Tag::inferred("routes", "svc", 9, "kubernetes: backend.service"),
                Tag::inferred("calls", "b", 9, "rust-analyzer: b"),
                Tag::inferred("calls", "e", 9, "rust-analyzer: e"),
                Tag::inferred("in", "uses from d", 9, "terraform reference"),
            ],
            None,
        );
        let web = node(
            "web",
            vec![
                Tag::authored("calls", "gateway (POST /api/scan)", 2),
                Tag::inferred(
                    REQUESTS,
                    "POST /api/scan -> ScanResult",
                    3,
                    "this.http.post(…)",
                ),
            ],
            None,
        );
        // check takes what resolve made through prune: `@in nodes from resolve` is via helpers.
        let check = node(
            "check",
            vec![
                Tag::authored("in", "nodes from resolve", 2),
                Tag::inferred("in", "nodes from prune", 3, "tree-sitter: let …"),
            ],
            None,
        );
        // The code's own lines: e calls c, prune takes what resolve made.
        let e = node(
            "e",
            vec![Tag::inferred("calls", "c", 2, "rust-analyzer: c")],
            None,
        );
        let prune = node(
            "prune",
            vec![Tag::inferred(
                "in",
                "nodes from resolve",
                2,
                "tree-sitter: let …",
            )],
            None,
        );
        let nodes = vec![
            a,
            web,
            check,
            e,
            prune,
            node("handler", vec![], Some(("POST", "/api/scan"))),
            node("gateway", vec![], None),
        ];
        let edges = vec![
            // gateway -> handler: the request's way.
            edge("gateway", "handler", EdgeKind::Route, Origin::Inferred),
        ];
        let g = Graph {
            schema: SCHEMA.into(),
            system: "s".into(),
            generator: String::new(),
            nodes,
            edges,
            flows: vec![],
            walkthroughs: vec![],
            view: View::default(),
            glossary: vec![],
            library: vec![],
            meanings: None,
            diagnostics: vec![],
        };
        let c = coverage(&g);
        let calls = &c["calls"];
        assert_eq!(
            (
                calls.authored,
                calls.confirmed,
                calls.via_helpers,
                calls.by_request,
                calls.missing(),
                calls.code_only
            ),
            (5, 2, 1, 1, 1, 2),
            "{calls:?}"
        );
        assert_eq!(
            (c["in"].authored, c["in"].confirmed, c["in"].via_helpers),
            (2, 1, 1)
        );
        assert!(!c.contains_key("out"), "a signature is no line");
    }
}

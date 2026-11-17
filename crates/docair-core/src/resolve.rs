//! Stage 5: refs become edges, flows get paths (SPEC §4, §7).

use std::collections::{BTreeMap, HashMap};

use petgraph::Graph as PGraph;
use petgraph::algo::astar;
use petgraph::graph::NodeIndex;
use petgraph::{Directed, EdgeType, Undirected};

use crate::model::{Diagnostic, Edge, EdgeKind, Flow, FlowStep, Node, Origin, SourceRef};

/// flow name → step number → (node ids, labels)
type StepsByFlow = BTreeMap<String, BTreeMap<u32, (Vec<String>, Vec<String>)>>;

/// A data-flow tag value: the ref, what happens `"…"`, what the tag's node sends `(…)` and what it receives `-> …` (SPEC §4.1).
#[derive(Debug, PartialEq, Eq)]
pub struct TagValue {
    pub target: String,
    pub text: Option<String>,
    pub sends: Option<String>,
    pub receives: Option<String>,
}

/// Take the first `"…"` outside parentheses out of a tag value.
fn take_quoted(value: &str) -> (String, Option<String>) {
    let mut depth = 0i32;
    let mut start = None;
    for (i, c) in value.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            '"' if depth == 0 => match start {
                None => start = Some(i),
                Some(s) => {
                    let text = value[s + 1..i].trim().to_string();
                    let rest = format!("{} {}", value[..s].trim_end(), value[i + 1..].trim_start());
                    return (rest.trim().to_string(), (!text.is_empty()).then_some(text));
                }
            },
            _ => {}
        }
    }
    (value.to_string(), None)
}

/// Split `docair-core.scan "scans the upload" (folder) -> Graph` into ref, text, sends and receives.
pub fn split_payload(value: &str) -> TagValue {
    let (value, text) = take_quoted(value.trim());
    let v = value.trim();
    let mut depth = 0i32;
    let mut arrow = None;
    for (i, c) in v.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            '-' if depth == 0 && v[i..].starts_with("->") => {
                arrow = Some(i);
                break;
            }
            _ => {}
        }
    }
    let (head, receives) = match arrow {
        Some(i) => (v[..i].trim_end(), Some(v[i + 2..].trim().to_string())),
        None => (v, None),
    };
    let mut target = head.to_string();
    let mut sends = None;
    if head.ends_with(')') {
        let mut depth = 0i32;
        for (i, c) in head.char_indices().rev() {
            match c {
                ')' => depth += 1,
                '(' => {
                    depth -= 1;
                    if depth == 0 {
                        if i > 0 && head[..i].ends_with(char::is_whitespace) {
                            target = head[..i].trim().to_string();
                            sends = Some(head[i + 1..head.len() - 1].trim().to_string());
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    let non_empty = |s: Option<String>| s.filter(|s| !s.is_empty());
    TagValue {
        target,
        text,
        sends: non_empty(sends),
        receives: non_empty(receives),
    }
}

/// The edge label: what a reader should see on the arrow.
fn edge_label(
    kind: EdgeKind,
    base: Option<String>,
    payload: &Option<String>,
    reply: &Option<String>,
) -> Option<String> {
    let text = match kind {
        EdgeKind::Call => match (payload, reply) {
            (Some(p), Some(r)) => Some(format!("({p}) → {r}")),
            (Some(p), None) => Some(format!("({p})")),
            (None, Some(r)) => Some(format!("→ {r}")),
            (None, None) => None,
        },
        _ => payload.clone(),
    };
    match (base, text) {
        (Some(b), Some(t)) if kind == EdgeKind::Call => Some(format!("{b} {t}")),
        (Some(b), Some(t)) => Some(format!("{b} · {t}")),
        (b, t) => b.or(t),
    }
}

/// Whether a line's words already say `new`, as whole words: `(POST /api/scan: files) → ScanResult` says `(POST /api/scan)`.
fn says(old: &str, new: &str) -> bool {
    let core = new
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();
    if core.is_empty() || old.split(", ").any(|x| x == new) {
        return true;
    }
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    old.match_indices(core).any(|(i, m)| {
        !word(old[..i].chars().next_back()) && !word(old[i + m.len()..].chars().next())
    })
}

struct Resolved {
    id: String,
    sub: Option<String>,
    via: Option<String>,
}

/***
 * Turn every data-flow tag into an edge, and give every flow step its path.
 *
 * Unresolved refs become placeholder nodes with E001 so that nothing is
 * dropped silently. Topics are created the first time they are referenced.
 * Edges between the same two nodes in the same direction are merged.
 *
 * @flow  scan 10 "refs become edges, flows get paths"
 * @in    nodes: Node[] from docair-core.build.build
 * @calls docair-core.resolve.match_http "compares called paths with routes"
 * @calls docair-core.resolve.flow_paths "orders the scenario steps and links them"
 * @out   graph: Node[] + Edge[] + Flow[]
 ***/
pub fn resolve(
    mut nodes: Vec<Node>,
    system: &str,
    diags: &mut Vec<Diagnostic>,
) -> (Vec<Node>, Vec<Edge>, Vec<Flow>) {
    let mut index: HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.clone(), i))
        .collect();
    let mut edges: Vec<Edge> = Vec::new();
    let mut by_pair: HashMap<(String, String), usize> = HashMap::new();

    for i in 0..nodes.len() {
        let node_id = nodes[i].id.clone();
        let file = nodes[i]
            .source
            .as_ref()
            .map_or(String::new(), |s| s.file.clone());
        for tag in nodes[i].tags.clone() {
            let (kind, label, target, outgoing) = match tag.name.as_str() {
                "in" | "out" => {
                    let sep = if tag.name == "in" { " from " } else { " to " };
                    let Some((label, r)) = tag.value.rsplit_once(sep) else {
                        continue;
                    };
                    (
                        EdgeKind::Data,
                        Some(label.trim().to_string()),
                        r.trim().to_string(),
                        tag.name == "out",
                    )
                }
                "calls" => (EdgeKind::Call, None, tag.value.clone(), true),
                "creates" => (EdgeKind::Create, None, tag.value.clone(), true),
                "covers" => (EdgeKind::Cover, None, tag.value.clone(), true),
                "routes" => (EdgeKind::Route, None, tag.value.clone(), true),
                "selects" => (EdgeKind::Select, None, tag.value.clone(), true),
                "reads" => (EdgeKind::Read, None, tag.value.clone(), false),
                "writes" => (EdgeKind::Write, None, tag.value.clone(), true),
                "emits" | "consumes" => {
                    let r = if tag.value.trim_start().starts_with("topic:") {
                        tag.value.clone()
                    } else {
                        format!("topic:{}", tag.value.trim_start())
                    };
                    (
                        if tag.name == "emits" {
                            EdgeKind::Emit
                        } else {
                            EdgeKind::Consume
                        },
                        None,
                        r,
                        tag.name == "emits",
                    )
                }
                _ => continue,
            };
            // `(…)` is what the tag's node sends, `-> …` what it receives. Along the
            // edge travels what flows in the edge's direction; a call's result comes back.
            let TagValue {
                target,
                text,
                sends,
                receives,
            } = split_payload(&target);
            let (payload, reply) = if outgoing {
                (sends, receives)
            } else {
                (receives, sends)
            };
            let resolved = match resolve_ref(&target, &nodes, &index) {
                Some(r) => r,
                None if target.starts_with("topic:") => {
                    let name = target.trim_start_matches("topic:");
                    add_node(
                        &mut nodes,
                        &mut index,
                        &target,
                        "topic",
                        name,
                        system,
                        Origin::Inferred,
                    );
                    Resolved {
                        id: target.clone(),
                        sub: None,
                        via: None,
                    }
                }
                None => {
                    let id = format!("unresolved:{target}");
                    add_node(
                        &mut nodes,
                        &mut index,
                        &id,
                        "unresolved",
                        &target,
                        system,
                        Origin::Placeholder,
                    );
                    diags.push(
                        Diagnostic::new(
                            "E001",
                            format!("unresolved ref `{target}` in @{}", tag.name),
                        )
                        .at(&file, tag.line)
                        .node(&node_id),
                    );
                    Resolved {
                        id,
                        sub: None,
                        via: None,
                    }
                }
            };
            let label = edge_label(kind, label.or(resolved.sub), &payload, &reply);
            let (from, to) = if outgoing {
                (node_id.clone(), resolved.id)
            } else {
                (resolved.id, node_id.clone())
            };
            if let Some(&k) = by_pair.get(&(from.clone(), to.clone())) {
                let e: &mut Edge = &mut edges[k];
                // Code facts stay on the line, and an author's words add to them
                // (docs/ADAPTERS.md §6); words that say the same show once.
                if let Some(l) = label {
                    match &mut e.label {
                        Some(old) if !says(old, &l) => *old = format!("{old}, {l}"),
                        Some(_) => {}
                        None => e.label = Some(l),
                    }
                }
                e.via = e.via.take().or(resolved.via);
                e.text = match (e.text.take(), text) {
                    (Some(a), Some(b)) if a != b => Some(format!("{a}; {b}")),
                    (a, b) => a.or(b),
                };
                e.payload = e.payload.take().or(payload);
                e.reply = e.reply.take().or(reply);
                continue;
            }
            by_pair.insert((from.clone(), to.clone()), edges.len());
            edges.push(Edge {
                from,
                to,
                kind,
                label,
                via: resolved.via,
                text,
                carries: vec![],
                payload,
                reply,
                from_code: false,
                from_api: None,
                tag: format!("@{} {}", tag.name, tag.value),
                source: SourceRef {
                    file: file.clone(),
                    line: tag.line,
                },
                origin: tag.origin,
            });
        }
    }

    word_calls_from_code(&nodes, &mut edges);
    find_carried_data(&nodes, &mut edges);
    let flows = flow_paths(&nodes, &edges, diags);
    (nodes, edges, flows)
}

/// Data nodes whose name (or a `@carried-as` word) occurs in what an edge carries (SPEC §3.6).
fn find_carried_data(nodes: &[Node], edges: &mut [Edge]) {
    let data: Vec<(&str, Vec<String>)> = nodes
        .iter()
        .filter(|n| n.kind == "data")
        .map(|n| {
            let mut words = vec![
                n.name.to_lowercase(),
                n.id.rsplit('.').next().unwrap_or(&n.id).to_lowercase(),
            ];
            for t in n.tags_named("carried-as") {
                words.extend(
                    t.value
                        .split(',')
                        .map(|w| w.trim().to_lowercase())
                        .filter(|w| !w.is_empty()),
                );
            }
            words.sort();
            words.dedup();
            (n.id.as_str(), words)
        })
        .collect();
    if data.is_empty() {
        return;
    }
    let kind_of: HashMap<&str, &str> = nodes
        .iter()
        .map(|n| (n.id.as_str(), n.kind.as_str()))
        .collect();
    for e in edges.iter_mut() {
        if kind_of.get(e.from.as_str()) == Some(&"data")
            || kind_of.get(e.to.as_str()) == Some(&"data")
        {
            continue;
        }
        let said = [&e.payload, &e.reply, &e.label]
            .into_iter()
            .flatten()
            .map(|s| s.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        for (id, words) in &data {
            if words.iter().any(|w| crate::glossary::occurs_word(w, &said)) {
                e.carries.push(id.to_string());
            }
        }
    }
}

/// A call that names no arguments and no result takes them from the callee's
/// signature, so the line says what goes in and what comes back (SPEC §4.2).
fn word_calls_from_code(nodes: &[Node], edges: &mut [Edge]) {
    let index: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    for e in edges.iter_mut() {
        if e.kind != EdgeKind::Call || e.payload.is_some() || e.reply.is_some() {
            continue;
        }
        let Some(sig) = index.get(e.to.as_str()).and_then(|n| n.signature.as_ref()) else {
            continue;
        };
        let (payload, reply) = sig.wording();
        if payload.is_none() && reply.is_none() {
            continue;
        }
        let text = edge_label(EdgeKind::Call, None, &payload, &reply);
        e.label = match (e.label.take(), text) {
            (Some(base), Some(t)) => Some(format!("{base} {t}")),
            (base, t) => base.or(t),
        };
        e.payload = payload;
        e.reply = reply;
        e.from_code = true;
    }
}

fn add_node(
    nodes: &mut Vec<Node>,
    index: &mut HashMap<String, usize>,
    id: &str,
    kind: &str,
    name: &str,
    system: &str,
    origin: Origin,
) {
    if index.contains_key(id) {
        return;
    }
    index.insert(id.to_string(), nodes.len());
    nodes.push(Node {
        id: id.to_string(),
        kind: kind.to_string(),
        name: name.to_string(),
        summary: String::new(),
        description: String::new(),
        part_of: Some(system.to_string()),
        source: None,
        symbol: None,
        http: None,
        deployed_as: None,
        docs: Vec::new(),
        tags: Vec::new(),
        origin,
        test: None,
        signature: None,
        runs_in: None,
        code_from: None,
    });
}

fn resolve_ref(r: &str, nodes: &[Node], index: &HashMap<String, usize>) -> Option<Resolved> {
    let r = r.trim();
    if let Some(rest) = r.strip_prefix("http:") {
        let (method, path) = rest.trim().split_once(char::is_whitespace)?;
        let node = nodes.iter().find(|n| {
            n.http.as_ref().is_some_and(|h| {
                h.method.eq_ignore_ascii_case(method) && match_http(path.trim(), &h.path)
            })
        })?;
        return Some(Resolved {
            id: node.id.clone(),
            sub: None,
            via: Some(r.to_string()),
        });
    }
    if index.contains_key(r) {
        return Some(Resolved {
            id: r.to_string(),
            sub: None,
            via: None,
        });
    }
    // Infrastructure by its own name (SPEC §4.3): `tf:helm_release.docair`, `k8s:Service/docair-web`.
    let by_symbol = |symbol: &str| {
        nodes
            .iter()
            .find(|n| n.symbol.as_deref() == Some(symbol))
            .map(|n| Resolved {
                id: n.id.clone(),
                sub: None,
                via: None,
            })
    };
    if let Some(addr) = r.strip_prefix("tf:") {
        return by_symbol(&format!("resource {addr}"))
            .or_else(|| by_symbol(&format!("data {addr}")))
            .or_else(|| by_symbol(&format!("module {addr}")));
    }
    if let Some(obj) = r.strip_prefix("k8s:") {
        let (kind, name) = obj.split_once('/')?;
        return by_symbol(&format!("{kind} {name}"));
    }
    if r.contains(':') {
        return None;
    }
    // Sub-part form: the longest id prefix wins, the rest becomes the label.
    let segs: Vec<&str> = r.split('.').collect();
    (1..segs.len()).rev().find_map(|k| {
        let prefix = segs[..k].join(".");
        index.contains_key(&prefix).then(|| Resolved {
            id: prefix,
            sub: Some(segs[k..].join(".")),
            via: None,
        })
    })
}

/***
 * Match a called path against a declared route, positionally, so that `{id}` and `:id` match any segment.
 ***/
pub fn match_http(called: &str, declared: &str) -> bool {
    let split = |p: &str| -> Vec<String> {
        p.split('?')
            .next()
            .unwrap_or("")
            .trim_matches('/')
            .split('/')
            .map(str::to_string)
            .collect()
    };
    let (a, b) = (split(called), split(declared));
    let is_param = |s: &str| s.starts_with(':') || (s.starts_with('{') && s.ends_with('}'));
    a.len() == b.len()
        && a.iter()
            .zip(&b)
            .all(|(x, y)| x == y || is_param(x) || is_param(y))
}

/***
 * Order the `@flow` steps of every scenario and find the edges that lead from each step to the next.
 *
 * Tries the edges' direction first, then ignores direction (a reply goes
 * back along a call). W104 when two steps are not connected at all.
 *
 * @in nodes: Node[] + Edge[] from docair-core.resolve.resolve
 ***/
pub fn flow_paths(nodes: &[Node], edges: &[Edge], diags: &mut Vec<Diagnostic>) -> Vec<Flow> {
    let mut steps: StepsByFlow = BTreeMap::new();
    for n in nodes {
        for t in n.tags_named("flow") {
            let mut parts = t
                .value
                .splitn(3, char::is_whitespace)
                .filter(|s| !s.is_empty());
            let (Some(name), Some(step)) = (
                parts.next(),
                parts.next().and_then(|s| s.parse::<u32>().ok()),
            ) else {
                let file = n.source.as_ref().map_or("", |s| s.file.as_str());
                diags.push(
                    Diagnostic::new(
                        "E006",
                        format!(
                            "malformed @flow `{}`: expected `<flow> <step> [\"label\"]`",
                            t.value
                        ),
                    )
                    .at(file, t.line)
                    .node(&n.id),
                );
                continue;
            };
            let label = t
                .value
                .splitn(3, char::is_whitespace)
                .nth(2)
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .to_string();
            let entry = steps
                .entry(name.to_string())
                .or_default()
                .entry(step)
                .or_default();
            entry.0.push(n.id.clone());
            if !label.is_empty() {
                entry.1.push(label);
            }
        }
    }

    let directed = petgraph_of::<Directed>(nodes, edges);
    let undirected = petgraph_of::<Undirected>(nodes, edges);
    let index: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();

    let mut flows = Vec::new();
    for (name, by_step) in steps {
        let mut out: Vec<FlowStep> = Vec::new();
        for (step, (ids, mut labels)) in by_step {
            let mut path = Vec::new();
            if let Some(prev) = out.last() {
                for to in &ids {
                    if prev.nodes.contains(to) {
                        continue;
                    }
                    // Stepping into or out of a group needs no edge: the step is inside it.
                    if prev
                        .nodes
                        .iter()
                        .any(|p| contains(nodes, &index, p, to) || contains(nodes, &index, to, p))
                    {
                        continue;
                    }
                    // Along the arrows from the latest step that leads here (a caller
                    // further up the stack counts, SPEC §7.3); against them only for
                    // a short hop back. A longer detour would tell a wrong story.
                    let found = out
                        .iter()
                        .rev()
                        .find_map(|st| {
                            // The step's nodes and those its path went through: the stack.
                            let mut stack = st.nodes.clone();
                            for &e in &st.path {
                                stack.push(edges[e].from.clone());
                                stack.push(edges[e].to.clone());
                            }
                            shortest(&directed, &stack, to, &index)
                                .filter(|p| p.len() <= MAX_FLOW_HOPS)
                        })
                        .or_else(|| {
                            shortest(&undirected, &prev.nodes, to, &index)
                                .filter(|p| p.len() <= MAX_FLOW_HOPS_BACK)
                        });
                    match found {
                        Some(p) => path.extend(
                            p.into_iter()
                                .filter(|e| !path.contains(e))
                                .collect::<Vec<_>>(),
                        ),
                        None => {
                            let n = &nodes[index[to.as_str()]];
                            let (file, line) = n
                                .source
                                .as_ref()
                                .map_or(("", 0), |s| (s.file.as_str(), s.line));
                            diags.push(Diagnostic::new("W104", format!("flow `{name}`: no path from step {} to step {step} (`{to}`)", prev.step)).at(file, line).node(to));
                        }
                    }
                }
            }
            // Parts of one step often share their label: say it once.
            let mut said = std::collections::HashSet::new();
            labels.retain(|l| said.insert(l.clone()));
            let label = (!labels.is_empty()).then(|| labels.join(" · "));
            out.push(FlowStep {
                step,
                nodes: ids,
                label,
                path,
            });
        }
        flows.push(Flow { name, steps: out });
    }
    flows
}

/// Whether `outer` is an ancestor of `inner` in the `@part-of` tree.
fn contains(nodes: &[Node], index: &HashMap<&str, usize>, outer: &str, inner: &str) -> bool {
    let mut cur = index.get(inner).and_then(|&i| nodes[i].part_of.as_deref());
    let mut hops = 0;
    while let Some(p) = cur {
        if p == outer {
            return true;
        }
        hops += 1;
        if hops > 64 {
            return false;
        }
        cur = index.get(p).and_then(|&i| nodes[i].part_of.as_deref());
    }
    false
}

fn petgraph_of<T: EdgeType>(nodes: &[Node], edges: &[Edge]) -> PGraph<usize, usize, T> {
    let mut g = PGraph::<usize, usize, T>::default();
    let ix: HashMap<&str, NodeIndex> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), g.add_node(i)))
        .collect();
    for (k, e) in edges.iter().enumerate() {
        if let (Some(&a), Some(&b)) = (ix.get(e.from.as_str()), ix.get(e.to.as_str())) {
            g.add_edge(a, b, k);
        }
    }
    g
}

/// Edge indices of the shortest path from any of `from` to `to`.
/// The longest path along the arrows that connects two steps of a scenario.
const MAX_FLOW_HOPS: usize = 6;
/// The longest path that may go against the arrows (a reply going back).
const MAX_FLOW_HOPS_BACK: usize = 3;

fn shortest<T: EdgeType>(
    g: &PGraph<usize, usize, T>,
    from: &[String],
    to: &str,
    index: &HashMap<&str, usize>,
) -> Option<Vec<usize>> {
    let goal = NodeIndex::new(*index.get(to)?);
    from.iter()
        .filter_map(|f| index.get(f.as_str()))
        .filter_map(|&s| astar(g, NodeIndex::new(s), |n| n == goal, |_| 1, |_| 0))
        .min_by_key(|(cost, _)| *cost)
        .map(|(_, hops)| {
            hops.windows(2)
                .filter_map(|w| g.find_edge(w[0], w[1]).map(|e| g[e]))
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Http, Tag};

    fn node(id: &str, tags: &[(&str, &str)]) -> Node {
        Node {
            id: id.into(),
            kind: "function".into(),
            name: id.into(),
            summary: String::new(),
            description: String::new(),
            part_of: Some("shop".into()),
            source: Some(SourceRef {
                file: "f".into(),
                line: 1,
            }),
            symbol: None,
            http: None,
            deployed_as: None,
            docs: Vec::new(),
            tags: tags.iter().map(|(n, v)| Tag::authored(n, v, 1)).collect(),
            origin: Origin::Authored,
            test: None,
            signature: None,
            runs_in: None,
            code_from: None,
        }
    }

    #[test]
    fn edges_carry_the_data_their_payload_names() {
        let mut q = node("data.question", &[("carried-as", "prompt, ask")]);
        q.kind = "data".into();
        q.name = "question".into();
        let mut r = node("data.reply", &[]);
        r.kind = "data".into();
        let nodes = vec![
            node("shop", &[]),
            node(
                "panel",
                &[
                    ("calls", "svc (question, graph) -> Reply"),
                    ("creates", "data.question \"the user types it\""),
                ],
            ),
            node("svc", &[("calls", "model (prompts)")]),
            node("model", &[]),
            q,
            r,
        ];
        let mut diags = vec![];
        let (_, edges, _) = resolve(nodes, "shop", &mut diags);
        let carries = |a: &str, b: &str| {
            edges
                .iter()
                .find(|e| e.from == a && e.to == b)
                .unwrap()
                .carries
                .clone()
        };
        assert_eq!(carries("panel", "svc"), vec!["data.question", "data.reply"]);
        assert_eq!(
            carries("svc", "model"),
            vec!["data.question"],
            "a carried-as word, in the plural"
        );
        assert!(
            carries("panel", "data.question").is_empty(),
            "the edge that makes it carries nothing"
        );
    }

    #[test]
    fn words_that_say_the_same_show_once() {
        assert!(super::says(
            "(POST /api/scan: files) → ScanResult",
            "(POST /api/scan)"
        ));
        assert!(!super::says("images: in containerd", "runs after"));
        assert!(!super::says("valid input", "id"));
        assert!(super::says("uses, runs after", "runs after"));
    }

    #[test]
    fn payload_syntax() {
        let t = split_payload("http:POST /api/orders (CreateOrder) -> Order");
        assert_eq!(t.target, "http:POST /api/orders");
        assert_eq!(t.sends.as_deref(), Some("CreateOrder"));
        assert_eq!(t.receives.as_deref(), Some("Order"));
        let t = split_payload("project-files.docair.toml -> Config");
        assert_eq!(
            (t.target.as_str(), t.sends, t.receives.as_deref()),
            ("project-files.docair.toml", None, Some("Config"))
        );
        let t = split_payload("docair-core.scan (root, f(a, b)) -> Graph");
        assert_eq!(t.sends.as_deref(), Some("root, f(a, b)"));
        let t = split_payload("orders.create");
        assert_eq!(
            (t.target.as_str(), t.sends, t.receives),
            ("orders.create", None, None)
        );
        let t = split_payload("http:GET /api/orders/{id}");
        assert_eq!(t.target, "http:GET /api/orders/{id}");
        let t = split_payload("docair-core.scan \"scans the upload\" (root) -> Graph");
        assert_eq!(
            (
                t.target.as_str(),
                t.text.as_deref(),
                t.sends.as_deref(),
                t.receives.as_deref()
            ),
            (
                "docair-core.scan",
                Some("scans the upload"),
                Some("root"),
                Some("Graph")
            )
        );
        let t = split_payload("cfg (a \"b\") \"reads it\"");
        assert_eq!(
            (t.text.as_deref(), t.sends.as_deref()),
            (Some("reads it"), Some("a \"b\""))
        );
    }

    #[test]
    fn payloads_become_labels_and_reply() {
        let nodes = vec![
            node(
                "cart",
                &[("calls", "http:POST /api/orders (CreateOrder) -> Order")],
            ),
            {
                let mut n = node(
                    "create",
                    &[
                        ("writes", "db.orders (Order)"),
                        ("reads", "db.stock -> Stock"),
                    ],
                );
                n.http = Some(Http {
                    method: "POST".into(),
                    path: "/api/orders".into(),
                });
                n
            },
            node("db", &[]),
        ];
        let mut d = Vec::new();
        let (_, edges, _) = resolve(nodes, "shop", &mut d);
        let call = edges.iter().find(|e| e.kind == EdgeKind::Call).unwrap();
        assert_eq!(call.label.as_deref(), Some("(CreateOrder) → Order"));
        assert_eq!(
            (call.payload.as_deref(), call.reply.as_deref()),
            (Some("CreateOrder"), Some("Order"))
        );
        let write = edges.iter().find(|e| e.kind == EdgeKind::Write).unwrap();
        assert_eq!(write.label.as_deref(), Some("orders · Order"));
        let read = edges.iter().find(|e| e.kind == EdgeKind::Read).unwrap();
        assert_eq!(
            (read.from.as_str(), read.payload.as_deref()),
            ("db", Some("Stock"))
        );
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn http_params_match_by_position() {
        assert!(match_http("/api/orders/:id", "/api/orders/{id}"));
        assert!(match_http("/api/orders/42?x=1", "/api/orders/{id}"));
        assert!(!match_http("/api/orders", "/api/orders/{id}"));
    }

    #[test]
    fn edges_topics_subparts_placeholders_and_merging() {
        let mut create = node(
            "orders.create",
            &[
                ("in", "req: CreateOrder from shop-ui.cart"),
                ("writes", "orders-db.orders"),
                ("emits", "topic:order.created"),
                ("calls", "inventory.reserve"),
                ("flow", "checkout 2 \"API stores the order\""),
            ],
        );
        create.http = Some(Http {
            method: "POST".into(),
            path: "/api/orders".into(),
        });
        let nodes = vec![
            node("shop", &[]),
            node(
                "shop-ui.cart",
                &[
                    ("calls", "http:POST /api/orders"),
                    ("flow", "checkout 1 \"user clicks Buy\""),
                ],
            ),
            create,
            node("orders-db", &[]),
            node(
                "mailer.send",
                &[("consumes", "order.created"), ("flow", "checkout 3")],
            ),
        ];
        let mut d = Vec::new();
        let (nodes, edges, flows) = resolve(nodes, "shop", &mut d);

        let cart_to_create: Vec<&Edge> = edges
            .iter()
            .filter(|e| e.from == "shop-ui.cart" && e.to == "orders.create")
            .collect();
        assert_eq!(
            cart_to_create.len(),
            1,
            "@calls and @in from merge into one edge"
        );
        assert_eq!(cart_to_create[0].label.as_deref(), Some("req: CreateOrder"));
        assert_eq!(
            cart_to_create[0].via.as_deref(),
            Some("http:POST /api/orders")
        );

        let write = edges.iter().find(|e| e.kind == EdgeKind::Write).unwrap();
        assert_eq!(
            (write.to.as_str(), write.label.as_deref()),
            ("orders-db", Some("orders"))
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.id == "topic:order.created" && n.kind == "topic")
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.id == "unresolved:inventory.reserve" && n.origin == Origin::Placeholder)
        );
        assert_eq!(
            d.iter().map(|d| d.code.as_str()).collect::<Vec<_>>(),
            vec!["E001"]
        );

        let checkout = &flows[0];
        assert_eq!(
            checkout.steps.iter().map(|s| s.step).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(checkout.steps[0].label.as_deref(), Some("user clicks Buy"));
        assert_eq!(checkout.steps[2].path.len(), 2, "create → topic → mailer");
    }

    #[test]
    fn stepping_into_a_group_needs_no_edge() {
        let mut inner = node("ns.deploy", &[("flow", "f 2")]);
        inner.part_of = Some("ns".into());
        let nodes = vec![node("shop", &[]), node("ns", &[("flow", "f 1")]), inner];
        let mut d = Vec::new();
        resolve(nodes, "shop", &mut d);
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn creates_is_its_own_edge_kind() {
        let nodes = vec![
            node("shop", &[]),
            node(
                "tf.app",
                &[("creates", "release.app (helm install, tag dev)")],
            ),
            node("release.app", &[]),
        ];
        let mut d = Vec::new();
        let (_, edges, _) = resolve(nodes, "shop", &mut d);
        assert_eq!(edges[0].kind, EdgeKind::Create);
        assert_eq!(
            (edges[0].from.as_str(), edges[0].to.as_str()),
            ("tf.app", "release.app")
        );
        assert_eq!(edges[0].payload.as_deref(), Some("helm install, tag dev"));
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn broken_flow_warns() {
        let nodes = vec![node("a", &[("flow", "f 1")]), node("b", &[("flow", "f 2")])];
        let mut d = Vec::new();
        let (_, _, flows) = resolve(nodes, "shop", &mut d);
        assert_eq!(d[0].code, "W104");
        assert!(flows[0].steps[1].path.is_empty());
    }
}

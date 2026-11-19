//! Stage 5: refs become edges, flows get paths (SPEC §4, §7).

use std::collections::{BTreeMap, HashMap};

use petgraph::Graph as PGraph;
use petgraph::algo::astar;
use petgraph::graph::NodeIndex;
use petgraph::{Directed, EdgeType, Undirected};

use crate::model::{Diagnostic, Edge, EdgeKind, Flow, FlowStep, Node, Origin, Reach, SourceRef};
use crate::route::{Fit, Route};

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

/// What code says it touches without saying which one: `file:` (a read or a write) binds to a store, `net:` (a request to a host the code computes) to an external system.
const OPEN_REFS: &[(&str, &[&str])] = &[("file:", &["store"]), ("net:", &["external", "host"])];

/***
 * Bind what code touches but doesn't name to what the part's own block names: `@reads file:` to the store of its `@reads`.
 *
 * The code says that a function reads a file; the block says which store
 * that is (docs/ADAPTERS.md §5.7). An inferred `file:` or `net:` tag becomes
 * the same tag to each target of that kind the block's own tags of the same
 * name give; when the block gives none, those of the blocks of the functions
 * that call it (a helper like `post(agent, url)` sends what its caller's
 * block names), up to three calls up. One of them when there are several;
 * with none the tag is dropped, and no line or warning comes of it.
 ***/
pub(crate) fn bind_open_refs(nodes: &mut [Node], index: &HashMap<String, usize>) {
    // Who calls whom, as the code says it.
    let mut callers: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        for t in n
            .tags
            .iter()
            .filter(|t| t.origin == Origin::Inferred && t.name == "calls")
        {
            if let Some(&k) = resolve_ref(&split_payload(&t.value).target, nodes, index)
                .and_then(|r| index.get(&r.id))
            {
                callers.entry(k).or_default().push(i);
            }
        }
    }
    // The targets of these kinds a part's own block names with tags of this name.
    let named = |at: usize, name: &str, kinds: &[&str]| -> Vec<String> {
        nodes[at]
            .tags
            .iter()
            .filter(|a| a.origin == Origin::Authored && a.name == name)
            .filter_map(|a| {
                let target = split_payload(&a.value).target;
                let r = resolve_ref(&target, nodes, index)?;
                let k = *index.get(&r.id)?;
                kinds
                    .contains(&nodes[k].kind.as_str())
                    .then(|| target.trim().to_string())
            })
            .collect()
    };
    let mut changes: Vec<(usize, Vec<usize>, Vec<crate::model::Tag>)> = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let open: Vec<usize> = node
            .tags
            .iter()
            .enumerate()
            .filter(|(_, tag)| {
                tag.origin == Origin::Inferred
                    && OPEN_REFS.iter().any(|(p, _)| tag.value.starts_with(p))
            })
            .map(|(t, _)| t)
            .collect();
        if open.is_empty() {
            continue;
        }
        let mut bound: Vec<crate::model::Tag> = Vec::new();
        for &t in &open {
            let tag = &node.tags[t];
            let kinds = OPEN_REFS
                .iter()
                .find(|(p, _)| tag.value.starts_with(p))
                .map_or(&[][..], |(_, k)| *k);
            // A path the code writes out finds the store whose `@path` fits it, for certain (docair-4wk.25).
            let path = tag.value.strip_prefix("file:").unwrap_or("").trim();
            let by_path = path_stores(nodes, path);
            // Else the part's own block first, then its callers', up to three calls up.
            let mut found = if by_path.is_empty() {
                named(i, &tag.name, kinds)
            } else {
                by_path
            };
            let mut seen = vec![i];
            let mut ring = vec![i];
            for _ in 0..3 {
                if !found.is_empty() {
                    break;
                }
                ring = ring
                    .iter()
                    .flat_map(|r| callers.get(r).into_iter().flatten().copied())
                    .filter(|c| !seen.contains(c))
                    .collect();
                ring.sort_unstable();
                ring.dedup();
                seen.extend(&ring);
                for &c in &ring {
                    for target in named(c, &tag.name, kinds) {
                        if !found.contains(&target) {
                            found.push(target);
                        }
                    }
                }
            }
            let several = found.len() > 1;
            for target in found {
                let mut b = tag.clone();
                b.value = target;
                if several {
                    b.reach = Some(Reach::OneOf);
                }
                bound.push(b);
            }
        }
        changes.push((i, open, bound));
    }
    for (i, open, bound) in changes {
        let tags = &mut nodes[i].tags;
        for &t in open.iter().rev() {
            tags.remove(t);
        }
        tags.extend(bound);
    }
}

/// The stores whose `@path` globs fit a path the code writes out.
fn path_stores(nodes: &[Node], path: &str) -> Vec<String> {
    if path.is_empty() {
        return Vec::new();
    }
    let path = path.trim_start_matches("./").trim_end_matches('/');
    nodes
        .iter()
        .filter(|n| n.kind == "store")
        .filter(|n| {
            n.tags_named("path")
                .any(|t| path_fits(path, t.value.trim()))
        })
        .map(|n| n.id.clone())
        .collect()
}

/// Whether a path fits a glob: `*` any part of one segment, `**` any number of segments; a path
/// that is the folder of the glob (`docs/walkthroughs` for `docs/walkthroughs/*.json`) fits too.
pub fn path_fits(path: &str, glob: &str) -> bool {
    let glob = glob.trim_start_matches("./").trim_end_matches('/');
    let path = path.trim_start_matches("./").trim_end_matches('/');
    let segs = |s: &str| -> Vec<String> {
        s.split('/')
            .filter(|x| !x.is_empty())
            .map(str::to_string)
            .collect()
    };
    let (p, g) = (segs(path), segs(glob));
    fn seg_fits(s: &str, g: &str) -> bool {
        // `*` and `?` inside one segment.
        let (s, g): (Vec<char>, Vec<char>) = (s.chars().collect(), g.chars().collect());
        let (mut i, mut j, mut star, mut mark) = (0, 0, None, 0);
        while i < s.len() {
            if j < g.len() && (g[j] == '?' || g[j] == s[i]) {
                i += 1;
                j += 1;
            } else if j < g.len() && g[j] == '*' {
                star = Some(j);
                mark = i;
                j += 1;
            } else if let Some(st) = star {
                j = st + 1;
                mark += 1;
                i = mark;
            } else {
                return false;
            }
        }
        g[j..].iter().all(|&c| c == '*')
    }
    fn fits(p: &[String], g: &[String]) -> bool {
        match g.first() {
            None => p.is_empty(),
            Some(first) if first == "**" => (0..=p.len()).any(|k| fits(&p[k..], &g[1..])),
            Some(first) => !p.is_empty() && seg_fits(&p[0], first) && fits(&p[1..], &g[1..]),
        }
    }
    if fits(&p, &g) {
        return true;
    }
    // The folder of the glob: the glob without its last segment.
    !p.is_empty() && fits(&p, &g[..g.len().saturating_sub(1)])
}

pub(crate) struct Resolved {
    pub(crate) id: String,
    sub: Option<String>,
    via: Option<String>,
}

/***
 * Turn every data-flow tag into an edge; the flow steps get their paths later, once request lines are in (`flow_paths`).
 *
 * Unresolved refs become placeholder nodes with E001 so that nothing is
 * dropped silently. Topics are created the first time they are referenced.
 * Edges between the same two nodes in the same direction are merged.
 *
 * @flow  scan 10 "refs become edges, flows get paths"
 * @in    nodes: Node[] from docair-core.build.build
 * @calls docair-core.route.route.fit "compares called paths with routes"
 * @out   graph: Node[] + Edge[]
 ***/
pub fn resolve(
    mut nodes: Vec<Node>,
    system: &str,
    diags: &mut Vec<Diagnostic>,
) -> (Vec<Node>, Vec<Edge>) {
    let mut index: HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.clone(), i))
        .collect();
    let mut edges: Vec<Edge> = Vec::new();
    let mut by_pair: HashMap<(String, String), usize> = HashMap::new();
    bind_open_refs(&mut nodes, &index);

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
            // A sub-part of code is no part a reader can find: a stale ref that kept a prefix
            // (`docair-core.infra.discover` after the module moved) binds to the whole crate.
            if let Some(sub) = &resolved.sub {
                let kind_of = index.get(&resolved.id).map(|&i| nodes[i].kind.as_str());
                if kind_of.is_some_and(|k| CODE_KINDS.contains(&k)) {
                    diags.push(
                        Diagnostic::new(
                            "W122",
                            format!(
                                "`{target}` in @{} names no part: `{}` is a {} and has no part `{sub}`, so the line goes to all of it",
                                tag.name,
                                resolved.id,
                                kind_of.unwrap_or("part")
                            ),
                        )
                        .at(&file, tag.line)
                        .node(&node_id),
                    );
                }
            }
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
                // A certain way to the same part makes the line certain.
                if tag.reach.is_none_or(|r| r == Reach::Certain) {
                    e.reach = None;
                }
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
                id: String::new(),
                from,
                to,
                kind,
                label,
                via: resolved.via,
                text,
                carries: vec![],
                carries_back: vec![],
                payload,
                reply,
                evidence: None,
                reach: tag.reach.filter(|r| *r != Reach::Certain),
                tag: format!("@{} {}", tag.name, tag.value),
                source: SourceRef {
                    file: file.clone(),
                    line: tag.line,
                    end: None,
                },
                origin: tag.origin,
            });
        }
    }

    word_calls_from_code(&nodes, &mut edges);
    find_carried_data(&nodes, &mut edges);
    (nodes, edges)
}

/// Data nodes whose name (or a `@carried-as` word) occurs in what an edge carries there or, for a call, back (SPEC §3.6).
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
        // A call carries its payload there and its reply back; any other line carries all it says there.
        let call = e.kind == EdgeKind::Call;
        let label = e.label.as_deref().map(|l| match (&e.reply, call) {
            (Some(r), true) => l.replace(&format!("→ {r}"), " "),
            _ => l.to_string(),
        });
        let mut there: Vec<&str> = e.payload.iter().map(String::as_str).collect();
        there.extend(label.as_deref());
        if !call {
            there.extend(e.reply.as_deref());
        }
        let there = there.join(" ").to_lowercase();
        let back = if call {
            e.reply.as_deref().unwrap_or("").to_lowercase()
        } else {
            String::new()
        };
        for (id, words) in &data {
            let names = |text: &str| words.iter().any(|w| crate::glossary::occurs_word(w, text));
            if names(&there) {
                e.carries.push(id.to_string());
            }
            if names(&back) {
                e.carries_back.push(id.to_string());
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
        e.evidence = Some(crate::model::LineEvidence::code(
            index.get(e.to.as_str()).and_then(|n| n.source.as_ref()),
        ));
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

/// Kinds of code: a ref into one of them names a part by its id, never by the sub-part form.
const CODE_KINDS: &[&str] = &[
    "system",
    "service",
    "module",
    "component",
    "function",
    "job",
    "workload",
    "test",
];

pub(crate) fn resolve_ref(
    r: &str,
    nodes: &[Node],
    index: &HashMap<String, usize>,
) -> Option<Resolved> {
    let r = r.trim();
    if let Some(rest) = r.strip_prefix("http:") {
        let (method, path) = rest.trim().split_once(char::is_whitespace)?;
        let wanted = Route::parse(path.trim());
        let node = nodes.iter().find(|n| {
            n.http.as_ref().is_some_and(|h| {
                h.method.eq_ignore_ascii_case(method)
                    && wanted.fit(&Route::parse(&h.path)) == Fit::Certain
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
                end: None,
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

    /// What code touches without naming it binds to what the block, or a caller's block, names.
    #[test]
    fn open_refs_bind_to_what_the_blocks_name() {
        let store = |id: &str| Node {
            kind: "store".into(),
            ..node(id, &[])
        };
        let external = Node {
            kind: "external".into(),
            ..node("ai", &[])
        };
        let mut load = node("load", &[("reads", "walks")]);
        load.tags
            .push(Tag::inferred("reads", "file:", 3, "fs::read_dir(…)"));
        // A helper without a block of its own: its caller names the endpoint.
        let mut post = node("post", &[]);
        post.tags.push(Tag::inferred(
            "calls",
            "net:",
            4,
            "rust-analyzer: ureq Agent::post",
        ));
        let mut chat = node("chat", &[("calls", "ai")]);
        chat.tags
            .push(Tag::inferred("calls", "post", 5, "rust-analyzer: post"));
        // Nothing names a store: no line, no warning.
        let mut lonely = node("lonely", &[]);
        lonely
            .tags
            .push(Tag::inferred("writes", "file:", 6, "fs::write(…)"));
        let nodes = vec![store("walks"), external, load, post, chat, lonely];
        let mut diags = Vec::new();
        let (_, edges) = resolve(nodes, "shop", &mut diags);
        let mut lines: Vec<(String, String, EdgeKind)> = edges
            .iter()
            .map(|e| (e.from.clone(), e.to.clone(), e.kind))
            .collect();
        lines.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        assert_eq!(
            lines,
            [
                ("chat".into(), "ai".into(), EdgeKind::Call),
                ("chat".into(), "post".into(), EdgeKind::Call),
                ("post".into(), "ai".into(), EdgeKind::Call),
                ("walks".into(), "load".into(), EdgeKind::Read),
            ]
        );
        assert!(diags.is_empty(), "{diags:?}");
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
        let (_, edges) = resolve(nodes, "shop", &mut diags);
        let edge = |a: &str, b: &str| edges.iter().find(|e| e.from == a && e.to == b).unwrap();
        let carries = |a: &str, b: &str| edge(a, b).carries.clone();
        assert_eq!(
            carries("panel", "svc"),
            vec!["data.question"],
            "the payload goes there"
        );
        assert_eq!(
            edge("panel", "svc").carries_back,
            vec!["data.reply"],
            "the reply comes back"
        );
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
            Node {
                kind: "store".into(),
                ..node("db", &[])
            },
        ];
        let mut d = Vec::new();
        let (_, edges) = resolve(nodes, "shop", &mut d);
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
        let fits = |a: &str, b: &str| Route::parse(a).fit(&Route::parse(b)) == Fit::Certain;
        assert!(fits("/api/orders/:id", "/api/orders/{id}"));
        assert!(fits("/api/orders/42?x=1", "/api/orders/{id}"));
        assert!(!fits("/api/orders", "/api/orders/{id}"));
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
            Node {
                kind: "store".into(),
                ..node("orders-db", &[])
            },
            node(
                "mailer.send",
                &[("consumes", "order.created"), ("flow", "checkout 3")],
            ),
        ];
        let mut d = Vec::new();
        let (nodes, edges) = resolve(nodes, "shop", &mut d);
        let flows = flow_paths(&nodes, &edges, &mut d);

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
        let (_, edges) = resolve(nodes, "shop", &mut d);
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
        let (nodes, edges) = resolve(nodes, "shop", &mut d);
        let flows = flow_paths(&nodes, &edges, &mut d);
        assert_eq!(d[0].code, "W104");
        assert!(flows[0].steps[1].path.is_empty());
    }

    /// A path the code writes out fits a store's `@path` glob (docair-4wk.25).
    #[test]
    fn paths_fit_store_globs() {
        assert!(path_fits(".env", ".env"));
        assert!(path_fits(
            "./docs/walkthroughs/a.json",
            "docs/walkthroughs/*.json"
        ));
        assert!(
            path_fits("docs/walkthroughs", "docs/walkthroughs/*.json"),
            "the folder of the glob"
        );
        assert!(path_fits("data/a/b/c.csv", "data/**"));
        assert!(path_fits("data/x.csv", "data/**/*.csv"));
        assert!(!path_fits("docs/other/a.json", "docs/walkthroughs/*.json"));
        assert!(
            !path_fits("docs", "docs/walkthroughs/*.json"),
            "a folder further up is no fit"
        );
        assert!(!path_fits(".env.example", ".env"));
    }

    /// A store that says where it is gets a certain line from code that writes its path out, without a block.
    #[test]
    fn a_file_the_code_names_binds_to_the_store_that_says_its_path() {
        let mut walks = Node {
            kind: "store".into(),
            ..node("walks", &[])
        };
        walks.tags.push(Tag {
            name: "path".into(),
            value: "docs/walkthroughs/*.json".into(),
            line: 1,
            origin: Origin::Authored,
            reason: None,
            reach: None,
        });
        let mut load = node("load", &[]);
        load.tags.push(Tag::inferred(
            "reads",
            "file:docs/walkthroughs",
            3,
            "fs::read_dir(\"docs/walkthroughs\")",
        ));
        let nodes = vec![walks, load];
        let mut diags = Vec::new();
        let (_, edges) = resolve(nodes, "shop", &mut diags);
        let read = edges
            .iter()
            .find(|e| e.kind == EdgeKind::Read)
            .expect("a read line");
        assert_eq!((read.from.as_str(), read.to.as_str()), ("walks", "load"));
        assert!(read.reach.is_none(), "certain");
    }
}

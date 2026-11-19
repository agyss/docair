//! Stage 6: rules that need the whole graph (SPEC §8).

use std::collections::HashMap;

use crate::model::{Diagnostic, Edge, Node, Origin, Severity};

pub const KNOWN_TAGS: &[&str] = &[
    "id",
    "kind",
    "part-of",
    "doc",
    "deployed-as",
    "in",
    "out",
    "calls",
    "reads",
    "writes",
    "emits",
    "consumes",
    "http",
    "flow",
    "creates",
    "covers",
    "routes",
    "selects",
    "runs-in",
    "carried-as",
    "path",
    "defines",
];

pub const KNOWN_KINDS: &[&str] = &[
    "system",
    "service",
    "module",
    "component",
    "function",
    "endpoint",
    "store",
    "topic",
    "external",
    "job",
    "workload",
    "config",
    "host",
    "cluster",
    "namespace",
    "network",
    "gateway",
    "container",
    "release",
    "test",
    "data",
    // An API description: an OpenAPI or AsyncAPI file (docs/IMPORTS.md).
    "api",
];

/***
 * Apply the graph-wide rules and return all diagnostics, sorted, with the severity policy applied.
 *
 * W101 unknown tags, W102 unknown kinds, W103 block/code mismatches. With
 * `lenient`, unresolved refs (E001) are warnings.
 *
 * @flow scan 11 "rules become diagnostics"
 * @flow check 3 "rules become diagnostics"
 * @in   graph: Node[] + Edge[] + Flow[] from docair-core.resolve.resolve
 * @out  graph: Graph
 ***/
pub fn check(
    nodes: &[Node],
    edges: &[Edge],
    mut diags: Vec<Diagnostic>,
    lenient: bool,
) -> Vec<Diagnostic> {
    // W108: an edge between a group and one of its own members cannot be drawn as an arrow.
    let parent: HashMap<&str, &str> = nodes
        .iter()
        .filter_map(|n| n.part_of.as_deref().map(|p| (n.id.as_str(), p)))
        .collect();
    let inside = |outer: &str, inner: &str| {
        let mut cur = parent.get(inner).copied();
        for _ in 0..64 {
            match cur {
                Some(p) if p == outer => return true,
                Some(p) => cur = parent.get(p).copied(),
                None => return false,
            }
        }
        false
    };
    for e in edges {
        if inside(&e.from, &e.to) || inside(&e.to, &e.from) {
            diags.push(
                Diagnostic::new(
                    "W108",
                    format!(
                        "`{}` → `{}`: one is inside the other, so the edge is not drawn ({})",
                        e.from, e.to, e.tag
                    ),
                )
                .at(&e.source.file, e.source.line),
            );
        }
    }

    for n in nodes {
        let (file, line) = n
            .source
            .as_ref()
            .map_or(("", 0), |s| (s.file.as_str(), s.line));
        for t in n
            .tags
            .iter()
            .filter(|t| t.origin == Origin::Authored && !KNOWN_TAGS.contains(&t.name.as_str()))
        {
            diags.push(
                Diagnostic::new("W101", format!("unknown tag `@{}`", t.name))
                    .at(file, t.line)
                    .node(&n.id),
            );
        }
        if n.origin != Origin::Placeholder && !KNOWN_KINDS.contains(&n.kind.as_str()) {
            let msg = if n.kind == "unknown" {
                "no @kind and none could be inferred".to_string()
            } else {
                format!("unknown kind `{}`", n.kind)
            };
            diags.push(Diagnostic::new("W102", msg).at(file, line).node(&n.id));
        }
        let authored = n.tags_named("http").find(|t| t.origin == Origin::Authored);
        let inferred = n.tags_named("http").find(|t| t.origin == Origin::Inferred);
        if let (Some(a), Some(i)) = (authored, inferred)
            && a.value.split_whitespace().ne(i.value.split_whitespace())
        {
            let why = i.reason.as_deref().unwrap_or("the code");
            diags.push(
                Diagnostic::new(
                    "W103",
                    format!("@http `{}` but {why} says `{}`", a.value, i.value),
                )
                .at(file, i.line)
                .node(&n.id),
            );
        }
    }
    if lenient {
        for d in diags.iter_mut().filter(|d| d.code == "E001") {
            d.severity = Severity::Warning;
        }
    }
    diags.sort_by(|a, b| (&a.file, a.line, &a.code).cmp(&(&b.file, b.line, &b.code)));
    diags.dedup();
    diags
}

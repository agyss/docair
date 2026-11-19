//! Tidy: the hand-written tags that only repeat what the code says (docs/ADAPTERS.md §6.1).
//!
//! A tag is repeated when the same part has a line the code says for certain, of the same kind
//! to the same target, and the tag adds nothing to it: no words of its own (`"…"`), and no
//! wording of what travels that differs from the code's. Tidy never touches `@id`, `@kind`,
//! `@part-of`, `@flow` or a block's text, and a shortcut (the author wrote A -> C, the code goes
//! A -> B -> C) is no repeat: it is the author's summary.

use std::collections::HashMap;

use serde::Serialize;

use crate::model::{Change, Graph, Origin, Tag};
use crate::resolve::{resolve_ref, split_payload};

/// The tags that make lines, as authors write them; `@covers` is intent, not code.
const LINES: &[&str] = &[
    "calls", "in", "out", "reads", "writes", "emits", "consumes", "creates", "routes", "selects",
];

/// One hand-written tag the code repeats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepeatedTag {
    /// The part whose block holds the tag.
    pub node: String,
    /// The tag as written: `@calls docair-core.scan`.
    pub tag: String,
    /// Its line in the block's file.
    pub line: usize,
    /// What the code says: the inferred tag's value, and where in the code.
    pub code_says: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_file: Option<String>,
    pub code_line: usize,
    /// Why the code says it: `rust-analyzer: call`, …
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// What a line tag points at: the ref after `from`/`to` for `@in`/`@out`; `None` for a signature.
fn target(tag: &Tag) -> Option<String> {
    let value = match tag.name.as_str() {
        "in" => tag.value.rsplit_once(" from ")?.1,
        "out" => tag.value.rsplit_once(" to ")?.1,
        _ => tag.value.as_str(),
    };
    Some(split_payload(value).target.trim().to_string())
}

/// The wording of what travels, as one string to compare: what is sent and what comes back.
fn wording(tag: &Tag) -> (Option<String>, Option<String>) {
    let value = match tag.name.as_str() {
        "in" => tag.value.rsplit_once(" from ").map_or("", |(v, _)| v),
        "out" => tag.value.rsplit_once(" to ").map_or("", |(v, _)| v),
        _ => tag.value.as_str(),
    };
    let v = split_payload(value);
    let norm = |s: Option<String>| {
        s.map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|s| !s.is_empty())
    };
    match tag.name.as_str() {
        // A signature-like `@in order: Order from x`: its words before `from` are what travels.
        "in" | "out" => (norm(Some(value.to_string())), None),
        _ => (norm(v.sends), norm(v.receives)),
    }
}

/***
 * Every hand-written tag the code repeats, in the order of the parts and their tags.
 *
 * @id      docair-core.tidy.repeated
 * @calls   docair-core.resolve.split_payload "reads a tag's target, words and wording"
 ***/
pub fn repeated(graph: &Graph) -> Vec<RepeatedTag> {
    let index: HashMap<String, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.clone(), i))
        .collect();
    let node_of = |t: String| match resolve_ref(&t, &graph.nodes, &index) {
        Some(r) => r.id,
        None => t,
    };
    let mut out = Vec::new();
    for n in &graph.nodes {
        let inferred: Vec<(&Tag, String)> = n
            .tags
            .iter()
            .filter(|t| {
                t.origin == Origin::Inferred
                    && t.reach.is_none()
                    && LINES.contains(&t.name.as_str())
            })
            .filter_map(|t| Some((t, node_of(target(t)?))))
            .collect();
        if inferred.is_empty() {
            continue;
        }
        for t in n
            .tags
            .iter()
            .filter(|t| t.origin == Origin::Authored && LINES.contains(&t.name.as_str()))
        {
            let Some(to) = target(t).map(&node_of) else {
                continue;
            };
            let value = match t.name.as_str() {
                "in" | "out" => "",
                _ => t.value.as_str(),
            };
            // Words of its own make a tag the author's, not the code's.
            if split_payload(value).text.is_some() || t.value.contains('"') {
                continue;
            }
            let Some((code, _)) = inferred
                .iter()
                .find(|(c, cto)| c.name == t.name && *cto == to)
            else {
                continue;
            };
            let (sends, receives) = wording(t);
            let (code_sends, code_receives) = wording(code);
            let adds =
                |mine: &Option<String>, theirs: &Option<String>| mine.is_some() && mine != theirs;
            if adds(&sends, &code_sends) || adds(&receives, &code_receives) {
                continue;
            }
            out.push(RepeatedTag {
                node: n.id.clone(),
                tag: format!("@{} {}", t.name, t.value),
                line: t.line,
                code_says: format!("@{} {}", code.name, code.value),
                code_file: n.source.as_ref().map(|s| s.file.clone()),
                code_line: code.line,
                reason: code.reason.clone(),
            });
        }
    }
    out
}

/***
 * The edits that remove the repeated tags: one per part, each removing its repeated tags, for `edit::resolve` and `edit::patch`.
 *
 * @id      docair-core.tidy.changes
 ***/
pub fn changes(repeated: &[RepeatedTag]) -> Vec<Change> {
    let mut by_node: Vec<Change> = Vec::new();
    for r in repeated {
        match by_node.iter_mut().find(|c| c.node == r.node) {
            Some(c) => c.remove.push(r.tag.clone()),
            None => by_node.push(Change {
                node: r.node.clone(),
                mode: String::new(),
                file: String::new(),
                line: 0,
                add: Vec::new(),
                remove: vec![r.tag.clone()],
                summary: None,
                why: "the code says the same: docair tidy".to_string(),
            }),
        }
    }
    by_node
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> Graph {
        let tag = |name: &str, value: &str, line: usize, origin: &str, reach: Option<&str>| {
            let mut t =
                serde_json::json!({ "name": name, "value": value, "line": line, "origin": origin });
            if let Some(r) = reach {
                t["reach"] = serde_json::json!(r);
            }
            t
        };
        let node = |id: &str, tags: Vec<serde_json::Value>| {
            serde_json::json!({
                "id": id, "kind": "function", "name": id, "summary": "", "description": "", "part_of": "app",
                "source": { "file": "src/lib.rs", "line": 1 }, "docs": [], "tags": tags, "origin": "authored"
            })
        };
        serde_json::from_value(serde_json::json!({
            "schema": "docair.graph/v0", "system": "app", "generator": "test", "flows": [], "edges": [], "diagnostics": [],
            "nodes": [
                node("app", vec![]),
                node("run", vec![
                    tag("calls", "load", 3, "authored", None),
                    tag("calls", "save \"keeps the result\"", 4, "authored", None),
                    tag("calls", "send (Order) -> Receipt", 5, "authored", None),
                    tag("calls", "guess", 6, "authored", None),
                    tag("calls", "shortcut", 7, "authored", None),
                    tag("calls", "load", 20, "inferred", None),
                    tag("calls", "save", 21, "inferred", None),
                    tag("calls", "send (Order) -> Bill", 22, "inferred", None),
                    tag("calls", "guess", 23, "inferred", Some("could")),
                ]),
                node("load", vec![]), node("save", vec![]), node("send", vec![]), node("guess", vec![]), node("shortcut", vec![]),
            ]
        }))
        .unwrap()
    }

    /// Only a tag the code says for certain, with no words and no other wording, is repeated.
    #[test]
    fn only_a_certain_wordless_tag_the_code_says_is_repeated() {
        let found = repeated(&graph());
        let tags: Vec<&str> = found.iter().map(|r| r.tag.as_str()).collect();
        assert_eq!(
            tags,
            ["@calls load"],
            "words, other wording, a could line and a shortcut stay"
        );
        assert_eq!(found[0].code_line, 20);
        assert_eq!(found[0].code_says, "@calls load");
        let c = changes(&found);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].node, "run");
        assert_eq!(c[0].remove, ["@calls load"]);
    }
}

//! Words for the lines: the model says what happens along each edge that has no wording (SPEC §10.8).

use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, bail};
use docair_core::edit;
use docair_core::model::{Change, Edge, EdgeKind, Graph, Node, Origin};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ask::Reply;
use crate::client::{Message, Usage, chat};
use crate::config::AiConfig;

/// The rules for the wording.
pub const WORDING_PROMPT: &str = r#"You write the words on the lines of an architecture chart. Each line connects two parts of a software system; docair draws it as an arrow. Today many lines carry no words, or only a type signature, so a reader cannot tell what happens along them.

For each line you get its number, its kind (call, read, write, create, cover, emit, consume, data), both ends with their kind, name and summary, and what the code says it passes, if anything.

Write for each line 2 to 7 words that say what happens along it, from the side of the line's source, in the present tense, starting with a verb. Say the purpose, not the types:
- call: "scans the uploaded folder", "asks the model", "redraws the picture"
- read: "reads the upload limits", "loads the recorded walkthroughs"
- write: "saves the walkthrough as JSON"
- cover (a test → what it checks): "checks that a folder becomes a page"
- create: "installs the chart with helm"
Do not repeat the target's name when the arrow already points at it. Write in the language of the summaries.

Reply with exactly one JSON object: {"lines": [{"n": 1, "text": "..."}]}, one entry per line you were given."#;

/// How many lines go into one request.
pub const BATCH: usize = 40;

/// The words for one edge, as the viewer shows them before they are applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worded {
    pub from: String,
    pub to: String,
    pub text: String,
}

/// Lines that say nothing yet: no authored words, no authored payload (with `all`: no words,
/// whatever their payload). Calls first, tests last.
pub fn unworded(graph: &Graph, scope: Option<&str>, all: bool) -> Vec<usize> {
    let parent: BTreeMap<&str, &str> = graph
        .nodes
        .iter()
        .filter_map(|n| n.part_of.as_deref().map(|p| (n.id.as_str(), p)))
        .collect();
    let inside = |id: &str, group: &str| {
        let mut cur = Some(id);
        for _ in 0..64 {
            match cur {
                Some(c) if c == group => return true,
                Some(c) => cur = parent.get(c).copied(),
                None => return false,
            }
        }
        false
    };
    let mut out: Vec<usize> = graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            e.text.is_none()
                && e.origin == Origin::Authored
                && e.kind != EdgeKind::Data
                && (all || e.words_from_code() || (e.payload.is_none() && e.reply.is_none()))
                && scope.is_none_or(|g| inside(&e.from, g) || inside(&e.to, g))
        })
        .map(|(i, _)| i)
        .collect();
    let rank = |k: EdgeKind| match k {
        EdgeKind::Call => 0,
        EdgeKind::Read | EdgeKind::Write => 1,
        EdgeKind::Cover => 3,
        _ => 2,
    };
    out.sort_by_key(|&i| rank(graph.edges[i].kind));
    out
}

/// The tag that made the edge, with the words inserted after its ref, as an edit of the node that carries it.
pub fn change_for(edge: &Edge, text: &str, nodes: &[Node]) -> Option<Change> {
    let tag = edge.tag.trim().trim_start_matches('@');
    let (name, value) = tag.split_once(char::is_whitespace)?;
    let owner = match name {
        "reads" | "consumes" | "in" => &edge.to,
        _ => &edge.from,
    };
    let node = nodes.iter().find(|n| &n.id == owner)?;
    if node.origin != Origin::Authored {
        return None;
    }
    let value = value.trim();
    let (target, rest) = value.split_once(char::is_whitespace).unwrap_or((value, ""));
    if value.contains('"') {
        return None;
    }
    let words = text.replace('"', "'");
    let new = format!("@{name} {target} \"{words}\" {rest}")
        .trim()
        .to_string();
    Some(Change {
        node: owner.clone(),
        mode: String::new(),
        file: String::new(),
        line: 0,
        add: vec![new],
        remove: vec![format!("@{name} {value}")],
        summary: None,
        why: format!("Words for the line {} → {}.", edge.from, edge.to),
    })
}

fn end_json(n: Option<&Node>) -> Value {
    match n {
        Some(n) => json!({ "id": n.id, "kind": n.kind, "name": n.name, "summary": n.summary }),
        None => Value::Null,
    }
}

#[derive(Deserialize)]
struct Lines {
    #[serde(default)]
    lines: Vec<Line>,
}

#[derive(Deserialize)]
struct Line {
    n: usize,
    #[serde(default)]
    text: String,
}

/***
 * Ask the model for words on the lines that say nothing, in batches, and turn them into edits of the tags that made the lines.
 *
 * @id      docair-ai.word.word
 * @calls   docair-ai.client.chat (lines with both ends) -> words per line
 * @calls   docair-core.edit.resolve "finds where each tag edit goes"
 * @out     reply: Reply with wording and edits
 ***/
pub fn word(
    cfg: &AiConfig,
    graph: &Graph,
    scope: Option<&str>,
    limit: usize,
    all: bool,
) -> Result<Reply> {
    let mut todo = unworded(graph, scope, all);
    let total = todo.len();
    todo.truncate(limit);
    if todo.is_empty() {
        return Ok(Reply {
            answer: "Every line already has words.".into(),
            walkthrough: None,
            view: None,
            changes: vec![],
            wording: vec![],
            glossary: vec![],
            problems: vec![],
            usage: Usage::default(),
            seconds: 0.0,
            requests: 0,
        });
    }
    let by_id: BTreeMap<&str, &Node> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut usage = Usage::default();
    let mut seconds = 0.0;
    let mut requests = 0u8;
    let mut wording = Vec::new();
    let mut problems = Vec::new();
    for batch in todo.chunks(BATCH) {
        let lines: Vec<Value> = batch
            .iter()
            .enumerate()
            .map(|(k, &i)| {
                let e = &graph.edges[i];
                let code = [&e.payload, &e.reply]
                    .iter()
                    .any(|x| x.is_some())
                    .then(|| e.label.clone())
                    .flatten();
                json!({
                    "n": k + 1,
                    "kind": e.kind,
                    "from": end_json(by_id.get(e.from.as_str()).copied()),
                    "to": end_json(by_id.get(e.to.as_str()).copied()),
                    "code": code,
                })
            })
            .collect();
        let messages = [
            Message::system(WORDING_PROMPT),
            Message::user(format!(
                "System `{}`. The lines:\n{}",
                graph.system,
                serde_json::to_string(&lines).expect("lines serialize")
            )),
        ];
        let reply = chat(cfg, &messages, true)?;
        requests = requests.saturating_add(1);
        usage += reply.usage;
        seconds += reply.seconds;
        let content = &reply.content;
        let json = content
            .find('{')
            .zip(content.rfind('}'))
            .map(|(a, b)| &content[a..=b])
            .unwrap_or("{}");
        let parsed: Lines = match serde_json::from_str(json) {
            Ok(l) => l,
            Err(e) => {
                problems.push(format!("a batch of words could not be read: {e}"));
                continue;
            }
        };
        let mut seen = HashSet::new();
        for line in parsed.lines {
            let text = line.text.trim().trim_matches('"').trim();
            let Some(&i) = line.n.checked_sub(1).and_then(|k| batch.get(k)) else {
                continue;
            };
            if text.is_empty() || text.chars().count() > 80 || !seen.insert(i) {
                continue;
            }
            wording.push((i, text.to_string()));
        }
    }
    if wording.is_empty() {
        bail!("the model gave no words for the lines");
    }
    // One edit per node, so a block gets all its new words at once.
    let mut per_node: BTreeMap<String, Change> = BTreeMap::new();
    let mut out = Vec::new();
    for (i, text) in &wording {
        let e = &graph.edges[*i];
        out.push(Worded {
            from: e.from.clone(),
            to: e.to.clone(),
            text: text.clone(),
        });
        let Some(c) = change_for(e, text, &graph.nodes) else {
            problems.push(format!(
                "the words for {} → {} have no authored tag to go into",
                e.from, e.to
            ));
            continue;
        };
        let entry = per_node.entry(c.node.clone()).or_insert_with(|| Change {
            why: String::new(),
            ..c.clone()
        });
        if entry.add != c.add {
            for (a, r) in c.add.iter().zip(&c.remove) {
                if !entry.add.contains(a) {
                    entry.add.push(a.clone());
                    entry.remove.push(r.clone());
                }
            }
        }
    }
    let mut changes = Vec::new();
    for (_, mut c) in per_node {
        c.why = format!("Words for {} lines of {}.", c.add.len(), c.node);
        match edit::resolve(&c, &graph.nodes) {
            Ok(c) => changes.push(c),
            Err(e) => problems.push(e),
        }
    }
    let more = total.saturating_sub(out.len());
    Ok(Reply {
        answer: if more > 0 {
            format!(
                "Worded {} lines; {more} more have no words yet. Ask again for the next ones.",
                out.len()
            )
        } else {
            format!("Worded {} lines.", out.len())
        },
        walkthrough: None,
        view: None,
        changes,
        wording: out,
        glossary: vec![],
        problems,
        usage,
        seconds,
        requests,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use docair_core::model::*;

    fn node(id: &str, part_of: Option<&str>) -> Node {
        Node {
            id: id.into(),
            kind: "function".into(),
            name: id.into(),
            summary: String::new(),
            description: String::new(),
            part_of: part_of.map(String::from),
            source: Some(SourceRef {
                file: "a.rs".into(),
                line: 1,
                end: None,
            }),
            symbol: None,
            http: None,
            deployed_as: None,
            docs: vec![],
            tags: vec![],
            origin: Origin::Authored,
            test: None,
            signature: None,
            runs_in: None,
            code_from: None,
        }
    }

    fn edge(from: &str, to: &str, kind: EdgeKind, tag: &str) -> Edge {
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
            tag: tag.into(),
            source: SourceRef {
                file: "a.rs".into(),
                line: 2,
                end: None,
            },
            origin: Origin::Authored,
        }
    }

    #[test]
    fn unworded_lines_calls_first_within_scope() {
        let mut worded = edge("a", "b", EdgeKind::Call, "@calls b \"does it\"");
        worded.text = Some("does it".into());
        let graph = Graph {
            schema: SCHEMA.into(),
            system: "s".into(),
            generator: "t".into(),
            nodes: vec![
                node("g", None),
                node("a", Some("g")),
                node("b", None),
                node("t", None),
            ],
            edges: vec![
                edge("t", "a", EdgeKind::Cover, "@covers a"),
                edge("a", "b", EdgeKind::Call, "@calls b"),
                worded,
                edge("b", "t", EdgeKind::Call, "@calls t"),
            ],
            flows: vec![],
            walkthroughs: vec![],
            view: View::default(),
            glossary: vec![],
            library: vec![],
            meanings: None,
            diagnostics: vec![],
        };
        assert_eq!(unworded(&graph, None, false), vec![1, 3, 0]);
        assert_eq!(unworded(&graph, Some("g"), false), vec![1, 0]);
    }

    #[test]
    fn words_go_after_the_ref_of_the_tag_that_made_the_line() {
        let nodes = vec![node("a", None), node("store", None)];
        let call = edge("a", "b", EdgeKind::Call, "@calls b (x) -> Y");
        let c = change_for(&call, "asks \"the\" model", &nodes).unwrap();
        assert_eq!(c.node, "a");
        assert_eq!(c.add, vec!["@calls b \"asks 'the' model\" (x) -> Y"]);
        assert_eq!(c.remove, vec!["@calls b (x) -> Y"]);
        let read = edge("store", "a", EdgeKind::Read, "@reads store -> Cfg");
        assert_eq!(
            change_for(&read, "reads the limits", &nodes).unwrap().node,
            "a".to_string()
        );
    }
}

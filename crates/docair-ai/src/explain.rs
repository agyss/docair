//! The AI explains what two commits change (docs/EXPLAIN.md): it links what docair could not prove
//! is the same, says what each changed flow now does, and words each change on its own, from the
//! user's side, what used to happen and what happens now. It sees only the changed flows, never the whole graph, and every claim is bound
//! against the two graphs: what names nothing is dropped and said.

use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use docair_core::changes::{Changes, FlowChange};
use docair_core::model::{Graph, Node};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::client::{Message, chat};
use crate::config::AiConfig;

pub const SCHEMA: &str = "docair.explained/0";
/// A request carries at most this much of the changed flows (docs/EXPLAIN.md §7).
pub const MAX_CHARS: usize = 60_000;
/// A run asks the model at most this often, unless `[explain] max_requests` says otherwise.
pub const MAX_REQUESTS: usize = 8;
/// At most this many requests of one explain go to the model at once (docair-8l6.14).
pub const AT_ONCE: usize = 3;

/// How far an explain is: what it asks, and how many of its requests have answered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ExplainProgress {
    /// The flows it asks about: those the store does not explain yet, `left` included.
    pub flows: usize,
    /// The requests it sends: at most `max_requests`.
    pub requests: usize,
    pub started: usize,
    pub answered: usize,
    /// The flows left for a later run, beyond `max_requests`.
    pub left: usize,
    /// The flows of the requests the model works on now.
    pub asking: Vec<String>,
}

const EXPLAIN_PROMPT: &str = r#"You explain changes between two commits of a software system to a reviewer. You get only the flows that changed: each with its parts at A (before) and at B (after), the parts and lines added and removed, which parts changed, and its change steps: each change on its own, with an id. Answer as JSON:
{"links": [{"a": "<id at A>", "b": "<id at B>", "why": "<a few words>"}],
 "flows": [{"seed": "<the flow's seed, as given>", "explanation": "<2 to 4 plain sentences: what the change does to this flow, and why it matters; name parts by their ids in backticks>",
            "steps": {"<a change step's id, as given>": "<1 or 2 plain sentences from the user's side: what used to happen here, and what happens now; name parts by their ids in backticks>"}}]}
"links": a part removed at A that is the same as a part added at B (renamed and changed); only from the lists "removed at A" and "added at B", and only when sure.
"steps": words for every change step of the flow, by the step's id exactly as given; only the changes, not the rest of the flow.
Name only ids you were given. Explain every flow you get."#;

/// Who wrote a finding, with what and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recorded {
    /// `model` or `human`: a human's finding is never asked about again.
    pub by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub at: String,
}

/// A part at A that is the same as a part at B, as the model says (docair proved no move).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeLink {
    pub a: String,
    pub b: String,
    pub why: String,
    pub recorded: Recorded,
}

/// What the change does to one flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowExplained {
    pub seed: String,
    pub explanation: String,
    /// The words on each of the flow's change steps, by the step's id (docs/EXPLAIN.md §8,
    /// `docair_core::change_steps`): what used to happen there and what happens now.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub steps: BTreeMap<String, String>,
    pub recorded: Recorded,
}

/// The findings about two commits: kept in the store, the next run asks only about what is new.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeFindings {
    pub schema: String,
    pub a: String,
    pub b: String,
    pub links: Vec<ChangeLink>,
    pub flows: Vec<FlowExplained>,
    /// What the model said that did not bind, and so was left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
    /// The changed flows not explained: the run reached its limit of requests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left: Vec<String>,
    #[serde(default)]
    pub requests: usize,
}

/// A part in a few words: id, kind, name and its summary's first line.
fn part_json(g: &Graph, id: &str) -> Value {
    match g.nodes.iter().find(|n| n.id == id) {
        Some(n) => json!({
            "id": n.id, "kind": n.kind, "name": n.name,
            "summary": n.summary.lines().next().unwrap_or(""),
            "signature": n.signature.as_ref().map(|s| s.wording()),
        }),
        None => json!({ "id": id }),
    }
}

/// A flow's change steps for the model: each with its id, and its parts in a few words, from the graph they are in.
fn steps_json(a: &Graph, b: &Graph, changes: &Changes, f: &FlowChange) -> Vec<Value> {
    docair_core::change_steps::change_steps(a, b, changes, f)
        .iter()
        .map(|s| {
            let at =
                |g: &Graph, ids: &[String]| ids.iter().map(|p| part_json(g, p)).collect::<Vec<_>>();
            json!({
                "id": s.id,
                "kind": format!("{:?}", s.kind).to_lowercase(),
                "added": at(b, &s.added),
                "removed": at(a, &s.removed),
                "code changed": at(b, &s.code),
                "between": at(b, &s.ends),
                "lines_added": s.lines_added,
                "lines_removed": s.lines_removed,
                "in": s.in_box.as_deref().map(|x| part_json(b, x)),
            })
        })
        .collect()
}

/// The ids of a flow's change steps, as docair works them out.
fn step_ids(a: &Graph, b: &Graph, changes: &Changes, f: &FlowChange) -> HashSet<String> {
    docair_core::change_steps::change_steps(a, b, changes, f)
        .into_iter()
        .map(|s| s.id)
        .collect()
}

/***
 * The AI's input for some changed flows: each flow's seed, how it changed, the parts on it that changed and those added and removed (in a few words, from the graph they are in), its lines added and removed, and its change steps with their ids; and, for linking, the parts removed at A and added at B.
 *
 * @id    docair-ai.explain.digest
 * @calls docair-core.change_steps.change_steps "each change of a flow on its own, with a stable id"
 ***/
pub fn digest(
    changes: &Changes,
    a: &Graph,
    b: &Graph,
    flows: &[&FlowChange],
    with_unlinked: bool,
) -> String {
    let flows: Vec<Value> = flows
        .iter()
        .map(|f| {
            json!({
                "seed": f.seed, "how": f.how, "certain": f.certain,
                "changed": f.through.iter().map(|p| part_json(b, p)).collect::<Vec<_>>(),
                "parts_added": f.parts_added.iter().map(|p| part_json(b, p)).collect::<Vec<_>>(),
                "parts_removed": f.parts_removed.iter().map(|p| part_json(a, p)).collect::<Vec<_>>(),
                "lines_added": f.lines_added, "lines_removed": f.lines_removed,
                "steps": steps_json(a, b, changes, f),
            })
        })
        .collect();
    let mut out = json!({ "system": b.system, "flows": flows });
    if with_unlinked {
        out["removed at A"] = json!(
            changes
                .graph
                .removed
                .iter()
                .map(|p| part_json(a, p))
                .collect::<Vec<_>>()
        );
        out["added at B"] = json!(
            changes
                .graph
                .added
                .iter()
                .map(|p| part_json(b, p))
                .collect::<Vec<_>>()
        );
    }
    out.to_string()
}

/// The ids an explanation names in backticks that look like ids (`docair-core.scan`, `data.order`).
fn named_ids(text: &str) -> Vec<&str> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter(|w| !w.contains(' ') && (w.contains('.') || w.contains(':')))
        .collect()
}

/// A JSON object in a reply, also inside a ```json fence.
fn reply_json(content: &str) -> Option<Value> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    serde_json::from_str(&content[start..=end]).ok()
}

/***
 * Bind the model's answer against the two graphs: a link only from a part removed at A to one added at B; an explanation only of a flow that changed, and only naming parts of A or B; words only on the flow's own change steps, by the ids docair made, and only naming parts of A or B. What does not bind is left out and said.
 *
 * @id docair-ai.explain.bind
 ***/
pub fn bind(
    changes: &Changes,
    a: &Graph,
    b: &Graph,
    reply: &Value,
    recorded: &Recorded,
) -> (Vec<ChangeLink>, Vec<FlowExplained>, Vec<String>) {
    let ids_a: HashSet<&str> = a.nodes.iter().map(|n| n.id.as_str()).collect();
    let ids_b: HashSet<&str> = b.nodes.iter().map(|n| n.id.as_str()).collect();
    let line_ids: HashSet<&str> = a
        .edges
        .iter()
        .chain(&b.edges)
        .map(|e| e.id.as_str())
        .collect();
    let removed: HashSet<&str> = changes.graph.removed.iter().map(String::as_str).collect();
    let added: HashSet<&str> = changes.graph.added.iter().map(String::as_str).collect();
    let seeds: HashSet<&str> = changes.flows.iter().map(|f| f.seed.as_str()).collect();
    let text = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let mut problems = Vec::new();
    let mut links = Vec::new();
    for l in reply
        .get("links")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (la, lb) = (text(l, "a"), text(l, "b"));
        if removed.contains(la.as_str()) && added.contains(lb.as_str()) {
            links.push(ChangeLink {
                a: la,
                b: lb,
                why: text(l, "why"),
                recorded: recorded.clone(),
            });
        } else {
            problems.push(format!("the link `{la}` → `{lb}` is not from a part removed at A to one added at B; left out"));
        }
    }
    // Words that name only parts and lines there are.
    let names_known = |words: &str| -> Vec<String> {
        named_ids(words)
            .into_iter()
            .filter(|i| !ids_a.contains(i) && !ids_b.contains(i) && !line_ids.contains(i))
            .map(str::to_string)
            .collect()
    };
    let mut flows = Vec::new();
    for f in reply
        .get("flows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let seed = text(f, "seed");
        if !seeds.contains(seed.as_str()) {
            problems.push(format!("`{seed}` is no flow that changed; left out"));
            continue;
        }
        let explanation = text(f, "explanation");
        let unknown = names_known(&explanation);
        if explanation.is_empty() || !unknown.is_empty() {
            problems.push(format!(
                "the explanation of `{seed}` names {}; left out",
                if unknown.is_empty() {
                    "nothing".to_string()
                } else {
                    unknown
                        .iter()
                        .map(|u| format!("`{u}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ));
            continue;
        }
        let flow = changes.flows.iter().find(|x| x.seed == seed);
        let known = flow.map(|x| step_ids(a, b, changes, x)).unwrap_or_default();
        let mut steps = BTreeMap::new();
        for (id, words) in f
            .get("steps")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let words = words.as_str().unwrap_or("").trim().to_string();
            if !known.contains(id) {
                problems.push(format!("`{id}` is no change step of `{seed}`; left out"));
                continue;
            }
            let unknown = names_known(&words);
            if words.is_empty() || !unknown.is_empty() {
                problems.push(format!(
                    "the words on step `{id}` name {}; left out",
                    if unknown.is_empty() {
                        "nothing".to_string()
                    } else {
                        unknown
                            .iter()
                            .map(|u| format!("`{u}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                ));
                continue;
            }
            steps.insert(id.clone(), words);
        }
        flows.push(FlowExplained {
            seed,
            explanation,
            steps,
            recorded: recorded.clone(),
        });
    }
    (links, flows, problems)
}

/***
 * Explain the changes: the flows the store does not explain yet (a human's finding is never asked about again) go to the model in requests of at most `MAX_CHARS`, at most `max_requests` of them and at most `AT_ONCE` at a time; the answers are bound against both graphs, in the requests' order, and joined with what the store had. A request that fails leaves its flows; when none answers, the error is returned.
 *
 * @id    docair-ai.explain.explain
 * @calls docair-ai.explain.digest "sends only the changed flows"
 * @calls docair-ai.client.chat (the changed flows and their change steps) -> links, explanations, words per change step
 * @calls docair-ai.explain.bind "keeps only what binds against both graphs"
 ***/
pub fn explain(
    cfg: &AiConfig,
    a: &Graph,
    b: &Graph,
    changes: &Changes,
    kept: Option<&ChangeFindings>,
    max_requests: usize,
) -> Result<ChangeFindings> {
    explain_with(cfg, a, b, changes, kept, max_requests, &|_| {})
}

/// As `explain`, saying how far it is each time a request starts or answers (`docair serve`'s explain jobs, docs/EXPLAIN.md §8).
pub fn explain_with(
    cfg: &AiConfig,
    a: &Graph,
    b: &Graph,
    changes: &Changes,
    kept: Option<&ChangeFindings>,
    max_requests: usize,
    on: &(dyn Fn(&ExplainProgress) + Sync),
) -> Result<ChangeFindings> {
    // A stored finding explains its flow when a human wrote it, or it words the flow's change
    // steps; one stored before the change steps (no step words) is asked about again, and
    // replaced when answered (docair-8l6.15).
    let explained = |k: &FlowExplained| {
        k.recorded.by == "human"
            || !k.steps.is_empty()
            || changes
                .flows
                .iter()
                .find(|f| f.seed == k.seed)
                .is_none_or(|f| step_ids(a, b, changes, f).is_empty())
    };
    let known: HashSet<&str> = kept
        .map(|k| {
            k.flows
                .iter()
                .filter(|f| explained(f))
                .map(|f| f.seed.as_str())
                .collect()
        })
        .unwrap_or_default();
    let todo: Vec<&FlowChange> = changes
        .flows
        .iter()
        .filter(|f| !known.contains(f.seed.as_str()))
        .collect();
    let mut out = ChangeFindings {
        schema: SCHEMA.to_string(),
        a: changes.a.clone(),
        b: changes.b.clone(),
        links: kept.map(|k| k.links.clone()).unwrap_or_default(),
        flows: kept.map(|k| k.flows.clone()).unwrap_or_default(),
        problems: Vec::new(),
        left: Vec::new(),
        requests: 0,
    };
    // Flows in requests of at most MAX_CHARS; the parts to link go with the first.
    let mut batches: Vec<Vec<&FlowChange>> = Vec::new();
    for f in todo {
        let fits = batches.last().is_some_and(|last| {
            let mut next = last.clone();
            next.push(f);
            digest(changes, a, b, &next, batches.len() == 1).len() <= MAX_CHARS
        });
        match batches.last_mut() {
            Some(last) if fits => last.push(f),
            _ => batches.push(vec![f]),
        }
    }
    let recorded = Recorded {
        by: "model".into(),
        model: Some(cfg.model.clone()),
        at: crate::ask::now_rfc3339(),
    };
    // At most `max_requests` go out, at most AT_ONCE at a time; each says how far it is.
    let sent = batches.len().min(max_requests);
    let progress = Mutex::new(ExplainProgress {
        flows: batches.iter().map(Vec::len).sum(),
        requests: sent,
        left: batches[sent..].iter().map(Vec::len).sum(),
        ..ExplainProgress::default()
    });
    on(&progress.lock().unwrap_or_else(|e| e.into_inner()));
    let next = AtomicUsize::new(0);
    let answers: Mutex<Vec<(usize, Result<String>)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..AT_ONCE.min(sent) {
            s.spawn(|| {
                loop {
                    let k = next.fetch_add(1, Ordering::SeqCst);
                    if k >= sent {
                        break;
                    }
                    let batch = &batches[k];
                    let seeds: Vec<String> = batch.iter().map(|f| f.seed.clone()).collect();
                    {
                        let mut p = progress.lock().unwrap_or_else(|e| e.into_inner());
                        p.started += 1;
                        p.asking.extend(seeds.iter().cloned());
                        on(&p);
                    }
                    let messages = [
                        Message::system(EXPLAIN_PROMPT),
                        Message::user(format!(
                            "The changes from `{}` to `{}`:\n{}",
                            changes.a,
                            changes.b,
                            digest(changes, a, b, batch, k == 0)
                        )),
                    ];
                    let reply = chat(cfg, &messages, true).map(|r| r.content);
                    {
                        let mut p = progress.lock().unwrap_or_else(|e| e.into_inner());
                        p.answered += 1;
                        p.asking.retain(|x| !seeds.contains(x));
                        on(&p);
                    }
                    answers
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push((k, reply));
                }
            });
        }
    });
    // The answers in the batches' order, so the findings do not depend on which came first;
    // the links go with the first batch, and are merged with the others.
    let mut answers = answers.into_inner().unwrap_or_else(|e| e.into_inner());
    answers.sort_by_key(|(k, _)| *k);
    let mut failed: Option<anyhow::Error> = None;
    let mut read = 0;
    for (k, reply) in answers {
        let batch = &batches[k];
        let content = match reply {
            Ok(c) => c,
            Err(e) => {
                out.problems.push(format!(
                    "a request to the model failed ({e:#}); its flows are left"
                ));
                out.left.extend(batch.iter().map(|f| f.seed.clone()));
                failed.get_or_insert(e);
                continue;
            }
        };
        read += 1;
        let Some(value) = reply_json(&content) else {
            out.problems
                .push("the model's answer cannot be read; its flows are left".into());
            out.left.extend(batch.iter().map(|f| f.seed.clone()));
            continue;
        };
        let (links, flows, problems) = bind(changes, a, b, &value, &recorded);
        for l in links {
            if !out.links.iter().any(|x| x.a == l.a && x.b == l.b) {
                out.links.push(l);
            }
        }
        for f in flows {
            out.flows.retain(|x| x.seed != f.seed);
            out.flows.push(f);
        }
        out.problems.extend(problems);
    }
    for batch in &batches[sent..] {
        out.left.extend(batch.iter().map(|f| f.seed.clone()));
    }
    out.requests = sent;
    // Not one request answered: the model cannot be reached, which is an error, not a finding.
    if read == 0
        && let Some(e) = failed
    {
        return Err(e);
    }
    Ok(out)
}

/***
 * The findings as a pull request comment: one line per changed flow with its explanation under it and the words on each of its changes, links folded in `<details>`.
 *
 * @id docair-ai.explain.comment
 ***/
pub fn comment(changes: &Changes, explained: &ChangeFindings, b: &Graph) -> String {
    let name = |id: &str| {
        b.nodes
            .iter()
            .find(|n| n.id == id)
            .map_or(id.to_string(), |n: &Node| n.name.clone())
    };
    let by_seed: BTreeMap<&str, &FlowExplained> = explained
        .flows
        .iter()
        .map(|f| (f.seed.as_str(), f))
        .collect();
    let mut out = format!(
        "### docair: what `{}` → `{}` changes\n\n",
        changes.a, changes.b
    );
    if changes.flows.is_empty() {
        out.push_str("No flow changes.\n");
    }
    for f in &changes.flows {
        let how = match f.how.as_str() {
            "new" => "new flow",
            "gone" => "flow gone",
            _ => "changed",
        };
        out.push_str(&format!(
            "- **{}** ({how}{})\n",
            f.seed,
            if f.certain {
                ""
            } else {
                ", rests on a line that may not go there"
            }
        ));
        if let Some(e) = by_seed.get(f.seed.as_str()) {
            out.push_str(&format!("  {}\n", e.explanation));
            // Each change on its own, in the model's words.
            for words in e.steps.values() {
                out.push_str(&format!("  - {words}\n"));
            }
        }
    }
    if !explained.links.is_empty() {
        out.push_str("\n<details><summary>Linked by the model</summary>\n\n");
        for l in &explained.links {
            out.push_str(&format!(
                "- `{}` → `{}` ({}): {}\n",
                l.a,
                l.b,
                name(&l.b),
                l.why
            ));
        }
        out.push_str("</details>\n");
    }
    if !explained.left.is_empty() {
        out.push_str(&format!(
            "\n{} flows not explained: the run reached its limit of requests.\n",
            explained.left.len()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use docair_core::diff::GraphDiff;

    fn graph(ids: &[&str]) -> Graph {
        let nodes: Vec<Value> = ids
            .iter()
            .map(|id| json!({ "id": id, "kind": "function", "name": id, "summary": "", "description": "", "docs": [], "tags": [], "origin": "authored" }))
            .collect();
        serde_json::from_value(json!({ "schema": "x", "system": "s", "generator": "t", "nodes": nodes, "edges": [], "flows": [], "diagnostics": [] })).unwrap()
    }

    /// docair-8l6.12: the model words each change step by docair's id; words on an id docair did
    /// not make, or naming a part that is not there, are left out; the digest carries the steps.
    #[test]
    fn step_words_bind_by_docairs_step_ids() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../docair-adapters/tests/golden/shop.graph.json"
        );
        let after: Graph = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut before = after.clone();
        before
            .edges
            .retain(|e| !(e.from == "orders.create" && e.to == "orders-db"));
        // The flow's change, from its two pictures: B also writes the order.
        let pic = |g: &Graph| docair_core::change_steps::flow_picture(g, "data.order").unwrap();
        let (pa, pb) = (pic(&before), pic(&after));
        let lines = |g: &Graph, p: &docair_core::layer_view::LayerPicture| -> HashSet<String> {
            p.lines
                .iter()
                .flat_map(|l| l.members.iter().map(|&m| g.edges[m].id.clone()))
                .collect()
        };
        let parts = |p: &docair_core::layer_view::LayerPicture| -> HashSet<String> {
            p.parts.iter().map(|x| x.id.clone()).collect()
        };
        let less = |x: HashSet<String>, y: HashSet<String>| -> Vec<String> {
            let mut v: Vec<String> = x.difference(&y).cloned().collect();
            v.sort();
            v
        };
        let flow = FlowChange {
            seed: "data.order".into(),
            how: "changed".into(),
            parts_added: less(parts(&pb), parts(&pa)),
            parts_removed: less(parts(&pa), parts(&pb)),
            lines_added: less(lines(&after, &pb), lines(&before, &pa)),
            lines_removed: less(lines(&before, &pa), lines(&after, &pb)),
            through: vec![],
            certain: true,
        };
        let changes = Changes {
            schema: "docair.changes/0".into(),
            a: "A".into(),
            b: "B".into(),
            working_tree: false,
            graph: GraphDiff::default(),
            parts: vec![],
            flows: vec![flow],
        };
        let flow = &changes.flows[0];
        let ids = step_ids(&before, &after, &changes, flow);
        let id = "data.order#path:orders-db";
        assert!(ids.contains(id), "{ids:?}");
        let sent = digest(&changes, &before, &after, &[flow], false);
        assert!(sent.contains(id), "the digest sends the step's id");
        let reply = json!({ "flows": [{
            "seed": "data.order",
            "explanation": "The order is now kept in `orders-db`.",
            "steps": {
                id: "Placing an order now also writes it to `orders-db`.",
                "data.order#path:invented": "Made up.",
                "data.order#line:a->b": "Names `ghost`.",
            }
        }]});
        let rec = Recorded {
            by: "model".into(),
            model: None,
            at: "t".into(),
        };
        let (_, flows, problems) = bind(&changes, &before, &after, &reply, &rec);
        assert_eq!(flows.len(), 1, "{problems:?}");
        assert_eq!(
            flows[0].steps.keys().collect::<Vec<_>>(),
            [id],
            "only docair's step: {problems:?}"
        );
        assert_eq!(problems.len(), 2, "{problems:?}");
        // A finding stored before the steps is asked about again; one with step words is not.
        let old = FlowExplained {
            seed: "data.order".into(),
            explanation: "x".into(),
            steps: BTreeMap::new(),
            recorded: rec.clone(),
        };
        let kept = |f: FlowExplained| ChangeFindings {
            schema: SCHEMA.into(),
            a: "A".into(),
            b: "B".into(),
            links: vec![],
            flows: vec![f],
            problems: vec![],
            left: vec![],
            requests: 0,
        };
        let off = AiConfig {
            endpoint: "http://127.0.0.1:9".into(),
            model: "m".into(),
            api_key: None,
            key_var: String::new(),
            key_source: None,
            max_tokens: 1,
        };
        let only_this = Changes {
            flows: vec![flow.clone()],
            ..changes.clone()
        };
        let asked = explain(
            &off,
            &before,
            &after,
            &only_this,
            Some(&kept(old.clone())),
            8,
        );
        assert!(
            asked.is_err(),
            "the old finding is asked about again (the model is unreachable)"
        );
        let worded = FlowExplained {
            steps: flows[0].steps.clone(),
            ..old
        };
        let found = explain(&off, &before, &after, &only_this, Some(&kept(worded)), 8).unwrap();
        assert_eq!(
            found.requests, 0,
            "a finding with step words is not asked again"
        );
    }

    #[test]
    fn only_what_binds_against_both_graphs_is_kept() {
        let a = graph(&["s.load", "s.run", "web"]);
        let b = graph(&["s.read_all", "s.run", "web", "s.cache"]);
        let changes = Changes {
            schema: "docair.changes/0".into(),
            a: "A".into(),
            b: "B".into(),
            working_tree: false,
            graph: GraphDiff {
                removed: vec!["s.load".into()],
                added: vec!["s.read_all".into(), "s.cache".into()],
                ..GraphDiff::default()
            },
            parts: vec!["s.run".into()],
            flows: vec![FlowChange {
                seed: "flow:buy".into(),
                how: "changed".into(),
                parts_added: vec!["s.cache".into()],
                parts_removed: vec![],
                lines_added: vec![],
                lines_removed: vec![],
                through: vec!["s.run".into()],
                certain: true,
            }],
        };
        let reply = json!({
            "links": [{ "a": "s.load", "b": "s.read_all", "why": "renamed and reads in bulk" }, { "a": "s.run", "b": "s.cache", "why": "no" }],
            "flows": [
                { "seed": "flow:buy", "explanation": "`s.run` now asks `s.cache` first." },
                { "seed": "flow:nope", "explanation": "x" },
            ]
        });
        let rec = Recorded {
            by: "model".into(),
            model: Some("m".into()),
            at: "t".into(),
        };
        let (links, flows, problems) = bind(&changes, &a, &b, &reply, &rec);
        assert_eq!(links.len(), 1, "{problems:?}");
        assert_eq!(
            (links[0].a.as_str(), links[0].b.as_str()),
            ("s.load", "s.read_all")
        );
        assert_eq!(flows.len(), 1);
        assert_eq!(
            problems.len(),
            2,
            "the bad link and the flow that did not change: {problems:?}"
        );
        let bad =
            json!({ "flows": [{ "seed": "flow:buy", "explanation": "`s.invented` does it." }] });
        let (_, flows, problems) = bind(&changes, &a, &b, &bad, &rec);
        assert!(
            flows.is_empty() && problems[0].contains("s.invented"),
            "{problems:?}"
        );
        let text = comment(
            &changes,
            &ChangeFindings {
                schema: SCHEMA.into(),
                a: "A".into(),
                b: "B".into(),
                links,
                flows: bind(&changes, &a, &b, &reply, &rec).1,
                problems: vec![],
                left: vec![],
                requests: 1,
            },
            &b,
        );
        assert!(
            text.contains("- **flow:buy** (changed)\n  `s.run` now asks `s.cache` first."),
            "{text}"
        );
        assert!(
            text.contains("<details><summary>Linked by the model</summary>"),
            "{text}"
        );
    }
}

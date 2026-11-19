//! The AI explains what two commits change (docs/EXPLAIN.md): it links what docair could not prove
//! is the same, says what each changed flow now does, and tells it from the user's side, before
//! and after. It sees only the changed flows, never the whole graph, and every claim is bound
//! against the two graphs: what names nothing is dropped and said.

use std::collections::{BTreeMap, HashSet};

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

const EXPLAIN_PROMPT: &str = r#"You explain changes between two commits of a software system to a reviewer. You get only the flows that changed: each with its parts at A (before) and at B (after), the parts and lines added and removed, and which parts changed. Answer as JSON:
{"links": [{"a": "<id at A>", "b": "<id at B>", "why": "<a few words>"}],
 "flows": [{"seed": "<the flow's seed, as given>", "explanation": "<2 to 4 plain sentences: what the change does to this flow, and why it matters; name parts by their ids in backticks>",
            "workflow": {"title": "<what the user does, in a few words>", "before": [{"title": "<a step>", "nodes": ["<id at A>"]}], "after": [{"title": "<a step>", "nodes": ["<id at B>"]}]}}]}
"links": a part removed at A that is the same as a part added at B (renamed and changed); only from the lists "removed at A" and "added at B", and only when sure.
"workflow": what a user sees, step by step, before and after; leave it out when the flow is not something a user does.
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

/// One step of a workflow: what the user sees, and the parts it is in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub title: String,
    pub nodes: Vec<String>,
}

/// What a user does along a flow, before and after the change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    pub title: String,
    pub before: Vec<WorkflowStep>,
    pub after: Vec<WorkflowStep>,
}

/// What the change does to one flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowExplained {
    pub seed: String,
    pub explanation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<Workflow>,
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

/***
 * The AI's input for some changed flows: each flow's seed, how it changed, the parts on it that changed and those added and removed (in a few words, from the graph they are in), its lines added and removed; and, for linking, the parts removed at A and added at B.
 *
 * @id docair-ai.explain.digest
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
 * Bind the model's answer against the two graphs: a link only from a part removed at A to one added at B; an explanation only of a flow that changed, and only naming parts of A or B; a workflow's steps only with parts of A (before) or of B (after). What does not bind is left out and said.
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
    let steps =
        |v: Option<&Value>, ids: &HashSet<&str>, problems: &mut Vec<String>| -> Vec<WorkflowStep> {
            v.and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|s| {
                    let nodes: Vec<String> = s
                        .get("nodes")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter(|n| {
                            let ok = ids.contains(n);
                            if !ok {
                                problems.push(format!(
                                    "a workflow step names `{n}`, which is not there; left out"
                                ));
                            }
                            ok
                        })
                        .map(str::to_string)
                        .collect();
                    (!nodes.is_empty()).then(|| WorkflowStep {
                        title: text(s, "title"),
                        nodes,
                    })
                })
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
        let unknown: Vec<&str> = named_ids(&explanation)
            .into_iter()
            .filter(|i| !ids_a.contains(i) && !ids_b.contains(i) && !line_ids.contains(i))
            .collect();
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
        let workflow = f.get("workflow").filter(|w| w.is_object()).and_then(|w| {
            let before = steps(w.get("before"), &ids_a, &mut problems);
            let after = steps(w.get("after"), &ids_b, &mut problems);
            (!before.is_empty() || !after.is_empty()).then(|| Workflow {
                title: text(w, "title"),
                before,
                after,
            })
        });
        flows.push(FlowExplained {
            seed,
            explanation,
            workflow,
            recorded: recorded.clone(),
        });
    }
    (links, flows, problems)
}

/***
 * Explain the changes: the flows the store does not explain yet (a human's finding is never asked about again) go to the model in requests of at most `MAX_CHARS`, at most `max_requests` of them; the answers are bound against both graphs and joined with what the store had.
 *
 * @id    docair-ai.explain.explain
 * @calls docair-ai.explain.digest "sends only the changed flows"
 * @calls docair-ai.client.chat (the changed flows) -> links, explanations, workflows
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
    let known: HashSet<&str> = kept
        .map(|k| k.flows.iter().map(|f| f.seed.as_str()).collect())
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
    for (k, batch) in batches.iter().enumerate() {
        if out.requests >= max_requests {
            out.left.extend(batch.iter().map(|f| f.seed.clone()));
            continue;
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
        let reply = chat(cfg, &messages, true)?;
        out.requests += 1;
        let Some(value) = reply_json(&reply.content) else {
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
        out.flows.extend(flows);
        out.problems.extend(problems);
    }
    Ok(out)
}

/***
 * The findings as a pull request comment: one line per changed flow with its explanation under it, links and workflows folded in `<details>`.
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
            if let Some(w) = &e.workflow {
                let steps = |s: &[WorkflowStep]| {
                    s.iter()
                        .map(|x| x.title.clone())
                        .collect::<Vec<_>>()
                        .join(" → ")
                };
                out.push_str(&format!(
                    "  <details><summary>{}</summary>\n\n  Before: {}\n\n  After: {}\n  </details>\n",
                    w.title,
                    steps(&w.before),
                    steps(&w.after)
                ));
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
                { "seed": "flow:buy", "explanation": "`s.run` now asks `s.cache` first.", "workflow": { "title": "buy", "before": [{ "title": "click", "nodes": ["web"] }], "after": [{ "title": "click", "nodes": ["web", "ghost"] }] } },
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
            flows[0].workflow.as_ref().unwrap().after[0].nodes,
            ["web"],
            "ghost is left out"
        );
        assert_eq!(problems.len(), 3, "{problems:?}");
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

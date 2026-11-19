//! A question becomes a reply: answer, walkthrough, view change, edits (SPEC §10.2, §10.7).

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use docair_core::edit;
use docair_core::model::{
    Change, EdgeRef, Graph, Origin, Recorded, WALKTHROUGH_SCHEMA, WalkStep, Walkthrough,
};
use docair_core::walkthrough::{Problem, bind, id_for};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::client::{Message, Usage, chat};
use crate::config::AiConfig;

/// The rules the model follows, and the reply format.
pub const SYSTEM_PROMPT: &str = r#"You explain how a software system works and help keep its documentation right. You get the system's architecture graph, extracted from its code and doc blocks by docair, the conversation so far, and the user's message. Your reply is JSON that docair turns into an answer, a walkthrough replayed on its interactive chart, a change of what the chart shows, and edits of the doc blocks in the code.

Reply with exactly one JSON object:
{
  "answer": "1 to 3 sentences that answer the message directly",
  "walkthrough": {"title": "...", "summary": "...", "steps": [{"title": "...", "text": "...", "nodes": ["id"], "edges": [{"from": "id", "to": "id"}]}]},
  "view": {"hide": ["id"], "show": ["id"]},
  "changes": [{"node": "id", "add": ["@tag value"], "remove": ["@tag value"], "summary": "optional new summary", "why": "one sentence"}]
}
"answer" is required; leave out the parts you do not need.

When to use which part:
- "How does ... work?", "What happens when ...?", "Walk me through ...": a walkthrough.
- "I don't care about ...", "hide ...", "show only ...", "show ... again": a view change. Hide or show whole sections by the id of their group node. For "show only X", hide every top-level group and show X: the chart then shows X inside its groups and hides everything beside it. Showing a part inside a hidden section brings the section back with only that part in it.
- "This is wrong", "... is missing", "... also calls ...", "document ...": changes to the doc blocks. Change only what the user says or what the graph clearly gets wrong; never rewrite what is right.

Walkthrough rules:
1. Use only node ids that appear in the graph's "nodes". Never invent an id.
2. Walk in the order things really happen, and say why that order: what depends on what, what must exist first, what is passed along.
3. Each step: "title" is one short sentence from the reader's side, e.g. "Terraform creates the kind cluster first". "text" is 2 to 5 sentences on what happens and why; name the concrete things: resources, files, ports, routes, payloads, commands. "nodes" are the 1 to 4 node ids the step is about, most important first. "edges" are the connections that carry the step, in the order they happen (a call, then the calls it makes in turn), only between nodes that have an edge in the graph.
4. 3 to 12 steps. Start where the action starts: a person, a command, a request.

Change rules (doc blocks, the format docair reads from the code):
- "node" is the id of the node whose block changes. For a part of the system that is not in the graph yet, use a new id and put "@kind <kind>" and "@part-of <id>" into "add"; give it a "summary".
- Tags, one per line in "add"/"remove", exactly as they would be written:
  @calls <id> [(arguments)] [-> result]      @reads <id> [-> data]       @writes <id> [(data)]
  @emits topic:<name> [(event)]              @consumes topic:<name>      @creates <id> [(how)]
  @covers <id> [(what)]                      @part-of <id>               @kind <kind>
  @http METHOD /path                         @flow <scenario> <step> "label"
  @in <name>: <Type> [from <id>]             @out <name>: <Type> [to <id>]
  In @in and @out, the words "from" and "to" introduce a node id; do not use them in the type text (write "@out graph: Graph", not "@out graph: Graph handed to the CLI").
  Kinds: system service module component function endpoint store topic external job workload config host cluster namespace network gateway container release test.
- Refer only to ids from the graph or ids you add in the same reply.
- For a node marked "undocumented", give a "summary" (one sentence: what it does) and the tags that say how data flows.
- "why" says in one sentence why the edit is right.

Code and data:
- Nodes of kind "data" are data: what users and the code make and carry (a question, uploaded files, a graph, a reply). All other nodes are code and the places it runs. Edges say which data they "carry".
- A node with "runs_in" runs there, even though its code comes from "code_from" (a web app is served by a container but runs in the browser). Its work, and the data it makes, happen where it runs.
- When you walk through a request or a use case, follow the data: start where it is made (the part that makes it, and where that part runs, e.g. the chat panel in the browser), then each line that carries it, and say how it changes on the way (what is added, what it turns into). Name the data with its data node name.

General:
- The conversation so far tells you what "it", "that" or "there" refers to.
- If the graph does not contain enough to answer, say so in "answer" and mark guesses with "probably".
- Write in the language of the user's message."#;

/// How much of a description goes into the digest.
const DESCRIPTION_CHARS: usize = 600;
/// How many earlier turns go along with a question.
pub const HISTORY_TURNS: usize = 8;
const FLOW_TAGS: &[&str] = &[
    "calls", "reads", "writes", "emits", "consumes", "creates", "covers", "in", "out",
];

/***
 * Shorten the graph to what the model needs: nodes with their data-flow tags, edges and scenarios, no source code.
 *
 * @id      docair-ai.ask.digest
 * @creates data.digest "shortens the graph for the model"
 * @flow    ask 3 "the graph becomes a digest, without source code"
 * @in      graph: Graph from docair-core.scan
 * @out     digest: JSON
 ***/
pub fn digest(graph: &Graph) -> String {
    let mut nodes = Vec::new();
    for n in &graph.nodes {
        let mut o = Map::new();
        o.insert("id".into(), json!(n.id));
        o.insert("kind".into(), json!(n.kind));
        if n.name != n.id {
            o.insert("name".into(), json!(n.name));
        }
        if let Some(p) = &n.part_of {
            o.insert("in".into(), json!(p));
        }
        if !n.summary.is_empty() {
            o.insert("summary".into(), json!(n.summary));
        }
        if !n.description.is_empty() {
            o.insert(
                "about".into(),
                json!(shorten(&n.description, DESCRIPTION_CHARS)),
            );
        }
        if let Some(h) = &n.http {
            o.insert("http".into(), json!(format!("{} {}", h.method, h.path)));
        }
        if let Some(s) = &n.source {
            o.insert("at".into(), json!(format!("{}:{}", s.file, s.line)));
        }
        if n.origin != Origin::Authored {
            o.insert("undocumented".into(), json!(true));
        }
        if let Some(r) = &n.runs_in {
            o.insert("runs_in".into(), json!(r));
        }
        if let Some(c) = &n.code_from {
            o.insert("code_from".into(), json!(c));
        }
        let tags: Vec<String> = n
            .tags
            .iter()
            .filter(|t| t.origin == Origin::Authored && FLOW_TAGS.contains(&t.name.as_str()))
            .map(|t| format!("@{} {}", t.name, t.value))
            .collect();
        if !tags.is_empty() {
            o.insert("tags".into(), json!(tags));
        }
        if let Some(t) = &n.test {
            o.insert("result".into(), json!(t.status));
        }
        nodes.push(Value::Object(o));
    }
    let edges: Vec<Value> = graph
        .edges
        .iter()
        .map(|e| {
            let mut o = Map::new();
            o.insert("from".into(), json!(e.from));
            o.insert("to".into(), json!(e.to));
            o.insert("kind".into(), json!(e.kind));
            let what = [&e.text, &e.label, &e.payload, &e.via]
                .into_iter()
                .flatten()
                .next()
                .cloned();
            if let Some(w) = what {
                o.insert("what".into(), json!(w));
            }
            if let Some(r) = &e.reply {
                o.insert("reply".into(), json!(r));
            }
            if !e.carries.is_empty() {
                o.insert("carries".into(), json!(e.carries));
            }
            if !e.carries_back.is_empty() {
                o.insert("carries_back".into(), json!(e.carries_back));
            }
            Value::Object(o)
        })
        .collect();
    let scenarios: Vec<Value> = graph
        .flows
        .iter()
        .map(|f| {
            json!({
                "name": f.name,
                "steps": f.steps.iter().map(|s| json!({ "step": s.step, "nodes": s.nodes, "label": s.label })).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::to_string(&json!({
        "system": graph.system,
        "nodes": nodes,
        "edges": edges,
        "scenarios": scenarios,
    }))
    .expect("the digest serializes")
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// One earlier turn of the conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub question: String,
    pub answer: String,
}

/// Sections to hide and show (SPEC §11).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewChange {
    #[serde(default)]
    pub hide: Vec<String>,
    #[serde(default)]
    pub show: Vec<String>,
}

impl ViewChange {
    pub fn is_empty(&self) -> bool {
        self.hide.is_empty() && self.show.is_empty()
    }
}

/// What the model sends.
#[derive(Debug, Default, Deserialize)]
struct DraftReply {
    #[serde(default)]
    answer: String,
    #[serde(default)]
    walkthrough: Option<Draft>,
    #[serde(default)]
    view: Option<ViewChange>,
    #[serde(default)]
    changes: Vec<DraftChange>,
    // An older reply shape: the walkthrough's fields at the top level.
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    steps: Option<Vec<DraftStep>>,
}

#[derive(Debug, Deserialize)]
struct Draft {
    #[serde(default)]
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    steps: Vec<DraftStep>,
}

#[derive(Debug, Deserialize)]
struct DraftStep {
    title: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    nodes: Vec<String>,
    #[serde(default)]
    edges: Vec<DraftEdge>,
}

/// Models write edges as objects or as pairs; both are fine.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DraftEdge {
    Object { from: String, to: String },
    Pair(String, String),
}

#[derive(Debug, Deserialize)]
struct DraftChange {
    node: String,
    #[serde(default)]
    add: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    why: String,
    #[serde(default)]
    file: Option<String>,
}

/// The JSON object in a reply, also when it is fenced or has text around it.
fn parse_reply(content: &str) -> Result<DraftReply, String> {
    let start = content
        .find('{')
        .ok_or("the reply contains no JSON object")?;
    let end = content.rfind('}').ok_or("the JSON object is not closed")?;
    if end < start {
        return Err("the reply contains no JSON object".into());
    }
    let mut reply: DraftReply =
        serde_json::from_str(&content[start..=end]).map_err(|e| e.to_string())?;
    if reply.walkthrough.is_none()
        && let Some(steps) = reply.steps.take()
    {
        reply.walkthrough = Some(Draft {
            title: reply.title.take().unwrap_or_default(),
            summary: reply.summary.clone().unwrap_or_default(),
            steps,
        });
    }
    if reply.answer.trim().is_empty() {
        reply.answer = reply
            .walkthrough
            .as_ref()
            .map(|w| {
                if w.summary.is_empty() {
                    w.title.clone()
                } else {
                    w.summary.clone()
                }
            })
            .unwrap_or_default();
    }
    if reply.answer.trim().is_empty() {
        return Err("the reply has no \"answer\"".into());
    }
    Ok(reply)
}

fn walkthrough_of(draft: Draft, question: &str, cfg: &AiConfig) -> Walkthrough {
    Walkthrough {
        schema: WALKTHROUGH_SCHEMA.to_string(),
        id: id_for(question),
        question: question.to_string(),
        title: if draft.title.trim().is_empty() {
            question.to_string()
        } else {
            draft.title.trim().to_string()
        },
        summary: draft.summary.trim().to_string(),
        recorded: Some(Recorded {
            at: now_rfc3339(),
            by: "ai".into(),
            model: Some(cfg.model.clone()),
            endpoint: Some(cfg.public_endpoint()),
        }),
        steps: draft
            .steps
            .into_iter()
            .map(|s| WalkStep {
                title: s.title.trim().to_string(),
                text: s.text.trim().to_string(),
                nodes: s.nodes.into_iter().map(|n| n.trim().to_string()).collect(),
                edges: s
                    .edges
                    .into_iter()
                    .map(|e| match e {
                        DraftEdge::Object { from, to } | DraftEdge::Pair(from, to) => {
                            EdgeRef { from, to }
                        }
                    })
                    .collect(),
                path: vec![],
            })
            .collect(),
        file: None,
    }
}

/// The reply, checked against the graph, and what it cost.
#[derive(Debug, Clone, Serialize)]
pub struct Reply {
    pub answer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub walkthrough: Option<Walkthrough>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewChange>,
    /// Edits, resolved to files and lines.
    pub changes: Vec<Change>,
    /// Words for lines, to show before the edits are applied (SPEC §10.8).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub wording: Vec<crate::word::Worded>,
    /// New glossary entries, to show before they are applied (SPEC §13.4).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub glossary: Vec<crate::glossary::TermDraft>,
    /// What the model got wrong and was left out.
    pub problems: Vec<String>,
    pub usage: Usage,
    pub seconds: f64,
    /// 1, or 2 when a repair was needed.
    pub requests: u8,
}

/// Bind and resolve a draft; the problems that are worth a repair request come second.
fn check_draft(
    draft: DraftReply,
    graph: &Graph,
    question: &str,
    cfg: &AiConfig,
) -> (Reply, Vec<String>) {
    let ids: HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    let mut problems = Vec::new();
    let mut repair = Vec::new();

    let walkthrough = draft
        .walkthrough
        .filter(|d| !d.steps.is_empty())
        .and_then(|d| {
            let mut wt = walkthrough_of(d, question, cfg);
            for p in bind(&mut wt, &graph.nodes, &graph.edges) {
                if let Problem::UnknownNode { id, .. } = &p {
                    repair.push(format!(
                        "node id `{id}` in the walkthrough is not in the graph"
                    ));
                }
                problems.push(p.to_string());
            }
            (!wt.steps.is_empty()).then_some(wt)
        });

    let view = draft.view.map(|v| {
        let keep = |list: Vec<String>, what: &str, repair: &mut Vec<String>| {
            list.into_iter()
                .filter(|id| {
                    let ok = ids.contains(id.as_str());
                    if !ok {
                        repair.push(format!("`{id}` in \"view\".\"{what}\" is not in the graph"));
                    }
                    ok
                })
                .collect::<Vec<_>>()
        };
        ViewChange {
            hide: keep(v.hide, "hide", &mut repair),
            show: keep(v.show, "show", &mut repair),
        }
    });

    // Nodes the reply adds itself may be named by its other edits.
    let new_ids: HashSet<String> = draft
        .changes
        .iter()
        .filter(|d| !ids.contains(d.node.as_str()))
        .map(|d| d.node.clone())
        .collect();
    let mut changes = Vec::new();
    for d in draft.changes {
        let c = Change {
            node: d.node,
            mode: String::new(),
            file: d.file.unwrap_or_default(),
            line: 0,
            add: d.add,
            remove: d.remove,
            summary: d.summary.filter(|s| !s.trim().is_empty()),
            why: d.why,
        };
        match edit::resolve(&c, &graph.nodes) {
            Ok(c) => {
                let unknown = edit::unknown_refs(&c, &|r| ids.contains(r) || new_ids.contains(r));
                if unknown.is_empty() {
                    changes.push(c);
                } else {
                    repair.push(format!("change of `{}`: {}", c.node, unknown.join("; ")));
                    problems.push(format!(
                        "edit of `{}` left out: {}",
                        c.node,
                        unknown.join("; ")
                    ));
                }
            }
            Err(e) => {
                repair.push(format!("change of `{}`: {e}", c.node));
                problems.push(format!("edit left out: {e}"));
            }
        }
    }

    let reply = Reply {
        answer: draft.answer.trim().to_string(),
        walkthrough,
        view: view.filter(|v| !v.is_empty()),
        changes,
        wording: vec![],
        glossary: vec![],
        problems,
        usage: Usage::default(),
        seconds: 0.0,
        requests: 0,
    };
    (reply, repair)
}

/***
 * Ask the model about the graph, with the earlier turns, and return its reply checked against the graph.
 *
 * When the reply is not valid JSON, or names ids that are not in the graph,
 * or proposes edits that cannot be made, the model gets one repair request
 * that lists the problems.
 *
 * @id      docair-ai.ask.ask
 * @creates data.walkthrough "checks the model's reply against the graph"
 * @calls   docair-ai.ask.digest "condenses the graph for the prompt"
 * @calls   docair-ai.client.chat (system prompt, turns, question + digest) -> reply JSON
 * @calls   docair-core.walkthrough.bind "checks the reply's walkthrough against the graph"
 * @calls   docair-core.edit.resolve "locates where each edit goes"
 * @out     reply: Reply
 ***/
pub fn ask(
    cfg: &AiConfig,
    graph: &Graph,
    question: &str,
    history: &[Turn],
    context: Option<&str>,
) -> Result<Reply> {
    let question = question.trim();
    if question.is_empty() {
        bail!("the question is empty");
    }
    let mut messages = vec![
        Message::system(SYSTEM_PROMPT),
        Message::user(format!(
            "The graph of the system `{}`:\n{}",
            graph.system,
            digest(graph)
        )),
        Message::assistant("{\"answer\": \"I have read the graph. What would you like to know?\"}"),
    ];
    let start = history.len().saturating_sub(HISTORY_TURNS);
    for turn in &history[start..] {
        messages.push(Message::user(turn.question.clone()));
        messages.push(Message::assistant(
            json!({ "answer": turn.answer }).to_string(),
        ));
    }
    let mut message = question.to_string();
    if let Some(ctx) = context.filter(|c| !c.trim().is_empty()) {
        message.push_str(&format!("\n\n(What the question points at:\n{ctx})"));
    }
    messages.push(Message::user(message));

    let mut usage = Usage::default();
    let mut seconds = 0.0;
    let mut requests = 0u8;
    loop {
        let reply = chat(cfg, &messages, true)?;
        requests += 1;
        usage += reply.usage;
        seconds += reply.seconds;
        let last = requests >= 2;
        let fix = match parse_reply(&reply.content) {
            Err(e) if last => bail!("the model's answer cannot be read: {e}"),
            Err(e) => {
                format!("Your reply could not be read: {e}. Reply again with only the JSON object.")
            }
            Ok(draft) => {
                let (mut out, repair) = check_draft(draft, graph, question, cfg);
                if repair.is_empty() || last {
                    out.usage = usage;
                    out.seconds = seconds;
                    out.requests = requests;
                    return Ok(out);
                }
                format!(
                    "Some parts of your reply do not fit the graph:\n- {}\nUse only ids from the graph's \"nodes\" (or new ids with @kind in \"add\") and reply again with the whole JSON object.",
                    repair.join("\n- ")
                )
            }
        };
        messages.push(Message::assistant(reply.content));
        messages.push(Message::user(fix));
    }
}

/***
 * Send one short request, to see that endpoint, model and key work.
 *
 * @id    docair-ai.ask.check
 * @calls docair-ai.client.chat (a one-line question) -> ok
 ***/
pub fn check(cfg: &AiConfig) -> Result<(String, f64)> {
    let mut short = cfg.clone();
    short.max_tokens = short.max_tokens.min(1000);
    let reply = chat(
        &short,
        &[Message::user("Reply with the single word: ok")],
        false,
    )?;
    Ok((reply.content.trim().to_string(), reply.seconds))
}

/// Now as RFC 3339 in UTC, to the second.
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    rfc3339(secs)
}

fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Env;
    use docair_core::config::AiSettings;
    use docair_core::model::*;

    fn node(id: &str, origin: Origin) -> Node {
        Node {
            id: id.into(),
            kind: "service".into(),
            name: id.into(),
            summary: "Does it.".into(),
            description: "x".repeat(2000),
            part_of: None,
            source: Some(SourceRef {
                file: "src/a.rs".into(),
                line: 3,
                end: None,
            }),
            symbol: Some("fn a".into()),
            http: None,
            deployed_as: None,
            docs: vec![],
            tags: vec![
                Tag::authored("calls", "b", 4),
                Tag::authored("kind", "service", 5),
            ],
            origin,
            test: None,
            signature: None,
            runs_in: None,
            code_from: None,
        }
    }

    fn graph() -> Graph {
        Graph {
            schema: SCHEMA.into(),
            system: "s".into(),
            generator: "t".into(),
            nodes: vec![node("a", Origin::Authored), node("b", Origin::Inferred)],
            edges: vec![],
            flows: vec![],
            walkthroughs: vec![],
            view: View::default(),
            glossary: vec![],
            library: vec![],
            meanings: None,
            diagnostics: vec![],
        }
    }

    fn cfg() -> AiConfig {
        AiConfig::resolve(
            Some(&AiSettings {
                endpoint: Some("http://x.test/v1".into()),
                model: Some("m".into()),
                ..Default::default()
            }),
            &Env::default(),
        )
        .unwrap()
    }

    #[test]
    fn dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_794_696_570), "2026-11-14T22:49:30Z");
    }

    #[test]
    fn replies_are_found_in_fences_and_old_shapes_still_read() {
        let r = parse_reply("Here:\n```json\n{\"answer\":\"A\",\"walkthrough\":{\"title\":\"T\",\"steps\":[{\"title\":\"s\",\"nodes\":[\"a\"],\"edges\":[{\"from\":\"a\",\"to\":\"b\"},[\"b\",\"c\"]]}]}}\n```").unwrap();
        assert_eq!(r.walkthrough.unwrap().steps[0].edges.len(), 2);
        let old = parse_reply("{\"title\":\"T\",\"summary\":\"S\",\"steps\":[{\"title\":\"s\"}]}")
            .unwrap();
        assert_eq!(old.answer, "S");
        assert_eq!(old.walkthrough.unwrap().title, "T");
        assert!(parse_reply("no json").is_err());
        assert!(
            parse_reply("{\"view\":{\"hide\":[\"a\"]}}").is_err(),
            "an answer is required"
        );
    }

    #[test]
    fn drafts_are_checked_against_the_graph() {
        let draft = parse_reply(
            r#"{"answer":"Hid it.","view":{"hide":["a","ghost"]},
                "changes":[{"node":"a","add":["@reads b"],"why":"w"},{"node":"nope","add":["@calls a"],"why":"w"},{"node":"b","summary":"B does b.","add":["@calls a"],"why":"w"}]}"#,
        )
        .unwrap();
        let (reply, repair) = check_draft(draft, &graph(), "q", &cfg());
        assert_eq!(reply.view.as_ref().unwrap().hide, vec!["a"]);
        assert_eq!(reply.changes.len(), 2);
        assert_eq!(reply.changes[0].mode, "edit");
        assert_eq!(reply.changes[1].mode, "insert");
        assert_eq!(repair.len(), 2, "{repair:?}");
    }

    #[test]
    fn the_digest_marks_undocumented_nodes_and_has_no_code() {
        let d = digest(&graph());
        let v: Value = serde_json::from_str(&d).unwrap();
        assert_eq!(v["nodes"][0]["at"], "src/a.rs:3");
        assert_eq!(v["nodes"][0]["tags"][0], "@calls b");
        assert!(v["nodes"][0].get("undocumented").is_none());
        assert_eq!(v["nodes"][1]["undocumented"], true);
        assert!(v["nodes"][0]["about"].as_str().unwrap().chars().count() <= DESCRIPTION_CHARS + 1);
        assert!(!d.contains("fn a"), "no symbols or code");
    }
}

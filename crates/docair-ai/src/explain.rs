//! The AI makes a change understandable (docs/EXPLAIN.md §9): it groups the pieces docair found
//! into changes, one difference in what the app does each, and says for each what the app does
//! differently, how its pieces work together, why (from the commit messages and the changed docs)
//! and what to check. It links what docair could not prove is the same. Every claim is bound
//! against the two graphs, the pieces and the sources: what names nothing is dropped and said, and
//! every piece no change holds is an other edit.

use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use docair_core::changes::Changes;
use docair_core::model::Graph;
use docair_core::review::{Intent, Piece};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::client::{Message, chat_with};
use crate::config::AiConfig;

pub const SCHEMA: &str = "docair.explained/1";
/// A request carries at most this much (docs/EXPLAIN.md §7, docair-8l6.8).
pub const MAX_CHARS: usize = 60_000;
/// A run asks the model at most this often, unless `[explain] max_requests` says otherwise.
pub const MAX_REQUESTS: usize = 8;
/// At most this many requests of one explain go to the model at once (docair-8l6.14).
pub const AT_ONCE: usize = 3;

/// How far an explain is: what it asks, and how many of its requests have answered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ExplainProgress {
    /// The pieces it asks about: those no stored change or other edit holds, `left` included.
    pub pieces: usize,
    /// The requests it sends: at most `max_requests`.
    pub requests: usize,
    pub started: usize,
    pub answered: usize,
    /// The pieces left for a later run, beyond `max_requests`.
    pub left: usize,
}

const REVIEW_PROMPT: &str = r#"You help a reviewer understand what changed between two versions of a software system. You get: the pieces of the change that docair found, each with an id (new, gone and edited parts and lines, the parts where it forks off and rejoins, the flows it touches); the commit messages between the versions; the changed Markdown docs; and the doc blocks whose words changed. Group the pieces into changes. A change is one difference in what the app does: behaviour added, removed or adjusted, with every piece that works together for it. Make each change easy to understand for someone who does not know the code. Answer as JSON:
{"changes": [{"title": "<what the app does differently: one plain sentence, from the user's side>",
              "how": [{"piece": "<a piece id, as given>", "words": "<one plain sentence: what this piece does for the change>"}],
              "why": "<why it was done, from the commit messages and the docs: one or two plain sentences>",
              "sources": ["<the id of each commit or doc you took the why from, as given>"],
              "check": "<what a reviewer should check: the risky part, one sentence>",
              "disagrees": ["<where a commit message or a doc says one thing and the pieces show another, one sentence each>"]}],
 "links": [{"a": "<id at A>", "b": "<id at B>", "why": "<a few words>"}]}
"how": the change's pieces in the order they work together; each piece in one change at most. Put every piece into the change it is part of, also the small ones (a new type, a constant, an edited helper); leave a piece out only when it belongs to no change.
"links": a part removed at A that is the same as a part added at B (renamed and changed); only from "removed at A" and "added at B", and only when sure.
"sources": the ids only there, never in the title, the why or the words.
For a piece whose code changed you see only that it changed, not how: do not say what the edit does unless a commit message, a doc or a summary you were given says it; then say where you have it from.
Name parts by their ids in backticks. Name only ids you were given. Make as many changes as there are."#;

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

/// One piece of a change, in the order the change's pieces work together, with what it does for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HowStep {
    /// The piece's id, as docair made it (`docair_core::review::pieces`).
    pub piece: String,
    pub words: String,
}

/// One change of the review (docs/EXPLAIN.md §9.1): one difference in what the app does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewChange {
    /// What the app does differently, from the user's side.
    pub title: String,
    pub how: Vec<HowStep>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// The commits and docs the why comes from: `commit:<sha>`, `doc:<path>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub check: String,
    /// Where a commit message or a doc and the pieces disagree.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disagrees: Vec<String>,
    pub recorded: Recorded,
}

impl ReviewChange {
    pub fn pieces(&self) -> impl Iterator<Item = &str> {
        self.how.iter().map(|h| h.piece.as_str())
    }
}

/// The review of two commits: kept in the store, the next run asks only about what is new.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeFindings {
    pub schema: String,
    pub a: String,
    pub b: String,
    #[serde(default)]
    pub links: Vec<ChangeLink>,
    #[serde(default)]
    pub changes: Vec<ReviewChange>,
    /// The pieces no change holds: the review's other edits (docair-8l6.28).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other: Vec<String>,
    /// What the model said that did not bind, and so was left out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
    /// The pieces not asked about: the run reached its limit of requests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left: Vec<String>,
    #[serde(default)]
    pub requests: usize,
}

/// A part in one line: `id (kind): its summary's first line`, cut short; just its id when the graph has none.
fn part_line(g: &Graph, id: &str) -> String {
    match g.nodes.iter().find(|n| n.id == id) {
        Some(n) => {
            let summary = n.summary.lines().next().unwrap_or("");
            if summary.is_empty() {
                format!("{} ({})", n.id, n.kind)
            } else {
                format!(
                    "{} ({}): {}",
                    n.id,
                    n.kind,
                    docair_core::review::cut(summary, 120)
                )
            }
        }
        None => id.to_string(),
    }
}

/// A piece for the model, compact (docair-8l6.34): its id and kind, its new and gone parts in a line each (at most 12), its edited parts by id (at most 16), its ends (8) and box by id, its lines with + or - (12), and up to four flows it touches.
fn piece_json(a: &Graph, b: &Graph, p: &Piece) -> Value {
    let s = &p.step;
    let lines = |g: &Graph, ids: &[String]| ids.iter().map(|x| part_line(g, x)).collect::<Vec<_>>();
    let mut out = json!({ "id": s.id, "kind": format!("{:?}", s.kind).to_lowercase() });
    let mut put = |k: &str, v: Vec<String>| {
        if !v.is_empty() {
            out[k] = json!(v);
        }
    };
    // At most this many of a list, then how many more: a piece's new and gone parts say most.
    let capped = |v: Vec<String>, n: usize| -> Vec<String> {
        let more = v.len().saturating_sub(n);
        let mut v: Vec<String> = v.into_iter().take(n).collect();
        if more > 0 {
            v.push(format!("and {more} more"));
        }
        v
    };
    put("new", capped(lines(b, &s.added), 12));
    put("gone", capped(lines(a, &s.removed), 12));
    put("code changed", capped(s.code.clone(), 16));
    put("between", capped(s.ends.clone(), 8));
    put(
        "lines",
        capped(
            s.lines_added
                .iter()
                .map(|l| format!("+ {l}"))
                .chain(s.lines_removed.iter().map(|l| format!("- {l}")))
                .collect(),
            12,
        ),
    );
    let mut flows: Vec<String> = p.flows.iter().take(4).cloned().collect();
    if p.flows.len() > 4 {
        flows.push(format!("and {} more", p.flows.len() - 4));
    }
    put("flows", flows);
    if let Some(x) = &s.in_box {
        out["in"] = json!(x);
    }
    out
}

/***
 * The AI's input for some pieces of the change (docs/EXPLAIN.md §9.2): each piece in a few words, the intent (the commit messages, the changed docs, the doc blocks whose words changed), and, for linking, the parts removed at A and added at B.
 *
 * @id docair-ai.explain.digest
 ***/
pub fn digest(
    changes: &Changes,
    a: &Graph,
    b: &Graph,
    pieces: &[&Piece],
    intent: &Intent,
    with_unlinked: bool,
) -> String {
    let mut out = json!({
        "system": b.system,
        "from": changes.a, "to": changes.b,
        "pieces": pieces.iter().map(|p| piece_json(a, b, p)).collect::<Vec<_>>(),
        "commits": intent.commits,
        "docs": intent.docs,
        "words changed": intent.words,
    });
    if intent.uncommitted {
        out["not committed"] =
            json!("the version after has changes that are not committed: they have no message");
    }
    if with_unlinked {
        out["removed at A"] = json!(
            changes
                .graph
                .removed
                .iter()
                .map(|p| part_line(a, p))
                .collect::<Vec<_>>()
        );
        out["added at B"] = json!(
            changes
                .graph
                .added
                .iter()
                .map(|p| part_line(b, p))
                .collect::<Vec<_>>()
        );
    }
    out.to_string()
}

/// The ids words name in backticks that look like ids (`docair-core.scan`, `data.order`).
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
 * Bind the model's review against the graphs, the pieces and the sources (docs/EXPLAIN.md §9.4): a change keeps only `how` entries that name a piece docair made and no earlier change holds (`taken`, docair-8l6.32); words keep only what names parts, lines, pieces or sources there are; a change with no title, a title that names what is not there, or no piece left is left out; a source only if it is a commit or doc docair sent; a link only from a part removed at A to one added at B. What does not bind is said.
 *
 * @id docair-ai.explain.bind
 ***/
#[allow(clippy::too_many_arguments)]
pub fn bind(
    changes: &Changes,
    (a, b): (&Graph, &Graph),
    pieces: &[Piece],
    intent: &Intent,
    reply: &Value,
    recorded: &Recorded,
    taken: &mut HashSet<String>,
) -> (Vec<ChangeLink>, Vec<ReviewChange>, Vec<String>) {
    let ids: HashSet<&str> = a
        .nodes
        .iter()
        .chain(&b.nodes)
        .map(|n| n.id.as_str())
        .chain(a.edges.iter().chain(&b.edges).map(|e| e.id.as_str()))
        .chain(pieces.iter().map(|p| p.id()))
        .collect();
    let sources = intent.sources();
    let piece_ids: HashSet<&str> = pieces.iter().map(|p| p.id()).collect();
    let removed: HashSet<&str> = changes.graph.removed.iter().map(String::as_str).collect();
    let added: HashSet<&str> = changes.graph.added.iter().map(String::as_str).collect();
    let text = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    // Words that name only what there is; else what they name that is not.
    let unknown = |words: &str| -> Vec<String> {
        named_ids(words)
            .into_iter()
            .filter(|i| !ids.contains(i) && !sources.contains(i))
            .map(|u| format!("`{u}`"))
            .collect()
    };
    let mut problems = Vec::new();
    let mut links = Vec::new();
    for l in reply
        .get("links")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        // The parts to link go as `id (kind): summary`; a model may copy the whole line: its id is the first word.
        let id_of = |k: &str| {
            text(l, k)
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string()
        };
        let (la, lb) = (id_of("a"), id_of("b"));
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
    // A piece by its id; or by a part it holds as new, gone or edited, also named like a piece
    // (`path:<part>`): the first such piece no change holds yet, else the first.
    let resolve = |named: &str, taken: &HashSet<String>| -> Option<String> {
        if piece_ids.contains(named) {
            return Some(named.to_string());
        }
        let part = ["path:", "line:", "code:", "box:"]
            .iter()
            .find_map(|k| named.strip_prefix(k))
            .unwrap_or(named);
        let holding: Vec<&Piece> = pieces
            .iter()
            .filter(|p| {
                let s = &p.step;
                s.added
                    .iter()
                    .chain(&s.removed)
                    .chain(&s.code)
                    .any(|x| x == part)
            })
            .collect();
        holding
            .iter()
            .find(|p| !taken.contains(p.id()))
            .or(holding.first())
            .map(|p| p.id().to_string())
    };
    let mut out = Vec::new();
    for c in reply
        .get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let title = text(c, "title");
        let bad = unknown(&title);
        if title.is_empty() || !bad.is_empty() {
            problems.push(format!(
                "a change {}; left out",
                if title.is_empty() {
                    "has no title".to_string()
                } else {
                    format!("titled \"{title}\" names {}", bad.join(", "))
                }
            ));
            continue;
        }
        let mut how = Vec::new();
        for h in c.get("how").and_then(Value::as_array).into_iter().flatten() {
            let (named, words) = (text(h, "piece"), text(h, "words"));
            // A part's id, or a piece's id guessed from it, binds to the piece that holds that part (docair-8l6.37).
            let piece = resolve(&named, taken).unwrap_or(named);
            if how.iter().any(|x: &HowStep| x.piece == piece) {
                continue;
            }
            if !piece_ids.contains(piece.as_str()) {
                problems.push(format!(
                    "\"{title}\": `{piece}` is no piece docair found; left out"
                ));
            } else if taken.contains(&piece) {
                problems.push(format!(
                    "\"{title}\": the piece `{piece}` is in an earlier change already; left there"
                ));
            } else if !unknown(&words).is_empty() || words.is_empty() {
                problems.push(format!(
                    "\"{title}\": the words on `{piece}` name {}; left out",
                    unknown(&words).join(", ")
                ));
            } else {
                taken.insert(piece.clone());
                how.push(HowStep { piece, words });
            }
        }
        if how.is_empty() {
            problems.push(format!("\"{title}\" holds no piece docair found; left out"));
            continue;
        }
        // The rest of the change: each text alone is left out when it names what is not there.
        let mut keep = |k: &str| {
            let t = text(c, k);
            let bad = unknown(&t);
            if bad.is_empty() {
                t
            } else {
                problems.push(format!(
                    "\"{title}\": its {k} names {}; left out",
                    bad.join(", ")
                ));
                String::new()
            }
        };
        let (why, check) = (keep("why"), keep("check"));
        let mut cited = Vec::new();
        for s in c
            .get("sources")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if sources.contains(s.trim()) {
                cited.push(s.trim().to_string());
            } else {
                problems.push(format!(
                    "\"{title}\": `{s}` is no commit or doc it was given; left out"
                ));
            }
        }
        let mut disagrees = Vec::new();
        for d in c
            .get("disagrees")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let d = d.trim();
            if d.is_empty() {
                continue;
            }
            let bad = unknown(d);
            if bad.is_empty() {
                disagrees.push(d.to_string());
            } else {
                problems.push(format!(
                    "\"{title}\": a disagreement names {}; left out",
                    bad.join(", ")
                ));
            }
        }
        out.push(ReviewChange {
            title,
            how,
            why,
            sources: cited,
            check,
            disagrees,
            recorded: recorded.clone(),
        });
    }
    (links, out, problems)
}

/***
 * Review the changes (docs/EXPLAIN.md §9): the pieces no stored change or other edit holds go to the model with the intent, in requests of at most `MAX_CHARS` (the pieces in their order, the intent with each; docair-8l6.31), at most `max_requests` of them and at most `AT_ONCE` at a time; the answers are bound in the requests' order and joined with what the store had. A change a human wrote is kept and its pieces are not asked about; a store of another schema is asked again. A request that fails leaves its pieces; when none answers, the error is returned.
 *
 * @id    docair-ai.explain.explain
 * @calls docair-core.review.pieces "the pieces of the whole change"
 * @calls docair-ai.explain.digest "sends the pieces and the intent"
 * @calls docair-ai.client.chat_with (the pieces, the commit messages, the changed docs; not reasoning first) -> changes, links
 * @calls docair-ai.explain.bind "keeps only what binds"
 ***/
pub fn explain(
    cfg: &AiConfig,
    (a, b): (&Graph, &Graph),
    changes: &Changes,
    intent: &Intent,
    kept: Option<&ChangeFindings>,
    max_requests: usize,
) -> Result<ChangeFindings> {
    explain_with(cfg, (a, b), changes, intent, kept, max_requests, &|_| {})
}

/// As `explain`, saying how far it is each time a request starts or answers (`docair serve`'s explain jobs).
pub fn explain_with(
    cfg: &AiConfig,
    (a, b): (&Graph, &Graph),
    changes: &Changes,
    intent: &Intent,
    kept: Option<&ChangeFindings>,
    max_requests: usize,
    on: &(dyn Fn(&ExplainProgress) + Sync),
) -> Result<ChangeFindings> {
    let pieces = docair_core::review::pieces(a, b, changes);
    let kept = kept.filter(|k| k.schema == SCHEMA);
    // What the store already holds: a human's changes always; a model's when they and the other
    // edits hold exactly the pieces there are now (else the model's are asked again, whole).
    let all: HashSet<&str> = pieces.iter().map(|p| p.id()).collect();
    let covered: HashSet<&str> = kept
        .map(|k| {
            k.changes
                .iter()
                .flat_map(|c| c.pieces())
                .chain(k.other.iter().map(String::as_str))
                .collect()
        })
        .unwrap_or_default();
    // Stale when the pieces are not the ones it was made for, either way: one is new, or one is gone.
    let stale = all != covered;
    let mut out = ChangeFindings {
        schema: SCHEMA.to_string(),
        a: changes.a.clone(),
        b: changes.b.clone(),
        links: kept.map(|k| k.links.clone()).unwrap_or_default(),
        changes: kept
            .map(|k| {
                k.changes
                    .iter()
                    .filter(|c| !stale || c.recorded.by == "human")
                    .map(|c| ReviewChange {
                        how: c
                            .how
                            .iter()
                            .filter(|h| all.contains(h.piece.as_str()))
                            .cloned()
                            .collect(),
                        ..c.clone()
                    })
                    .filter(|c| !c.how.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        other: Vec::new(),
        problems: Vec::new(),
        left: Vec::new(),
        requests: 0,
    };
    let mut taken: HashSet<String> = out
        .changes
        .iter()
        .flat_map(|c| c.pieces().map(str::to_string))
        .collect();
    // Only pieces that are there now count as taken.
    taken.retain(|p| all.contains(p.as_str()));
    let todo: Vec<&Piece> = if stale {
        pieces.iter().filter(|p| !taken.contains(p.id())).collect()
    } else {
        Vec::new()
    };
    // Pieces in requests of at most MAX_CHARS, in their order; the parts to link go with the first.
    let mut batches: Vec<Vec<&Piece>> = Vec::new();
    for p in todo {
        let fits = batches.last().is_some_and(|last| {
            let mut next = last.clone();
            next.push(p);
            digest(changes, a, b, &next, intent, batches.len() == 1).len() <= MAX_CHARS
        });
        match batches.last_mut() {
            Some(last) if fits => last.push(p),
            _ => batches.push(vec![p]),
        }
    }
    let recorded = Recorded {
        by: "model".into(),
        model: Some(cfg.model.clone()),
        at: crate::ask::now_rfc3339(),
    };
    let sent = batches.len().min(max_requests);
    let progress = Mutex::new(ExplainProgress {
        pieces: batches.iter().map(Vec::len).sum(),
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
                    {
                        let mut p = progress.lock().unwrap_or_else(|e| e.into_inner());
                        p.started += 1;
                        on(&p);
                    }
                    let messages = [
                        Message::system(REVIEW_PROMPT),
                        Message::user(format!(
                            "The change from `{}` to `{}`:\n{}",
                            changes.a,
                            changes.b,
                            digest(changes, a, b, &batches[k], intent, k == 0)
                        )),
                    ];
                    // Not reasoning first: grouping pieces, a reasoning model deliberates until max_tokens runs out (docair-8l6.36).
                    let reply = chat_with(cfg, &messages, true, false).map(|r| r.content);
                    {
                        let mut p = progress.lock().unwrap_or_else(|e| e.into_inner());
                        p.answered += 1;
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
    // The answers in the batches' order, so the review does not depend on which came first.
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
                    "a request to the model failed ({e:#}); its pieces are left"
                ));
                out.left.extend(batch.iter().map(|p| p.id().to_string()));
                failed.get_or_insert(e);
                continue;
            }
        };
        read += 1;
        let Some(value) = reply_json(&content) else {
            out.problems
                .push("the model's answer cannot be read; its pieces are left".into());
            out.left.extend(batch.iter().map(|p| p.id().to_string()));
            continue;
        };
        // A change may name only the pieces of its own request.
        let own: Vec<Piece> = batch.iter().map(|p| (*p).clone()).collect();
        let (links, found, problems) =
            bind(changes, (a, b), &own, intent, &value, &recorded, &mut taken);
        for l in links {
            if !out.links.iter().any(|x| x.a == l.a && x.b == l.b) {
                out.links.push(l);
            }
        }
        out.changes.extend(found);
        out.problems.extend(problems);
    }
    for batch in &batches[sent..] {
        out.left.extend(batch.iter().map(|p| p.id().to_string()));
    }
    out.requests = sent;
    if read == 0
        && let Some(e) = failed
    {
        return Err(e);
    }
    // Every piece belongs somewhere: no change holds it, nor was it left for later: an other edit.
    if stale || kept.is_none() {
        let left: HashSet<&str> = out.left.iter().map(String::as_str).collect();
        out.other = pieces
            .iter()
            .map(|p| p.id())
            .filter(|p| !taken.contains(*p) && !left.contains(p))
            .map(str::to_string)
            .collect();
    } else if let Some(k) = kept {
        out.other = k.other.clone();
    }
    Ok(out)
}

/// A piece in a few words, for a reader: its new, gone and edited parts by name.
pub fn piece_words(p: &Piece, a: &Graph, b: &Graph) -> String {
    let name = |g: &Graph, id: &str| {
        g.nodes
            .iter()
            .find(|n| n.id == id)
            .map_or(id.to_string(), |n| n.name.clone())
    };
    let list = |g: &Graph, ids: &[String]| {
        ids.iter()
            .map(|i| format!("`{}`", name(g, i)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let s = &p.step;
    let mut said = Vec::new();
    if !s.added.is_empty() {
        said.push(format!("new {}", list(b, &s.added)));
    }
    if !s.removed.is_empty() {
        said.push(format!("gone {}", list(a, &s.removed)));
    }
    if !s.code.is_empty() {
        said.push(format!("code of {}", list(b, &s.code)));
    }
    if said.is_empty() {
        said.push(
            s.lines
                .iter()
                .map(|l| format!("`{}` → `{}`", name(b, &l.from), name(b, &l.to)))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    said.join("; ")
}

/***
 * The review as a pull request comment (docs/EXPLAIN.md §9.6): each change with what the app does differently, why (its sources) and what to check, and how its pieces work together folded in `<details>`; the other edits and the links folded.
 *
 * @id docair-ai.explain.comment
 ***/
pub fn comment(changes: &Changes, review: &ChangeFindings, (a, b): (&Graph, &Graph)) -> String {
    let pieces = docair_core::review::pieces(a, b, changes);
    let piece = |id: &str| pieces.iter().find(|p| p.id() == id);
    let mut out = format!(
        "### docair: the review of `{}` → `{}`\n\n",
        changes.a, changes.b
    );
    // What docair puts with a change by module, and what stays an other edit (docair-8l6.41).
    let held: Vec<Vec<String>> = review
        .changes
        .iter()
        .map(|c| {
            c.pieces()
                .filter(|p| piece(p).is_some())
                .map(str::to_string)
                .collect()
        })
        .collect();
    let others: Vec<String> = review
        .other
        .iter()
        .filter(|id| piece(id).is_some())
        .cloned()
        .collect();
    let (also, rest) = docair_core::review::attach_by_module(&pieces, &held, &others);
    if review.changes.is_empty() {
        out.push_str(if pieces.is_empty() {
            "Nothing changed.\n"
        } else {
            "No change explained yet: `docair explain` asks the AI.\n"
        });
    }
    for (k, c) in review.changes.iter().enumerate() {
        out.push_str(&format!("**{}. {}**\n\n", k + 1, c.title));
        if !c.why.is_empty() {
            let from = if c.sources.is_empty() {
                String::new()
            } else {
                format!(" ({})", c.sources.join(", "))
            };
            out.push_str(&format!("Why: {}{from}\n\n", c.why));
        }
        if !c.check.is_empty() {
            out.push_str(&format!("Check: {}\n\n", c.check));
        }
        for d in &c.disagrees {
            out.push_str(&format!("⚠ {d}\n\n"));
        }
        out.push_str("<details><summary>How</summary>\n\n");
        for h in &c.how {
            if let Some(p) = piece(&h.piece) {
                out.push_str(&format!("- {} ({})\n", h.words, piece_words(p, a, b)));
            }
        }
        // What docair put with it: in the same module as its pieces.
        for id in &also[k] {
            if let Some(p) = piece(id) {
                out.push_str(&format!("- also, same module: {}\n", piece_words(p, a, b)));
            }
        }
        out.push_str("</details>\n\n");
    }
    let other: Vec<&Piece> = rest.iter().filter_map(|id| piece(id)).collect();
    if !other.is_empty() {
        out.push_str(&format!(
            "<details><summary>Other edits ({})</summary>\n\n",
            other.len()
        ));
        for p in other {
            out.push_str(&format!("- {}\n", piece_words(p, a, b)));
        }
        out.push_str("</details>\n\n");
    }
    if !review.links.is_empty() {
        out.push_str("<details><summary>Linked by the model</summary>\n\n");
        for l in &review.links {
            out.push_str(&format!("- `{}` → `{}`: {}\n", l.a, l.b, l.why));
        }
        out.push_str("</details>\n\n");
    }
    if !review.left.is_empty() {
        out.push_str(&format!(
            "{} pieces not asked about: the run reached its limit of requests.\n",
            review.left.len()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use docair_core::diff::{ChangedItem, GraphDiff};
    use docair_core::review::{CommitNote, DocChange};

    fn shop() -> Graph {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../docair-adapters/tests/golden/shop.graph.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// The shop before it wrote the order, with the cart's code edited too.
    fn two_changes() -> (Graph, Graph, Changes, Intent) {
        let after = shop();
        let mut before = after.clone();
        let at = before
            .edges
            .iter()
            .position(|e| e.from == "orders.create" && e.to == "orders-db")
            .unwrap();
        let line = before.edges.remove(at).id;
        let graph = GraphDiff {
            lines_added: vec![line],
            changed: vec![
                ChangedItem {
                    id: "orders.create".into(),
                    what: vec!["tags".into()],
                },
                ChangedItem {
                    id: "shop-ui.cart.total".into(),
                    what: vec!["body".into()],
                },
            ],
            ..GraphDiff::default()
        };
        let changes = docair_core::changes::changes(&before, &after, graph, &[], ("A", "B"));
        let intent = Intent {
            commits: vec![CommitNote {
                id: "commit:abc1234".into(),
                subject: "Keep every order".into(),
                body: String::new(),
            }],
            uncommitted: false,
            docs: vec![DocChange {
                id: "doc:README.md".into(),
                path: "README.md".into(),
                lines: "+The API stores the order.".into(),
            }],
            words: vec![],
        };
        (before, after, changes, intent)
    }

    fn rec() -> Recorded {
        Recorded {
            by: "model".into(),
            model: Some("m".into()),
            at: "t".into(),
        }
    }

    /// docair-8l6.20, 8l6.21: the digest carries the pieces and the intent; binding keeps a change
    /// with a known piece, a known source and words that name what there is, and leaves out the rest.
    #[test]
    fn a_review_binds_to_pieces_and_sources() {
        let (before, after, changes, intent) = two_changes();
        let pieces = docair_core::review::pieces(&before, &after, &changes);
        let write = pieces
            .iter()
            .find(|p| p.step.lines_added.iter().any(|l| l.contains("orders-db")))
            .unwrap();
        let code = pieces
            .iter()
            .find(|p| p.step.code.contains(&"shop-ui.cart.total".to_string()))
            .unwrap();
        let refs: Vec<&Piece> = pieces.iter().collect();
        let sent = digest(&changes, &before, &after, &refs, &intent, true);
        assert!(
            sent.contains(write.id())
                && sent.contains("commit:abc1234")
                && sent.contains("doc:README.md")
        );
        let reply = json!({ "changes": [
            { "title": "A placed order is now kept in the orders table.",
              "how": [ { "piece": write.id(), "words": "`orders.create` now writes the order to `orders-db`." },
                       { "piece": "path:invented", "words": "Made up." } ],
              "why": "The commit says every order must be kept.", "sources": ["commit:abc1234", "commit:nothere"],
              "check": "Writing must not slow down `orders.create`.", "disagrees": ["`ghost.part` disagrees."] },
            { "title": "This one takes the same piece again.", "how": [ { "piece": write.id(), "words": "Again." } ] },
            { "title": "Names `nowhere.part`.", "how": [ { "piece": code.id(), "words": "x" } ] },
            // A part's id where a piece's belongs: bound to the piece that holds that part.
            { "title": "The cart's total is worked out anew.", "how": [ { "piece": "shop-ui.cart.total", "words": "The total changed." } ] },
        ]});
        let mut taken = HashSet::new();
        let (_, found, problems) = bind(
            &changes,
            (&before, &after),
            &pieces,
            &intent,
            &reply,
            &rec(),
            &mut taken,
        );
        assert_eq!(found.len(), 2, "{problems:#?}");
        assert_eq!(
            found[1].how[0].piece,
            code.id(),
            "the part's id binds to its piece"
        );
        let c = &found[0];
        assert_eq!(c.how.len(), 1);
        assert_eq!(c.sources, ["commit:abc1234"]);
        assert!(c.disagrees.is_empty() && !c.check.is_empty() && !c.why.is_empty());
        assert!(taken.contains(write.id()) && taken.contains(code.id()));
        assert!(problems.iter().any(|p| p.contains("path:invented")));
        assert!(problems.iter().any(|p| p.contains("commit:nothere")));
        assert!(problems.iter().any(|p| p.contains("earlier change")));
        assert!(problems.iter().any(|p| p.contains("nowhere.part")));
        // The comment says it, and the edit no change holds is an other edit.
        let found: Vec<ReviewChange> = found.into_iter().take(1).collect();
        let review = ChangeFindings {
            schema: SCHEMA.into(),
            a: "A".into(),
            b: "B".into(),
            links: vec![],
            changes: found,
            other: vec![code.id().to_string()],
            problems: vec![],
            left: vec![],
            requests: 1,
        };
        let text = comment(&changes, &review, (&before, &after));
        assert!(
            text.contains("**1. A placed order is now kept in the orders table.**"),
            "{text}"
        );
        assert!(
            text.contains("Why: The commit says every order must be kept. (commit:abc1234)"),
            "{text}"
        );
        assert!(
            text.contains("Other edits (1)") && text.contains("code of `total`"),
            "{text}"
        );
    }

    /// docair-8l6.21: a store that covers every piece is not asked again; one of another schema, or
    /// one that misses a piece, is; a human's change is kept and its pieces are not asked about.
    #[test]
    fn the_store_is_asked_again_only_when_it_does_not_cover() {
        let (before, after, changes, intent) = two_changes();
        let pieces = docair_core::review::pieces(&before, &after, &changes);
        let off = AiConfig {
            endpoint: "http://127.0.0.1:9".into(),
            model: "m".into(),
            api_key: None,
            key_var: String::new(),
            key_source: None,
            max_tokens: 1,
        };
        let change = |by: &str, ids: &[&str]| ReviewChange {
            title: "t".into(),
            how: ids
                .iter()
                .map(|p| HowStep {
                    piece: p.to_string(),
                    words: "w".into(),
                })
                .collect(),
            why: String::new(),
            sources: vec![],
            check: String::new(),
            disagrees: vec![],
            recorded: Recorded {
                by: by.into(),
                ..rec()
            },
        };
        let store =
            |schema: &str, changes_: Vec<ReviewChange>, other: Vec<String>| ChangeFindings {
                schema: schema.into(),
                a: "A".into(),
                b: "B".into(),
                links: vec![],
                changes: changes_,
                other,
                problems: vec![],
                left: vec![],
                requests: 0,
            };
        let (first, rest) = (
            pieces[0].id(),
            pieces[1..]
                .iter()
                .map(|p| p.id().to_string())
                .collect::<Vec<_>>(),
        );
        let full = store(SCHEMA, vec![change("model", &[first])], rest.clone());
        let found = explain(&off, (&before, &after), &changes, &intent, Some(&full), 8).unwrap();
        assert_eq!(found.requests, 0, "every piece is covered: not asked");
        assert_eq!(found.changes.len(), 1);
        // Made for a piece that is gone now: asked again (the model is unreachable here).
        let mut more = rest.clone();
        more.push("path:gone.piece".into());
        let extra = store(SCHEMA, vec![change("model", &[first])], more);
        assert!(
            explain(&off, (&before, &after), &changes, &intent, Some(&extra), 8).is_err(),
            "a piece that is gone: asked"
        );
        let old = store("docair.explained/0", vec![], vec![]);
        assert!(
            explain(&off, (&before, &after), &changes, &intent, Some(&old), 8).is_err(),
            "another schema: asked (unreachable)"
        );
        let missing = store(SCHEMA, vec![change("human", &[first])], vec![]);
        assert!(
            explain(
                &off,
                (&before, &after),
                &changes,
                &intent,
                Some(&missing),
                8
            )
            .is_err(),
            "a piece missing: asked"
        );
    }
}

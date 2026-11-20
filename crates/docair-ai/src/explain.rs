//! The AI makes a change understandable (docs/EXPLAIN.md §9): it groups the pieces docair found
//! into changes, one difference in what the app does each, and says for each what the app does
//! differently, how its pieces work together, why (from the commit messages and the changed docs)
//! and what to check. It links what docair could not prove is the same. Every claim is bound
//! against the two graphs, the pieces and the sources: what names nothing is dropped and said. Every
//! piece is a change: one the AI puts in none is kept in `other`, and shown as a change of its own.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use docair_core::changes::Changes;
use docair_core::model::Graph;
use docair_core::review::{Intent, PartText, Piece};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::client::{Message, chat_with};
use crate::config::AiConfig;

pub const SCHEMA: &str = "docair.explained/2";
/// A request carries at most this much (docs/EXPLAIN.md §7, docair-8l6.8).
pub const MAX_CHARS: usize = 60_000;
/// A run asks the model at most this often, unless `[explain] max_requests` says otherwise.
pub const MAX_REQUESTS: usize = 8;
/// A request of a review asked in parts names the others' pieces in at most this many characters (docair-8l6.60).
pub const MAX_ELSEWHERE: usize = MAX_CHARS / 4;
/// At most this many requests of one explain go to the model at once (docair-8l6.14).
pub const AT_ONCE: usize = 3;

/// How far an explain is: what it asks, how many of its requests have answered, and the changes made so far.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ExplainProgress {
    /// The changes this run has made so far, bound as the answers come (docair-8l6.65); the last
    /// request may still merge them and place pieces into them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub found: Vec<ReviewChange>,
    /// The pieces it asks about: those no stored change holds nor `other`, `left` included.
    pub pieces: usize,
    /// The requests it sends: at most `max_requests`.
    pub requests: usize,
    pub started: usize,
    pub answered: usize,
    /// The pieces left for a later run, beyond `max_requests`.
    pub left: usize,
}

const REVIEW_PROMPT: &str = r#"You review what changed between two versions of a software system, for someone who does not know the code. You get: the pieces of the change docair found, each with an id (new, gone and edited parts and lines, each part's whole code before and after with its comments, the parts where it forks off and rejoins, the flows it touches); the commit messages between the versions; the changed Markdown docs; the doc blocks whose words changed; and the changed files that hold no part.
Read the code first. The commit messages and docs are claims: check each against the code, and believe the code. Group the pieces into changes. A change is one difference in what the app does: behaviour added, removed or adjusted, with every piece that works together for it. Answer as JSON:
{"changes": [{"title": "<what the app does differently: one plain sentence, from the user's side>",
              "how": [{"piece": "<a piece id, as given>", "words": "<one plain sentence: what this piece does for the change, as its code shows>"}],
              "why": "<why it was done and what it is good for: your own reading of the code, the commits and the docs together, one or two plain sentences>",
              "off": [{"kind": "<contradicts, unsaid or risk>", "source": "<commit, doc or comment; \"\" unless it contradicts>", "where": "<the commit's subject, the doc's path, or the id of the part whose comment it is; \"\" unless it contradicts>", "says": "<what it says, briefly and close to its words; \"\" unless it contradicts>", "code": "<what the code seems to do: one plain sentence, naming parts by their ids in backticks>", "parts": ["<the id of each part where the code does it: a part's id, not a piece's>"]}],
              "check": "<what a reviewer should still check by hand: the risky part, one sentence>"}],
 "links": [{"a": "<id at A>", "b": "<id at B>", "why": "<a few words>"}]}
"how": the change's pieces in the order they work together; each piece in one change at most. Every piece is part of a change: put each into the change it is part of, also the small ones (a new type, a constant, an edited helper); a piece that is part of no other change is a change of its own.
"off": what you found to be off, each of one of three kinds, so a reader can tell them apart. "contradicts": a commit message, a doc or a code comment (a doc block or a comment in a part's source) says something the code clearly does otherwise, so both cannot be true; or a doc or comment the change leaves out of date, still saying what the code no longer does. Give its source, where, what it says, and in "code" what the code does instead. Only a clear contradiction you can see in the code: something a commit, doc or comment does not mention, or mentions only in part, is not one, and what you cannot see enough of to be sure is not one either. "unsaid": the code does something a reader would want to know (it behaves differently, has a new effect, drops a case) and no commit message, doc or comment mentions it; "code" says what it does. "risk": a mistake, a missed case or a risk you see in the code itself; "code" says what the code does and why that is a problem. For "unsaid" and "risk", "source", "where" and "says" are "". Only what you found: not what you could not see or check, and not that work which is not committed has no message. [] when you checked and found nothing.
Write everything in your own words, as your own reading. Only an "off" names and repeats what was said; everywhere else never quote a commit message or a doc, never say what a commit or doc says, and never name a commit, a doc, an issue or a ticket (no hashes, no file names of docs, no issue numbers). Not: "The commit and the README say auditors want a trail of each order." But: "Auditors need a trail of each order; checkout now records one."
A piece's "source" gives each of its parts' whole code: "before" at the version before, "after" at the version after. Where it is cut (…) or not shown, say only what you can see. "files" are changed files that hold no part (a stylesheet, a config), as their changed lines.
"links": a part removed at A that is the same as a part added at B (renamed and changed); only from "removed at A" and "added at B", and only when sure.
Name parts by their ids in backticks. Name only ids you were given. Make as many changes as there are."#;

const SETTLE_PROMPT: &str = r#"You finish a review of what changed between two versions of a software system. You get the changes it found, each by number (its title, why and what its pieces do), and the pieces no change holds yet ("left"), each with its parts and their code before and after. Answer as JSON:
{"same": [[<number>, <number>]],
 "place": [{"piece": "<a left piece's id, as given>", "change": <number>, "words": "<one plain sentence: what this piece does for that change, as its code shows>"}],
 "new": [{"title": "<what the app does differently: one plain sentence, from the user's side>", "how": [{"piece": "<a left piece's id, as given>", "words": "<one plain sentence: what this piece does for the change, as its code shows>"}], "why": "<why it was done and what it is good for, one or two plain sentences>", "check": "<what a reviewer should still check by hand, one sentence>"}]}
"same": the review was asked in parts, so the same change may have come back more than once, each time from other pieces of it (its prompt, its stored data, its page, its output). Each group: the numbers of changes that serve one and the same purpose, one feature or one fix; a number in one group at most; changes that are only related stay apart.
"place": each left piece that is part of one of the changes, read from its code. "new": the left pieces that are part of none of them, as changes of their own, the pieces that work together in one change. Every left piece is in "place" or in "new", once.
Write in your own words; never name a commit, a doc, an issue or a ticket. Name parts by their ids in backticks; name only ids you were given."#;

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

/// One thing the model found off (docs/EXPLAIN.md §9.1, docair-8l6.58, 8l6.62), of one of three kinds: a
/// contradiction, what a commit message, a doc or a code comment says set against what the code clearly does
/// otherwise; unsaid, what the code does that nothing mentions; a risk, a mistake or missed case in the code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundOff {
    /// `contradicts`, `unsaid` or `risk`; empty in a review stored before the kinds, see [`FoundOff::kind`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// `commit`, `doc` or `comment` for a contradiction; `none` for the other kinds.
    pub source: String,
    /// Which one: the commit's subject, the doc's path, the id of the part whose comment it is; empty when not known.
    #[serde(rename = "where", default, skip_serializing_if = "String::is_empty")]
    pub place: String,
    /// What it says, briefly; empty for `none`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub says: String,
    /// What the code seems to do.
    pub code: String,
    /// The parts where the code does it, by id (docair-8l6.61): parts of A or B.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<String>,
}

/// A field of the model's answer as text, trimmed; empty when it is not there.
fn field(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

impl FoundOff {
    /// Its kind; a review stored before the kinds has a contradiction where something was said, else a risk.
    pub fn kind(&self) -> &str {
        match self.kind.as_str() {
            "" if self.source != "none" && !self.says.is_empty() => "contradicts",
            "" => "risk",
            k => k,
        }
    }

    /// An item of the model's "off" as docair keeps it: its kind one of three, its source one of four, only a
    /// contradiction saying what was said, `where` without backticks. A bare sentence is a risk: what the
    /// code does; an item without it is none. A contradiction that says nothing was said is no contradiction:
    /// it is kept as a risk, and `true` says so.
    fn read(v: &Value) -> Option<(FoundOff, bool)> {
        if let Some(t) = v.as_str() {
            let t = t.trim();
            return (!t.is_empty()).then(|| {
                (
                    FoundOff {
                        kind: "risk".into(),
                        source: "none".into(),
                        place: String::new(),
                        says: String::new(),
                        code: t.to_string(),
                        parts: Vec::new(),
                    },
                    false,
                )
            });
        }
        let code = field(v, "code");
        if code.is_empty() {
            return None;
        }
        let says = field(v, "says");
        let source = match field(v, "source").to_lowercase().as_str() {
            _ if says.is_empty() => "none",
            "commit" | "commits" | "commit message" => "commit",
            "doc" | "docs" | "markdown" | "readme" => "doc",
            "comment" | "code comment" | "doc block" => "comment",
            _ => "none",
        };
        let said = source != "none";
        let kind = match field(v, "kind").to_lowercase().as_str() {
            "contradicts" | "contradiction" | "contradicted" | "contradict" | "out of date"
            | "outdated" | "stale" => "contradicts",
            "unsaid" | "said nowhere" | "unmentioned" | "not mentioned" | "nowhere"
            | "undocumented" => "unsaid",
            "risk" | "risky" | "mistake" | "bug" | "missed case" | "problem" => "risk",
            _ if said => "contradicts",
            _ => "risk",
        };
        let demoted = kind == "contradicts" && !said;
        let kind = if demoted { "risk" } else { kind };
        let said = kind == "contradicts";
        Some((
            FoundOff {
                kind: kind.into(),
                source: if said { source } else { "none" }.into(),
                place: if said {
                    field(v, "where").trim_matches('`').trim().to_string()
                } else {
                    String::new()
                },
                says: if said { says } else { String::new() },
                code,
                parts: v
                    .get("parts")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|p| p.trim().trim_matches('`').trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect(),
            },
            demoted,
        ))
    }
}

/// One change of the review (docs/EXPLAIN.md §9.1): one difference in what the app does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewChange {
    /// What the app does differently, from the user's side.
    pub title: String,
    pub how: Vec<HowStep>,
    /// Why it was done and what it is good for, the model's own reading (docair-8l6.55).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// What the model found to be off, checking the commits, docs and comments against the code: empty when it found nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub off: Vec<FoundOff>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub check: String,
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
    /// The pieces no change holds (docair-8l6.28): each is shown as a change of its own, in docair's words
    /// (docair-8l6.63).
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

/// A piece for the model (docair-8l6.34, 8l6.47): its id and kind, its new and gone parts in a line each (at most 12), its edited parts by id (at most 16), its ends (8) and box by id, its lines with + or - (12), up to four flows it touches, and its parts' whole texts before and after.
fn piece_json(a: &Graph, b: &Graph, p: &Piece, texts: &HashMap<&str, &PartText>) -> Value {
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
    // Its parts' whole texts before and after, comments with them, so the model can read what
    // changed (docair-8l6.47); at most MAX_PIECE_TEXT a piece, then how many more.
    let mut room = docair_core::review::MAX_PIECE_TEXT;
    let mut source = serde_json::Map::new();
    let mut more = 0;
    for id in s.added.iter().chain(&s.removed).chain(&s.code) {
        let Some(t) = texts.get(id.as_str()) else {
            continue;
        };
        let size =
            t.before.as_ref().map_or(0, String::len) + t.after.as_ref().map_or(0, String::len);
        if size > room {
            more += 1;
            continue;
        }
        room -= size;
        let mut v = serde_json::Map::new();
        if let Some(x) = &t.before {
            v.insert("before".into(), json!(x));
        }
        if let Some(x) = &t.after {
            v.insert("after".into(), json!(x));
        }
        source.insert(id.clone(), Value::Object(v));
    }
    if !source.is_empty() {
        out["source"] = Value::Object(source);
    }
    if more > 0 {
        out["source not shown"] = json!(format!("{more} more parts, too long for this request"));
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
    let texts: HashMap<&str, &PartText> =
        intent.code.iter().map(|t| (t.part.as_str(), t)).collect();
    let mut out = json!({
        "system": b.system,
        "from": changes.a, "to": changes.b,
        "pieces": pieces.iter().map(|p| piece_json(a, b, p, &texts)).collect::<Vec<_>>(),
        // Without their ids: nothing to cite (docair-8l6.55).
        "commits": intent.commits.iter().map(|c| json!({ "subject": c.subject, "body": c.body })).collect::<Vec<_>>(),
        "docs": intent.docs.iter().map(|d| json!({ "path": d.path, "lines": d.lines })).collect::<Vec<_>>(),
        "words changed": intent.words,
        "files": intent.files.iter().map(|d| json!({ "path": d.path, "lines": d.lines })).collect::<Vec<_>>(),
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

/// Every id words may name: the parts and lines of both graphs, and the pieces.
fn known_ids<'g>((a, b): (&'g Graph, &'g Graph), pieces: &'g [Piece]) -> HashSet<&'g str> {
    a.nodes
        .iter()
        .chain(&b.nodes)
        .map(|n| n.id.as_str())
        .chain(a.edges.iter().chain(&b.edges).map(|e| e.id.as_str()))
        .chain(pieces.iter().map(|p| p.id()))
        .collect()
}

/// What words name in backticks that is not there, each in backticks.
fn unknown_in(words: &str, ids: &HashSet<&str>) -> Vec<String> {
    named_ids(words)
        .into_iter()
        .filter(|i| !ids.contains(i))
        .map(|u| format!("`{u}`"))
        .collect()
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

/// Words with each piece id in backticks named as its part (docair-8l6.64): `path:orders.audit` becomes
/// `orders.audit` when the piece stands for one part, or is named after a part there is.
fn as_parts(words: &str, pieces: &[Piece], part_ids: &HashSet<&str>) -> String {
    if !words.contains('`') || words.matches('`').count() % 2 == 1 {
        return words.to_string();
    }
    let part = |x: &str| -> Option<String> {
        if let Some(p) = pieces.iter().find(|p| p.id() == x) {
            let mut ids: Vec<&String> = p
                .step
                .added
                .iter()
                .chain(&p.step.removed)
                .chain(&p.step.code)
                .collect();
            ids.sort();
            ids.dedup();
            if let [one] = ids.as_slice() {
                return Some((*one).clone());
            }
        }
        ["path:", "line:", "code:", "box:"]
            .iter()
            .find_map(|k| x.strip_prefix(k))
            .filter(|r| part_ids.contains(r))
            .map(str::to_string)
    };
    words
        .split('`')
        .enumerate()
        .map(|(i, x)| match i % 2 {
            1 => part(x).unwrap_or_else(|| x.to_string()),
            _ => x.to_string(),
        })
        .collect::<Vec<_>>()
        .join("`")
}

/***
 * Bind the model's review against the graphs and the pieces (docs/EXPLAIN.md §9.4): a change keeps only `how` entries that name a piece docair made and no earlier change holds (`taken`, docair-8l6.32); words keep only what names parts, lines or pieces there are; a change with no title, a title that names what is not there, or no piece left is left out; a link only from a part removed at A to one added at B. What does not bind is said.
 *
 * @id docair-ai.explain.bind
 ***/
#[allow(clippy::too_many_arguments)]
pub fn bind(
    changes: &Changes,
    (a, b): (&Graph, &Graph),
    pieces: &[Piece],
    reply: &Value,
    recorded: &Recorded,
    taken: &mut HashSet<String>,
) -> (Vec<ChangeLink>, Vec<ReviewChange>, Vec<String>) {
    let ids = known_ids((a, b), pieces);
    let piece_ids: HashSet<&str> = pieces.iter().map(|p| p.id()).collect();
    let part_ids: HashSet<&str> = a
        .nodes
        .iter()
        .chain(&b.nodes)
        .map(|n| n.id.as_str())
        .collect();
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
    let unknown = |words: &str| unknown_in(words, &ids);
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
        let mut off = Vec::new();
        for v in c.get("off").and_then(Value::as_array).into_iter().flatten() {
            let Some((mut d, demoted)) = FoundOff::read(v) else {
                problems.push(format!(
                    "\"{title}\": something it found off says nothing of the code; left out"
                ));
                continue;
            };
            if demoted {
                problems.push(format!(
                    "\"{title}\": a contradiction it found names nothing that was said; kept as looks wrong or risky"
                ));
            }
            let bad: Vec<String> = unknown(&d.says)
                .into_iter()
                .chain(unknown(&d.code))
                .collect();
            if !bad.is_empty() {
                problems.push(format!(
                    "\"{title}\": what it found off names {}; left out",
                    bad.join(", ")
                ));
                continue;
            }
            // A comment is a part's: one that is not there is not said where.
            if d.source == "comment"
                && !d.place.is_empty()
                && !unknown(&format!("`{}`", d.place)).is_empty()
            {
                problems.push(format!(
                    "\"{title}\": a comment it found off is on `{}`, which is not there; where left out",
                    d.place
                ));
                d.place.clear();
            }
            // The parts where the code does it: those there are, else the parts its sentence names (docair-8l6.61).
            // A piece named for its parts stands for them.
            let named: Vec<String> = std::mem::take(&mut d.parts)
                .into_iter()
                .flat_map(|p| match pieces.iter().find(|x| x.id() == p) {
                    Some(x) => x
                        .step
                        .added
                        .iter()
                        .chain(&x.step.removed)
                        .chain(&x.step.code)
                        .cloned()
                        .collect(),
                    None => vec![p],
                })
                .collect();
            let (there, not): (Vec<String>, Vec<String>) = named
                .into_iter()
                .partition(|p| part_ids.contains(p.as_str()));
            if !not.is_empty() {
                problems.push(format!(
                    "\"{title}\": something it found off is in {}, which is not there; left out",
                    not.iter()
                        .map(|p| format!("`{p}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            for p in there.into_iter().chain(
                named_ids(&d.code)
                    .into_iter()
                    .filter(|i| part_ids.contains(i))
                    .map(str::to_string),
            ) {
                if !d.parts.contains(&p) {
                    d.parts.push(p);
                }
            }
            off.push(d);
        }
        // What the reader sees names parts, not pieces (docair-8l6.64).
        let named = |t: &str| as_parts(t, pieces, &part_ids);
        for h in &mut how {
            h.words = named(&h.words);
        }
        for d in &mut off {
            (d.says, d.code) = (named(&d.says), named(&d.code));
        }
        out.push(ReviewChange {
            title: named(&title),
            how,
            why: named(&why),
            off,
            check: named(&check),
            recorded: recorded.clone(),
        });
    }
    (links, out, problems)
}

/// A run's state while its answers come: what is bound so far, and the answers waiting for those before them.
struct RunState {
    out: ChangeFindings,
    taken: HashSet<String>,
    from: HashSet<usize>,
    failed: Option<anyhow::Error>,
    read: usize,
    /// The next request to bind.
    next: usize,
    waiting: std::collections::BTreeMap<usize, Result<String>>,
}

/// The pieces of all requests but `k`.
fn others_of<'p>(batches: &[Vec<&'p Piece>], k: usize) -> Vec<&'p Piece> {
    batches
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != k)
        .flat_map(|(_, b)| b.iter().copied())
        .collect()
}

/// What a request is told of the others (docair-8l6.60): the pieces they hold, to leave to them; at
/// most `MAX_ELSEWHERE` characters of their ids, then how many more.
fn elsewhere(others: &[&Piece]) -> String {
    if others.is_empty() {
        return String::new();
    }
    let mut ids: Vec<&str> = Vec::new();
    let mut size = 0;
    for p in others {
        size += p.id().len() + 3;
        if size > MAX_ELSEWHERE {
            break;
        }
        ids.push(p.id());
    }
    let more = others.len() - ids.len();
    format!(
        "\n\nThe review is asked in parts. These pieces are asked about in the other parts, not here: never put them, or the parts they hold, into \"how\"; a change that needs them is made there, and the last request merges it with yours.\n{}{}",
        serde_json::to_string(&ids).unwrap_or_default(),
        if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        }
    )
}

/// Leave to the other requests the "how" entries that name their pieces (docair-8l6.60), and a change
/// that holds nothing else: a name this request's pieces do not hold, but another's do. Returns how
/// many entries and how many changes.
fn leave_elsewhere(value: &mut Value, own: &[&Piece], others: &[&Piece]) -> (usize, usize) {
    let holds = |p: &Piece, named: &str| {
        let part = ["path:", "line:", "code:", "box:"]
            .iter()
            .find_map(|k| named.strip_prefix(k))
            .unwrap_or(named);
        p.id() == named
            || p.step
                .added
                .iter()
                .chain(&p.step.removed)
                .chain(&p.step.code)
                .any(|x| x == part)
    };
    let theirs = |named: &str| {
        !own.iter().any(|p| holds(p, named)) && others.iter().any(|p| holds(p, named))
    };
    let (mut entries, mut gone) = (0, 0);
    if let Some(list) = value.get_mut("changes").and_then(Value::as_array_mut) {
        list.retain_mut(|c| {
            let Some(how) = c.get_mut("how").and_then(Value::as_array_mut) else {
                return true;
            };
            let was = how.len();
            how.retain(|h| !theirs(h.get("piece").and_then(Value::as_str).unwrap_or("").trim()));
            entries += was - how.len();
            let keep = !(was > 0 && how.is_empty());
            gone += usize::from(!keep);
            keep
        });
    }
    (entries, gone)
}

/// Bind request `k`'s answer into the run: a failed or unreadable one leaves its pieces; else its changes, links and problems are kept.
#[allow(clippy::too_many_arguments)]
fn bind_answer(
    r: &mut RunState,
    k: usize,
    reply: Result<String>,
    (batch, others): (&[&Piece], &[&Piece]),
    (changes, a, b): (&Changes, &Graph, &Graph),
    intent: &Intent,
    recorded: &Recorded,
) {
    let content = match reply {
        Ok(c) => c,
        Err(e) => {
            r.out.problems.push(format!(
                "a request to the model failed ({e:#}); its pieces are left"
            ));
            r.out.left.extend(batch.iter().map(|p| p.id().to_string()));
            r.failed.get_or_insert(e);
            return;
        }
    };
    r.read += 1;
    let Some(mut value) = reply_json(&content) else {
        r.out
            .problems
            .push("the model's answer cannot be read; its pieces are left".into());
        r.out.left.extend(batch.iter().map(|p| p.id().to_string()));
        return;
    };
    // What belongs to another request is left to it, said once, not as a problem each (docair-8l6.60).
    let (entries, gone) = leave_elsewhere(&mut value, batch, others);
    if entries > 0 {
        r.out.problems.push(format!(
            "request {}: {} of its pieces were another request's, left to it{}",
            k + 1,
            entries,
            if gone > 0 {
                format!(", and {gone} of its changes held only those")
            } else {
                String::new()
            }
        ));
    }
    // A change may name only the pieces of its own request.
    let own: Vec<Piece> = batch.iter().map(|p| (*p).clone()).collect();
    let (links, mut found, problems) = bind(changes, (a, b), &own, &value, recorded, &mut r.taken);
    r.out.problems.extend(check_where(&mut found, intent));
    for l in links {
        if !r.out.links.iter().any(|x| x.a == l.a && x.b == l.b) {
            r.out.links.push(l);
        }
    }
    if !found.is_empty() {
        r.from.insert(k);
    }
    r.out.changes.extend(found);
    r.out.problems.extend(problems);
}

/***
 * Review the changes (docs/EXPLAIN.md §9): the pieces no stored change holds, nor `other`, go to the model with the intent, in requests of at most `MAX_CHARS` (the pieces in their order, the intent with each; docair-8l6.31), at most `max_requests` of them and at most `AT_ONCE` at a time; the answers are bound in the requests' order, each as soon as those before it are, the changes so far going out with the progress (docair-8l6.65); a last request says which changes came back twice and where the pieces no change holds belong, with its words on them (docair-8l6.56), or makes changes of them (docair-8l6.63); then joined with what the store had. A change a human wrote is kept and its pieces are not asked about; a store of another schema is asked again. A request that fails leaves its pieces; when none answers, the error is returned.
 *
 * @id    docair-ai.explain.explain
 * @calls docair-core.review.pieces "the pieces of the whole change"
 * @calls docair-ai.explain.digest "sends the pieces and the intent"
 * @calls docair-ai.client.chat_with (the pieces, the commit messages, the changed docs; not reasoning first) -> changes, links
 * @calls docair-ai.explain.bind "keeps only what binds"
 * @calls docair-ai.explain.place_left "puts the pieces no change held where the model says"
 * @calls docair-ai.explain.merge_same "merges what came back more than once"
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
    let out = ChangeFindings {
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
    let pack = |limit: usize| {
        let mut batches: Vec<Vec<&Piece>> = Vec::new();
        for p in &todo {
            let fits = batches.last().is_some_and(|last| {
                let mut next = last.clone();
                next.push(*p);
                digest(changes, a, b, &next, intent, batches.len() == 1).len() <= limit
            });
            match batches.last_mut() {
                Some(last) if fits => last.push(*p),
                _ => batches.push(vec![*p]),
            }
        }
        batches
    };
    // Asked in parts, each request also names the pieces the others hold (docair-8l6.60): room for them.
    // The room is what that list says at most, its words included, so no request goes past MAX_CHARS.
    let mut batches = pack(MAX_CHARS);
    if batches.len() > 1 {
        batches = pack(MAX_CHARS - elsewhere(&todo).len());
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
    // The changes this run made, from which request each: asked in parts, one may come back twice.
    let fresh = out.changes.len();
    // Each answer is bound once those before it are (docair-8l6.65): the review does not depend on
    // which came first, and the changes bound so far go out with the progress as they are made.
    let run = Mutex::new(RunState {
        out,
        taken,
        from: HashSet::new(),
        failed: None,
        read: 0,
        next: 0,
        waiting: std::collections::BTreeMap::new(),
    });
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
                            "The change from `{}` to `{}`:\n{}{}",
                            changes.a,
                            changes.b,
                            digest(changes, a, b, &batches[k], intent, k == 0),
                            elsewhere(&others_of(&batches, k))
                        )),
                    ];
                    // Not reasoning first: grouping pieces, a reasoning model deliberates until max_tokens runs out (docair-8l6.36).
                    let reply = chat_with(cfg, &messages, true, false).map(|r| r.content);
                    let mut r = run.lock().unwrap_or_else(|e| e.into_inner());
                    r.waiting.insert(k, reply);
                    let mut bound = false;
                    while let Some(reply) = {
                        let at = r.next;
                        r.waiting.remove(&at)
                    } {
                        let at = r.next;
                        r.next += 1;
                        bind_answer(
                            &mut r,
                            at,
                            reply,
                            (&batches[at], &others_of(&batches, at)),
                            (changes, a, b),
                            intent,
                            &recorded,
                        );
                        bound = true;
                    }
                    let mut p = progress.lock().unwrap_or_else(|e| e.into_inner());
                    p.answered += 1;
                    if bound {
                        p.found = r.out.changes[fresh..].to_vec();
                    }
                    on(&p);
                }
            });
        }
    });
    let RunState {
        mut out,
        mut taken,
        from,
        failed,
        read,
        ..
    } = run.into_inner().unwrap_or_else(|e| e.into_inner());
    // The last request (docair-8l6.56): which changes came back twice, and where the pieces no
    // change holds belong, with the model's words on them, or which changes of their own they make (8l6.63).
    let left_now: HashSet<&str> = out.left.iter().map(String::as_str).collect();
    let mut leftover: Vec<&Piece> = pieces
        .iter()
        .filter(|p| !taken.contains(p.id()) && !left_now.contains(p.id()))
        .collect();
    let fresh_n = out.changes.len() - fresh;
    let mut settled = 0;
    if fresh_n > 0 && (from.len() > 1 || !leftover.is_empty()) {
        let texts: HashMap<&str, &PartText> =
            intent.code.iter().map(|t| (t.part.as_str(), t)).collect();
        let mut sent_left: Vec<&Piece> = Vec::new();
        let mut asked = settle_digest(&out.changes[fresh..], &[], a, b, &texts);
        for p in leftover.drain(..) {
            let mut next = sent_left.clone();
            next.push(p);
            let d = settle_digest(&out.changes[fresh..], &next, a, b, &texts);
            if d.len() > MAX_CHARS {
                break;
            }
            (sent_left, asked) = (next, d);
        }
        {
            let mut p = progress.lock().unwrap_or_else(|e| e.into_inner());
            p.requests += 1;
            p.started += 1;
            on(&p);
        }
        let messages = [Message::system(SETTLE_PROMPT), Message::user(asked)];
        let reply = chat_with(cfg, &messages, true, false).map(|r| r.content);
        {
            let mut p = progress.lock().unwrap_or_else(|e| e.into_inner());
            p.answered += 1;
            on(&p);
        }
        settled = 1;
        match reply.ok().as_deref().and_then(reply_json) {
            Some(v) => {
                let mut mine = out.changes.split_off(fresh);
                let (placed, problems) =
                    place_left(&mut mine, &v, &sent_left, &known_ids((a, b), &pieces));
                taken.extend(placed);
                out.problems.extend(problems);
                out.problems.extend(merge_same(&mut mine, &v));
                // The left pieces part of none of them: changes of their own (docair-8l6.63), of the left
                // pieces only, bound as the review's are.
                if let Some(new) = v.get("new").filter(|n| n.is_array()) {
                    let own: Vec<Piece> = sent_left.iter().map(|p| (*p).clone()).collect();
                    let (_, mut made, problems) = bind(
                        changes,
                        (a, b),
                        &own,
                        &json!({ "changes": new }),
                        &recorded,
                        &mut taken,
                    );
                    out.problems.extend(problems);
                    out.problems.extend(check_where(&mut made, intent));
                    mine.extend(made);
                }
                out.changes.extend(mine);
            }
            None => out.problems.push(
                "the last request failed: a change may show twice, and each piece no change holds shows as a change of its own".into(),
            ),
        }
    }
    for batch in &batches[sent..] {
        out.left.extend(batch.iter().map(|p| p.id().to_string()));
    }
    out.requests = sent + settled;
    if read == 0
        && let Some(e) = failed
    {
        return Err(e);
    }
    // Every piece is a change: one no change holds, nor left for later, is kept in `other`, and shown as a
    // change of its own (docair-8l6.63).
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

/// The last request's input (docair-8l6.56): each change by number, from 1, with its title, why and what its pieces do (at most 8), and the pieces no change holds, as the review's request sends pieces.
fn settle_digest(
    changes: &[ReviewChange],
    left: &[&Piece],
    a: &Graph,
    b: &Graph,
    texts: &HashMap<&str, &PartText>,
) -> String {
    let list: Vec<Value> = changes
        .iter()
        .enumerate()
        .map(|(i, c)| {
            json!({
                "number": i + 1,
                "title": c.title,
                "why": c.why,
                "pieces do": c.how.iter().take(8).map(|h| h.words.as_str()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let left: Vec<Value> = left.iter().map(|p| piece_json(a, b, p, texts)).collect();
    serde_json::to_string_pretty(&json!({ "changes": list, "left": left })).unwrap_or_default()
}

/// The left piece a name means: its id; else the one that holds the part as new, gone or edited (also named like a piece, `code:<part>`); else the one whose box the part is in.
fn resolve_left<'p>(named: &str, left: &[&'p Piece]) -> Option<&'p str> {
    if let Some(p) = left.iter().find(|p| p.id() == named) {
        return Some(p.id());
    }
    let part = ["path:", "line:", "code:", "box:"]
        .iter()
        .find_map(|k| named.strip_prefix(k))
        .unwrap_or(named);
    let holds = |p: &&&Piece| {
        let s = &p.step;
        s.added
            .iter()
            .chain(&s.removed)
            .chain(&s.code)
            .any(|x| x == part)
    };
    if let Some(p) = left.iter().find(holds) {
        return Some(p.id());
    }
    let in_box = |p: &&&Piece| {
        let bx = p.id().strip_prefix("box:").or(p.step.in_box.as_deref());
        bx.is_some_and(|bx| part == bx || part.starts_with(&format!("{bx}.")))
    };
    let mut boxes = left.iter().filter(in_box);
    match (boxes.next(), boxes.next()) {
        (Some(p), None) => Some(p.id()),
        _ => None,
    }
}

/***
 * Put the pieces no change held into the change the model says (docair-8l6.56): `{"place": [{"piece", "change", "words"}]}`, numbers from 1 into `changes`. The piece by its id, or by a part it holds, or by a part in its box (`resolve_left`). Only a piece of `left`, once, into a change there is, with words that name only what there is (`ids`); else it is left out and said. Returns the pieces placed, and what was left out.
 ***/
pub fn place_left(
    changes: &mut [ReviewChange],
    answer: &Value,
    left: &[&Piece],
    ids: &HashSet<&str>,
) -> (Vec<String>, Vec<String>) {
    let (mut placed, mut problems) = (Vec::new(), Vec::new());
    let left_ids: Vec<&str> = left.iter().map(|p| p.id()).collect();
    for x in answer["place"].as_array().into_iter().flatten() {
        let named = x["piece"].as_str().unwrap_or("").trim();
        let piece = resolve_left(named, left).unwrap_or(named).to_string();
        let words = x["words"].as_str().unwrap_or("").trim().to_string();
        let to = x["change"].as_u64().map(|n| n as usize);
        let bad = unknown_in(&words, ids);
        match to {
            _ if !left_ids.contains(&piece.as_str()) || placed.contains(&piece) => problems.push(
                format!("the model placed `{piece}`, which no change was missing; left out"),
            ),
            Some(n) if (1..=changes.len()).contains(&n) && !words.is_empty() && bad.is_empty() => {
                changes[n - 1].how.push(HowStep {
                    piece: piece.clone(),
                    words,
                });
                placed.push(piece);
            }
            _ => problems.push(format!(
                "the model placed `{piece}` into no change there is, or with words that name {}; left an other edit",
                if bad.is_empty() { "nothing".to_string() } else { bad.join(", ") }
            )),
        }
    }
    (placed, problems)
}

/***
 * Merge what the model says is one change (docair-8l6.56): `{"same": [[1, 4], …]}`, numbers from 1 into `changes`. Each group goes into its first change: the pieces in order without repeats, what was found off without repeats, the first title, why and check that are not empty. A group that names no change, only one, or a change already in another group is left out and said. Returns what was left out.
 ***/
pub fn merge_same(changes: &mut Vec<ReviewChange>, answer: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    let mut used: HashSet<usize> = HashSet::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for g in answer["same"].as_array().into_iter().flatten() {
        let nums: Vec<Option<usize>> = g
            .as_array()
            .into_iter()
            .flatten()
            .map(|n| n.as_u64().map(|n| n as usize))
            .collect();
        let mut group: Vec<usize> = Vec::new();
        for n in nums.iter().flatten() {
            if !group.contains(n) {
                group.push(*n);
            }
        }
        let fine = nums.iter().all(Option::is_some)
            && group.len() >= 2
            && group
                .iter()
                .all(|n| (1..=changes.len()).contains(n) && !used.contains(n));
        if !fine {
            problems.push(format!("the model's merge {g} names no two changes of this review, or one twice; left apart"));
            continue;
        }
        used.extend(group.iter().copied());
        groups.push(group.into_iter().map(|n| n - 1).collect());
    }
    let mut gone: HashSet<usize> = HashSet::new();
    for group in &groups {
        let (first, rest) = (group[0], &group[1..]);
        for &i in rest {
            let other = changes[i].clone();
            let into = &mut changes[first];
            for h in other.how {
                if !into.how.iter().any(|x| x.piece == h.piece) {
                    into.how.push(h);
                }
            }
            for x in other.off {
                if !into.off.contains(&x) {
                    into.off.push(x);
                }
            }
            if into.why.is_empty() {
                into.why = other.why;
            }
            if into.check.is_empty() {
                into.check = other.check;
            }
            gone.insert(i);
        }
    }
    let mut i = 0;
    changes.retain(|_| {
        i += 1;
        !gone.contains(&(i - 1))
    });
    problems
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

/// An off says where it was said only of what is there (docair-8l6.58): a commit by its subject, a doc by its
/// path (made whole when the model gave its end); else its where is left out, and said.
fn check_where(changes: &mut [ReviewChange], intent: &Intent) -> Vec<String> {
    let plain = |t: &str| t.trim().trim_end_matches('.').trim().to_lowercase();
    let mut problems = Vec::new();
    for c in changes.iter_mut() {
        for d in &mut c.off {
            if d.place.is_empty() {
                continue;
            }
            let found = match d.source.as_str() {
                "commit" => {
                    let place = plain(&d.place);
                    intent
                        .commits
                        .iter()
                        .find(|m| {
                            let subject = plain(&m.subject);
                            subject == place || (place.len() >= 12 && subject.starts_with(&place))
                        })
                        .map(|m| m.subject.clone())
                }
                "doc" => intent
                    .docs
                    .iter()
                    .chain(&intent.files)
                    .find(|f| f.path == d.place || f.path.ends_with(&format!("/{}", d.place)))
                    .map(|f| f.path.clone()),
                _ => Some(d.place.clone()),
            };
            match found {
                Some(p) => d.place = p,
                None => {
                    problems.push(format!(
                        "\"{}\": what it found off cites the {} \"{}\", which is not there; where left out",
                        c.title, d.source, d.place
                    ));
                    d.place.clear();
                }
            }
        }
    }
    problems
}

/// Who said what an off sets against the code, and what: `the commit "Audit each order": every order is recorded`;
/// said nowhere, that nothing says it.
pub fn said(d: &FoundOff) -> String {
    let who = match (d.source.as_str(), d.place.as_str()) {
        ("none", _) => return "nothing: no commit, doc or comment mentions it.".into(),
        ("commit", "") => "a commit message".to_string(),
        ("commit", p) => format!("the commit \u{201c}{p}\u{201d}"),
        ("doc", "") => "a doc".to_string(),
        ("doc", p) => format!("`{p}`"),
        ("comment", "") => "a code comment".to_string(),
        ("comment", p) => format!("the comment on `{p}`"),
        (_, _) => "it was said".to_string(),
    };
    format!("{who}: {}", d.says)
}

/***
 * The review as a pull request comment (docs/EXPLAIN.md §9.6): each change with what the app does differently, why, what the AI found off by kind (or that it found nothing) and what to check, and how its pieces work together folded in `<details>`; then each piece the AI put in no change as a change of its own, in docair's words; the links folded.
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
    let other: Vec<&Piece> = review.other.iter().filter_map(|id| piece(id)).collect();
    if review.changes.is_empty() && other.is_empty() {
        out.push_str(if pieces.is_empty() {
            "Nothing changed.\n"
        } else {
            "No change explained yet: `docair explain` asks the AI.\n"
        });
    }
    for (k, c) in review.changes.iter().enumerate() {
        out.push_str(&format!("**{}. {}**\n\n", k + 1, c.title));
        if !c.why.is_empty() {
            out.push_str(&format!("Why: {}\n\n", c.why));
        }
        // What the AI found off: what was said against what the code does (docair-8l6.55, 8l6.58).
        if c.off.is_empty() {
            out.push_str("Checked against the code: nothing found off.\n\n");
        } else {
            // By kind, each under its heading, the clear contradictions first (docair-8l6.62).
            out.push_str("⚠ Found off:\n\n");
            for (kind, heading) in docair_core::review::OFF_KINDS {
                let mine: Vec<&FoundOff> = c.off.iter().filter(|d| d.kind() == kind).collect();
                if mine.is_empty() {
                    continue;
                }
                out.push_str(&format!("*{heading}:*\n\n"));
                for d in mine {
                    let at = if d.parts.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " (in {})",
                            d.parts
                                .iter()
                                .map(|p| format!("`{p}`"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    };
                    if kind == "contradicts" {
                        out.push_str(&format!(
                            "- **Said:** {}  \n  **The code:** {}{at}\n",
                            said(d),
                            d.code
                        ));
                    } else {
                        out.push_str(&format!("- {}{at}\n", d.code));
                    }
                }
                out.push('\n');
            }
        }
        if !c.check.is_empty() {
            out.push_str(&format!("Check: {}\n\n", c.check));
        }
        out.push_str("<details><summary>How</summary>\n\n");
        for h in &c.how {
            if let Some(p) = piece(&h.piece) {
                out.push_str(&format!("- {} ({})\n", h.words, piece_words(p, a, b)));
            }
        }
        out.push_str("</details>\n\n");
    }
    // Every piece is a change (docair-8l6.63): one the AI put in none is a change of its own, in docair's words.
    for (k, p) in other.into_iter().enumerate() {
        let words = piece_words(p, a, b);
        let mut title = words.chars();
        let title = title.next().map_or(String::new(), |f| {
            f.to_uppercase().chain(title).collect::<String>()
        });
        out.push_str(&format!(
            "**{}. {title}**\n\n_In docair's words: the AI put this piece in no change._\n\n",
            review.changes.len() + k + 1
        ));
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
            code: vec![],
            files: vec![],
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

    /// docair-8l6.20, 8l6.21, 8l6.55: the digest carries the pieces and the intent, the commits and
    /// docs without ids to cite; binding keeps a change with a known piece and words that name what
    /// there is, with what the model found off, and leaves out the rest.
    /// Asked in parts, a request is told the others' pieces, and what it still names of them is left to
    /// them, said once (docair-8l6.60).
    #[test]
    fn a_request_leaves_the_others_pieces_to_them() {
        let (before, after, changes, _) = two_changes();
        let pieces = docair_core::review::pieces(&before, &after, &changes);
        let write = pieces
            .iter()
            .find(|p| p.step.lines_added.iter().any(|l| l.contains("orders-db")))
            .unwrap();
        let code = pieces
            .iter()
            .find(|p| p.step.code.contains(&"shop-ui.cart.total".to_string()))
            .unwrap();
        let told = elsewhere(&[code]);
        assert!(
            told.contains("asked about in the other parts") && told.contains(code.id()),
            "{told}"
        );
        assert_eq!(elsewhere(&[]), "", "one request is told nothing");
        let mut answer = json!({ "changes": [
            { "title": "Orders are kept.", "how": [ { "piece": code.id() }, { "piece": write.id() } ] },
            { "title": "The cart costs more.", "how": [ { "piece": "path:shop-ui.cart.total" } ] },
            { "title": "Names nothing known.", "how": [ { "piece": "path:nowhere" } ] }
        ]});
        assert_eq!(leave_elsewhere(&mut answer, &[write], &[code]), (2, 1));
        let left = answer["changes"].as_array().unwrap();
        assert_eq!(left.len(), 2, "{answer}");
        assert_eq!(left[0]["how"], json!([{ "piece": write.id() }]));
        assert_eq!(
            left[1]["title"], "Names nothing known.",
            "what no request holds stays, for bind to say"
        );
    }

    #[test]
    fn a_review_binds_to_pieces_in_its_own_words() {
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
            sent.contains(write.id()) && sent.contains("README.md"),
            "{sent}"
        );
        // The words name a piece of one part as that part (docair-8l6.64); a guessed piece id too.
        let parts: HashSet<&str> = after.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            as_parts(
                &format!(
                    "`{}` is empty; `path:shop-ui.cart.total` too, and `{}`.",
                    code.id(),
                    write.id()
                ),
                &pieces,
                &parts
            ),
            format!(
                "`shop-ui.cart.total` is empty; `shop-ui.cart.total` too, and `{}`.",
                as_parts(&format!("`{}`", write.id()), &pieces, &parts).trim_matches('`')
            )
        );
        assert_eq!(as_parts("an odd ` tick", &pieces, &parts), "an odd ` tick");
        assert!(
            !sent.contains("commit:") && !sent.contains("doc:"),
            "nothing to cite: {sent}"
        );
        // docair-8l6.47: a piece carries its parts' whole texts, before and after; files as lines.
        let mut with_text = intent.clone();
        with_text.code.push(docair_core::review::PartText {
            part: "shop-ui.cart.total".into(),
            before: Some("/// Total of the cart.\ntotal() { return n * 100; }".into()),
            after: Some("/// Total of the cart.\ntotal() { return n * 120; }".into()),
        });
        with_text.files.push(DocChange {
            id: "file:ui/style.css".into(),
            path: "ui/style.css".into(),
            lines: "+body { color: red; }".into(),
        });
        let sent = digest(&changes, &before, &after, &refs, &with_text, true);
        let v: Value = serde_json::from_str(&sent).unwrap();
        let piece = v["pieces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == code.id())
            .unwrap();
        assert_eq!(
            piece["source"]["shop-ui.cart.total"]["after"],
            "/// Total of the cart.\ntotal() { return n * 120; }"
        );
        assert!(
            piece["source"]["shop-ui.cart.total"]["before"]
                .as_str()
                .unwrap()
                .contains("n * 100")
        );
        assert_eq!(v["files"][0]["path"], "ui/style.css");
        assert!(v["files"][0].get("id").is_none());
        let reply = json!({ "changes": [
            { "title": "A placed order is now kept in the orders table.",
              "how": [ { "piece": write.id(), "words": "`orders.create` now writes the order to `orders-db`." },
                       { "piece": "path:invented", "words": "Made up." } ],
              "why": "Orders were lost on a restart; kept in a table, they last.",
              "off": [
                  { "kind": "contradicts", "source": "commit", "where": "Keep every order.", "says": "Every order is checked, then kept.",
                    "code": "`orders.create` writes before it checks the cart.",
                    "parts": ["`orders.create`", "nowhere.x"] },
                  { "source": "none", "code": "`ghost.part` is off." },
                  { "kind": "Contradiction", "source": "doc", "where": "NOTES.md", "says": "The cart is checked.", "code": "Nothing checks the cart.",
                    "parts": [code.id()] },
                  { "kind": "unsaid", "source": "commit", "where": "x", "says": "y", "code": "Nothing tests `orders.create`'s write." },
                  { "source": "commit", "says": "Faster." },
                  { "kind": "contradicts", "code": "`orders.create` checks nothing." },
                  "A bare sentence." ],
              "check": "Writing must not slow down `orders.create`." },
            { "title": "This one takes the same piece again.", "how": [ { "piece": write.id(), "words": "Again." } ] },
            { "title": "Names `nowhere.part`.", "how": [ { "piece": code.id(), "words": "x" } ] },
            // A part's id where a piece's belongs: bound to the piece that holds that part.
            { "title": "The cart's total is worked out anew.", "how": [ { "piece": "shop-ui.cart.total", "words": "The total changed." } ] },
        ]});
        let mut taken = HashSet::new();
        let (_, mut found, mut problems) = bind(
            &changes,
            (&before, &after),
            &pieces,
            &reply,
            &rec(),
            &mut taken,
        );
        problems.extend(check_where(&mut found, &intent));
        assert_eq!(found.len(), 2, "{problems:#?}");
        assert_eq!(
            found[1].how[0].piece,
            code.id(),
            "the part's id binds to its piece"
        );
        let c = &found[0];
        assert_eq!(c.how.len(), 1);
        // docair-8l6.58: what was said against what the code does; a commit by its subject as it is.
        // docair-8l6.61: the parts where the code does it; a piece stands for its parts.
        let code_parts: Vec<&str> = code
            .step
            .added
            .iter()
            .chain(&code.step.removed)
            .chain(&code.step.code)
            .map(String::as_str)
            .collect();
        assert!(code_parts.contains(&"shop-ui.cart.total"));
        let off =
            |kind: &str, source: &str, place: &str, says: &str, code: &str, parts: &[&str]| {
                FoundOff {
                    kind: kind.into(),
                    source: source.into(),
                    place: place.into(),
                    says: says.into(),
                    code: code.into(),
                    parts: parts.iter().map(|p| p.to_string()).collect(),
                }
            };
        assert_eq!(
            c.off,
            [
                off(
                    "contradicts",
                    "commit",
                    "Keep every order",
                    "Every order is checked, then kept.",
                    "`orders.create` writes before it checks the cart.",
                    &["orders.create"]
                ),
                off(
                    "contradicts",
                    "doc",
                    "",
                    "The cart is checked.",
                    "Nothing checks the cart.",
                    &code_parts
                ),
                off(
                    "unsaid",
                    "none",
                    "",
                    "",
                    "Nothing tests `orders.create`'s write.",
                    &["orders.create"]
                ),
                off(
                    "risk",
                    "none",
                    "",
                    "",
                    "`orders.create` checks nothing.",
                    &["orders.create"]
                ),
                off("risk", "none", "", "", "A bare sentence.", &[]),
            ],
            "a part that is not there, or no code: left out; a doc that is not there: no where; only a contradiction says what was said, and one that names nothing said is a risk"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("kept as looks wrong or risky")),
            "{problems:#?}"
        );
        // A review stored before the kinds: a contradiction where something was said, else a risk.
        let old = |v: Value| serde_json::from_value::<FoundOff>(v).unwrap();
        assert_eq!(
            old(json!({"source": "commit", "where": "x", "says": "y", "code": "z"})).kind(),
            "contradicts"
        );
        assert_eq!(old(json!({"source": "none", "code": "z"})).kind(), "risk");
        assert!(
            problems.iter().any(|p| p.contains("NOTES.md")),
            "{problems:#?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("`nowhere.x`")),
            "{problems:#?}"
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("says nothing of the code"))
        );
        assert!(!c.check.is_empty() && !c.why.is_empty());
        assert!(taken.contains(write.id()) && taken.contains(code.id()));
        assert!(problems.iter().any(|p| p.contains("path:invented")));
        assert!(problems.iter().any(|p| p.contains("ghost.part")));
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
            text.contains("Why: Orders were lost on a restart; kept in a table, they last.\n")
                && text.contains(
                    "- **Said:** the commit \u{201c}Keep every order\u{201d}: Every order is checked, then kept.  \n  **The code:** `orders.create` writes before it checks the cart. (in `orders.create`)\n"
                )
                && text.contains("⚠ Found off:\n\n*Said, but the code does otherwise:*\n\n- **Said:** the commit")
                && text.contains("*Done, but said nowhere:*\n\n- Nothing tests `orders.create`'s write. (in `orders.create`)\n\n*Looks wrong or risky:*\n\n- `orders.create` checks nothing. (in `orders.create`)\n- A bare sentence.\n"),
            "{text}"
        );
        assert!(
            // Every piece is a change (docair-8l6.63): one the AI put in none is one of its own.
            text.contains(
                ". Code of `total`**\n\n_In docair's words: the AI put this piece in no change._"
            ) && !text.contains("Other edits"),
            "{text}"
        );
    }

    /// docair-8l6.56: what the model says is one change is merged into the first; a bad group is
    /// left apart and said. A left piece goes into the change it names, once, with words that name
    /// what there is.
    #[test]
    fn the_last_request_merges_and_places() {
        let c = |title: &str, why: &str, pieces: &[&str], off: &[&str]| ReviewChange {
            title: title.into(),
            how: pieces
                .iter()
                .map(|p| HowStep {
                    piece: p.to_string(),
                    words: format!("does {p}"),
                })
                .collect(),
            why: why.into(),
            off: off
                .iter()
                .map(|x| FoundOff {
                    kind: "risk".into(),
                    source: "none".into(),
                    place: String::new(),
                    says: String::new(),
                    code: x.to_string(),
                    parts: vec![],
                })
                .collect(),
            check: String::new(),
            recorded: rec(),
        };
        let mut changes = vec![
            c("Orders are audited", "", &["p1", "p2"], &["no test"]),
            c("The cart costs more", "a price change", &["p3"], &[]),
            c(
                "Every order gets an audit entry",
                "auditors asked",
                &["p2", "p4"],
                &["the README is out of date", "no test"],
            ),
        ];
        let g = shop();
        let sent = settle_digest(&changes, &[], &g, &g, &HashMap::new());
        assert!(
            sent.contains("\"number\": 3") && sent.contains("does p4"),
            "{sent}"
        );
        // Placed first, by the numbers the model was given; then merged.
        let ids: HashSet<&str> = ["p5", "p6", "shop.part"].into_iter().collect();
        let answer = json!({
            "same": [[1, 3], [2, 9], [3, 2]],
            "place": [
                { "piece": "p5", "change": 3, "words": "`shop.part` keeps the entry." },
                { "piece": "p5", "change": 1, "words": "Again." },
                { "piece": "p6", "change": 7, "words": "Nowhere." },
                { "piece": "p7", "change": 2, "words": "Not left." },
                // A part in a left piece's box names that piece.
                { "piece": "shop.cart.total", "change": 2, "words": "Shows the total." },
            ]
        });
        let left_piece = |id: &str, code: &[&str]| Piece {
            step: docair_core::change_steps::ChangeStep {
                id: id.into(),
                kind: docair_core::change_steps::StepKind::Code,
                added: vec![],
                removed: vec![],
                code: code.iter().map(|c| c.to_string()).collect(),
                ends: vec![],
                lines_added: vec![],
                lines_removed: vec![],
                lines: vec![],
                in_box: None,
            },
            flows: vec![],
        };
        let (p5, p6, bx) = (
            left_piece("p5", &[]),
            left_piece("p6", &[]),
            left_piece("box:shop.cart", &["shop.cart.Cart"]),
        );
        let (placed, missed) = place_left(&mut changes, &answer, &[&p5, &p6, &bx], &ids);
        assert_eq!(placed, ["p5", "box:shop.cart"]);
        assert_eq!(missed.len(), 3, "{missed:?}");
        let problems = merge_same(&mut changes, &answer);
        assert_eq!(changes.len(), 2, "{changes:?}");
        let audit = &changes[0];
        assert_eq!(audit.title, "Orders are audited");
        assert_eq!(audit.pieces().collect::<Vec<_>>(), ["p1", "p2", "p4", "p5"]);
        assert_eq!(
            audit
                .off
                .iter()
                .map(|o| o.code.as_str())
                .collect::<Vec<_>>(),
            ["no test", "the README is out of date"]
        );
        assert_eq!(
            audit.why, "auditors asked",
            "the first why that is not empty"
        );
        assert_eq!(changes[1].title, "The cart costs more");
        assert_eq!(
            changes[1].pieces().collect::<Vec<_>>(),
            ["p3", "box:shop.cart"]
        );
        assert_eq!(
            problems.len(),
            2,
            "9 is no change; 3 is already merged: {problems:?}"
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
            off: vec![],
            check: String::new(),
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

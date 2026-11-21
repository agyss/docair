//! docair's review on a pull request (docs/PULL_REQUESTS.md): a summary comment and one comment
//! for each change of the review, numbered as the page numbers them, each with its cut drawn as
//! text; and the plan that brings a pull request's comments up to date after a push.
//!
//! The comments hold `PAGE` where the page's address goes; `plan` puts the address in, or drops
//! the lines that link to it when there is none.

use std::collections::HashSet;
use std::fmt::Write;

use docair_core::change_steps;
use docair_core::model::{Graph, Node};
use docair_core::review::Piece;
use serde_json::{Value, json};

use crate::doc_page::{Changed, CodeLinks};
use crate::{layout_input, page_changes, svg, text_pic};

/// Where the page's address goes in a comment.
pub const PAGE: &str = "{{docair-page}}";

/// The hidden mark that tells docair's comments from others, and what each is about.
const MARK: &str = "<!-- docair:pr ";

/// A picture past this many parts is folded.
const FOLD_PAST: usize = 30;
/// GitHub takes a comment of at most 65,536 characters; past this, the picture is left to the page.
const MAX_BODY: usize = 60_000;
/// A piece's parts listed under how, at most.
const PARTS_LISTED: usize = 8;

/// One comment docair writes on a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrComment {
    /// `summary`, `change`, or `gone` for a change no longer in the pull request.
    pub kind: String,
    /// The pieces it is about, by id; none for the summary.
    pub pieces: Vec<String>,
    /// Its Markdown, the hidden mark first; `PAGE` where the page's address goes.
    pub body: String,
}

impl PrComment {
    pub fn to_json(&self) -> Value {
        json!({ "kind": self.kind, "pieces": self.pieces, "body": self.body })
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        Some(PrComment {
            kind: v.get("kind")?.as_str()?.to_string(),
            pieces: v
                .get("pieces")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            body: v.get("body")?.as_str()?.to_string(),
        })
    }
}

/// The review as comments: the summary, then each change in the page's order.
pub struct Review {
    pub summary: PrComment,
    pub changes: Vec<PrComment>,
    /// Each change's cut as an SVG file of its own, in the page's colours; empty for a change
    /// whose cut has no line.
    pub svgs: Vec<String>,
}

/// The hidden mark: what a comment is about, as JSON in an HTML comment. A `>` in a piece's id
/// (`line:a->b`) is written `>`, so the mark cannot end early.
fn mark(kind: &str, pieces: &[String]) -> String {
    let what = json!({ "kind": kind, "pieces": pieces }).to_string();
    format!("{MARK}{} -->", what.replace('>', "\\u003e"))
}

/// A comment's hidden mark read back: its kind and its pieces; none when it has no mark.
pub fn read_mark(body: &str) -> Option<(String, Vec<String>)> {
    let start = body.find(MARK)? + MARK.len();
    let end = body[start..].find(" -->")? + start;
    let v: Value = serde_json::from_str(&body[start..end]).ok()?;
    let pieces = v["pieces"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    Some((v["kind"].as_str()?.to_string(), pieces))
}

fn text(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn node<'g>(g: &'g Graph, id: &str) -> Option<&'g Node> {
    g.nodes.iter().find(|n| n.id == id)
}

/// A part as Markdown: its id in code, linked to its file and line at B (or at A, for a part only
/// A has) when the repository host is known.
fn part_link(id: &str, before: &Graph, after: &Graph, links: &CodeLinks) -> String {
    let at = node(after, id)
        .map(|n| (n, links.at_b.as_ref()))
        .or_else(|| node(before, id).map(|n| (n, links.at_a.as_ref())));
    let url = at.and_then(|(n, t)| {
        let s = n.source.as_ref()?;
        CodeLinks::to(t, &s.file, s.line)
    });
    match url {
        Some(u) if u.starts_with("http") => format!("[`{id}`]({u})"),
        _ => format!("`{id}`"),
    }
}

/// What a found-off item says was said: who said it, and what.
fn said(off: &Value) -> String {
    let (source, place, says) = (text(off, "source"), text(off, "where"), text(off, "says"));
    let who = match (source.as_str(), place.as_str()) {
        ("commit", "") => "a commit message".to_string(),
        ("commit", p) => format!("the commit \u{201c}{p}\u{201d}"),
        ("doc", "") => "a doc".to_string(),
        ("doc", p) => format!("`{p}`"),
        ("comment", "") => "a code comment".to_string(),
        ("comment", p) => format!("the comment on `{p}`"),
        _ => "it was said".to_string(),
    };
    format!("{who}: {says}")
}

/// A found-off item's kind; one stored before the kinds is a contradiction when it says what was
/// said, else a risk (docair-tc4).
fn off_kind(off: &Value) -> String {
    let k = text(off, "kind");
    if !k.is_empty() {
        k
    } else if !text(off, "says").is_empty() {
        "contradicts".into()
    } else {
        "risk".into()
    }
}

/// A drawn SVG as a file of its own: the page's colours, light and dark, and the pictures' styles.
fn standalone(drawn: &str) -> String {
    let colours = crate::CSS.split("\n* {").next().unwrap_or_default();
    let Some(end) = drawn.find('>') else {
        return drawn.to_string();
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}<style><![CDATA[\n{colours}\n{}]]></style>{}",
        &drawn[..=end],
        svg::STYLE,
        &drawn[end + 1..]
    )
}

/// A change's cut, laid out: as text for the comment (with how many parts it has) and as an SVG
/// file; none when it has no line, as a picture of parts alone says nothing the text does not.
fn cut(
    changed: &Changed,
    after: &Graph,
    addr: &str,
    its: &[&Piece],
) -> Option<(String, usize, String)> {
    let (m, f) =
        docair_core::review::change_cut(changed.before, after, changed.changes, addr, its)?;
    if m.pic.lines.is_empty() {
        return None;
    }
    let code = change_steps::changed_inside(&changed.changes.graph).code;
    let marks = change_steps::marks(&m, &f, &code);
    let laid = layout_input::lay_out(&m.graph, &m.pic);
    let drawn = text_pic::draw(&m.graph, &m.pic, &laid, Some(&marks));
    let c = page_changes::classes(&marks);
    let svg = svg::draw_marked(
        &m.graph,
        &m.pic,
        &laid,
        &svg::ChangeMarks {
            parts: &c.parts,
            lines: &c.lines,
            tags: &c.tags,
        },
    );
    Some((drawn, m.pic.parts.len(), standalone(&svg)))
}

/// Why a change is in docair's words, not the AI's.
fn docair_note(ch: &Value, no_ai: bool) -> &'static str {
    if no_ai {
        return "_In docair's words: the AI did not review this pull request._";
    }
    match ch["note"].as_str() {
        Some("left") => "_In docair's words: the review did not reach this piece._",
        Some("newer") => "_In docair's words: the review is older than this piece._",
        _ => "_In docair's words: the AI put this piece in no change._",
    }
}

/***
 * The review as comments on a pull request (docs/PULL_REQUESTS.md §2, §3): for each change in the page's order (`page_changes::review_list`), its title, its cut drawn as text with the marks' legend, why, what the AI found off by kind or that it found nothing, what to check, how its pieces work together folded with their parts linked to the code, and a link to walk it on the page at `#review/<n>`; a hidden mark with its pieces. And a summary that lists them. `no_ai` says why there is no review by the AI (no key, or the review failed): every piece is then a change of its own, in docair's words.
 *
 * @calls docair-viewer.page_changes.review_list "the changes in the page's order"
 * @calls docair-core.review.change_cut "each change's cut"
 * @calls docair-viewer.text_pic.draw "draws the cut as text"
 ***/
pub fn review(after: &Graph, changed: &Changed, no_ai: Option<&str>) -> Review {
    let c = changed.changes;
    let before = changed.before;
    let pieces = docair_core::review::pieces(before, after, c);
    let piece = |id: &str| pieces.iter().find(|p| p.id() == id);
    let findings = if no_ai.is_some() {
        None
    } else {
        changed.findings
    };
    let list = page_changes::review_list(after, findings, &pieces);
    let n = list.found.len();
    let mut changes = Vec::with_capacity(n);
    let mut svgs = Vec::with_capacity(n);
    let mut index = String::new();
    for (k, ch) in list.found.iter().enumerate() {
        let addr = format!("review/{}", k + 1);
        let title = text(ch, "title");
        let how: Vec<(String, String)> = ch["how"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|h| (text(h, "piece"), text(h, "words")))
            .filter(|(p, _)| piece(p).is_some())
            .collect();
        let its: Vec<&Piece> = how.iter().filter_map(|(p, _)| piece(p)).collect();
        let ids: Vec<String> = its.iter().map(|p| p.id().to_string()).collect();
        let by_docair = ch["recorded"]["by"].as_str() == Some("docair");
        let _ = writeln!(
            index,
            "{}. **{title}** · {} piece{}{}",
            k + 1,
            its.len(),
            if its.len() == 1 { "" } else { "s" },
            if by_docair {
                " · in docair's words"
            } else {
                ""
            }
        );

        let mut body = format!(
            "{}\n### {} of {n} · {title}\n\n",
            mark("change", &ids),
            k + 1
        );
        if by_docair {
            let _ = write!(body, "{}\n\n", docair_note(ch, no_ai.is_some()));
        }
        let drawn = cut(changed, after, &addr, &its);
        let picture = drawn.as_ref().map(|(t, parts, _)| {
            let block = format!("```text\n{t}```\n<sub>{}</sub>\n\n", text_pic::LEGEND);
            if *parts > FOLD_PAST {
                format!("<details><summary>Its picture · {parts} parts</summary>\n\n{block}</details>\n\n")
            } else {
                block
            }
        });
        svgs.push(drawn.map(|d| d.2).unwrap_or_default());
        let picture_at = body.len();
        if let Some(p) = &picture {
            body.push_str(p);
        }
        let why = text(ch, "why");
        if !why.is_empty() {
            let _ = write!(body, "**Why:** {why}\n\n");
        }
        if !by_docair {
            let off: Vec<&Value> = ch["off"].as_array().into_iter().flatten().collect();
            if off.is_empty() {
                body.push_str("Checked against the code: nothing found off.\n\n");
            } else {
                body.push_str("**⚠ Found off**\n\n");
                for (kind, heading) in docair_core::review::OFF_KINDS {
                    let mine: Vec<&&Value> = off.iter().filter(|o| off_kind(o) == kind).collect();
                    if mine.is_empty() {
                        continue;
                    }
                    let _ = write!(body, "*{heading}:*\n\n");
                    for o in mine {
                        let parts: Vec<String> = o["parts"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                            .map(|p| part_link(p, before, after, changed.links))
                            .collect();
                        let at = if parts.is_empty() {
                            String::new()
                        } else {
                            format!(" (in {})", parts.join(", "))
                        };
                        if kind == "contradicts" {
                            let _ = writeln!(
                                body,
                                "- **Said:** {}  \n  **The code:** {}{at}",
                                said(o),
                                text(o, "code")
                            );
                        } else {
                            let _ = writeln!(body, "- {}{at}", text(o, "code"));
                        }
                    }
                    body.push('\n');
                }
            }
            let check = text(ch, "check");
            if !check.is_empty() {
                let _ = write!(body, "**Check:** {check}\n\n");
            }
        }
        let _ = write!(
            body,
            "<details><summary>How · {} piece{}</summary>\n\n",
            its.len(),
            if its.len() == 1 { "" } else { "s" }
        );
        for ((_, words), p) in how.iter().zip(&its) {
            let words = if words.is_empty() {
                page_changes::plain_words(&page_changes::docair_words(after, &p.step))
            } else {
                words.clone()
            };
            let mut parts: Vec<&String> = Vec::new();
            for id in p.step.parts() {
                if !parts.contains(&id) {
                    parts.push(id);
                }
            }
            let more = parts.len().saturating_sub(PARTS_LISTED);
            let mut listed: Vec<String> = parts
                .iter()
                .take(PARTS_LISTED)
                .map(|id| part_link(id, before, after, changed.links))
                .collect();
            if more > 0 {
                listed.push(format!("{more} more"));
            }
            let _ = writeln!(body, "1. {words}  \n   {}", listed.join(", "));
        }
        body.push_str("\n</details>\n\n");
        let _ = writeln!(body, "[▶ Walk through it on the page]({PAGE}#{addr})");
        // Too long for a comment: the picture is left to the page.
        if body.len() > MAX_BODY
            && let Some(p) = &picture
        {
            body.replace_range(
                picture_at..picture_at + p.len(),
                "_Its picture is too large for a comment: the page has it._\n\n",
            );
        }
        changes.push(PrComment {
            kind: "change".into(),
            pieces: ids,
            body,
        });
    }

    let mut summary = format!(
        "{}\n### docair: the review of `{}` → `{}`\n\n",
        mark("summary", &[]),
        short(&c.a),
        short(&c.b)
    );
    if let Some(why) = no_ai {
        let _ = write!(
            summary,
            "> **No review by the AI:** {why}. Each piece is a change of its own, in docair's words.\n\n"
        );
    }
    if n == 0 {
        summary.push_str("Nothing changed in what docair reads.\n");
    } else {
        let _ = write!(
            summary,
            "{n} change{}, each in a comment of its own below.\n\n{index}\n",
            if n == 1 { "" } else { "s" }
        );
        let _ = writeln!(
            summary,
            "[▶ Walk through the review on the page]({PAGE}#review/1)"
        );
    }
    if !c.graph.toml.is_empty() {
        let _ = write!(
            summary,
            "\n<details><summary>docair.toml changed</summary>\n\n```diff\n{}\n```\n</details>\n",
            c.graph.toml.join("\n")
        );
    }
    let _ = write!(
        summary,
        "\n<sub>From the merge base to the head, as GitHub's Files changed. Pictures are drawn as text: {}.</sub>\n",
        text_pic::LEGEND
    );
    Review {
        summary: PrComment {
            kind: "summary".into(),
            pieces: Vec::new(),
            body: summary,
        },
        changes,
        svgs,
    }
}

/// A commit's id, short; a name (`HEAD`, a branch) as it is.
fn short(rev: &str) -> &str {
    if rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit()) {
        &rev[..12]
    } else {
        rev
    }
}

/// A comment already on the pull request, as GitHub lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Existing {
    pub id: u64,
    pub body: String,
    pub login: String,
}

/***
 * The pull request's comments from GitHub's JSON (`gh api …/issues/<n>/comments`): an array, the arrays of `--paginate` one after the other, or one comment per line.
 ***/
pub fn existing_from(text: &str) -> Vec<Existing> {
    let mut out = Vec::new();
    let one = |v: &Value, out: &mut Vec<Existing>| {
        if let (Some(id), Some(body)) = (v["id"].as_u64(), v["body"].as_str()) {
            out.push(Existing {
                id,
                body: body.to_string(),
                login: v["user"]["login"].as_str().unwrap_or("").to_string(),
            });
        }
    };
    for v in serde_json::Deserializer::from_str(text)
        .into_iter::<Value>()
        .map_while(Result::ok)
    {
        match &v {
            Value::Array(items) => items.iter().for_each(|c| one(c, &mut out)),
            _ => one(&v, &mut out),
        }
    }
    out
}

/// What to do with one comment: `create` (no id), `update` or `keep` an existing one, or `gone`:
/// update an existing one whose change is no longer in the pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub action: &'static str,
    pub id: Option<u64>,
    /// The comment's new text; empty for `keep`.
    pub body: String,
}

impl Action {
    pub fn to_json(&self) -> Value {
        json!({ "action": self.action, "id": self.id, "body": self.body })
    }
}

/// A comment's text with the page's address put in, or, without one, its lines that link the page dropped.
pub fn with_page(body: &str, page: Option<&str>) -> String {
    match page {
        Some(url) => body.replace(PAGE, url),
        None => body
            .lines()
            .filter(|l| !l.contains(PAGE))
            .map(|l| format!("{l}\n"))
            .collect(),
    }
}

/// A gone change's comment: its title struck, its old text folded, its mark kept so it can come back.
fn gone_body(old: &str, pieces: &[String]) -> String {
    let title = old
        .lines()
        .find_map(|l| l.strip_prefix("### "))
        .map(|t| t.split_once(" · ").map_or(t, |(_, t)| t))
        .unwrap_or("A change");
    let rest: String = old
        .lines()
        .filter(|l| !l.starts_with(MARK))
        .map(|l| format!("{l}\n"))
        .collect();
    format!(
        "{}\n~~{title}~~ · no longer in this pull request.\n\n<details><summary>What it said</summary>\n\n{rest}\n</details>\n",
        mark("gone", pieces)
    )
}

/***
 * Bring a pull request's comments up to date (docs/PULL_REQUESTS.md §4): the summary is updated, or written the first time; each change goes to the comment of docair's that shares the most of its pieces, at least half of the larger of the two, once each, and is updated there (kept when the text is the same) or written new; a change comment no change took is cut to one line, its old text folded (`gone`). Only comments with docair's mark count, and with `author` only that login's.
 ***/
pub fn plan(
    new: &[PrComment],
    existing: &[Existing],
    page: Option<&str>,
    author: Option<&str>,
) -> Vec<Action> {
    let ours: Vec<(&Existing, String, Vec<String>)> = existing
        .iter()
        .filter(|e| author.is_none_or(|a| e.login == a))
        .filter_map(|e| read_mark(&e.body).map(|(k, p)| (e, k, p)))
        .collect();
    let mut out = Vec::new();
    let write = |c: &PrComment, old: Option<&Existing>, out: &mut Vec<Action>| {
        let body = with_page(&c.body, page);
        out.push(match old {
            Some(e) if e.body.trim_end() == body.trim_end() => Action {
                action: "keep",
                id: Some(e.id),
                body: String::new(),
            },
            Some(e) => Action {
                action: "update",
                id: Some(e.id),
                body,
            },
            None => Action {
                action: "create",
                id: None,
                body,
            },
        });
    };
    for c in new.iter().filter(|c| c.kind == "summary") {
        let old = ours
            .iter()
            .find(|(_, k, _)| k == "summary")
            .map(|(e, _, _)| *e);
        write(c, old, &mut out);
    }
    // Each change to the old comment that shares the most of its pieces, best pairs first.
    let changes: Vec<&PrComment> = new.iter().filter(|c| c.kind == "change").collect();
    let olds: Vec<usize> = (0..ours.len())
        .filter(|&j| ours[j].1 == "change" || ours[j].1 == "gone")
        .collect();
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (i, c) in changes.iter().enumerate() {
        let mine: HashSet<&str> = c.pieces.iter().map(String::as_str).collect();
        for &j in &olds {
            let theirs: HashSet<&str> = ours[j].2.iter().map(String::as_str).collect();
            let shared = mine.intersection(&theirs).count();
            let larger = mine.len().max(theirs.len());
            if larger > 0 && shared * 2 >= larger && shared > 0 {
                pairs.push((shared as f64 / larger as f64, i, j));
            }
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut took: Vec<Option<usize>> = vec![None; changes.len()];
    let mut taken: HashSet<usize> = HashSet::new();
    for (_, i, j) in pairs {
        if took[i].is_none() && !taken.contains(&j) {
            took[i] = Some(j);
            taken.insert(j);
        }
    }
    for (i, c) in changes.iter().enumerate() {
        write(c, took[i].map(|j| ours[j].0), &mut out);
    }
    for &j in &olds {
        let (e, kind, pieces) = &ours[j];
        if taken.contains(&j) || kind == "gone" {
            continue;
        }
        out.push(Action {
            action: "gone",
            id: Some(e.id),
            body: gone_body(&e.body, pieces),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use docair_core::changes::Changes;
    use docair_core::diff::{ChangedItem, GraphDiff};

    /// The shop before it wrote the order, and the change as `changes` finds it (as docair-core's
    /// review tests make it).
    fn a_new_write() -> (Graph, Graph, Changes) {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../docair-adapters/tests/golden/shop.graph.json"
        );
        let after: Graph = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut before = after.clone();
        let at = before
            .edges
            .iter()
            .position(|e| e.from == "orders.create" && e.to == "orders-db")
            .unwrap();
        let line = before.edges.remove(at).id;
        let graph = GraphDiff {
            lines_added: vec![line],
            changed: vec![ChangedItem {
                id: "orders.create".into(),
                what: vec!["tags".into()],
            }],
            ..GraphDiff::default()
        };
        let changes = docair_core::changes::changes(&before, &after, graph, &[], ("A", "B"));
        (before, after, changes)
    }

    fn links() -> CodeLinks {
        CodeLinks {
            at_a: Some("https://github.com/o/r/blob/aaa/{path}#L{line}".into()),
            at_b: Some("https://github.com/o/r/blob/bbb/{path}#L{line}".into()),
            rev_a: Some("aaa".into()),
            rev_b: Some("bbb".into()),
        }
    }

    #[test]
    fn without_the_ai_each_piece_is_a_change_in_docairs_words_with_its_picture() {
        let (before, after, changes) = a_new_write();
        let links = links();
        let changed = Changed {
            before: &before,
            changes: &changes,
            findings: None,
            links: &links,
        };
        let r = review(&after, &changed, Some("no AI key in this run"));
        assert!(!r.changes.is_empty());
        let first = &r.changes[0];
        assert_eq!(
            read_mark(&first.body).map(|m| m.1),
            Some(first.pieces.clone())
        );
        assert!(
            first
                .body
                .contains("In docair's words: the AI did not review"),
            "{}",
            first.body
        );
        assert!(
            first.body.contains("```text\n"),
            "the cut drawn as text:\n{}",
            first.body
        );
        assert!(first.body.contains("orders-db"), "{}", first.body);
        assert!(
            first.body.contains('═') || first.body.contains('║'),
            "the new line is double:\n{}",
            first.body
        );
        assert!(first.body.contains(&format!("({PAGE}#review/1)")));
        assert!(
            first.body.contains("https://github.com/o/r/blob/bbb/"),
            "parts link to the code at B"
        );
        assert!(r.svgs[0].starts_with("<?xml") && r.svgs[0].contains("ch-added"));
        assert!(
            r.summary
                .body
                .contains("No review by the AI:** no AI key in this run")
        );
        assert!(r.summary.body.contains("1. **"));
    }

    #[test]
    fn the_ais_change_brings_its_title_why_what_is_off_and_what_to_check() {
        let (before, after, changes) = a_new_write();
        let pieces = docair_core::review::pieces(&before, &after, &changes);
        let how: Vec<Value> = pieces
            .iter()
            .map(|p| json!({ "piece": p.id(), "words": "the order is written" }))
            .collect();
        let findings = json!({
            "schema": "docair.explained/2", "a": "A", "b": "B",
            "changes": [{
                "title": "Orders are stored",
                "how": how,
                "why": "so an order survives a restart",
                "off": [{ "kind": "contradicts", "source": "commit", "where": "Store orders",
                          "says": "every order is audited", "code": "nothing audits", "parts": ["orders.create"] }],
                "check": "that the write is awaited",
                "recorded": { "by": "model" }
            }]
        });
        let links = links();
        let changed = Changed {
            before: &before,
            changes: &changes,
            findings: Some(&findings),
            links: &links,
        };
        let r = review(&after, &changed, None);
        assert_eq!(r.changes.len(), 1, "one change holds every piece");
        let b = &r.changes[0].body;
        if std::env::var_os("DOCAIR_SHOW").is_some() {
            println!("{}\n----\n{b}", r.summary.body);
        }
        assert!(b.contains("### 1 of 1 · Orders are stored"), "{b}");
        assert!(b.contains("**Why:** so an order survives a restart"));
        assert!(
            b.contains("**Said:** the commit \u{201c}Store orders\u{201d}: every order is audited"),
            "{b}"
        );
        assert!(
            b.contains("[`orders.create`](https://github.com/o/r/blob/bbb/"),
            "{b}"
        );
        assert!(b.contains("**Check:** that the write is awaited"));
        assert!(b.contains("1. the order is written"));
        assert!(r.summary.body.contains("1 change, each in a comment"));
        assert!(!r.summary.body.contains("No review by the AI"));
    }

    fn change(pieces: &[&str], body: &str) -> PrComment {
        let pieces: Vec<String> = pieces.iter().map(|p| p.to_string()).collect();
        PrComment {
            kind: "change".into(),
            body: format!(
                "{}\n### 1 of 1 · {body}\n\n[walk]({PAGE}#review/1)\n",
                mark("change", &pieces)
            ),
            pieces,
        }
    }

    fn existing(id: u64, c: &PrComment) -> Existing {
        Existing {
            id,
            body: with_page(&c.body, Some("https://x")),
            login: "github-actions[bot]".into(),
        }
    }

    #[test]
    fn a_mark_reads_back_with_its_pieces_and_cannot_end_early() {
        let pieces = vec!["line:a-->b".to_string(), "code:shop".to_string()];
        let m = mark("change", &pieces);
        assert_eq!(m.matches("-->").count(), 1, "{m}");
        assert_eq!(
            read_mark(&format!("{m}\n### x")),
            Some(("change".into(), pieces))
        );
        assert_eq!(read_mark("a comment of someone else"), None);
    }

    #[test]
    fn without_a_page_its_links_go() {
        let c = change(&["p"], "title");
        assert!(!with_page(&c.body, None).contains("walk"));
        assert!(with_page(&c.body, Some("https://x")).contains("(https://x#review/1)"));
    }

    #[test]
    fn a_push_updates_the_comments_by_their_pieces() {
        let summary = PrComment {
            kind: "summary".into(),
            pieces: vec![],
            body: format!("{}\n### docair: the review\n", mark("summary", &[])),
        };
        let a = change(&["p1", "p2", "p3"], "Audits");
        let b = change(&["q1"], "Prices");
        let gone = change(&["r1"], "Old");
        let mut old = vec![
            existing(1, &summary),
            existing(2, &a),
            existing(3, &b),
            existing(4, &gone),
        ];
        old.push(Existing {
            id: 5,
            body: "LGTM".into(),
            login: "someone".into(),
        });
        // Next push: Audits lost a piece and gained one, Prices is the same, Old is gone, New is new.
        let a2 = change(&["p1", "p2", "p4"], "Audits, again");
        let new_one = change(&["s1"], "New");
        let plan = plan(
            &[summary.clone(), b.clone(), a2.clone(), new_one],
            &old,
            Some("https://x"),
            Some("github-actions[bot]"),
        );
        let got: Vec<(&str, Option<u64>)> = plan.iter().map(|a| (a.action, a.id)).collect();
        assert_eq!(
            got,
            [
                ("keep", Some(1)),
                ("keep", Some(3)),
                ("update", Some(2)),
                ("create", None),
                ("gone", Some(4)),
            ]
        );
        assert!(plan[2].body.contains("Audits, again"));
        let g = &plan[4].body;
        assert!(
            g.contains("~~Old~~ · no longer in this pull request"),
            "{g}"
        );
        assert_eq!(read_mark(g).map(|m| m.0), Some("gone".into()));
        // A gone comment whose change comes back is taken again.
        let back = plan_again(&old, g, &gone);
        assert_eq!(back, ("update", Some(4)));
    }

    fn plan_again(
        old: &[Existing],
        gone_body: &str,
        comes_back: &PrComment,
    ) -> (&'static str, Option<u64>) {
        let mut old = old.to_vec();
        old.retain(|e| e.id == 4);
        old[0].body = gone_body.to_string();
        let p = plan(
            std::slice::from_ref(comes_back),
            &old,
            Some("https://x"),
            None,
        );
        (p[0].action, p[0].id)
    }

    #[test]
    fn half_of_the_pieces_or_more_make_a_match() {
        let old = change(&["a", "b", "c", "d"], "Old");
        let half = change(&["a", "b", "x", "y"], "Half");
        let less = change(&["a", "x", "y", "z"], "Less");
        let p = plan(&[half], &[existing(9, &old)], None, None);
        assert_eq!(p[0].action, "update");
        let p = plan(&[less], &[existing(9, &old)], None, None);
        assert_eq!((p[0].action, p[1].action), ("create", "gone"));
    }

    #[test]
    fn github_lists_comments_as_one_array_or_pages_of_them() {
        let one = r#"[{"id":1,"body":"a","user":{"login":"x"}},{"id":2,"body":"b","user":{"login":"y"}}]"#;
        let pages = r#"[{"id":1,"body":"a","user":{"login":"x"}}][{"id":2,"body":"b","user":{"login":"y"}}]"#;
        let lines = "{\"id\":1,\"body\":\"a\",\"user\":{\"login\":\"x\"}}\n{\"id\":2,\"body\":\"b\",\"user\":{\"login\":\"y\"}}\n";
        for t in [one, pages, lines] {
            let got: Vec<u64> = existing_from(t).iter().map(|e| e.id).collect();
            assert_eq!(got, [1, 2], "{t}");
        }
    }
}

//! What changed between two versions, in the page (docs/CHANGES.md §5, §7 and §8, docair-zrr.7,
//! zrr.18): the header's button with the count of changed flows, the list it opens, and for each
//! flow one picture of its slice before and after, drawn like any slice: a part both versions have
//! once and faded, what only the flow before had red, what only the flow after has green, a part
//! changed inside amber.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::Path;

use docair_core::change_steps::{
    self, ChangeStep, FlowMarks, LineMark, Merged, PartMark, StepKind, StepLine,
};
use docair_core::changes::FlowChange;
use docair_core::model::Graph;
use docair_layout::{Input, Layout, Monospace};
use serde_json::{Map, Value, json};

use crate::doc_page::{Changed, slice_title};
use crate::page_panels::href;
use crate::svg::ChangeMarks;
use crate::{escape_text, layout_input, svg};

/// The changes as the page shows them.
pub(crate) struct ChangeView {
    /// The header's button: `Δ Changes · n`.
    pub button: String,
    /// The list the button opens.
    pub list: String,
    /// Each changed flow's details, by its seed: shown with its picture.
    pub details: Map<String, Value>,
    /// Each changed flow's picture.
    pub pictures: Vec<ChangePicture>,
}

/// A picture of the changes section.
pub(crate) struct ChangePicture {
    pub addr: String,
    pub title: String,
    pub crumbs: Vec<(String, String)>,
    pub svg: String,
    /// The changed flow's seed, whose details show with the picture.
    pub change: Option<String>,
    /// The walk over its changes (docs/CHANGES.md §8), one step per change.
    pub walk: Option<Value>,
}

/// The model's words, escaped, each `code` span a code element (the linker links those that name a part).
fn with_code(text: &str) -> String {
    let escaped = escape_text(text);
    if escaped.matches('`').count() % 2 == 1 {
        return escaped;
    }
    escaped
        .split('`')
        .enumerate()
        .map(|(i, s)| {
            if i % 2 == 1 {
                format!("<code>{s}</code>")
            } else {
                s.to_string()
            }
        })
        .collect()
}

/// Who recorded a finding, and when: `by fake-model · 2026-11-19`; docair's own words say so. Every
/// review is the AI's: there are no hand corrections (docair-8l6.75).
fn recorded(f: &Value) -> String {
    let r = &f["recorded"];
    let at = r["at"].as_str().unwrap_or("");
    if r["by"].as_str() == Some("docair") {
        return format!("in docair's words: {}", docair_note(f));
    }
    format!("by {} · {at}", r["model"].as_str().unwrap_or("the model"))
}

/// A box around some parts of a laid-out picture, with room around it: where the camera of a step goes.
fn camera_of(laid: &Layout, parts: &[usize]) -> Value {
    let (mut x0, mut y0, mut x1, mut y1) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for r in parts.iter().filter_map(|&i| laid.placement.parts.get(i)) {
        x0 = x0.min(r.x - r.w / 2.0);
        y0 = y0.min(r.y - r.h / 2.0);
        x1 = x1.max(r.x + r.w / 2.0);
        y1 = y1.max(r.y + r.h / 2.0);
    }
    if x0 <= x1 {
        json!([x0 - 60.0, y0 - 60.0, x1 - x0 + 120.0, y1 - y0 + 120.0])
    } else {
        Value::Null
    }
}

/// One step's card of a walk on a change's picture: its title, where it is, what it says, Back and Next.
fn step_card(glyph: &str, title: &str, at: &str, body: &str, nav: &str) -> String {
    format!(
        r#"<section class="card flow" data-test="step-card"><div class="card-head"><span class="card-glyph">{glyph}</span><b>{}</b><button class="card-close" title="Close the walk">×</button></div><div class="card-step">{at}</div>{body}<div class="card-nav">{nav}</div></section>"#,
        escape_text(title)
    )
}

/// Back and Next of step `k` of `n`, as the walk goes through them.
fn walk_nav(k: usize, n: usize) -> String {
    format!(
        r#"<button class="btn" data-step="-1"{}>◀ Back</button><button class="btn" data-step="1"{}>Next ▶</button>"#,
        if k == 0 { " disabled" } else { "" },
        if k + 1 >= n { " disabled" } else { "" }
    )
}

/// Back and Next between the pictures of a big flow's changes, each on its own (docair-zrr.27): links to their addresses.
fn picture_nav(prev: Option<&str>, next: Option<&str>) -> String {
    let link = |to: Option<&str>, words: &str, test: &str| match to {
        Some(a) => format!(
            r#"<a class="btn" href="{}" data-test="{test}">{words}</a>"#,
            href(a)
        ),
        None => format!(r#"<button class="btn" disabled>{words}</button>"#),
    };
    format!(
        "{}{}",
        link(prev, "◀ Back", "change-back"),
        link(next, "Next ▶", "change-next")
    )
}

/// Where a step's picture stands among the pictures of a big flow's changes: its place, how many, and the addresses before and after it.
struct StepAt {
    k: usize,
    n: usize,
    prev: Option<String>,
    next: Option<String>,
}

/// Names as code, which the page links to their parts: `a`, `b` and `c`; past eight, how many more.
fn names_as_code(graph: &Graph, ids: &[&str]) -> String {
    let mut named: Vec<String> = ids
        .iter()
        .take(MAX_NAMED)
        .map(|id| format!("<code>{}</code>", escape_text(&name_in(graph, id))))
        .collect();
    if ids.len() > MAX_NAMED {
        named.push(format!("{} more", ids.len() - MAX_NAMED));
    }
    match named.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A step's card names at most this many parts, and a step holds at most this many new or gone parts.
const MAX_NAMED: usize = 8;

/// Where the changed parts are, at A and at B, and how their files open (docair-8l6.82): the review
/// links each part to its file and line instead of embedding its code.
pub(crate) struct CodeRefs<'c> {
    pub before: &'c Graph,
    pub after: &'c Graph,
    pub links: &'c crate::doc_page::CodeLinks,
}

/***
 * Where a step's parts are in the code (docair-8l6.49, 8l6.82): each of its new, gone and edited parts by id (a link to the part), and a link to its file and line, at B for a new one, at A for a gone one, before and after for an edited one; the file and line alone where a version has no link. Empty when none of them has a place in a file.
 ***/
fn piece_code(s: &ChangeStep, refs: &CodeRefs) -> String {
    let ids: Vec<(&str, &str)> = s
        .added
        .iter()
        .map(|i| (i.as_str(), "new"))
        .chain(s.removed.iter().map(|i| (i.as_str(), "gone")))
        .chain(s.code.iter().map(|i| (i.as_str(), "edited")))
        .collect();
    parts_code(ids, refs)
}

/// Where parts are in the code, each with what happened to it (`new`, `gone`, `edited`), as `piece_code` says it.
fn parts_code(mut ids: Vec<(&str, &str)>, refs: &CodeRefs) -> String {
    use crate::doc_page::CodeLinks;
    let mut seen = HashSet::new();
    ids.retain(|(i, _)| seen.insert(*i));
    let place = |g: &Graph, id: &str| {
        g.nodes
            .iter()
            .find(|x| x.id == id)
            .and_then(|x| x.source.as_ref())
            .map(|src| (src.file.clone(), src.line))
    };
    // A version's place as a link, labelled; the file and line alone without a link.
    let link = |label: &str, template: Option<&String>, at: &(String, usize)| match CodeLinks::to(
        template, &at.0, at.1,
    ) {
        Some(href) => format!(
            r#"<a class="link" href="{}" target="_blank" rel="noopener" data-test="code-link" title="{}:{}">{label}</a>"#,
            escape_text(&href),
            escape_text(&at.0),
            at.1
        ),
        None => format!("<code>{}:{}</code>", escape_text(&at.0), at.1),
    };
    // Where the part is at each version, for the page in the editor (docs/EDITOR.md §5).
    let side = |rev: Option<&String>, p: &(String, usize)| {
        crate::doc_page::place(rev.map(String::as_str), &p.0, p.1)
    };
    let (rev_a, rev_b) = (refs.links.rev_a.as_ref(), refs.links.rev_b.as_ref());
    let mut items = String::new();
    let mut n = 0;
    for (id, what) in ids {
        let (a, b) = (place(refs.before, id), place(refs.after, id));
        let at = |p: &(String, usize)| format!("{}:{}", p.0, p.1);
        let open = match (what, &a, &b) {
            ("new", _, Some(b)) => crate::doc_page::data_open(None, Some(side(rev_b, b))),
            ("gone", Some(a), _) => crate::doc_page::data_open(Some(side(rev_a, a)), None),
            (_, Some(a), Some(b)) => {
                crate::doc_page::data_open(Some(side(rev_a, a)), Some(side(rev_b, b)))
            }
            (_, _, Some(b)) => crate::doc_page::data_open(None, Some(side(rev_b, b))),
            (_, Some(a), None) => crate::doc_page::data_open(Some(side(rev_a, a)), None),
            _ => String::new(),
        };
        let links = match (what, &a, &b) {
            ("new", _, Some(b)) => link(&at(b), refs.links.at_b.as_ref(), b),
            ("gone", Some(a), _) => link(&at(a), refs.links.at_a.as_ref(), a),
            (_, Some(a), Some(b)) => format!(
                "{} · {}",
                link("before", refs.links.at_a.as_ref(), a),
                link(&format!("after, {}", at(b)), refs.links.at_b.as_ref(), b)
            ),
            (_, _, Some(b)) => link(&at(b), refs.links.at_b.as_ref(), b),
            (_, Some(a), None) => link(&at(a), refs.links.at_a.as_ref(), a),
            _ => continue,
        };
        let _ = write!(
            items,
            r#"<li data-test="code-part"{open}><code>{}</code> <span class="muted">{what}</span> · {links}</li>"#,
            escape_text(id)
        );
        n += 1;
    }
    if n == 0 {
        return String::new();
    }
    format!(
        r#"<div class="piece-code" data-test="piece-code"><span class="muted">The code · {}:</span><ul class="code-links">{items}</ul></div>"#,
        count(n, "part")
    )
}

/***
 * A walk over a flow's changes only (docs/CHANGES.md §8, docair-zrr.19, 8l6.12): one step per change as docair-core works them out, the path's steps first, each kind in reading order; each step's camera frames its change at a size to read, and its card says what changed: the model's words on that step, by its id, over docair's own.
 *
 * @calls docair-core.change_steps.steps "the flow's changes in steps"
 ***/
fn changes_walk(
    m: &Merged,
    laid: &Layout,
    flow_steps: &[ChangeStep],
    (title, words, intro): (&str, Option<&Value>, Option<&str>),
    at_picture: Option<&StepAt>,
    linker: &crate::page_glossary::Linker,
    refs: &CodeRefs,
) -> Option<Value> {
    // A review's change (docair-8l6.43): its pieces in the order given, after a first card on what and why.
    let in_order = intro.is_some();
    if flow_steps.is_empty() {
        return None;
    }
    let at: HashMap<&str, usize> = m
        .pic
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.as_str(), i))
        .collect();
    let mut line_at: HashMap<&str, usize> = HashMap::new();
    for (k, l) in m.pic.lines.iter().enumerate() {
        for e in l.members.iter().filter_map(|&e| m.graph.edges.get(e)) {
            line_at.entry(e.id.as_str()).or_insert(k);
        }
    }
    let places = |s: &ChangeStep| -> Vec<usize> {
        let mut v: Vec<usize> = s
            .parts()
            .filter_map(|p| at.get(p.as_str()).copied())
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let top_left = |parts: &[usize]| {
        parts
            .iter()
            .filter_map(|&i| laid.placement.parts.get(i))
            .map(|r| ((r.y - r.h / 2.0) as i64, (r.x - r.w / 2.0) as i64))
            .min()
            .unwrap_or_default()
    };
    let mut order: Vec<&ChangeStep> = flow_steps.iter().collect();
    if !in_order {
        order.sort_by_key(|s| {
            (
                matches!(s.kind, StepKind::Code | StepKind::Box),
                top_left(&places(s)),
            )
        });
    }
    let count = order.len();
    let first = usize::from(intro.is_some());
    let total = count + first;
    let head = if in_order {
        docair_core::review::cut(&title.replace('`', ""), 70)
    } else {
        "The changes".to_string()
    };
    let mut steps: Vec<Value> = Vec::new();
    if let Some(intro) = intro {
        // What was done and why, framed on the whole change, before its pieces.
        let mut all: Vec<usize> = order.iter().flat_map(|s| places(s)).collect();
        all.sort_unstable();
        all.dedup();
        let mut lines: Vec<usize> = order
            .iter()
            .flat_map(|s| {
                s.lines
                    .iter()
                    .filter_map(|l| line_at.get(l.id.as_str()).copied())
            })
            .collect();
        lines.sort_unstable();
        lines.dedup();
        // The full title heads it; the pieces' cards carry it cut (docair-8l6.54).
        let card = step_card(
            "Δ",
            &title.replace('`', ""),
            "What and why",
            intro,
            &walk_nav(0, total),
        );
        steps.push(json!({
            "id": "what-and-why",
            "card": linker.link(&card, None),
            "parts": all.iter().map(|&i| m.pic.parts[i].id.clone()).collect::<Vec<_>>(),
            "lines": lines,
            "camera": camera_of(laid, &all),
            "tokens": [],
        }));
    }
    let pieces: Vec<Value> = order
        .iter()
        .enumerate()
        .map(|(k, s)| {
            let parts = places(s);
            let mut lines: Vec<usize> = s
                .lines
                .iter()
                .filter_map(|l| line_at.get(l.id.as_str()).copied())
                .collect();
            lines.sort_unstable();
            lines.dedup();
            let said = docair_words(&m.graph, s);
            let route = if matches!(s.kind, StepKind::Path | StepKind::Line) {
                before_after(&m.graph, s)
            } else {
                String::new()
            };
            // Down to the code: where its parts are, as links (docair-8l6.49, 8l6.82).
            let code = piece_code(s, refs);
            let body = match words.and_then(|w| w.get(&s.id)).and_then(Value::as_str) {
                Some(w) => format!(
                    r#"<p data-test="step-words">{}</p>{route}<p class="muted">{said}</p>{code}"#,
                    with_code(w)
                ),
                None => format!("<p>{said}</p>{route}{code}"),
            };
            // A review's change is walked through its pieces, the how; a flow through its changes.
            let unit = if in_order { "How ·" } else { "change" };
            let (at, nav) = match at_picture {
                Some(p) => (
                    format!("{unit} {} of {}", p.k + 1, p.n),
                    picture_nav(p.prev.as_deref(), p.next.as_deref()),
                ),
                None => (
                    format!("{unit} {} of {count}", k + 1),
                    walk_nav(k + first, total),
                ),
            };
            let card = step_card("Δ", &head, &at, &body, &nav);
            json!({
                "id": s.id,
                "card": linker.link(&card, None),
                "parts": parts.iter().map(|&i| m.pic.parts[i].id.clone()).collect::<Vec<_>>(),
                "lines": lines,
                "camera": camera_of(laid, &parts),
                "tokens": [],
            })
        })
        .collect();
    steps.extend(pieces);
    Some(json!({ "title": format!("The changes · {title}"), "glyph": "Δ", "steps": steps }))
}

/// The routes some of a step's lines make, as chains of parts: from each part none of them reaches, along them to where they end; at most four.
fn routes<'s>(lines: &[&'s StepLine]) -> Vec<Vec<&'s str>> {
    fn walk<'s>(lines: &[&'s StepLine], path: &mut Vec<&'s str>, out: &mut Vec<Vec<&'s str>>) {
        let at = path[path.len() - 1];
        let next: Vec<&'s str> = lines
            .iter()
            .filter(|l| l.from == at)
            .map(|l| l.to.as_str())
            .collect();
        if next.is_empty() || path.len() > 8 || out.len() >= 4 {
            out.push(path.clone());
            return;
        }
        for n in next {
            path.push(n);
            if path[..path.len() - 1].contains(&n) {
                out.push(path.clone());
            } else {
                walk(lines, path, out);
            }
            path.pop();
        }
    }
    let reached: HashSet<&str> = lines.iter().map(|l| l.to.as_str()).collect();
    let mut starts: Vec<&str> = Vec::new();
    for l in lines {
        if !reached.contains(l.from.as_str()) && !starts.contains(&l.from.as_str()) {
            starts.push(&l.from);
        }
    }
    if starts.is_empty()
        && let Some(l) = lines.first()
    {
        starts.push(&l.from);
    }
    let mut out = Vec::new();
    for s in starts {
        walk(lines, &mut vec![s], &mut out);
    }
    out.truncate(4);
    out
}

/// A step's route before and after, as chains of parts (docair-zrr.26): *Before · `a` → `b`*, *After · `a` → `c` → `b`*; a part without lines on its own, *—* for nothing.
fn before_after(graph: &Graph, s: &ChangeStep) -> String {
    let of = |ids: &[String]| -> Vec<&StepLine> {
        s.lines.iter().filter(|l| ids.contains(&l.id)).collect()
    };
    let chain = |r: &[&str]| -> String {
        r.iter()
            .map(|id| names_as_code(graph, &[id]))
            .collect::<Vec<_>>()
            .join(" → ")
    };
    let show = |rs: Vec<Vec<&str>>, parts: &[String]| -> String {
        if !rs.is_empty() {
            rs.iter().map(|r| chain(r)).collect::<Vec<_>>().join("<br>")
        } else if !parts.is_empty() {
            let ids: Vec<&str> = parts.iter().map(String::as_str).collect();
            names_as_code(graph, &ids)
        } else {
            "—".into()
        }
    };
    format!(
        r#"<p class="ch-route ch-before" data-test="route-before"><b>Before</b> · {}</p><p class="ch-route ch-after" data-test="route-after"><b>After</b> · {}</p>"#,
        show(routes(&of(&s.lines_removed)), &s.removed),
        show(routes(&of(&s.lines_added)), &s.added)
    )
}

/// What a step of a change is about, in docair's own words: what the flow now and no longer goes through, the line, or the code that changed.
fn docair_words(graph: &Graph, s: &ChangeStep) -> String {
    fn refs(v: &[String]) -> Vec<&str> {
        v.iter().map(String::as_str).collect()
    }
    let mut said = match s.kind {
        StepKind::Path => {
            let mut parts = Vec::new();
            if !s.added.is_empty() {
                parts.push(format!(
                    "Now goes through {}",
                    names_as_code(graph, &refs(&s.added))
                ));
            }
            if !s.removed.is_empty() {
                parts.push(format!(
                    "no longer through {}",
                    names_as_code(graph, &refs(&s.removed))
                ));
            }
            parts.join("; ")
        }
        StepKind::Line => {
            let line = |ids: &[String]| -> Vec<String> {
                s.lines
                    .iter()
                    .filter(|l| ids.contains(&l.id))
                    .map(|l| {
                        format!(
                            "{} → {}",
                            names_as_code(graph, &[&l.from]),
                            names_as_code(graph, &[&l.to])
                        )
                    })
                    .collect()
            };
            let (added, removed) = (line(&s.lines_added), line(&s.lines_removed));
            let mut parts = Vec::new();
            if !added.is_empty() {
                parts.push(format!("A new line {}", added.join(", ")));
            }
            if !removed.is_empty() {
                parts.push(format!("no more line {}", removed.join(", ")));
            }
            parts.join("; ")
        }
        StepKind::Code => format!("Code changed in {}", names_as_code(graph, &refs(&s.code))),
        StepKind::Box => format!(
            "{} itself changed, not the parts drawn in it",
            names_as_code(graph, &refs(&s.code))
        ),
    };
    if matches!(s.kind, StepKind::Path | StepKind::Line) && !s.ends.is_empty() {
        let _ = write!(said, ", at {}", names_as_code(graph, &refs(&s.ends)));
    }
    if let Some(b) = s
        .in_box
        .as_deref()
        .filter(|b| matches!(s.kind, StepKind::Code) && !s.code.iter().any(|c| c == b))
    {
        let _ = write!(said, " (in {})", names_as_code(graph, &[b]));
    }
    said.push('.');
    said
}

/// A commit as people read it: a hash cut to 8 characters, a name as it is.
fn short(c: &str) -> &str {
    if c.len() > 12 && c.chars().all(|ch| ch.is_ascii_hexdigit()) {
        &c[..8]
    } else {
        c
    }
}

/// A part's name in a version, or its id.
fn name_in(graph: &Graph, id: &str) -> String {
    graph
        .nodes
        .iter()
        .find(|n| n.id == id)
        .map_or(id.to_string(), |n| n.name.clone())
}

/// How a flow changed, for the list's order and the details' words: its path, only its parts' code, or neither.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FlowKind {
    Path,
    Code,
    Words,
}

/// The two ends of a graph line, by its id: `<from> -<kind>-> <to>`, with ` "<label>"` or ` #2` after it (docair-svh).
fn line_ends(id: &str) -> Option<(&str, &str)> {
    let (from, rest) = id.split_once(" -")?;
    let (_, to) = rest.split_once("-> ")?;
    let to = to.split(" \"").next()?.split(" #").next()?;
    Some((from, to))
}

/// What a flow's change holds: how it changed, its parts changed in their code, those whose words alone changed.
struct FlowSort<'f> {
    kind: FlowKind,
    code: Vec<&'f String>,
    words: Vec<&'f String>,
}

fn sort_flow<'f>(f: &'f FlowChange, inside: &change_steps::ChangedInside) -> FlowSort<'f> {
    let on_path = |p: &String| !f.parts_added.contains(p) && !f.parts_removed.contains(p);
    let code: Vec<&String> = f
        .through
        .iter()
        .filter(|p| on_path(p) && inside.code.contains(p.as_str()))
        .collect();
    // A part whose tags changed what it calls is drawn by its lines, not listed as words.
    let touched: HashSet<&str> = f
        .lines_added
        .iter()
        .chain(&f.lines_removed)
        .filter_map(|l| line_ends(l))
        .flat_map(|(a, b)| [a, b])
        .collect();
    let words: Vec<&String> = f
        .through
        .iter()
        .filter(|p| {
            on_path(p) && inside.words.contains(p.as_str()) && !touched.contains(p.as_str())
        })
        .collect();
    let path = f.how != "changed"
        || !(f.parts_added.is_empty()
            && f.parts_removed.is_empty()
            && f.lines_added.is_empty()
            && f.lines_removed.is_empty());
    let kind = if path {
        FlowKind::Path
    } else if !code.is_empty() {
        FlowKind::Code
    } else {
        FlowKind::Words
    };
    FlowSort { kind, code, words }
}

/// A flow's marks as the drawing takes them: a class for each part and graph line, a tag on each part a change is about.
struct Classes {
    parts: HashMap<String, &'static str>,
    lines: HashMap<String, &'static str>,
    tags: HashMap<String, &'static str>,
}

/// The page's classes and tags for what docair-core marked (docs/CHANGES.md §8): `ch-added` *new*, `ch-removed` *gone*, `ch-changed` *code*, `ch-same`.
fn classes(marks: &FlowMarks) -> Classes {
    let mut c = Classes {
        parts: HashMap::new(),
        lines: HashMap::new(),
        tags: HashMap::new(),
    };
    for (id, mark) in &marks.parts {
        let (class, tag) = match mark {
            PartMark::Added => ("ch-added", Some("new")),
            PartMark::Removed => ("ch-removed", Some("gone")),
            PartMark::Code => ("ch-changed", Some("code")),
            PartMark::Same => ("ch-same", None),
        };
        c.parts.insert(id.clone(), class);
        if let Some(t) = tag {
            c.tags.insert(id.clone(), t);
        }
    }
    for (id, mark) in &marks.lines {
        let class = match mark {
            LineMark::Added => "ch-added",
            LineMark::Removed => "ch-removed",
            LineMark::Same => "ch-same",
        };
        c.lines.insert(id.clone(), class);
    }
    c
}

/// Lay out a picture's input, or take its kept layout.
fn lay_out_kept(input: &Input, options: &docair_layout::Options, keep: Option<&Path>) -> Layout {
    let path = keep.map(|dir| dir.join(format!("{}.json", docair_layout::key(input, options))));
    if let Some(laid) = path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Layout>(&t).ok())
    {
        return laid;
    }
    let laid = docair_layout::lay_out(input, options);
    if let Some(p) = &path
        && let Some(dir) = p.parent()
        && std::fs::create_dir_all(dir).is_ok()
        && let Ok(text) = serde_json::to_string(&laid)
    {
        let _ = std::fs::write(p, text);
    }
    laid
}

/// A flow's one picture, laid out (or its kept layout taken) and drawn with its marks, in the page's class for it.
fn draw_one(m: &Merged, marks: &FlowMarks, keep: Option<&Path>) -> (Layout, String) {
    let input = layout_input::input_with(&m.graph, &m.pic, &Monospace::default(), &HashMap::new());
    let laid = lay_out_kept(&input, &layout_input::options_for(&m.pic), keep);
    let c = classes(marks);
    let drawn = svg::draw_marked(
        &m.graph,
        &m.pic,
        &laid,
        &ChangeMarks {
            parts: &c.parts,
            lines: &c.lines,
            tags: &c.tags,
        },
    );
    let svg = drawn.replacen(r#"class="docair-pic""#, r#"class="docair-pic changes""#, 1);
    (laid, svg)
}

/***
 * A big flow's pictures (docs/CHANGES.md §8, docair-zrr.27): each change on its own, the rest folded away, its card's Back and Next linking the pictures before and after it, the first at the flow's own address; then all changes in one picture (`/all`) and the whole flow (`/whole`). Their addresses.
 *
 * @calls docair-core.change_steps.focused_on "each change on its own"
 * @calls docair-core.change_steps.focused "all changes in one picture"
 ***/
fn pictures_by_change(
    (m, mk, flow_steps): (&Merged, &FlowMarks, &[ChangeStep]),
    f: &FlowChange,
    code: &HashSet<&str>,
    (addr, title, pair): (&str, &str, &str),
    words: Option<&Value>,
    keep: Option<&Path>,
    (linker, refs): (&crate::page_glossary::Linker, &CodeRefs),
) -> (Vec<ChangePicture>, String, String) {
    let mut out = Vec::new();
    let n = flow_steps.len();
    let at_of = |k: usize| {
        if k == 0 {
            addr.to_string()
        } else {
            format!("{addr}/{}", k + 1)
        }
    };
    for (k, step) in flow_steps.iter().enumerate() {
        let one_change = change_steps::focused_on(m, step);
        let its_marks = change_steps::marks(&one_change, f, code);
        let (laid, svg) = draw_one(&one_change, &its_marks, keep);
        let here = StepAt {
            k,
            n,
            prev: (k > 0).then(|| at_of(k - 1)),
            next: (k + 1 < n).then(|| at_of(k + 1)),
        };
        let step_title = format!("{pair} · change {} of {n}", k + 1);
        out.push(ChangePicture {
            addr: at_of(k),
            title: if k == 0 {
                pair.to_string()
            } else {
                step_title.clone()
            },
            crumbs: if k == 0 {
                vec![(pair.to_string(), addr.to_string())]
            } else {
                vec![(pair.to_string(), addr.to_string()), (step_title, at_of(k))]
            },
            svg,
            change: Some(f.seed.clone()),
            walk: changes_walk(
                &one_change,
                &laid,
                std::slice::from_ref(step),
                (title, words, None),
                Some(&here),
                linker,
                refs,
            ),
        });
    }
    // All changes in one picture, the rest folded away; and the whole flow.
    let all = change_steps::focused(m, mk, flow_steps);
    let all_marks = change_steps::marks(&all, f, code);
    let (laid, svg) = draw_one(&all, &all_marks, keep);
    let (all_addr, whole_addr) = (format!("{addr}/all"), format!("{addr}/whole"));
    for (to, what, svg, walk) in [
        (
            all_addr.to_string(),
            "all changes in one picture",
            svg,
            changes_walk(
                &all,
                &laid,
                flow_steps,
                (title, words, None),
                None,
                linker,
                refs,
            ),
        ),
        (
            whole_addr.to_string(),
            "the whole flow",
            draw_one(m, mk, keep).1,
            None,
        ),
    ] {
        let t = format!("{title}: {what}");
        out.push(ChangePicture {
            addr: to.clone(),
            title: t.clone(),
            crumbs: vec![(pair.to_string(), addr.to_string()), (t, to)],
            svg,
            change: Some(f.seed.clone()),
            walk,
        });
    }
    (out, all_addr, whole_addr)
}

/// The review's changes in a stored finding of `docair.explained/2` (docs/EXPLAIN.md §9.4); none for another schema: an older review quotes and cites, and is asked again (docair-8l6.55).
fn review_changes(findings: Option<&Value>) -> &[Value] {
    findings
        .filter(|f| f["schema"].as_str() == Some("docair.explained/2"))
        .and_then(|f| f["changes"].as_array())
        .map_or(&[][..], Vec::as_slice)
}

/// The pieces no change of a stored review holds (docair-8l6.63): each is a change of its own.
fn review_other(findings: Option<&Value>) -> Vec<&str> {
    review_ids(findings, "other")
}

/// A list of piece ids in a stored review of `docair.explained/2`: `other`, `left`.
fn review_ids<'f>(findings: Option<&'f Value>, k: &str) -> Vec<&'f str> {
    findings
        .filter(|f| f["schema"].as_str() == Some("docair.explained/2"))
        .and_then(|f| f[k].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// Why a change is in docair's words, not the AI's (docair-8l6.63, 8l6.67).
fn docair_note(ch: &Value) -> &'static str {
    match ch["note"].as_str() {
        Some("newer") => "this piece is newer than the review",
        Some("left") => "the run reached its limit of requests before this piece",
        _ => "the AI put this piece in no change",
    }
}

/// docair's words as plain text with code in backticks: `<code>total</code>` → `` `total` ``.
fn plain_words(html: &str) -> String {
    html.replace("<code>", "`")
        .replace("</code>", "`")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// A changed flow's title, from the version that has it: *Datum order*, *Scenario checkout*.
fn flow_title(after: &Graph, before: &Graph, seed: &str) -> String {
    match docair_core::slice::find(after, seed) {
        Some(s) => slice_title(&s, &|id| name_in(after, id)),
        None => docair_core::slice::find(before, seed).map_or_else(
            || seed.to_string(),
            |s| slice_title(&s, &|id| name_in(before, id)),
        ),
    }
}

/// The review as the page shows it: what leads the changes' list, each change's details and pictures.
struct ReviewView {
    list: String,
    /// How many changes the review lists: none without a review.
    n: usize,
    details: Map<String, Value>,
    pictures: Vec<ChangePicture>,
}

/***
 * The review (docs/EXPLAIN.md §9.5, docair-8l6.22): the AI's changes in its order, each at `#review/<n>` on its own picture (its pieces' parts only, docair-8l6.33), walked: first what and why under its title (why, what the AI found off or that it found nothing, what to check), then its pieces in the order of *how*, each card the model's words for that piece over docair's own, with its code. Its details hold only what the card does not (docair-8l6.54): which change, the flows it touches, who wrote it. Every piece is a change (docair-8l6.63): one the AI put in none is a change of its own after the AI's, in docair's words, marked so. Without a review: that the AI writes it, the command, and, served, a button (docair-8l6.26).
 *
 * @calls docair-core.review.pieces "the pieces of the whole change"
 * @calls docair-core.review.change_picture "each change's own picture"
 ***/
fn review_view(
    after: &Graph,
    changed: &Changed,
    keep: Option<&Path>,
    (linker, refs): (&crate::page_glossary::Linker, &CodeRefs),
    code: &HashSet<&str>,
) -> ReviewView {
    let c = changed.changes;
    let before = changed.before;
    let stored = review_changes(changed.findings);
    let other = review_other(changed.findings);
    let mut out = ReviewView {
        list: String::new(),
        n: 0,
        details: Map::new(),
        pictures: Vec::new(),
    };
    // The command that writes the review, and, served, a button that asks for it with its progress under it.
    let command = format!(
        "<code>docair explain {}{}</code>",
        escape_text(&c.a),
        if c.working_tree {
            String::new()
        } else {
            format!(" {}", escape_text(&c.b))
        }
    );
    let button = |label: &str| {
        format!(
            r#"<button class="btn primary" type="button" data-test="explain" data-served hidden data-a="{}"{}>{label}</button><div class="explain-progress" data-test="explain-progress" hidden></div>"#,
            escape_text(&c.a),
            if c.working_tree {
                String::new()
            } else {
                format!(r#" data-b="{}""#, escape_text(&c.b))
            }
        )
    };
    if stored.is_empty() && other.is_empty() {
        let _ = write!(
            out.list,
            r#"<p class="muted" data-test="review-needs-ai">The AI writes the review: what each change does, how, why and what to check. {command} writes it.</p>{}"#,
            button("✦ Review the changes")
        );
        return out;
    }
    let pieces = docair_core::review::pieces(before, after, c);
    let piece = |id: &str| pieces.iter().find(|p| p.id() == id);
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    // Every piece is a change (docair-8l6.63, 8l6.67): one the AI put in none, one the run did not reach,
    // and one newer than the review are each a change of its own, in docair's words.
    // A stored change none of whose pieces are there now no longer says anything of this change.
    let mut found: Vec<Value> = stored
        .iter()
        .filter(|ch| {
            ch["how"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|h| h["piece"].as_str().is_some_and(|p| piece(p).is_some()))
        })
        .cloned()
        .collect();
    let dropped = stored.len() - found.len();
    let left = review_ids(changed.findings, "left");
    let held: HashSet<&str> = stored
        .iter()
        .flat_map(|ch| ch["how"].as_array().into_iter().flatten())
        .filter_map(|h| h["piece"].as_str())
        .chain(other.iter().copied())
        .chain(left.iter().copied())
        .collect();
    let newer: Vec<&str> = pieces
        .iter()
        .map(|p| p.id())
        .filter(|p| !held.contains(p))
        .collect();
    for (ids, note) in [(&other, ""), (&left, "left"), (&newer, "newer")] {
        for p in ids.iter().filter_map(|id| piece(id)) {
            found.push(json!({
                "title": plain_words(&docair_words(after, &p.step)),
                "how": [{ "piece": p.id(), "words": "" }],
                "recorded": { "by": "docair" },
                "note": note,
            }));
        }
    }
    // A review older than the change says so, what it misses and what it left out, and offers to
    // review again (docair-8l6.67).
    if !newer.is_empty() || dropped > 0 {
        let mut said = Vec::new();
        if !newer.is_empty() {
            said.push(format!(
                "{} it does not cover {} in docair's words",
                count(newer.len(), "piece"),
                if newer.len() == 1 { "is" } else { "are" }
            ));
        }
        if dropped > 0 {
            said.push(format!(
                "{} of its changes, about pieces gone now, {} left out with what it found off",
                dropped,
                if dropped == 1 { "is" } else { "are" }
            ));
        }
        let _ = write!(
            out.list,
            r#"<p class="muted" data-test="review-stale">The review is older than these changes: {}. {command} reviews them again.</p>{}"#,
            said.join("; "),
            button("✦ Review again")
        );
    }
    out.list
        .push_str(r#"<ol class="review-list" data-test="review-list">"#);
    let n = found.len();
    out.n = n;
    for (k, ch) in found.iter().enumerate() {
        let addr = format!("review/{}", k + 1);
        let title = text(ch, "title");
        // The model's pieces, in the order they work together, with its words on each.
        let how: Vec<(String, String)> = ch["how"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|h| (text(h, "piece"), text(h, "words")))
            .filter(|(p, _)| piece(p).is_some())
            .collect();
        let docair = ch["recorded"]["by"].as_str() == Some("docair");
        let its: Vec<&docair_core::review::Piece> =
            how.iter().filter_map(|(p, _)| piece(p)).collect();
        let mut flows: Vec<&String> = its.iter().flat_map(|p| &p.flows).collect();
        flows.sort();
        flows.dedup();
        let _ = write!(
            out.list,
            r#"<li><a class="link" href="{}" data-test="review-change">{}</a> <span class="muted">· {}{}</span></li>"#,
            href(&addr),
            with_code(&title),
            count(how.len(), "piece"),
            if docair { " · in docair's words" } else { "" },
        );
        // What and why, the card it opens on: why, what the AI found off (or that it found nothing), what to check.
        let why = text(ch, "why");
        let off: Vec<(&str, String)> = ch["off"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| off_item(v, refs))
            .collect();
        let check = text(ch, "check");
        let mut intro = String::new();
        if !why.is_empty() {
            let _ = write!(
                intro,
                r#"<h4>Why</h4><p data-test="review-why">{}</p>"#,
                with_code(&why)
            );
        }
        if docair {
            // The AI did not see it as a change, so it checked nothing of it (docair-8l6.63, 8l6.67).
            let _ = write!(
                intro,
                r#"<p class="muted" data-test="review-docair">In docair's words: {}, so nothing of it was checked.</p>"#,
                docair_note(ch)
            );
        } else if off.is_empty() {
            intro.push_str(r#"<p class="review-ok" data-test="review-ok">✓ Checked against the code: nothing found off.</p>"#);
        } else {
            // By kind, each under its heading, the clear contradictions first (docair-8l6.62).
            intro
                .push_str(r#"<div class="review-off" data-test="review-off"><h4>⚠ Found off</h4>"#);
            for (kind, heading) in docair_core::review::OFF_KINDS {
                let mine: Vec<&String> = off
                    .iter()
                    .filter(|(k, _)| *k == kind)
                    .map(|(_, x)| x)
                    .collect();
                if mine.is_empty() {
                    continue;
                }
                let _ = write!(
                    intro,
                    r#"<section class="off-group" data-test="off-group" data-kind="{kind}"><h5>{heading} <span class="muted">· {}</span></h5>"#,
                    mine.len()
                );
                for x in mine {
                    intro.push_str(x);
                }
                intro.push_str("</section>");
            }
            intro.push_str("</div>");
        }
        if !check.is_empty() {
            let _ = write!(
                intro,
                r#"<h4>What to check</h4><p data-test="review-check">{}</p>"#,
                with_code(&check)
            );
        }
        // What the AI read for it, folded: the commits by their subject, the docs by their path (docair-8l6.71).
        let sources: Vec<String> = ch["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| {
                let place = s["where"].as_str()?;
                Some(match s["source"].as_str() {
                    Some("doc") => format!("<li><code>{}</code></li>", escape_text(place)),
                    _ => format!("<li>The commit \u{201c}{}\u{201d}</li>", escape_text(place)),
                })
            })
            .collect();
        if !sources.is_empty() {
            let _ = write!(
                intro,
                r#"<details class="review-sources" data-test="review-sources"><summary>Sources · {}</summary><ul>{}</ul></details>"#,
                sources.len(),
                sources.join("")
            );
        }
        let _ = write!(
            intro,
            r#"<p class="muted">Next: how, {}.</p>"#,
            count(how.len(), "piece")
        );
        // Its picture, walked: what and why first, then its pieces in the order of how.
        let words = Value::Object(
            how.iter()
                .filter(|(_, w)| !w.is_empty())
                .map(|(p, w)| (p.clone(), Value::String(w.clone())))
                .collect(),
        );
        let steps: Vec<ChangeStep> = its.iter().map(|p| p.step.clone()).collect();
        let drawn =
            docair_core::review::change_picture(before, after, c, &addr, &its).map(|(m, f)| {
                let marked = change_steps::marks(&m, &f, code);
                let (laid, svg) = draw_one(&m, &marked, keep);
                let walk = changes_walk(
                    &m,
                    &laid,
                    &steps,
                    (&title, Some(&words), Some(&intro)),
                    None,
                    linker,
                    refs,
                );
                (svg, walk)
            });
        let short_title = docair_core::review::cut(&title.replace('`', ""), 80);
        out.pictures.push(ChangePicture {
            addr: addr.clone(),
            title: short_title.clone(),
            crumbs: vec![(short_title, addr.clone())],
            svg: drawn
                .as_ref()
                .map_or_else(|| NOT_FOUND.to_string(), |d| d.0.clone()),
            change: Some(addr.clone()),
            walk: drawn.and_then(|d| d.1),
        });
        // Its details: only what the card does not say (docair-8l6.54).
        let mut d = format!(
            r#"<div class="meta" data-test="review-meta">change {} of {n} · from <code>{}</code> to <code>{}</code></div>"#,
            k + 1,
            escape_text(short(&c.a)),
            escape_text(short(&c.b))
        );
        if !flows.is_empty() {
            d.push_str(r#"<h4>The flows it touches</h4><ul data-test="review-flows">"#);
            for seed in flows {
                let _ = write!(
                    d,
                    r#"<li><a class="link" href="{}">{}</a></li>"#,
                    href(&format!("changes/{seed}")),
                    escape_text(&flow_title(after, before, seed))
                );
            }
            d.push_str("</ul>");
        }
        let _ = write!(
            d,
            r#"<div class="muted">{}</div><p><a class="link" data-changes-list>← All changes</a></p>"#,
            escape_text(&recorded(ch))
        );
        out.details
            .insert(addr, Value::String(linker.link(&d, None)));
    }
    out.list.push_str("</ol>");
    out.list = linker.link(&out.list, None);
    out
}

/// The picture of a flow neither version has: it says so.
const NOT_FOUND: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" class="docair-pic changes" viewBox="0 0 280 120" width="280" height="120"><g class="side-missing"><rect x="0" y="0" width="280" height="120" rx="8"/><text x="140" y="60">Not found in either version</text></g></svg>"#;

/// One thing the AI found off, as the card shows it, and its kind (docair-8l6.58, 8l6.62): a contradiction
/// as what was said, by which commit, doc or code comment, over what the code seems to do; what nothing
/// mentions and a risk as what the code does. A bare sentence (as the model may answer) is a risk; an item
/// stored before the kinds is a contradiction where something was said, else a risk.
fn off_item<'a>(v: &'a Value, refs: &CodeRefs) -> Option<(&'a str, String)> {
    let (after, before) = (refs.after, refs.before);
    let field = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").trim();
    let (source, place, says, code) = match v.as_str() {
        Some(t) => ("none", "", "", t.trim()),
        None => (
            field("source"),
            field("where"),
            field("says"),
            field("code"),
        ),
    };
    if code.is_empty() {
        return None;
    }
    let said = source != "none" && !says.is_empty();
    let kind = match field("kind") {
        "contradicts" if said => "contradicts",
        "unsaid" => "unsaid",
        "" if said => "contradicts",
        _ => "risk",
    };
    let who = match (source, place) {
        _ if kind != "contradicts" => String::new(),
        ("commit", "") => "A commit message".into(),
        ("commit", p) => format!("The commit \u{201c}{}\u{201d}", escape_text(p)),
        ("doc", "") => "A doc".into(),
        ("doc", p) => format!("<code>{}</code>", escape_text(p)),
        ("comment", "") => "A code comment".into(),
        ("comment", p) => format!("The comment on <code>{}</code>", escape_text(p)),
        _ => "It".into(),
    };
    let said = if who.is_empty() {
        String::new()
    } else {
        format!(
            r#"<dt>Said</dt><dd data-test="off-said"><span class="off-who">{who}:</span> {}</dd>"#,
            with_code(says)
        )
    };
    // Where the code does it (docair-8l6.61): each part a link to it, gone ones by id, and their code folded.
    let parts: Vec<&str> = v["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut at = String::new();
    for (k, id) in parts.iter().enumerate() {
        if k > 0 {
            at.push_str(", ");
        }
        match after.nodes.iter().find(|n| n.id == *id) {
            Some(n) => {
                let _ = write!(
                    at,
                    r#"<code class="node-ref" data-node="{}" tabindex="0" title="{}">{}</code>"#,
                    escape_text(id),
                    escape_text(id),
                    escape_text(&n.name)
                );
            }
            None => {
                let _ = write!(
                    at,
                    r#"<code title="gone">{}</code> <span class="muted">gone</span>"#,
                    escape_text(id)
                );
            }
        }
    }
    let has = |g: &Graph, id: &str| g.nodes.iter().any(|n| n.id == id);
    let folded = parts_code(
        parts
            .iter()
            .map(|id| {
                let what = match (has(before, id), has(after, id)) {
                    (false, _) => "new",
                    (_, false) => "gone",
                    _ => "edited",
                };
                (*id, what)
            })
            .collect(),
        refs,
    );
    let at = if at.is_empty() {
        String::new()
    } else {
        format!(r#"<div class="off-parts" data-test="off-parts">in {at}</div>"#)
    };
    Some((
        kind,
        format!(
            r#"<dl class="off-item" data-test="off-item" data-kind="{kind}" data-source="{}">{said}<dt>The code</dt><dd data-test="off-code">{}{at}{folded}</dd></dl>"#,
            if who.is_empty() { "none" } else { source },
            with_code(code)
        ),
    ))
}

/// A count and its noun: `1 part`, `4 parts`.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// How a flow changed, in a word.
fn how(f: &FlowChange) -> &'static str {
    match f.how.as_str() {
        "new" => "new",
        "gone" => "gone",
        _ => "changed",
    }
}

const UNCERTAIN: &str = r#" <span class="badge warn" title="A line of this change only may reach its part: it may be a false alarm">uncertain</span>"#;

/// Parts as a list: linked when the page draws them (B's), plain when only A had them.
fn parts_list(graph: &Graph, ids: &[String], linked: bool) -> String {
    let mut out = String::from("<ul>");
    for id in ids {
        let n = escape_text(&name_in(graph, id));
        if linked {
            let _ = write!(
                out,
                r#"<li><a class="link" href="{}">{n}</a></li>"#,
                href(&format!("part/{id}"))
            );
        } else {
            let _ = write!(
                out,
                r#"<li>{n} <span class="muted mono">{}</span></li>"#,
                escape_text(id)
            );
        }
    }
    out.push_str("</ul>");
    out
}

/***
 * Every flow of the system (docair-8l6.57), folded under the flows the changes touch: the scenarios, the routes, the data and the walkthroughs, each a link to its picture, a changed one marked and linked to its picture before and after.
 ***/
fn all_flows_list(
    after: &Graph,
    seeds: &[docair_core::slice::SliceSeed],
    changed: &[FlowChange],
) -> String {
    use docair_core::slice::SeedKind;
    let name = |id: &str| name_in(after, id);
    let mut out = format!(
        r#"<details class="all-flows" data-test="all-flows"><summary>All {} of the system</summary>"#,
        count(seeds.len(), "flow")
    );
    for (kind, head) in [
        (SeedKind::Flow, "Scenarios"),
        (SeedKind::Route, "Routes"),
        (SeedKind::Data, "Data"),
        (SeedKind::Walkthrough, "Walkthroughs"),
    ] {
        let of: Vec<_> = seeds.iter().filter(|s| s.kind == kind).collect();
        if of.is_empty() {
            continue;
        }
        let _ = write!(out, "<h4>{head} · {}</h4><ul>", of.len());
        for s in of {
            let key = match kind {
                SeedKind::Flow => format!("flow:{}", s.id),
                SeedKind::Walkthrough => format!("walkthrough:{}", s.id),
                _ => s.id.clone(),
            };
            // A walkthrough by its title, a route with its method and path.
            let title = match kind {
                SeedKind::Flow => s.id.clone(),
                SeedKind::Walkthrough => after
                    .walkthroughs
                    .iter()
                    .find(|w| w.id == s.id)
                    .map_or_else(|| s.id.clone(), |w| w.title.clone()),
                _ => name(&s.id),
            };
            let route = after
                .nodes
                .iter()
                .find(|n| n.id == s.id)
                .and_then(|n| n.http.as_ref())
                .filter(|_| kind == SeedKind::Route)
                .map(|h| {
                    format!(
                        r#" <span class="muted mono">{} {}</span>"#,
                        escape_text(&h.method),
                        escape_text(&h.path)
                    )
                })
                .unwrap_or_default();
            let _ = write!(
                out,
                r#"<li><a class="link" href="{}">{}</a>{route}"#,
                href(&format!("slice/{key}")),
                escape_text(&title)
            );
            if let Some(f) = changed.iter().find(|f| f.seed == key) {
                let _ = write!(
                    out,
                    r#" <span class="badge ch-{0}">{0}</span> <a class="link" href="{1}" data-test="flow-changes">before and after</a>"#,
                    how(f),
                    href(&format!("changes/{key}"))
                );
            }
            out.push_str("</li>");
        }
        out.push_str("</ul>");
    }
    out.push_str("</details>");
    out
}

/***
 * The changes between two versions as the page shows them: the header's button, the list of changed flows with the moves apart, and for each flow one picture of its slice before and after, marked.
 *
 * @calls docair-core.slice.find "finds each changed flow's seed in each version"
 * @calls docair-viewer.svg.draw_marked "draws each flow's one picture with what changed marked"
 ***/
pub(crate) fn change_view(after: &Graph, changed: &Changed, keep: Option<&Path>) -> ChangeView {
    let c = changed.changes;
    let (a, b) = (short(&c.a), short(&c.b));
    let n = c.flows.len();
    // Code the model names links to its part, as in every other panel.
    let linker = crate::page_glossary::Linker::new(after);
    // The review leads (docs/EXPLAIN.md §9.5); the flows it is built from follow.
    let inside_code = change_steps::changed_inside(&c.graph).code;
    // Where the changed parts are, to get down to the code from every change (docair-8l6.49, 8l6.82).
    let refs = CodeRefs {
        before: changed.before,
        after,
        links: changed.links,
    };
    let reviewed = review_view(after, changed, keep, (&linker, &refs), &inside_code);
    // Every change the review lists counts, those in docair's words too (docair-8l6.63, 8l6.67).
    let review_n = reviewed.n;
    let button = format!(
        r#"<button class="btn" id="btn-changes" data-test="changes-button" title="What changed from {} to {}">Δ Changes · {}</button>"#,
        escape_text(a),
        escape_text(b),
        if review_n > 0 { review_n } else { n }
    );
    let mut list = format!(
        r#"<h2>Δ Changes</h2><div class="meta">from <code>{}</code> to <code>{}</code> · {}</div>"#,
        escape_text(a),
        escape_text(b),
        if review_n > 0 {
            format!(
                "{} reviewed by the AI · {}",
                count(review_n, "change"),
                count(n, "flow")
            )
        } else {
            format!("{} changed", count(n, "flow"))
        }
    );
    // Where the comparison starts when it is not A itself: where B left A (docs/EDITOR.md §2).
    if let Some(base) = &c.base {
        let _ = write!(
            list,
            r#"<p class="muted" data-test="split-from">Compared from where {} left <code>{}</code>: <code>{}</code>.</p>"#,
            if c.working_tree {
                "the working tree".to_string()
            } else {
                format!("<code>{}</code>", escape_text(b))
            },
            escape_text(a),
            escape_text(&base[..base.len().min(12)])
        );
    }
    // Each commit is read with its own docair.toml: what changed in it comes first (docair-zrr.31).
    if !c.graph.toml.is_empty() {
        let lines: Vec<String> = c
            .graph
            .toml
            .iter()
            .map(|l| {
                let class = if l.starts_with('+') { "add" } else { "del" };
                format!(r#"<span class="{class}">{}</span>"#, escape_text(l))
            })
            .collect();
        let _ = write!(
            list,
            r#"<div class="warn" data-test="toml-changed"><b>docair.toml changed</b>, so some differences may come from how docair read the code, not from the code:<pre class="toml-diff">{}</pre></div>"#,
            lines.join("\n")
        );
    }
    if !c.graph.reading.is_empty() {
        let _ = write!(
            list,
            r#"<p class="warn" data-test="reading-changed">The reading changed too ({}): some differences may come from docair, not from the code.</p>"#,
            escape_text(&c.graph.reading.join("; "))
        );
    }
    let mut details = Map::new();
    let mut pictures = Vec::new();
    list.push_str(&reviewed.list);
    details.extend(reviewed.details);
    pictures.extend(reviewed.pictures);
    // The flows next: what the review is built from, and how many of the system's flows changed (docair-8l6.42).
    let every_flow: Vec<docair_core::slice::SliceSeed> = docair_core::slice::all(after)
        .into_iter()
        .map(|s| s.seed)
        .filter(|s| s.kind != docair_core::slice::SeedKind::Part)
        .collect();
    let all_flows = every_flow.len();
    let _ = write!(
        list,
        r#"<h3 class="flows-head" data-test="flows-head">The flows the changes touch · {n} of the system's {all_flows}</h3><ul class="changes-list" data-test="changes-list">"#
    );
    let name_b = |id: &str| name_in(after, id);
    let name_a = |id: &str| name_in(changed.before, id);
    // The parts that moved or were renamed, and the parts changed inside, in their code or only their words.
    let moves = change_steps::id_moves(&c.graph);
    let inside = change_steps::changed_inside(&c.graph);
    // The list's items: flows whose path changed first, then those whose parts changed only in their code, then words.
    let mut items: [String; 3] = Default::default();
    for f in &c.flows {
        let seed = docair_core::slice::find(after, &f.seed)
            .or_else(|| docair_core::slice::find(changed.before, &f.seed));
        let title = match &seed {
            Some(s) if docair_core::slice::find(after, &f.seed).is_some() => {
                slice_title(s, &name_b)
            }
            Some(s) => slice_title(s, &name_a),
            None => f.seed.clone(),
        };
        let addr = format!("changes/{}", f.seed);
        let uncertain = if f.certain { "" } else { UNCERTAIN };
        let sort = sort_flow(f, &inside);
        let mut item = String::new();
        let _ = write!(
            item,
            r#"<li><a class="link" href="{}">{}</a> <span class="badge ch-{}">{}</span>{}{uncertain}"#,
            href(&addr),
            escape_text(&title),
            how(f),
            how(f),
            // Matched by its parts when its seed was renamed (docair-zrr.32).
            f.renamed_from.as_ref().map_or(String::new(), |was| format!(
                r#" <span class="muted" data-test="renamed-from">renamed from <code>{}</code></span>"#,
                escape_text(was)
            ))
        );
        let mut gained = Vec::new();
        if !f.parts_added.is_empty() {
            gained.push(format!(
                "+ {}",
                f.parts_added
                    .iter()
                    .map(|p| name_b(p))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !f.parts_removed.is_empty() {
            gained.push(format!(
                "− {}",
                f.parts_removed
                    .iter()
                    .map(|p| name_a(p))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let few = |ids: &[&String]| {
            let mut names: Vec<String> = ids.iter().take(6).map(|p| name_b(p)).collect();
            if ids.len() > 6 {
                names.push("…".into());
            }
            names.join(", ")
        };
        match sort.kind {
            FlowKind::Path => {}
            FlowKind::Code => gained.push(format!(
                "same path · {} changed in their code: {}",
                count(sort.code.len(), "part"),
                few(&sort.code)
            )),
            FlowKind::Words if sort.words.is_empty() => {
                gained.push("same path and code: parts only moved or reformatted".into())
            }
            FlowKind::Words => gained.push(format!("words or tags of {}", few(&sort.words))),
        }
        if !gained.is_empty() {
            let _ = write!(
                item,
                r#"<div class="muted">{}</div>"#,
                escape_text(&gained.join(" · "))
            );
        }
        item.push_str("</li>");
        items[sort.kind as usize].push_str(&item);
        // The picture: both versions as one, what changed marked, and its changes in steps.
        let (pa, pb) = (
            change_steps::flow_picture(changed.before, &f.seed),
            change_steps::flow_picture(after, &f.seed),
        );
        let one = change_steps::merged(
            pa.as_ref().map(|p| (changed.before, p)),
            pb.as_ref().map(|p| (after, p)),
            &moves,
        );
        let marked = one
            .as_ref()
            .map(|m| change_steps::marks(m, f, &inside.code));
        let flow_steps = match (&one, &marked) {
            // One step per changed part (docair-zrr.35).
            (Some(m), Some(mk)) => change_steps::part_steps(m, f, mk),
            _ => Vec::new(),
        };
        // A big flow (docair-zrr.27): each change on its own picture, the rest folded away, Back and Next
        // between them; all changes in one picture and the whole flow a link away.
        let whole_parts = one.as_ref().map_or(0, |m| m.pic.parts.len());
        let big = whole_parts > change_steps::FOCUS_ABOVE && !flow_steps.is_empty();
        // The model's words are on the review's changes (docs/EXPLAIN.md §9), not on a flow's steps.
        let words: Option<&Value> = None;
        let pair = format!("{title}: before and after");
        let mut focus_links = String::new();
        match (&one, &marked) {
            (Some(m), Some(mk)) if big => {
                let (more, all_addr, whole_addr) = pictures_by_change(
                    (m, mk, &flow_steps),
                    f,
                    &inside.code,
                    (&addr, &title, &pair),
                    words,
                    keep,
                    (&linker, &refs),
                );
                pictures.extend(more);
                focus_links = format!(
                    r#"<p data-test="focus">A big change: each change is drawn on its own, with what it forks off and rejoins at and the boxes they are in; the rest is folded away. <a class="link" href="{}" data-test="changes-only">Change by change</a> · <a class="link" href="{}" data-test="all-changes">All changes in one picture</a> · <a class="link" href="{}" data-test="whole-flow">The whole flow · {whole_parts} parts</a></p>"#,
                    href(&addr),
                    href(&all_addr),
                    href(&whole_addr)
                );
            }
            _ => {
                let drawn = one
                    .as_ref()
                    .zip(marked.as_ref())
                    .map(|(m, mk)| draw_one(m, mk, keep));
                pictures.push(ChangePicture {
                    addr: addr.clone(),
                    title: pair.clone(),
                    crumbs: vec![(pair.clone(), addr.clone())],
                    svg: drawn
                        .as_ref()
                        .map_or_else(|| NOT_FOUND.to_string(), |d| d.1.clone()),
                    change: Some(f.seed.clone()),
                    // It opens on its first change, and steps from change to change.
                    walk: match (&one, &drawn) {
                        (Some(m), Some((laid, _))) => changes_walk(
                            m,
                            laid,
                            &flow_steps,
                            (&title, words, None),
                            None,
                            &linker,
                            &refs,
                        ),
                        _ => None,
                    },
                });
            }
        }
        // Its details, shown with its picture.
        let mut d = format!(
            r#"<h2>{}</h2><div class="meta">{} · from <code>{}</code> to <code>{}</code>{uncertain}</div><p>{}</p>{focus_links}"#,
            escape_text(&title),
            how(f),
            escape_text(a),
            escape_text(b),
            match sort.kind {
                FlowKind::Path =>
                    "The flow before and after in one picture: what is new is green, what is gone red and dashed, a part whose code changed amber; what stayed the same is faded.",
                FlowKind::Code =>
                    "The path is the same before and after: the parts in amber changed in their code; the rest stayed the same and is faded.",
                FlowKind::Words =>
                    "The path and the code are the same before and after: only words or tags changed, so the picture marks nothing.",
            }
        );
        let code_ids: Vec<String> = sort.code.iter().map(|p| p.to_string()).collect();
        let word_ids: Vec<String> = sort.words.iter().map(|p| p.to_string()).collect();
        for (head, ids, graph, linked) in [
            ("Now goes through", &f.parts_added, after, true),
            (
                "No longer goes through",
                &f.parts_removed,
                changed.before,
                false,
            ),
            ("Changed in their code", &code_ids, after, true),
            ("Only their words or tags changed", &word_ids, after, true),
        ] {
            if !ids.is_empty() {
                let _ = write!(d, "<h4>{head}</h4>{}", parts_list(graph, ids, linked));
            }
        }
        d.push_str(r#"<p><a class="link" data-changes-list>← All changes</a></p>"#);
        details.insert(f.seed.clone(), Value::String(linker.link(&d, None)));
    }
    let [path_items, code_items, word_items] = items;
    list.push_str(&path_items);
    list.push_str(&code_items);
    list.push_str("</ul>");
    if !word_items.is_empty() {
        let _ = write!(
            list,
            r#"<h4>Only words or tags changed</h4><ul class="changes-list" data-test="changes-words">{word_items}</ul>"#
        );
    }
    // Every flow of the system, folded: the list the count above promises (docair-8l6.57).
    list.push_str(&all_flows_list(after, &every_flow, &c.flows));
    if !c.graph.moved.is_empty() {
        list.push_str(r#"<h4>Moved and renamed</h4><ul class="moved" data-test="moved">"#);
        for m in &c.graph.moved {
            let _ = write!(
                list,
                "<li><code>{}</code> → <code>{}</code></li>",
                escape_text(&m.from),
                escape_text(&m.to)
            );
        }
        list.push_str("</ul>");
    }
    // What the model linked: a part at A that is a part at B, where docair proved no move.
    let links = changed
        .findings
        .and_then(|x| x.get("links"))
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    if !links.is_empty() {
        list.push_str(r#"<h4>✦ Linked by the model</h4><ul class="moved" data-test="links">"#);
        for l in links {
            let _ = write!(
                list,
                "<li><code>{}</code> at A is <code>{}</code> at B: {}</li>",
                escape_text(l["a"].as_str().unwrap_or("")),
                escape_text(l["b"].as_str().unwrap_or("")),
                escape_text(l["why"].as_str().unwrap_or(""))
            );
        }
        list.push_str("</ul>");
    }
    ChangeView {
        button,
        list,
        details,
        pictures,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use docair_core::change_steps::{flow_picture, marks, merged, steps};

    fn shop() -> Graph {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../e2e/fixtures/shop");
        docair_core::scan(
            &root,
            docair_adapters::all(),
            &docair_core::Options::default(),
        )
        .unwrap()
    }

    /// A changed docair.toml comes first in the changes, as its lines (docair-zrr.31).
    #[test]
    fn a_changed_docair_toml_comes_first() {
        let g = shop();
        let changes = docair_core::changes::Changes {
            schema: docair_core::changes::SCHEMA.into(),
            a: "A".into(),
            b: "B".into(),
            working_tree: false,
            base: None,
            graph: docair_core::diff::GraphDiff {
                toml: vec!["- hide = [\"web\"]".into(), "+ hide = []".into()],
                ..Default::default()
            },
            parts: vec![],
            flows: vec![],
        };
        let changed = crate::doc_page::Changed {
            before: &g,
            changes: &changes,
            findings: None,
            links: &crate::doc_page::CodeLinks::default(),
        };
        let view = change_view(&g, &changed, None);
        let toml = view
            .list
            .find(r#"data-test="toml-changed""#)
            .expect("shown");
        assert!(
            view.list.contains(
                r#"<pre class="toml-diff"><span class="del">- hide = [&quot;web&quot;]</span>"#
            ) || view
                .list
                .contains(r#"<pre class="toml-diff"><span class="del">- hide = ["web"]</span>"#),
            "{}",
            view.list
        );
        assert!(
            view.list.find("review-needs-ai").is_none_or(|r| toml < r),
            "first, before the review"
        );
    }

    /// The shop without its line from `orders.create` to `orders-db`, and the change of its order's flow.
    fn a_new_line() -> (Graph, Graph, FlowChange) {
        let after = shop();
        let mut before = after.clone();
        let at = before
            .edges
            .iter()
            .position(|e| e.from == "orders.create" && e.to == "orders-db")
            .unwrap();
        let line = before.edges.remove(at).id;
        let f = FlowChange {
            seed: "data.order".into(),
            how: "changed".into(),
            parts_added: vec!["orders-db".into()],
            parts_removed: vec![],
            lines_added: vec![line],
            lines_removed: vec![],
            through: vec![],
            certain: true,
            renamed_from: None,
        };
        (before, after, f)
    }

    /// docair-zrr.18, zrr.19, 8l6.12: the one picture is drawn with its marks and tags and opens as a
    /// walk over its one change; the model's words on that step, by its id, show over docair's.
    #[test]
    fn a_change_is_drawn_and_walked_with_its_words() {
        let (before, after, f) = a_new_line();
        let (pa, pb) = (
            flow_picture(&before, "data.order").unwrap(),
            flow_picture(&after, "data.order").unwrap(),
        );
        let m = merged(Some((&before, &pa)), Some((&after, &pb)), &HashMap::new()).unwrap();
        let marked = marks(&m, &f, &HashSet::new());
        let (laid, svg) = draw_one(&m, &marked, None);
        assert!(
            svg.starts_with(
                r#"<svg xmlns="http://www.w3.org/2000/svg" class="docair-pic changes""#
            )
        );
        assert!(svg.contains(r#"ch-added" data-id="orders-db""#), "{svg}");
        assert!(
            svg.matches(r#" ch-same" data-id"#).count() >= 3,
            "what stayed is faded"
        );
        let linker = crate::page_glossary::Linker::new(&after);
        let flow_steps = steps(&m, &f, &marked);
        let walk = changes_walk(
            &m,
            &laid,
            &flow_steps,
            ("Datum order", None, None),
            None,
            &linker,
            &CodeRefs {
                before: &before,
                after: &after,
                links: &crate::doc_page::CodeLinks::default(),
            },
        )
        .unwrap();
        let st = &walk["steps"].as_array().unwrap()[0];
        assert_eq!(walk["steps"].as_array().unwrap().len(), 1);
        assert_eq!(st["id"], "data.order#path:orders-db");
        let card = st["card"].as_str().unwrap();
        assert!(
            card.contains("change 1 of 1") && card.contains("Now goes through"),
            "{card}"
        );
        assert!(
            card.contains(r#"data-test="route-before"><b>Before</b> · —"#)
                && card.contains(r#"data-test="route-after"><b>After</b> · "#)
                && card.contains(" → "),
            "the route before (none) and after: {card}"
        );
        assert!(
            st["parts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p == "orders-db")
        );
        assert!(st["camera"].is_array());
        // The model's words on that step, kept by its id.
        let words = json!({ "data.order#path:orders-db": "The order is now kept in `orders-db`." });
        let walk = changes_walk(
            &m,
            &laid,
            &flow_steps,
            ("Datum order", Some(&words), None),
            None,
            &linker,
            &CodeRefs {
                before: &before,
                after: &after,
                links: &crate::doc_page::CodeLinks::default(),
            },
        )
        .unwrap();
        let card = walk["steps"][0]["card"].as_str().unwrap();
        assert!(
            card.contains(r#"data-test="step-words""#) && card.contains("is now kept in"),
            "{card}"
        );
        assert!(
            card.contains("Now goes through"),
            "docair's words stay under the model's"
        );
    }

    /// docair-zrr.27: a big flow's changes each get a picture of their own, Back and Next linking them,
    /// the first at the flow's address; then all changes in one picture, and the whole flow.
    #[test]
    fn a_big_flow_gets_a_picture_per_change() {
        let (before, after, mut f) = a_new_line();
        f.through = vec!["shop-ui.cart".into()];
        let code: HashSet<&str> = ["shop-ui.cart"].into_iter().collect();
        let (pa, pb) = (
            flow_picture(&before, "data.order").unwrap(),
            flow_picture(&after, "data.order").unwrap(),
        );
        let m = merged(Some((&before, &pa)), Some((&after, &pb)), &HashMap::new()).unwrap();
        let marked = marks(&m, &f, &code);
        let flow_steps = steps(&m, &f, &marked);
        assert_eq!(flow_steps.len(), 2, "the new line, and the cart's code");
        let linker = crate::page_glossary::Linker::new(&after);
        let (pics, all, whole) = pictures_by_change(
            (&m, &marked, &flow_steps),
            &f,
            &code,
            (
                "changes/data.order",
                "Datum order",
                "Datum order: before and after",
            ),
            None,
            None,
            (
                &linker,
                &CodeRefs {
                    before: &before,
                    after: &after,
                    links: &crate::doc_page::CodeLinks::default(),
                },
            ),
        );
        let addrs: Vec<&str> = pics.iter().map(|p| p.addr.as_str()).collect();
        assert_eq!(
            addrs,
            [
                "changes/data.order",
                "changes/data.order/2",
                all.as_str(),
                whole.as_str()
            ]
        );
        assert_eq!(
            (all.as_str(), whole.as_str()),
            ("changes/data.order/all", "changes/data.order/whole")
        );
        let card = |k: usize| {
            pics[k].walk.as_ref().unwrap()["steps"][0]["card"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert!(
            card(0).contains("change 1 of 2")
                && card(0).contains(r#"data-test="change-next""#)
                && card(0).contains("data.order/2"),
            "{}",
            card(0)
        );
        assert!(
            card(1).contains("change 2 of 2") && card(1).contains(r#"data-test="change-back""#),
            "{}",
            card(1)
        );
        assert!(
            pics[0].walk.as_ref().unwrap()["steps"]
                .as_array()
                .unwrap()
                .len()
                == 1,
            "one change per picture"
        );
        assert_eq!(
            pics[2].walk.as_ref().unwrap()["steps"]
                .as_array()
                .unwrap()
                .len(),
            2,
            "all changes, walked"
        );
        assert!(pics[3].walk.is_none(), "the whole flow is just shown");
    }

    /// docair-zrr.26: a step's lines as routes, from where none of them arrives to where they end;
    /// a line's ends read from its id.
    #[test]
    fn lines_make_routes_and_ids_name_their_ends() {
        let line = |from: &str, to: &str| StepLine {
            id: format!("{from} -call-> {to}"),
            from: from.into(),
            to: to.into(),
        };
        let (a, b, c, d) = (
            line("a", "b"),
            line("b", "c"),
            line("a", "d"),
            line("x", "y"),
        );
        assert_eq!(routes(&[&b, &a]), vec![vec!["a", "b", "c"]]);
        assert_eq!(
            routes(&[&a, &b, &c]),
            vec![vec!["a", "b", "c"], vec!["a", "d"]]
        );
        assert_eq!(routes(&[&d]), vec![vec!["x", "y"]]);
        assert!(routes(&[]).is_empty());
        assert_eq!(
            line_ends("orders.create -write-> orders-db"),
            Some(("orders.create", "orders-db"))
        );
        assert_eq!(line_ends(r#"a -call-> b "posts" #2"#), Some(("a", "b")));
        assert_eq!(line_ends("not a line"), None);
    }

    /// docair-zrr.22: the drawing's classes and tags for what docair-core marked.
    #[test]
    fn classes_say_new_gone_and_code() {
        let mut m = FlowMarks::default();
        for (id, mark) in [
            ("a", PartMark::Added),
            ("r", PartMark::Removed),
            ("c", PartMark::Code),
            ("s", PartMark::Same),
        ] {
            m.parts.insert(id.into(), mark);
        }
        m.lines.insert("x -call-> y".into(), LineMark::Removed);
        let c = classes(&m);
        assert_eq!((c.parts["a"], c.tags["a"]), ("ch-added", "new"));
        assert_eq!((c.parts["r"], c.tags["r"]), ("ch-removed", "gone"));
        assert_eq!((c.parts["c"], c.tags["c"]), ("ch-changed", "code"));
        assert_eq!(c.parts["s"], "ch-same");
        assert!(!c.tags.contains_key("s"));
        assert_eq!(c.lines["x -call-> y"], "ch-removed");
    }
}

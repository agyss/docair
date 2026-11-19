//! What changed between two versions, in the page (docs/CHANGES.md §5, §7 and §8, docair-zrr.7,
//! zrr.18): the header's button with the count of changed flows, the list it opens, and for each
//! flow one picture of its slice before and after, drawn like any slice: a part both versions have
//! once and faded, what only the flow before had red, what only the flow after has green, a part
//! changed inside amber.

use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;

use docair_core::change_steps::{
    self, ChangeStep, FlowMarks, LineMark, Merged, PartMark, StepKind,
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

/// The stored finding about a flow (`docair_ai::explain::FlowExplained` as JSON).
fn finding<'v>(findings: Option<&'v Value>, seed: &str) -> Option<&'v Value> {
    findings?
        .get("flows")?
        .as_array()?
        .iter()
        .find(|f| f.get("seed").and_then(Value::as_str) == Some(seed))
}

/// Who recorded a finding, and when: `by fake-model · 2026-11-19`, `corrected by hand · …`.
fn recorded(f: &Value) -> String {
    let r = &f["recorded"];
    let at = r["at"].as_str().unwrap_or("");
    if r["by"].as_str() == Some("human") {
        format!("corrected by hand · {at}")
    } else {
        format!("by {} · {at}", r["model"].as_str().unwrap_or("the model"))
    }
}

/// A box around some parts of a laid-out picture, with room around it: a walk step's camera.
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
fn step_card(glyph: &str, title: &str, at: &str, body: &str, (k, n): (usize, usize)) -> String {
    format!(
        r#"<section class="card flow" data-test="step-card"><div class="card-head"><span class="card-glyph">{glyph}</span><b>{}</b><button class="card-close" title="Close the walk">×</button></div><div class="card-step">{at}</div>{body}<div class="card-nav"><button class="btn" data-step="-1"{}>◀ Back</button><button class="btn" data-step="1"{}>Next ▶</button></div></section>"#,
        escape_text(title),
        if k == 0 { " disabled" } else { "" },
        if k + 1 >= n { " disabled" } else { "" }
    )
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

/***
 * A walk over a flow's changes only (docs/CHANGES.md §8, docair-zrr.19, 8l6.12): one step per change as docair-core works them out, the path's steps first, each kind in reading order; each step's camera frames its change at a size to read, and its card says what changed: the model's words on that step, by its id, over docair's own.
 *
 * @calls docair-core.change_steps.steps "the flow's changes in steps"
 ***/
fn changes_walk(
    m: &Merged,
    laid: &Layout,
    flow_steps: &[ChangeStep],
    (title, words): (&str, Option<&Value>),
    linker: &crate::page_glossary::Linker,
) -> Option<Value> {
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
    order.sort_by_key(|s| {
        (
            matches!(s.kind, StepKind::Code | StepKind::Box),
            top_left(&places(s)),
        )
    });
    let count = order.len();
    let steps: Vec<Value> = order
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
            let body = match words.and_then(|w| w.get(&s.id)).and_then(Value::as_str) {
                Some(w) => format!(
                    r#"<p data-test="step-words">{}</p><p class="muted">{said}</p>"#,
                    with_code(w)
                ),
                None => format!("<p>{said}</p>"),
            };
            let card = step_card(
                "Δ",
                "The changes",
                &format!("change {} of {count}", k + 1),
                &body,
                (k, count),
            );
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
    Some(json!({ "title": format!("The changes · {title}"), "glyph": "Δ", "steps": steps }))
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
    let words: Vec<&String> = f
        .through
        .iter()
        .filter(|p| on_path(p) && inside.words.contains(p.as_str()))
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

/// The picture of a flow neither version has: it says so.
const NOT_FOUND: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" class="docair-pic changes" viewBox="0 0 280 120" width="280" height="120"><g class="side-missing"><rect x="0" y="0" width="280" height="120" rx="8"/><text x="140" y="60">Not found in either version</text></g></svg>"#;

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
 * The changes between two versions as the page shows them: the header's button, the list of changed flows with the moves apart, and for each flow one picture of its slice before and after, marked.
 *
 * @calls docair-core.slice.find "finds each changed flow's seed in each version"
 * @calls docair-viewer.svg.draw_marked "draws each flow's one picture with what changed marked"
 ***/
pub(crate) fn change_view(after: &Graph, changed: &Changed, keep: Option<&Path>) -> ChangeView {
    let c = changed.changes;
    let (a, b) = (short(&c.a), short(&c.b));
    let n = c.flows.len();
    let button = format!(
        r#"<button class="btn" id="btn-changes" data-test="changes-button" title="What changed from {} to {}">Δ Changes · {n}</button>"#,
        escape_text(a),
        escape_text(b)
    );
    let mut list = format!(
        r#"<h2>Δ Changes</h2><div class="meta">from <code>{}</code> to <code>{}</code> · {n} {} changed</div>"#,
        escape_text(a),
        escape_text(b),
        if n == 1 { "flow" } else { "flows" }
    );
    if !c.graph.reading.is_empty() {
        let _ = write!(
            list,
            r#"<p class="warn" data-test="reading-changed">The reading changed too ({}): some differences may come from docair, not from the code.</p>"#,
            escape_text(&c.graph.reading.join("; "))
        );
    }
    list.push_str(r#"<ul class="changes-list" data-test="changes-list">"#);
    let mut details = Map::new();
    let mut pictures = Vec::new();
    // Code the model names links to its part, as in every other panel.
    let linker = crate::page_glossary::Linker::new(after);
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
            r#"<li><a class="link" href="{}">{}</a> <span class="badge ch-{}">{}</span>{uncertain}"#,
            href(&addr),
            escape_text(&title),
            how(f),
            how(f)
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
        let drawn = one
            .as_ref()
            .zip(marked.as_ref())
            .map(|(m, mk)| draw_one(m, mk, keep));
        let flow_steps = match (&one, &marked) {
            (Some(m), Some(mk)) => change_steps::steps(m, f, mk),
            _ => Vec::new(),
        };
        let found = finding(changed.findings, &f.seed);
        let pair = format!("{title}: before and after");
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
                    (&title, found.and_then(|x| x.get("steps"))),
                    &linker,
                ),
                _ => None,
            },
        });
        // What the model explained about it; its words on each change are on the walk's steps.
        let mut explained = String::new();
        if let Some(x) = found {
            let _ = write!(
                explained,
                r#"<h4>✦ Explained</h4><p data-test="explanation">{}</p><div class="muted">{}</div>"#,
                with_code(x["explanation"].as_str().unwrap_or("")),
                escape_text(&recorded(x))
            );
        }
        // Its details, shown with its picture.
        let mut d = format!(
            r#"<h2>{}</h2><div class="meta">{} · from <code>{}</code> to <code>{}</code>{uncertain}</div><p>{}</p>{explained}"#,
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
    // The flows the model has not explained yet: docair serve asks for them, a file says how.
    let left = c
        .flows
        .iter()
        .filter(|f| finding(changed.findings, &f.seed).is_none())
        .count();
    if left > 0 {
        let b_arg = if c.working_tree {
            String::new()
        } else {
            format!(" {}", c.b)
        };
        let _ = write!(
            list,
            r#"<p class="muted" data-test="unexplained">{left} of {n} not explained yet: <code>docair explain {}{}</code> explains them.</p><button class="btn primary" type="button" data-test="explain" data-served hidden data-a="{}"{}>✦ Explain them</button>"#,
            escape_text(&c.a),
            escape_text(&b_arg),
            escape_text(&c.a),
            if c.working_tree {
                String::new()
            } else {
                format!(r#" data-b="{}""#, escape_text(&c.b))
            }
        );
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
        let walk = changes_walk(&m, &laid, &flow_steps, ("Datum order", None), &linker).unwrap();
        let st = &walk["steps"].as_array().unwrap()[0];
        assert_eq!(walk["steps"].as_array().unwrap().len(), 1);
        assert_eq!(st["id"], "data.order#path:orders-db");
        let card = st["card"].as_str().unwrap();
        assert!(
            card.contains("change 1 of 1") && card.contains("Now goes through"),
            "{card}"
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
            ("Datum order", Some(&words)),
            &linker,
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

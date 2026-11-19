//! What changed between two versions, in the page (docs/CHANGES.md §5 and §7, docair-zrr.7): the
//! header's button with the count of changed flows, the list it opens, and for each flow one
//! picture of its slice at A and at B side by side, drawn like any slice, with what the flow now
//! goes through marked at B and what it no longer does marked at A.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::Path;

use docair_core::changes::FlowChange;
use docair_core::layer_view::{LayerPicture, slice_picture};
use docair_core::model::Graph;
use docair_layout::draw::num;
use docair_layout::{Input, Layout, Monospace, Routes};
use serde_json::{Map, Value, json};

use crate::doc_page::{Changed, slice_title};
use crate::page_panels::href;
use crate::svg::ChangeMarks;
use crate::{escape_text, layout_input, svg};

/// The gap between the two sides, and the band above them for their titles.
const GAP: f64 = 80.0;
const BAND: f64 = 44.0;

/// The changes as the page shows them.
pub(crate) struct ChangeView {
    /// The header's button: `Δ Changes · n`.
    pub button: String,
    /// The list the button opens.
    pub list: String,
    /// Each changed flow's details, by its seed: shown with its picture.
    pub details: Map<String, Value>,
    /// Each changed flow's picture, and each side of its explained workflow.
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
    /// A workflow's steps (docs/EXPLAIN.md §3), when the picture is walked through.
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

/***
 * A workflow's steps on one side, as a walk the page steps through: each step lights the parts it is in (or the box the picture folds them into), frames them, and says what the user does.
 ***/
fn workflow_walk(
    graph: &Graph,
    pic: &LayerPicture,
    laid: &Layout,
    (title, which): (&str, &str),
    steps: &[Value],
    linker: &crate::page_glossary::Linker,
) -> Value {
    let drawn: HashMap<&str, usize> = pic
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.as_str(), i))
        .collect();
    let parent = |id: &str| {
        graph
            .nodes
            .iter()
            .find(|n| n.id == id)
            .and_then(|n| n.part_of.clone())
    };
    let n = steps.len();
    let out: Vec<Value> = steps
        .iter()
        .enumerate()
        .map(|(k, st)| {
            let mut parts: Vec<usize> = Vec::new();
            for id in st["nodes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                let mut at = Some(id.to_string());
                while let Some(c) = at {
                    if let Some(&i) = drawn.get(c.as_str()) {
                        if !parts.contains(&i) {
                            parts.push(i);
                        }
                        break;
                    }
                    at = parent(&c);
                }
            }
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
            let camera = if x0 <= x1 {
                json!([x0 - 40.0, y0 - 40.0, x1 - x0 + 80.0, y1 - y0 + 80.0])
            } else {
                Value::Null
            };
            let card = format!(
                r#"<section class="card flow" data-test="step-card"><div class="card-head"><span class="card-glyph">✦</span><b>{} · {which}</b><button class="card-close" title="Close the workflow">×</button></div><div class="card-step"><span class="muted">step {} of {n}</span> {}</div><div class="card-nav"><button class="btn" data-step="-1"{}>◀ Back</button><button class="btn" data-step="1"{}>Next ▶</button></div></section>"#,
                escape_text(title),
                k + 1,
                with_code(st["title"].as_str().unwrap_or("")),
                if k == 0 { " disabled" } else { "" },
                if k + 1 >= n { " disabled" } else { "" }
            );
            json!({
                "card": linker.link(&card, None),
                "parts": parts.iter().map(|&i| pic.parts[i].id.clone()).collect::<Vec<_>>(),
                "lines": [],
                "camera": camera,
                "tokens": [],
            })
        })
        .collect();
    json!({ "title": format!("{title} · {which}"), "glyph": "✦", "steps": out })
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

/// One version's slice through a seed: its picture, and the picture as the layout reads it.
struct VersionSlice<'g> {
    graph: &'g Graph,
    pic: LayerPicture,
    input: Input,
}

fn version_slice<'g>(graph: &'g Graph, seed: &str) -> Option<VersionSlice<'g>> {
    let found = docair_core::slice::find(graph, seed)?;
    let s = docair_core::slice::slice(graph, &found)?;
    let pic = slice_picture(graph, &s);
    let input = layout_input::input_with(graph, &pic, &Monospace::default(), &HashMap::new());
    Some(VersionSlice { graph, pic, input })
}

/// What a layout line stands for, the same in both versions: the graph line's stable id.
fn line_key(v: &VersionSlice, line: &docair_layout::Line) -> String {
    let edge = line
        .id
        .parse::<usize>()
        .ok()
        .and_then(|k| v.pic.lines.get(k))
        .and_then(|l| l.members.first())
        .and_then(|&m| v.graph.edges.get(m));
    match edge {
        Some(e) if !e.id.is_empty() => e.id.clone(),
        Some(e) => format!("{} {:?} {}", e.from, e.kind, e.to),
        None => format!("#{}", line.id),
    }
}

/// Where each part and line of either version is in the two laid out as one.
struct Union {
    input: Input,
    parts: HashMap<String, usize>,
    lines: HashMap<String, usize>,
}

/***
 * Both versions as one picture to lay out (docair-zrr.17): every part and line of either, a part both have once, at the larger of its two sizes; so a part that stayed is in the same place on both sides, and a part only one side has leaves its room empty on the other.
 ***/
fn union(a: &VersionSlice, b: &VersionSlice) -> Union {
    let mut input = b.input.clone();
    let mut parts: HashMap<String, usize> = input
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.clone(), i))
        .collect();
    for p in &a.input.parts {
        match parts.get(&p.id) {
            Some(&i) => {
                let q = &mut input.parts[i];
                q.width = q.width.max(p.width);
                q.height = q.height.max(p.height);
                q.min_width = q.min_width.max(p.min_width);
                q.name_width = q.name_width.max(p.name_width);
            }
            None => {
                parts.insert(p.id.clone(), input.parts.len());
                input.parts.push(docair_layout::Part {
                    parent: None,
                    ..p.clone()
                });
            }
        }
    }
    // A's own parts in A's boxes, once every part has its place.
    for p in &a.input.parts {
        let i = parts[&p.id];
        if i >= b.input.parts.len() {
            input.parts[i].parent = p
                .parent
                .and_then(|q| parts.get(&a.input.parts[q].id).copied());
        }
    }
    let mut lines: HashMap<String, usize> = HashMap::new();
    for (k, l) in b.input.lines.iter().enumerate() {
        lines.entry(line_key(b, l)).or_insert(k);
    }
    for l in &a.input.lines {
        let key = line_key(a, l);
        if lines.contains_key(&key) {
            continue;
        }
        lines.insert(key, input.lines.len());
        input.lines.push(docair_layout::Line {
            id: format!("a{}", l.id),
            from: parts[&a.input.parts[l.from].id],
            to: parts[&a.input.parts[l.to].id],
            ..l.clone()
        });
    }
    Union {
        input,
        parts,
        lines,
    }
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

/// One version's layout, taken from the two laid out as one.
fn project(v: &VersionSlice, all: &Layout, u: &Union) -> Layout {
    let mut placement = all.placement.clone();
    let at: Vec<usize> = v.input.parts.iter().map(|p| u.parts[&p.id]).collect();
    placement.parts = at.iter().map(|&i| all.placement.parts[i]).collect();
    placement.scale = at.iter().map(|&i| all.placement.scale[i]).collect();
    let lines: Vec<usize> = v
        .input
        .lines
        .iter()
        .map(|l| u.lines[&line_key(v, l)])
        .collect();
    Layout {
        placement,
        routes: Routes {
            lines: lines.iter().map(|&i| all.routes.lines[i].clone()).collect(),
            price: all.routes.price,
            none: all.routes.none,
        },
        words: lines.iter().map(|&i| all.words[i]).collect(),
    }
}

/// The two versions' layouts: laid out as one when the flow is in both, else the one on its own.
fn lay_out_pair(
    a: Option<&VersionSlice>,
    b: Option<&VersionSlice>,
    keep: Option<&Path>,
) -> (Option<Layout>, Option<Layout>) {
    let alone = |v: &VersionSlice| lay_out_kept(&v.input, &layout_input::options_for(&v.pic), keep);
    match (a, b) {
        (Some(a), Some(b)) => {
            let u = union(a, b);
            let all = lay_out_kept(&u.input, &layout_input::options_for(&b.pic), keep);
            (Some(project(a, &all, &u)), Some(project(b, &all, &u)))
        }
        (a, b) => (a.map(alone), b.map(alone)),
    }
}

/// A drawn picture's viewBox, and what it holds inside its outer element.
fn inside(svg: &str) -> Option<([f64; 4], &str)> {
    let vb: Vec<f64> = svg
        .split("viewBox=\"")
        .nth(1)?
        .split('"')
        .next()?
        .split(' ')
        .filter_map(|x| x.parse().ok())
        .collect();
    let start = svg.find('>')? + 1;
    let end = svg.rfind("</svg>")?;
    (vb.len() == 4 && start <= end).then(|| ([vb[0], vb[1], vb[2], vb[3]], &svg[start..end]))
}

/// The two versions side by side, each under its title; a side the flow is missing from says so.
fn side_by_side(sides: [(String, Option<String>, &str); 2]) -> String {
    let (mut x, mut height, mut body) = (0.0_f64, 0.0_f64, String::new());
    for (title, drawn, missing) in sides {
        let (w, h) = match drawn.as_deref().and_then(inside) {
            Some(([vx, vy, w, h], held)) => {
                let _ = write!(
                    body,
                    r#"<svg x="{}" y="{BAND}" width="{}" height="{}" viewBox="{} {} {} {}" overflow="visible">{held}</svg>"#,
                    num(x),
                    num(w),
                    num(h),
                    num(vx),
                    num(vy),
                    num(w),
                    num(h)
                );
                (w, h)
            }
            None => {
                let (w, h) = (280.0, 120.0);
                let _ = write!(
                    body,
                    r#"<g class="side-missing"><rect x="{}" y="{BAND}" width="{w}" height="{h}" rx="8"/><text x="{}" y="{}">{}</text></g>"#,
                    num(x),
                    num(x + w / 2.0),
                    num(BAND + h / 2.0),
                    escape_text(missing)
                );
                (w, h)
            }
        };
        // Each side's title stands over its middle.
        let _ = write!(
            body,
            r#"<text class="side-title" x="{}" y="28" text-anchor="middle">{}</text>"#,
            num(x + w / 2.0),
            escape_text(&title)
        );
        x += w + GAP;
        height = height.max(h);
    }
    let (w, h) = (x - GAP, height + BAND);
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" class="docair-pic changes" viewBox="0 0 {} {}" width="{}" height="{}">{body}</svg>"#,
        num(w),
        num(h),
        num(w),
        num(h)
    )
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
 * The changes between two versions as the page shows them: the header's button, the list of changed flows with the moves apart, and for each flow its slice at A and at B side by side, marked.
 *
 * @calls docair-core.slice.find "finds each changed flow's seed in each version"
 * @calls docair-viewer.svg.draw_marked "draws each side with what changed marked"
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
        let _ = write!(
            list,
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
        if !gained.is_empty() {
            let _ = write!(
                list,
                r#"<div class="muted">{}</div>"#,
                escape_text(&gained.join(" · "))
            );
        }
        list.push_str("</li>");
        // The picture: A with what the flow no longer goes through marked, B with what it now does.
        let set = |v: &[String]| v.iter().cloned().collect::<HashSet<String>>();
        let (lost, lost_lines, won, won_lines) = (
            set(&f.parts_removed),
            set(&f.lines_removed),
            set(&f.parts_added),
            set(&f.lines_added),
        );
        let (va, vb) = (
            version_slice(changed.before, &f.seed),
            version_slice(after, &f.seed),
        );
        let (la, lb) = lay_out_pair(va.as_ref(), vb.as_ref(), keep);
        // Each side as the walks need it too: its picture, its layout and its drawing.
        let drawn = |v: VersionSlice, l: Layout, marks: ChangeMarks| {
            let svg = svg::draw_marked(v.graph, &v.pic, &l, &marks);
            (v.pic, l, svg)
        };
        let before = va.zip(la).map(|(v, l)| {
            drawn(
                v,
                l,
                ChangeMarks {
                    class: "ch-removed",
                    parts: &lost,
                    lines: &lost_lines,
                },
            )
        });
        let now = vb.zip(lb).map(|(v, l)| {
            drawn(
                v,
                l,
                ChangeMarks {
                    class: "ch-added",
                    parts: &won,
                    lines: &won_lines,
                },
            )
        });
        let svg = side_by_side([
            (
                format!("Before · {a}"),
                before.as_ref().map(|s| s.2.clone()),
                "Not there before",
            ),
            (
                format!("After · {b}"),
                now.as_ref().map(|s| s.2.clone()),
                "Gone after",
            ),
        ]);
        let pair = format!("{title}: before and after");
        pictures.push(ChangePicture {
            addr: addr.clone(),
            title: pair.clone(),
            crumbs: vec![(pair.clone(), addr.clone())],
            svg,
            change: Some(f.seed.clone()),
            walk: None,
        });
        // What the model explained about it, and the workflow before and after, each walked through on its side.
        let found = finding(changed.findings, &f.seed);
        let mut explained = String::new();
        if let Some(x) = found {
            let _ = write!(
                explained,
                r#"<h4>✦ Explained</h4><p data-test="explanation">{}</p><div class="muted">{}</div>"#,
                with_code(x["explanation"].as_str().unwrap_or("")),
                escape_text(&recorded(x))
            );
            let wf = &x["workflow"];
            let wf_title = wf["title"].as_str().unwrap_or("The workflow");
            let mut plays = Vec::new();
            for (which, drawn, graph) in
                [("before", &before, changed.before), ("after", &now, after)]
            {
                let steps = wf[which].as_array().map_or(&[][..], Vec::as_slice);
                let Some((pic, laid, svg)) = drawn.as_ref().filter(|_| !steps.is_empty()) else {
                    continue;
                };
                let walk_addr = format!("{addr}/{which}");
                plays.push(format!(
                    r#"<a class="link" href="{}" data-test="workflow-{which}">▶ {which} · {} {}</a>"#,
                    href(&walk_addr),
                    steps.len(),
                    if steps.len() == 1 { "step" } else { "steps" }
                ));
                pictures.push(ChangePicture {
                    addr: walk_addr.clone(),
                    title: format!("{wf_title} · {which}"),
                    crumbs: vec![
                        (pair.clone(), addr.clone()),
                        (format!("{wf_title} · {which}"), walk_addr),
                    ],
                    svg: svg.clone(),
                    change: None,
                    walk: Some(workflow_walk(
                        graph,
                        pic,
                        laid,
                        (wf_title, which),
                        steps,
                        &linker,
                    )),
                });
            }
            if !plays.is_empty() {
                let _ = write!(
                    explained,
                    r#"<h4>{}</h4><p data-test="workflow">{}</p>"#,
                    escape_text(wf_title),
                    plays.join(" · ")
                );
            }
        }
        // Its details, shown with its picture.
        let mut d = format!(
            r#"<h2>{}</h2><div class="meta">{} · from <code>{}</code> to <code>{}</code>{uncertain}</div><p>Left, the flow before; right, after. What it now goes through is marked green, what it no longer does red and dashed.</p>{explained}"#,
            escape_text(&title),
            how(f),
            escape_text(a),
            escape_text(b)
        );
        for (head, ids, graph, linked) in [
            ("Now goes through", &f.parts_added, after, true),
            (
                "No longer goes through",
                &f.parts_removed,
                changed.before,
                false,
            ),
            ("The changed parts on it", &f.through, after, true),
        ] {
            if !ids.is_empty() {
                let _ = write!(d, "<h4>{head}</h4>{}", parts_list(graph, ids, linked));
            }
        }
        d.push_str(r#"<p><a class="link" data-changes-list>← All changes</a></p>"#);
        details.insert(f.seed.clone(), Value::String(linker.link(&d, None)));
    }
    list.push_str("</ul>");
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
    use super::*;

    /// docair-zrr.17: the two versions are laid out as one, so every part both have is in the same
    /// place on both sides; a part only the new version has leaves its room empty in the old one.
    #[test]
    fn a_part_that_stayed_keeps_its_place() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../e2e/fixtures/shop");
        let after = docair_core::scan(
            &root,
            docair_adapters::all(),
            &docair_core::Options::default(),
        )
        .unwrap();
        let mut before = after.clone();
        before
            .edges
            .retain(|e| !(e.from == "orders.create" && e.to == "orders-db"));
        let (a, b) = (
            version_slice(&before, "data.order").unwrap(),
            version_slice(&after, "data.order").unwrap(),
        );
        let has = |v: &VersionSlice, id: &str| v.input.parts.iter().any(|p| p.id == id);
        assert!(
            has(&b, "orders-db") && !has(&a, "orders-db"),
            "only B writes the order"
        );
        let (la, lb) = lay_out_pair(Some(&a), Some(&b), None);
        let (la, lb) = (la.unwrap(), lb.unwrap());
        let place = |v: &VersionSlice, l: &Layout, id: &str| {
            let i = v.input.parts.iter().position(|p| p.id == id).unwrap();
            l.placement.parts[i]
        };
        let mut both = 0;
        for p in &a.input.parts {
            if has(&b, &p.id) {
                both += 1;
                assert_eq!(
                    place(&a, &la, &p.id),
                    place(&b, &lb, &p.id),
                    "{} stays",
                    p.id
                );
            }
        }
        assert!(both >= 3, "the flow's other parts are on both sides");
        assert_eq!(
            (la.placement.width, la.placement.height),
            (lb.placement.width, lb.placement.height),
            "both sides are the same size"
        );
        assert_eq!(la.routes.lines.len(), a.input.lines.len());
        assert_eq!(lb.routes.lines.len(), b.input.lines.len());
    }
}

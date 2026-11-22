//! Walks in the documentation page (docs/SLICES.md §4, §13 choice 4): a scenario, a walkthrough
//! or a datum's way is one picture, its slice. Each step is the parts and lines to light, a box
//! for the camera, the tokens that run along its lines and a card that says what happens, all
//! written by the CLI; the page's script plays them.

use std::collections::HashMap;
use std::fmt::Write;

use docair_core::layer_view::LayerPicture;
use docair_core::model::{Graph, Node};
use docair_core::slice::{CarryWay, SeedKind, Slice, SliceSeed};
use docair_layout::{Layout, Monospace};
use serde_json::{Value, json};

use crate::escape_text;
use crate::page_glossary::Linker;
use crate::page_panels::href;

/// The mark of each kind of message in a step card (viewer.js `MSG_ICON`).
fn icon(kind: &str) -> &'static str {
    match kind {
        "return" => "↩",
        "read" => "⇠",
        "write" => "✎",
        "emit" | "consume" => "⇢",
        "create" => "✚",
        "cover" => "✓",
        "route" | "select" => "⇶",
        _ => "→",
    }
}

/// The mark of a walk: ▶ a scenario, ✦ a walkthrough, ◇ a datum's way.
fn glyph(kind: SeedKind) -> &'static str {
    match kind {
        SeedKind::Walkthrough => "✦",
        SeedKind::Data => "◇",
        _ => "▶",
    }
}

/// Whether a seed's slice is a walk: its steps are played.
pub(crate) fn is_walk(seed: &SliceSeed) -> bool {
    matches!(
        seed.kind,
        SeedKind::Flow | SeedKind::Walkthrough | SeedKind::Data
    )
}

/// Text with `code` spans, as HTML.
fn rich(text: &str) -> String {
    text.split('`')
        .enumerate()
        .map(|(i, part)| {
            if i % 2 == 1 {
                format!("<code>{}</code>", escape_text(part))
            } else {
                escape_text(part)
            }
        })
        .collect()
}

/// The part a walk's id stands for in the picture: itself, else the nearest box around it that is drawn.
fn drawn_as(graph: &Graph, at: &HashMap<&str, usize>, id: &str) -> Option<usize> {
    let mut cur = Some(id);
    let mut hops = 0;
    while let Some(c) = cur {
        if let Some(&i) = at.get(c) {
            return Some(i);
        }
        cur = graph
            .nodes
            .iter()
            .find(|n| n.id == c)
            .and_then(|n| n.part_of.as_deref());
        hops += 1;
        if hops > 64 {
            break;
        }
    }
    None
}

/***
 * The hint a walk gives its layout (viewer.js `flowHints`): a line points ahead when it reaches a part the walk has not been to yet, and gives way when it goes back to an earlier one.
 *
 * Returns, for the picture's lines the walk goes along, how much each points ahead.
 ***/
pub(crate) fn walked(graph: &Graph, pic: &LayerPicture, slice: &Slice) -> HashMap<usize, u32> {
    let at: HashMap<&str, usize> = pic
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.as_str(), i))
        .collect();
    let line_of: HashMap<usize, usize> = pic
        .lines
        .iter()
        .enumerate()
        .flat_map(|(k, l)| l.members.iter().map(move |&m| (m, k)))
        .collect();
    let mut seen: HashMap<usize, usize> = HashMap::new();
    let mut out: HashMap<usize, u32> = HashMap::new();
    for (k, st) in slice.steps.iter().enumerate() {
        for &i in &st.lines {
            let Some(e) = graph.edges.get(i) else {
                continue;
            };
            let (a, b) = (drawn_as(graph, &at, &e.from), drawn_as(graph, &at, &e.to));
            if let Some(a) = a {
                seen.entry(a).or_insert(k);
            }
            let back = match (a, b) {
                (Some(a), Some(b)) => seen
                    .get(&b)
                    .is_some_and(|&sb| sb < seen.get(&a).copied().unwrap_or(k)),
                (None, Some(b)) => seen.get(&b).is_some_and(|&sb| sb < k),
                _ => false,
            };
            if let Some(b) = b {
                seen.entry(b).or_insert(k);
            }
            if let Some(&line) = line_of.get(&i) {
                out.entry(line).or_insert(if back { 0 } else { 20 });
            }
        }
        for id in &st.nodes {
            if let Some(i) = drawn_as(graph, &at, id) {
                seen.entry(i).or_insert(k);
            }
        }
    }
    out
}

/// What a walk is called, what it is about, and its steps' own words where it has them.
struct WalkWords {
    title: String,
    question: String,
    summary: String,
    foot: String,
    /// Each step's title and text.
    steps: Vec<(String, String)>,
}

fn walk_words(
    graph: &Graph,
    nodes: &HashMap<&str, &Node>,
    seed: &SliceSeed,
    slice: &Slice,
    messages: &[Vec<docair_core::slice::StepMessage>],
) -> WalkWords {
    let name = |id: &str| nodes.get(id).map_or(id.to_string(), |n| n.name.clone());
    match seed.kind {
        SeedKind::Walkthrough => {
            let w = graph.walkthroughs.iter().find(|w| w.id == seed.id);
            let own: Vec<(String, String)> = w
                .map(|w| {
                    w.steps
                        .iter()
                        .map(|s| (s.title.clone(), s.text.clone()))
                        .collect()
                })
                .unwrap_or_default();
            let steps = slice
                .steps
                .iter()
                .enumerate()
                .map(
                    |(k, st)| match own.get(k).filter(|_| own.len() == slice.steps.len()) {
                        Some((t, x)) => (t.clone(), x.clone()),
                        None => (st.label.clone().unwrap_or_default(), String::new()),
                    },
                )
                .collect();
            let foot = w
                .map(|w| {
                    // Who wrote it, when the AI did; a walkthrough a person wrote has no label
                    // (docair-8l6.86).
                    let by = match &w.recorded {
                        Some(r) if r.by == "human" => None,
                        Some(r) => Some(format!(
                            "written by {}",
                            r.model.as_deref().unwrap_or("an AI model")
                        )),
                        None => Some("written by an AI model".to_string()),
                    };
                    let when = w
                        .recorded
                        .as_ref()
                        .map(|r| r.at.chars().take(10).collect::<String>());
                    let file = Some(w.file.clone().unwrap_or_else(|| "not recorded yet".into()));
                    [by, when, file]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" · ")
                })
                .unwrap_or_default();
            WalkWords {
                title: w.map_or(seed.id.clone(), |w| {
                    if w.title.is_empty() {
                        w.id.clone()
                    } else {
                        w.title.clone()
                    }
                }),
                question: w.map(|w| w.question.clone()).unwrap_or_default(),
                summary: w.map(|w| w.summary.clone()).unwrap_or_default(),
                foot,
                steps,
            }
        }
        SeedKind::Data => {
            let dn = format!("◇ {}", name(&seed.id));
            let steps = slice
                .steps
                .iter()
                .enumerate()
                .map(|(k, st)| {
                    if st.made {
                        let maker = st.nodes.first().map(|m| name(m)).unwrap_or_default();
                        return (format!("{maker} makes {dn}"), String::new());
                    }
                    let first = messages.get(k).and_then(|m| m.first());
                    let title = first
                        .map(|m| m.who.join(" "))
                        .unwrap_or_else(|| st.label.clone().unwrap_or_default());
                    let back = st.way == Some(CarryWay::Back);
                    let how = if back {
                        format!("brings {dn} back")
                    } else if st.inside {
                        format!("carries {dn} along")
                    } else {
                        first.map(|m| m.carry.clone()).unwrap_or_default()
                    };
                    let note = if st.apart {
                        format!("Not reached from where {dn} is made.")
                    } else {
                        String::new()
                    };
                    let text = [note, how]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    (title, text)
                })
                .collect();
            WalkWords {
                title: format!("Follow {}", name(&seed.id)),
                question: String::new(),
                summary: nodes
                    .get(seed.id.as_str())
                    .map(|n| n.summary.clone())
                    .unwrap_or_default(),
                foot: String::new(),
                steps,
            }
        }
        _ => {
            let flow = graph.flows.iter().find(|f| f.name == seed.id);
            let steps = slice
                .steps
                .iter()
                .map(|st| {
                    (
                        st.label.clone().unwrap_or_else(|| {
                            st.nodes
                                .iter()
                                .map(|n| name(n))
                                .collect::<Vec<_>>()
                                .join(", ")
                        }),
                        String::new(),
                    )
                })
                .collect();
            WalkWords {
                title: flow.map_or(seed.id.clone(), |f| f.name.clone()),
                question: String::new(),
                summary: String::new(),
                foot: String::new(),
                steps,
            }
        }
    }
}

/// "Carried by 2 lines, all in this walk.": the lines that carry a datum, the ones its walk does
/// not go along, and the stretches of their own.
fn out_carried(card: &mut String, graph: &Graph, seed: &SliceSeed, slice: &Slice) {
    let walked: std::collections::HashSet<usize> = slice
        .steps
        .iter()
        .flat_map(|s| s.lines.iter().copied())
        .collect();
    let carrying: Vec<usize> = graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| e.carries.contains(&seed.id) || e.carries_back.contains(&seed.id))
        .map(|(i, _)| i)
        .collect();
    if carrying.is_empty() {
        return;
    }
    let count = |n: usize, word: &str| {
        if n == 1 {
            format!("1 {word}")
        } else {
            format!("{n} {word}s")
        }
    };
    let missing = carrying.iter().filter(|i| !walked.contains(i)).count();
    let apart = slice.steps.iter().filter(|s| s.apart).count();
    let mut said = format!("Carried by {}", count(carrying.len(), "line"));
    if missing > 0 {
        let _ = write!(
            said,
            "; {missing} of them {} not in this walk.",
            if missing == 1 { "is" } else { "are" }
        );
    } else {
        said.push_str(if carrying.len() > 1 {
            ", all in this walk"
        } else {
            ", in this walk"
        });
        if apart > 0 {
            let _ = write!(
                said,
                "; {apart} of them {}, not reached from where it is made",
                if apart == 1 {
                    "starts a stretch of its own"
                } else {
                    "start stretches of their own"
                }
            );
        }
        said.push('.');
    }
    let _ = write!(
        card,
        r#"<div class="card-carried" data-test="carried"><p>{}</p></div>"#,
        escape_text(&said)
    );
}

/***
 * A walk's steps, worked out against its layout: the parts and lines each step lights, the box the camera shows, the tokens on its lines, and its card.
 *
 * @calls docair-core.slice.messages "works out what each step's lines say"
 ***/
pub(crate) fn steps(
    graph: &Graph,
    pic: &LayerPicture,
    layout: &Layout,
    seed: &SliceSeed,
    slice: &Slice,
    linker: &Linker,
) -> Value {
    let nodes: HashMap<&str, &Node> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let at: HashMap<&str, usize> = pic
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.as_str(), i))
        .collect();
    let line_of: HashMap<usize, usize> = pic
        .lines
        .iter()
        .enumerate()
        .flat_map(|(k, l)| l.members.iter().map(move |&m| (m, k)))
        .collect();
    // The layout's lines are the picture's lines it was given: from a picture's line to its route.
    let input = crate::layout_input::input(graph, pic, &Monospace::default());
    let route_of: HashMap<usize, usize> = input
        .lines
        .iter()
        .enumerate()
        .filter_map(|(j, l)| l.id.parse::<usize>().ok().map(|k| (k, j)))
        .collect();
    let messages = docair_core::slice::messages(graph, slice);
    let told = walk_words(graph, &nodes, seed, slice, &messages);
    let n = slice.steps.len();
    let mut out = Vec::new();
    for (k, st) in slice.steps.iter().enumerate() {
        let mut parts: Vec<usize> = st
            .nodes
            .iter()
            .filter_map(|id| drawn_as(graph, &at, id))
            .collect();
        let mut lines: Vec<usize> = Vec::new();
        for &i in &st.lines {
            if let Some(&l) = line_of.get(&i)
                && !lines.contains(&l)
            {
                lines.push(l);
            }
            if let Some(e) = graph.edges.get(i) {
                parts.extend(drawn_as(graph, &at, &e.from));
                parts.extend(drawn_as(graph, &at, &e.to));
            }
        }
        parts.sort_unstable();
        parts.dedup();
        // The camera's box: the step's parts and lines, and some room.
        let (mut x0, mut y0, mut x1, mut y1) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        let mut take = |x: f64, y: f64| {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        };
        for &i in &parts {
            if let Some(r) = layout.placement.parts.get(i) {
                take(r.x - r.w / 2.0, r.y - r.h / 2.0);
                take(r.x + r.w / 2.0, r.y + r.h / 2.0);
            }
        }
        for l in &lines {
            for p in route_of
                .get(l)
                .and_then(|&j| layout.routes.lines.get(j))
                .into_iter()
                .flatten()
            {
                take(p.0, p.1);
            }
        }
        let camera = if x0 <= x1 {
            json!([x0 - 40.0, y0 - 40.0, x1 - x0 + 80.0, y1 - y0 + 80.0])
        } else {
            Value::Null
        };
        let msgs = messages.get(k).cloned().unwrap_or_default();
        let tokens: Vec<Value> = msgs
            .iter()
            .filter_map(|m| {
                line_of
                    .get(&m.edge)
                    .map(|&l| json!({ "line": l, "back": m.kind == "return", "word": m.token }))
            })
            .collect();
        // The card.
        let (title, text) = told.steps.get(k).cloned().unwrap_or_default();
        let mut card = format!(
            r#"<section class="card {}" data-test="step-card"><div class="card-head"><span class="card-glyph">{}</span><b>{}</b><button class="card-close" title="Close the walkthrough">×</button></div>"#,
            match seed.kind {
                SeedKind::Walkthrough => "walk",
                SeedKind::Data => "data",
                _ => "flow",
            },
            glyph(seed.kind),
            escape_text(&told.title)
        );
        if !told.question.is_empty() {
            let _ = write!(
                card,
                r#"<div class="card-q">asked: {}</div>"#,
                escape_text(&told.question)
            );
        }
        if k == 0 && !told.summary.is_empty() {
            let _ = write!(
                card,
                r#"<p class="card-summary">{}</p>"#,
                rich(&told.summary)
            );
        }
        // A datum's way: how many lines carry it, and how many of them the walk reaches (viewer.js carriedLines).
        if k == 0 && seed.kind == SeedKind::Data {
            out_carried(&mut card, graph, seed, slice);
        }
        let _ = write!(
            card,
            r#"<div class="card-step">step {} of {n}</div><h3>{}</h3>"#,
            k + 1,
            rich(&title)
        );
        for para in text.split("\n\n").filter(|p| !p.trim().is_empty()) {
            let _ = write!(card, "<p>{}</p>", rich(para.trim()));
        }
        card.push_str(r#"<div class="chips">"#);
        for id in st.nodes.iter().filter(|id| nodes.contains_key(id.as_str())) {
            let node = nodes[id.as_str()];
            let _ = write!(
                card,
                r#"<a class="chip" href="{}" data-part="{}" title="{}">{} {}</a>"#,
                href(&format!("part/{id}")),
                escape_text(id),
                escape_text(id),
                crate::page_panels::glyph_of(&node.kind),
                escape_text(&node.name)
            );
        }
        card.push_str("</div>");
        if !msgs.is_empty() {
            card.push_str(r#"<div class="hops-box"><div class="card-sub">What happens</div><ol class="hops" data-test="hops">"#);
            for m in &msgs {
                let why = if m.why.is_empty() {
                    String::new()
                } else {
                    format!(": {}", rich(&m.why))
                };
                let carry = if m.carry.is_empty() {
                    String::new()
                } else {
                    format!(
                        r#"<span class="hop-carry">{}</span>"#,
                        escape_text(&m.carry)
                    )
                };
                let (a, verb, b) = (
                    m.who.first().cloned().unwrap_or_default(),
                    m.who.get(1).cloned().unwrap_or_default(),
                    m.who.get(2).cloned().unwrap_or_default(),
                );
                let _ = write!(
                    card,
                    r#"<li class="hop {}" data-line="{}" title="Show this on the chart"><span class="hop-icon">{}</span><span class="hop-text"><b>{}</b> {} <b>{}</b>{why}{carry}</span></li>"#,
                    escape_text(&m.kind),
                    line_of
                        .get(&m.edge)
                        .map_or(String::new(), |l| l.to_string()),
                    icon(&m.kind),
                    escape_text(&a),
                    escape_text(&verb),
                    escape_text(&b)
                );
            }
            card.push_str("</ol>");
            let has: Vec<&String> = msgs.iter().filter_map(|m| m.has.as_ref()).collect();
            if !has.is_empty() {
                card.push_str(r#"<div class="hop-has" data-test="built-up"><span class="card-sub">Now it has</span><ul>"#);
                for h in has {
                    let _ = write!(card, "<li>{}</li>", escape_text(h));
                }
                card.push_str("</ul></div>");
            }
            card.push_str("</div>");
        }
        let _ = write!(
            card,
            r#"<div class="card-nav"><button class="btn ghost" title="Zoom out to everything this step touches" data-test="whole-step">⤢ Whole step</button><button class="btn" data-step="-1"{}>◀ Back</button><button class="btn" data-step="1"{}>Next ▶</button></div>"#,
            if k == 0 { " disabled" } else { "" },
            if k + 1 >= n { " disabled" } else { "" }
        );
        if !told.foot.is_empty() {
            let _ = write!(
                card,
                r#"<div class="card-foot">{}</div>"#,
                escape_text(&told.foot)
            );
        }
        card.push_str("</section>");
        out.push(json!({
            "card": linker.link(&card, None),
            "parts": parts.iter().map(|&i| pic.parts[i].id.clone()).collect::<Vec<_>>(),
            "lines": lines,
            "camera": camera,
            "tokens": tokens,
        }));
    }
    json!({ "title": told.title, "glyph": glyph(seed.kind), "steps": out })
}

/// The walks to start (docair-7h5): the header's *Walkthroughs* button and the panel it opens, and "Walk
/// me through" in the details before anything is selected. Empty when there is no walk.
pub(crate) struct WalkList {
    pub button: String,
    pub panel: String,
    pub tours: String,
}

pub(crate) fn walk_list(walks: &[(String, SeedKind, String, usize)]) -> WalkList {
    let mut list = String::new();
    let mut n = 0;
    for (addr, kind, title, steps) in walks.iter().filter(|w| w.1 != SeedKind::Data) {
        let _ = write!(
            list,
            r#"<a class="tour" href="{}" data-walk="{}"><span class="tour-title">{} {}</span><span class="stats">{steps} steps</span></a>"#,
            href(addr),
            escape_text(addr),
            glyph(*kind),
            escape_text(title)
        );
        n += 1;
    }
    if n == 0 {
        return WalkList {
            button: String::new(),
            panel: String::new(),
            tours: String::new(),
        };
    }
    WalkList {
        button: format!(
            r#"<button class="btn" id="btn-walks" data-test="walks-button" title="Walk through the scenarios and the recorded walkthroughs, step by step">▶ Walkthroughs ({n})</button>"#
        ),
        panel: format!(
            r#"<h2>▶ Walkthroughs</h2><div class="meta">{n} walks · the scenarios (▶) and the walkthroughs the AI recorded (✦), step by step</div><section class="tours" data-test="walks-list">{list}</section>"#
        ),
        tours: format!(r#"<section class="tours"><h4>Walk me through</h4>{list}</section>"#),
    }
}

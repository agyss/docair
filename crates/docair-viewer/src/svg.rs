//! Each picture drawn as one SVG (docs/SLICES.md §6): boxes with their names, parts in the shape
//! and colour of their kind, lines at right angles with their arrows and words, and the data a
//! line carries as chips in its words.
//!
//! The SVG holds classes, not colours: `assets/pictures.css` gives each class the page's colours
//! (viewer.css: `--k-*`, `--e-*`, `--t-*`), so one picture serves light and dark. Every part and
//! line carries its id (`data-id`) for the page's script. Where things go comes from
//! docair-layout, and the words from `layout_input`: the same words the layout made room for.
//! Nothing here measures or places anything.

use std::collections::HashMap;
use std::fmt::Write;

use docair_core::layer_view::{LayerLine, LayerPicture};
use docair_core::model::{EdgeKind, Graph, Node, Reach};
use docair_layout::text::{columns, wrap};
use docair_layout::{Layout, Monospace, Rect};

use crate::layout_input;

/// The stylesheet of the pictures' classes, in the page's colours.
pub const STYLE: &str = include_str!("../assets/pictures.css");

/// A part's words, in px at scale 1 (as layout_input sizes them).
const PART_FONT: f64 = 12.0;
/// A box's name, in the band at its top: its size, and the middle of the band below the box's top.
const BOX_FONT: f64 = 11.5;
const BOX_NAME_AT: f64 = 12.0;
/// A line's words: their size, the height of one line of them, the columns they wrap at
/// (as docair_layout::words_size sizes them).
const WORDS_FONT: f64 = 11.0;
const WORDS_LINE: f64 = 14.0;
const WORDS_WRAP: usize = 28;
/// How round a line's corners are, in px.
const CORNER: f64 = 6.0;
/// Room around the picture, in px.
const MARGIN: f64 = 16.0;

/// The kinds a part is drawn as (viewer.js `KINDS`); a part of another kind is drawn as `unknown`.
const KINDS: &[&str] = &[
    "system",
    "service",
    "module",
    "api",
    "component",
    "function",
    "endpoint",
    "store",
    "topic",
    "external",
    "job",
    "workload",
    "config",
    "host",
    "cluster",
    "namespace",
    "network",
    "gateway",
    "container",
    "release",
    "test",
    "data",
    "unresolved",
    "unknown",
];

/// The outline of a part (the node shapes viewer.js gives Cytoscape).
#[derive(Clone, Copy)]
enum PartShape {
    Rect,
    RoundRect,
    Ellipse,
    Barrel,
    /// Corners in a square from -1 to 1, scaled to the part.
    Polygon(&'static [f64]),
}

/// An OpenAPI or AsyncAPI description: a page with a folded corner.
const API: &[f64] = &[-1.0, -1.0, 0.78, -1.0, 1.0, -0.45, 1.0, 1.0, -1.0, 1.0];
const TAG: &[f64] = &[-1.0, -1.0, 0.25, -1.0, 1.0, 0.0, 0.25, 1.0, -1.0, 1.0];
const RHOMBOID: &[f64] = &[-1.0, -1.0, 0.333, -1.0, 1.0, 1.0, -0.333, 1.0];
const HEXAGON: &[f64] = &[
    -0.5, -1.0, -1.0, 0.0, -0.5, 1.0, 0.5, 1.0, 1.0, 0.0, 0.5, -1.0,
];
const OCTAGON: &[f64] = &[
    -0.414, -1.0, -1.0, -0.414, -1.0, 0.414, -0.414, 1.0, 0.414, 1.0, 1.0, 0.414, 1.0, -0.414,
    0.414, -1.0,
];
const CONCAVE_HEXAGON: &[f64] = &[
    -1.0, -0.95, -0.75, 0.0, -1.0, 0.95, 1.0, 0.95, 0.75, 0.0, 1.0, -0.95,
];

fn kind_of(kind: &str) -> &str {
    if KINDS.contains(&kind) {
        kind
    } else {
        "unknown"
    }
}

fn shape_of(kind: &str) -> PartShape {
    match kind {
        "api" => PartShape::Polygon(API),
        "endpoint" | "config" => PartShape::Polygon(TAG),
        "topic" => PartShape::Polygon(RHOMBOID),
        "workload" => PartShape::Polygon(HEXAGON),
        "job" => PartShape::Polygon(OCTAGON),
        "gateway" => PartShape::Polygon(CONCAVE_HEXAGON),
        "store" => PartShape::Barrel,
        "network" => PartShape::Ellipse,
        "component" | "external" | "host" | "unresolved" | "unknown" => PartShape::Rect,
        _ => PartShape::RoundRect,
    }
}

fn edge_kind(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Data => "data",
        EdgeKind::Call => "call",
        EdgeKind::Read => "read",
        EdgeKind::Write => "write",
        EdgeKind::Emit => "emit",
        EdgeKind::Consume => "consume",
        EdgeKind::Route => "route",
        EdgeKind::Select => "select",
        EdgeKind::Create => "create",
        EdgeKind::Cover => "cover",
    }
}

/// A number as the SVG writes it: to a tenth of a px.
fn num(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r == 0.0 {
        "0".to_string()
    } else if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r:.1}")
    }
}

/// Text as XML writes it, in an element or an attribute.
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// How round a rounded rectangle's corners are (Cytoscape's `round-rectangle`).
fn round_radius(w: f64, h: f64) -> f64 {
    (w / 4.0).min(h / 4.0).min(8.0)
}

/// The outline of a part or a box, centred on its place.
fn outline(out: &mut String, shape: PartShape, r: &Rect, class: &str) {
    let (x0, y0) = (r.x - r.w / 2.0, r.y - r.h / 2.0);
    let (x1, y1) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let _ = match shape {
        PartShape::Rect => write!(
            out,
            r#"<rect class="{class}" x="{}" y="{}" width="{}" height="{}"/>"#,
            num(x0),
            num(y0),
            num(r.w),
            num(r.h)
        ),
        PartShape::RoundRect => write!(
            out,
            r#"<rect class="{class}" x="{}" y="{}" width="{}" height="{}" rx="{}"/>"#,
            num(x0),
            num(y0),
            num(r.w),
            num(r.h),
            num(round_radius(r.w, r.h))
        ),
        PartShape::Ellipse => write!(
            out,
            r#"<ellipse class="{class}" cx="{}" cy="{}" rx="{}" ry="{}"/>"#,
            num(r.x),
            num(r.y),
            num(r.w / 2.0),
            num(r.h / 2.0)
        ),
        PartShape::Polygon(corners) => {
            let points: Vec<String> = corners
                .chunks(2)
                .map(|p| {
                    format!(
                        "{},{}",
                        num(r.x + p[0] * r.w / 2.0),
                        num(r.y + p[1] * r.h / 2.0)
                    )
                })
                .collect();
            write!(
                out,
                r#"<polygon class="{class}" points="{}"/>"#,
                points.join(" ")
            )
        }
        PartShape::Barrel => {
            // Cytoscape's barrel: the corners curve in, a little along the top and bottom.
            let (ho, wo) = ((0.05 * r.h).min(15.0), (0.25 * r.w).min(100.0));
            let c = 0.05 * wo;
            let p = |x: f64, y: f64| format!("{},{}", num(x), num(y));
            write!(
                out,
                r#"<path class="{class}" d="M{} L{} Q{} {} L{} Q{} {} L{} Q{} {} L{} Q{} {} Z"/>"#,
                p(x0, y0 + ho),
                p(x0, y1 - ho),
                p(x0 + c, y1),
                p(x0 + wo, y1),
                p(x1 - wo, y1),
                p(x1 - c, y1),
                p(x1, y1 - ho),
                p(x1, y0 + ho),
                p(x1 - c, y0),
                p(x1 - wo, y0),
                p(x0 + wo, y0),
                p(x0 + c, y0),
                p(x0, y0 + ho)
            )
        }
    };
}

/// How wide a line is drawn: a bundle by how many lines it stands for, data a little wider.
fn line_width(l: &LayerLine) -> f64 {
    if l.bundled {
        let n = l.members.len() as f64;
        (1.6 + (n - 1.0) / 7.0 * 2.9).clamp(1.6, 4.5)
    } else if l.kind == EdgeKind::Data {
        2.0
    } else {
        1.4
    }
}

/// How long a line's arrow is, for its width (Cytoscape's triangle, at `arrow-scale` 0.9).
fn arrow_length(width: f64) -> f64 {
    0.3 * 0.9 * (width * 13.37).powf(0.9).max(29.0)
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

/// A line's points as a path: straight pieces, rounded where they turn.
fn rounded(pts: &[(f64, f64)]) -> String {
    let mut d = format!("M{},{}", num(pts[0].0), num(pts[0].1));
    for i in 1..pts.len() {
        let b = pts[i];
        if i + 1 == pts.len() {
            let _ = write!(d, " L{},{}", num(b.0), num(b.1));
            break;
        }
        let (a, c) = (pts[i - 1], pts[i + 1]);
        let (l1, l2) = (dist(a, b), dist(b, c));
        let u1 = ((b.0 - a.0) / l1, (b.1 - a.1) / l1);
        let u2 = ((c.0 - b.0) / l2, (c.1 - b.1) / l2);
        let r = CORNER.min(l1 / 2.0).min(l2 / 2.0);
        if (u1.0 * u2.1 - u1.1 * u2.0).abs() < 1e-6 || r < 0.5 {
            let _ = write!(d, " L{},{}", num(b.0), num(b.1));
        } else {
            let _ = write!(
                d,
                " L{},{} Q{},{} {},{}",
                num(b.0 - u1.0 * r),
                num(b.1 - u1.1 * r),
                num(b.0),
                num(b.1),
                num(b.0 + u2.0 * r),
                num(b.1 + u2.1 * r)
            );
        }
    }
    d
}

/// Which way a line reaches its part, when the code is not sure (docs/ADAPTERS.md §5.5).
fn reach_class(graph: &Graph, l: &LayerLine) -> &'static str {
    let reach: Vec<Option<Reach>> = l
        .members
        .iter()
        .filter_map(|&i| graph.edges.get(i))
        .map(|e| e.reach)
        .collect();
    if reach.is_empty() {
        ""
    } else if reach.iter().all(|r| *r == Some(Reach::OneOf)) {
        " reach-one-of"
    } else if reach.iter().all(|r| *r == Some(Reach::Could)) {
        " reach-could"
    } else {
        ""
    }
}

/// The words docair finds the data a line carries by, there and back (as core's
/// `find_carried_data`): each datum's name, the last part of its id and its `@carried-as` words,
/// in lower case; the longest first.
fn carried(graph: &Graph, nodes: &HashMap<&str, &Node>, l: &LayerLine) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for &i in l.members.iter().chain(l.says.iter().flatten()) {
        let Some(e) = graph.edges.get(i) else {
            continue;
        };
        for d in e.carries.iter().chain(&e.carries_back) {
            let Some(n) = nodes.get(d.as_str()) else {
                continue;
            };
            let mut of = vec![
                n.name.to_lowercase(),
                n.id.rsplit('.').next().unwrap_or(&n.id).to_lowercase(),
            ];
            for t in n.tags_named("carried-as") {
                of.extend(t.value.split(',').map(|w| w.trim().to_lowercase()));
            }
            for w in of {
                if !w.is_empty() && !words.contains(&w) {
                    words.push(w);
                }
            }
        }
    }
    words.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()).then(a.cmp(b)));
    words
}

/// Where data stand in one line of words, as docair finds them (glossary::occurs_word): one of a
/// datum's words, or its plural, as a whole word, in any case. Byte ranges, in order.
fn chips_in(line: &str, words: &[String]) -> Vec<(usize, usize)> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|&(_, c)| c.to_lowercase().next().unwrap_or(c))
        .collect();
    let byte = |i: usize| chars.get(i).map_or(line.len(), |&(b, _)| b);
    let is_word =
        |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '-');
    let mut taken: Vec<(usize, usize)> = Vec::new();
    for w in words {
        let w: Vec<char> = w.chars().collect();
        if w.is_empty() {
            continue;
        }
        let mut s = 0;
        while s + w.len() <= lower.len() {
            if lower[s..s + w.len()] == w[..] {
                let mut e = s + w.len();
                for plural in [&['e', 's'][..], &['s'][..]] {
                    if lower[e..].starts_with(plural) {
                        e += plural.len();
                        break;
                    }
                }
                let before = s.checked_sub(1).and_then(|b| lower.get(b));
                if !is_word(before)
                    && !is_word(lower.get(e))
                    && taken.iter().all(|&(a, b)| e <= a || s >= b)
                {
                    taken.push((s, e));
                    s = e;
                    continue;
                }
            }
            s += 1;
        }
    }
    taken.sort_unstable();
    taken.into_iter().map(|(s, e)| (byte(s), byte(e))).collect()
}

/// What a picture is drawn from: its layout, and the words the layout made room for.
struct SvgPicture<'a> {
    graph: &'a Graph,
    pic: &'a LayerPicture,
    layout: &'a Layout,
    input: docair_layout::Input,
    part_words: Vec<String>,
    line_words: Vec<String>,
    nodes: HashMap<&'a str, &'a Node>,
    is_box: Vec<bool>,
}

impl SvgPicture<'_> {
    fn scale(&self, i: usize) -> f64 {
        self.layout.placement.scale.get(i).copied().unwrap_or(1.0)
    }

    fn kind(&self, i: usize) -> &str {
        let id = self.pic.parts[i].id.as_str();
        kind_of(self.nodes.get(id).map_or("", |n| n.kind.as_str()))
    }

    /// The picture's extent: every part, line and word, and some room around them.
    fn extent(&self) -> (f64, f64, f64, f64) {
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
        let rects = self
            .layout
            .placement
            .parts
            .iter()
            .chain(self.layout.words.iter().flatten());
        for r in rects {
            take(r.x - r.w / 2.0, r.y - r.h / 2.0);
            take(r.x + r.w / 2.0, r.y + r.h / 2.0);
        }
        for p in self.layout.routes.lines.iter().flatten() {
            take(p.0, p.1);
        }
        if x0 > x1 {
            return (0.0, 0.0, 2.0 * MARGIN, 2.0 * MARGIN);
        }
        (
            x0 - MARGIN,
            y0 - MARGIN,
            x1 - x0 + 2.0 * MARGIN,
            y1 - y0 + 2.0 * MARGIN,
        )
    }

    fn open_tag(&self, out: &mut String, extent: (f64, f64, f64, f64)) {
        let (x, y, w, h) = extent;
        let placed = &self.layout.placement;
        let mut about = String::new();
        if let Some(key) = self
            .pic
            .layer
            .and_then(|k| serde_json::to_value(k).ok())
            .and_then(|v| v.as_str().map(str::to_string))
        {
            let _ = write!(about, r#" data-layer="{}""#, esc(&key));
        }
        if let Some(seed) = &self.pic.slice {
            let _ = write!(about, r#" data-slice="{}""#, esc(seed));
        }
        let direction = match placed.direction {
            docair_layout::Direction::Down => "down",
            docair_layout::Direction::Right => "right",
        };
        let _ = write!(
            out,
            r#"<svg xmlns="http://www.w3.org/2000/svg" class="docair-pic" viewBox="{} {} {} {}" width="{}" height="{}"{about} data-direction="{direction}" data-zoom="{:.2}">"#,
            num(x),
            num(y),
            num(w),
            num(h),
            num(w),
            num(h),
            placed.zoom
        );
    }

    /// An open box: its outline, and its name in the band at its top.
    fn draw_box(&self, out: &mut String, i: usize) {
        let p = &self.pic.parts[i];
        let r = &self.layout.placement.parts[i];
        let kind = self.kind(i);
        let sc = self.scale(i);
        let _ = write!(out, r#"<g class="box k-{kind}" data-id="{}">"#, esc(&p.id));
        outline(out, PartShape::RoundRect, r, "sh");
        if kind == "container" {
            outline(out, PartShape::RoundRect, r, "gap");
        }
        let _ = write!(
            out,
            r#"<text class="bn" x="{}" y="{}" font-size="{}">{}</text></g>"#,
            num(r.x),
            num(r.y - r.h / 2.0 + BOX_NAME_AT * sc),
            num(BOX_FONT * sc),
            esc(&self.part_words[i])
        );
    }

    /// A part: its outline in the shape of its kind, and its words.
    fn draw_part(&self, out: &mut String, i: usize) {
        let p = &self.pic.parts[i];
        let r = &self.layout.placement.parts[i];
        let kind = self.kind(i);
        let node = self.nodes.get(p.id.as_str());
        let mut class = format!("part k-{kind}");
        if p.closed {
            class.push_str(" closed");
        }
        if node.is_some_and(|n| n.kind == "test") && !p.closed {
            let status = node
                .and_then(|n| n.test.as_ref())
                .map_or("none", |t| t.status.as_str());
            let _ = write!(class, " t-{}", esc(status));
        }
        let _ = write!(out, r#"<g class="{class}" data-id="{}">"#, esc(&p.id));
        outline(out, shape_of(kind), r, "sh");
        if p.closed {
            outline(out, shape_of(kind), r, "gap");
        }
        let _ = write!(
            out,
            r#"<text x="{}" y="{}" font-size="{}">{}</text></g>"#,
            num(r.x),
            num(r.y),
            num(PART_FONT * self.scale(i)),
            esc(&self.part_words[i])
        );
    }

    /// A line: its path at right angles, rounded where it turns, and its arrow.
    fn draw_line(&self, out: &mut String, j: usize, k: usize) {
        let l = &self.pic.lines[k];
        let mut pts: Vec<(f64, f64)> = Vec::new();
        for &q in &self.layout.routes.lines[j] {
            if pts.last().is_none_or(|p| dist(*p, q) > 0.01) {
                pts.push(q);
            }
        }
        if pts.len() < 2 {
            return;
        }
        let width = line_width(l);
        let a = arrow_length(width);
        let tip = pts[pts.len() - 1];
        let prev = pts[pts.len() - 2];
        let len = dist(prev, tip);
        let u = ((tip.0 - prev.0) / len, (tip.1 - prev.1) / len);
        let base = (tip.0 - u.0 * a, tip.1 - u.1 * a);
        if len > a {
            // The line stops where its arrow starts.
            let last = pts.len() - 1;
            pts[last] = base;
        }
        let (vx, vy) = (-u.1 * a / 2.0, u.0 * a / 2.0);
        let mut class = format!("line e-{}", edge_kind(l.kind));
        if l.bundled {
            class.push_str(" bundle");
        }
        if l.dangling {
            class.push_str(" dangling");
        }
        class.push_str(reach_class(self.graph, l));
        let members: Vec<String> = l.members.iter().map(|m| m.to_string()).collect();
        let _ = write!(
            out,
            r#"<g class="{class}" data-id="{k}" data-from="{}" data-to="{}" data-edges="{}"><path class="ln" d="{}" stroke-width="{}"/><path class="ah" d="M{},{} L{},{} L{},{} Z"/></g>"#,
            esc(&l.from),
            esc(&l.to),
            members.join(" "),
            rounded(&pts),
            num(width),
            num(tip.0),
            num(tip.1),
            num(base.0 + vx),
            num(base.1 + vy),
            num(base.0 - vx),
            num(base.1 - vy)
        );
    }

    /// A line's words where the layout put them, the data it carries as chips.
    fn draw_words(&self, out: &mut String, j: usize, k: usize, at: &Rect) {
        let line = &self.input.lines[j];
        let l = &self.pic.lines[k];
        let f = self.scale(line.from).min(self.scale(line.to));
        let fs = WORDS_FONT * f;
        let advance = Monospace::default().advance * fs;
        let names = carried(self.graph, &self.nodes, l);
        let top = at.y - at.h / 2.0;
        let _ = write!(
            out,
            r#"<g class="words{}" data-line="{k}"><rect x="{}" y="{}" width="{}" height="{}" rx="{}"/>"#,
            if l.bundled { " bundle" } else { "" },
            num(at.x - at.w / 2.0),
            num(top),
            num(at.w),
            num(at.h),
            num(round_radius(at.w, at.h))
        );
        let mut text = String::new();
        for (i, words) in wrap(&self.line_words[k], WORDS_WRAP).iter().enumerate() {
            let cy = top + (1.5 + WORDS_LINE / 2.0 + WORDS_LINE * i as f64) * f;
            let chips = chips_in(words, &names);
            let left = at.x - columns(words) as f64 * advance / 2.0;
            for &(s, e) in &chips {
                let x = left + columns(&words[..s]) as f64 * advance;
                let w = columns(&words[s..e]) as f64 * advance;
                let _ = write!(
                    out,
                    r#"<rect class="chip" x="{}" y="{}" width="{}" height="{}" rx="{}"/>"#,
                    num(x - 2.0 * f),
                    num(cy - 0.65 * fs),
                    num(w + 4.0 * f),
                    num(1.3 * fs),
                    num(3.0 * f)
                );
            }
            let _ = write!(text, r#"<tspan x="{}" y="{}">"#, num(at.x), num(cy));
            let mut pos = 0;
            for &(s, e) in &chips {
                text.push_str(&esc(&words[pos..s]));
                let _ = write!(text, r#"<tspan class="d">{}</tspan>"#, esc(&words[s..e]));
                pos = e;
            }
            text.push_str(&esc(&words[pos..]));
            text.push_str("</tspan>");
        }
        let _ = write!(out, r#"<text font-size="{}">{text}</text></g>"#, num(fs));
    }

    fn draw(&self, standalone: bool) -> String {
        let extent = self.extent();
        let mut out = String::new();
        if standalone {
            out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        }
        self.open_tag(&mut out, extent);
        if standalone {
            // The page's colours, light and dark: viewer.css up to its first rule that is not a colour.
            let colours = crate::CSS.split("\n* {").next().unwrap_or_default();
            let _ = write!(
                out,
                "<style><![CDATA[\n{colours}\n{STYLE}]]></style><rect class=\"bg\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
                num(extent.0),
                num(extent.1),
                num(extent.2),
                num(extent.3)
            );
        }
        let n = self.pic.parts.len().min(self.layout.placement.parts.len());
        let depth = |mut i: usize| {
            let mut d = 0;
            while let Some(q) = self.input.parts[i].parent {
                d += 1;
                i = q;
                if d > n {
                    break;
                }
            }
            d
        };
        let mut boxes: Vec<usize> = (0..n).filter(|&i| self.is_box[i]).collect();
        boxes.sort_by_key(|&i| (depth(i), i));
        out.push_str(r#"<g class="boxes">"#);
        for i in boxes {
            self.draw_box(&mut out, i);
        }
        // The lines the layout was given, each with the index of the picture's line it draws.
        let lines: Vec<(usize, usize)> = self
            .input
            .lines
            .iter()
            .enumerate()
            .filter(|(j, _)| *j < self.layout.routes.lines.len())
            .filter_map(|(j, l)| Some((j, l.id.parse::<usize>().ok()?)))
            .filter(|(_, k)| *k < self.pic.lines.len())
            .collect();
        out.push_str(r#"</g><g class="lines">"#);
        for &(j, k) in &lines {
            self.draw_line(&mut out, j, k);
        }
        out.push_str(r#"</g><g class="parts">"#);
        for i in (0..n).filter(|&i| !self.is_box[i]) {
            self.draw_part(&mut out, i);
        }
        out.push_str(r#"</g><g class="labels">"#);
        for &(j, k) in &lines {
            if let Some(Some(at)) = self.layout.words.get(j) {
                self.draw_words(&mut out, j, k, at);
            }
        }
        out.push_str("</g></svg>\n");
        out
    }
}

impl<'a> SvgPicture<'a> {
    fn new(graph: &'a Graph, pic: &'a LayerPicture, layout: &'a Layout) -> Self {
        let input = layout_input::input(graph, pic, &Monospace::default());
        let (part_words, line_words) = layout_input::words(graph, pic);
        let mut is_box = vec![false; input.parts.len()];
        for p in &input.parts {
            if let Some(q) = p.parent.filter(|&q| q < is_box.len()) {
                is_box[q] = true;
            }
        }
        SvgPicture {
            graph,
            pic,
            layout,
            input,
            part_words,
            line_words,
            nodes: graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect(),
            is_box,
        }
    }
}

/***
 * Draw a laid-out picture as one SVG: its boxes, parts, lines and their words, as classes the page's stylesheet (`STYLE`) colours.
 *
 * `layout` is this picture's layout (`layout_input::lay_out`, or a kept one), so the words are those it made room for.
 *
 * @calls docair-viewer.layout_input.input "the lines the layout was given"
 * @calls docair-viewer.layout_input.words "the words of the parts and lines"
 ***/
pub fn draw(graph: &Graph, pic: &LayerPicture, layout: &Layout) -> String {
    SvgPicture::new(graph, pic, layout).draw(false)
}

/***
 * A picture as a file of its own: the SVG with the page's colours, light and dark, the pictures' stylesheet and a background.
 *
 * @calls docair-viewer.layout_input.input "the lines the layout was given"
 * @calls docair-viewer.layout_input.words "the words of the parts and lines"
 ***/
pub fn standalone(graph: &Graph, pic: &LayerPicture, layout: &Layout) -> String {
    SvgPicture::new(graph, pic, layout).draw(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_written_to_a_tenth() {
        assert_eq!(num(12.0), "12");
        assert_eq!(num(-0.04), "0");
        assert_eq!(num(12.345), "12.3");
        assert_eq!(num(-7.25), "-7.3");
    }

    #[test]
    fn data_are_found_as_docair_finds_them() {
        let words = vec!["orderline".to_string(), "order".to_string()];
        let line = "POST /api/orders (CreateOrder, OrderLine) → Order · order-id";
        let found: Vec<&str> = chips_in(line, &words)
            .iter()
            .map(|&(s, e)| &line[s..e])
            .collect();
        assert_eq!(found, ["orders", "OrderLine", "Order"]);
    }

    #[test]
    fn a_turn_is_rounded_and_a_straight_point_is_not() {
        let d = rounded(&[(0.0, 0.0), (0.0, 50.0), (40.0, 50.0)]);
        assert_eq!(d, "M0,0 L0,44 Q0,50 6,50 L40,50");
        let d = rounded(&[(0.0, 0.0), (0.0, 20.0), (0.0, 50.0)]);
        assert_eq!(d, "M0,0 L0,20 L0,50");
    }

    #[test]
    fn the_colours_come_from_viewer_css() {
        let colours = crate::CSS.split("\n* {").next().unwrap_or_default();
        assert!(colours.contains("--k-service-bg"));
        assert!(colours.contains("prefers-color-scheme: dark"));
        assert_eq!(colours.matches('{').count(), colours.matches('}').count());
        for k in KINDS.iter().filter(|k| **k != "data") {
            assert!(
                colours.contains(&format!("--k-{k}-bg")),
                "viewer.css has the colours of {k}"
            );
            assert!(
                STYLE.contains(&format!(".k-{k} ")),
                "pictures.css gives {k} its colours"
            );
        }
    }
}

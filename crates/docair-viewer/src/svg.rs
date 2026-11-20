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
const WORDS_WRAP: usize = docair_layout::text::WORDS_COLUMNS;
/// How large a crowded line's words must be on the screen, in px, before the page shows them.
const READABLE: f64 = 9.0;
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
#[derive(Clone, Copy, Debug, PartialEq)]
enum PartShape {
    Rect,
    RoundRect,
    /// Round ends: a datum, like the chips in a line's words.
    Pill,
    /// All four corners cut: a service.
    Cut,
    /// The top corners cut: a container.
    CutTop,
    /// A folder with its tab: a module.
    Folder,
    /// Only the top corners round: a host.
    TopRound,
    /// Only the bottom corners round: a test.
    BottomRound,
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
/// The wheel of Kubernetes: a cluster.
const HEPTAGON: &[f64] = &[
    0.0, -1.0, -0.8019, -0.6039, -1.0, 0.2862, -0.4451, 1.0, 0.4451, 1.0, 1.0, 0.2862, 0.8019,
    -0.6039,
];
/// Wider at the bottom: a config.
const TRAPEZOID: &[f64] = &[-0.82, -1.0, 0.82, -1.0, 1.0, 1.0, -1.0, 1.0];
/// A pennant, its point to the left: a release.
const PENNANT: &[f64] = &[-1.0, 0.0, -0.72, -1.0, 1.0, -1.0, 1.0, 1.0, -0.72, 1.0];
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

/// Each kind its own outline, so no two read alike (the user's ask, 2026-11-18); kinds that share
/// one differ by their dashes: a component is solid, outside parts and unresolved refs dashed.
fn shape_of(kind: &str) -> PartShape {
    match kind {
        "api" => PartShape::Polygon(API),
        "endpoint" => PartShape::Polygon(TAG),
        "config" => PartShape::Polygon(TRAPEZOID),
        "release" => PartShape::Polygon(PENNANT),
        "cluster" => PartShape::Polygon(HEPTAGON),
        "topic" => PartShape::Polygon(RHOMBOID),
        "workload" => PartShape::Polygon(HEXAGON),
        "job" => PartShape::Polygon(OCTAGON),
        "gateway" => PartShape::Polygon(CONCAVE_HEXAGON),
        "store" => PartShape::Barrel,
        "network" => PartShape::Ellipse,
        "service" => PartShape::Cut,
        "container" => PartShape::CutTop,
        "module" => PartShape::Folder,
        "data" => PartShape::Pill,
        "host" => PartShape::TopRound,
        "test" => PartShape::BottomRound,
        "component" | "external" | "unresolved" | "unknown" => PartShape::Rect,
        _ => PartShape::RoundRect,
    }
}

/// What a line of the graph goes by in the page: its id (docair-svh), or its place in the graph's
/// lines when the graph was made without ids.
pub(crate) fn edge_key(graph: &Graph, i: usize) -> String {
    match graph.edges.get(i) {
        Some(e) if !e.id.is_empty() => e.id.clone(),
        _ => i.to_string(),
    }
}

pub(crate) fn edge_kind(kind: EdgeKind) -> &'static str {
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
        EdgeKind::Use => "use",
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
        PartShape::Pill => write!(
            out,
            r#"<rect class="{class}" x="{}" y="{}" width="{}" height="{}" rx="{}"/>"#,
            num(x0),
            num(y0),
            num(r.w),
            num(r.h),
            num(r.h.min(r.w) / 2.0)
        ),
        PartShape::Cut | PartShape::CutTop | PartShape::Folder => {
            let c = (r.h * 0.3).min(r.w * 0.25);
            let pts: Vec<(f64, f64)> = match shape {
                PartShape::Cut => vec![
                    (x0 + c, y0),
                    (x1 - c, y0),
                    (x1, y0 + c),
                    (x1, y1 - c),
                    (x1 - c, y1),
                    (x0 + c, y1),
                    (x0, y1 - c),
                    (x0, y0 + c),
                ],
                PartShape::CutTop => vec![
                    (x0 + c, y0),
                    (x1 - c, y0),
                    (x1, y0 + c),
                    (x1, y1),
                    (x0, y1),
                    (x0, y0 + c),
                ],
                _ => {
                    // The tab on the left of the top, as wide as two fifths of the folder.
                    let (t, tab) = ((r.h * 0.22).min(6.0), (r.w * 0.4).min(30.0));
                    vec![
                        (x0, y0),
                        (x0 + tab - 3.0, y0),
                        (x0 + tab, y0 + t),
                        (x1, y0 + t),
                        (x1, y1),
                        (x0, y1),
                    ]
                }
            };
            let points: Vec<String> = pts
                .iter()
                .map(|(x, y)| format!("{},{}", num(*x), num(*y)))
                .collect();
            write!(
                out,
                r#"<polygon class="{class}" points="{}"/>"#,
                points.join(" ")
            )
        }
        PartShape::TopRound | PartShape::BottomRound => {
            let k = round_radius(r.w, r.h);
            let p = |x: f64, y: f64| format!("{},{}", num(x), num(y));
            let d = if shape == PartShape::TopRound {
                format!(
                    "M{} L{} Q{} {} L{} Q{} {} L{} Z",
                    p(x0, y1),
                    p(x0, y0 + k),
                    p(x0, y0),
                    p(x0 + k, y0),
                    p(x1 - k, y0),
                    p(x1, y0),
                    p(x1, y0 + k),
                    p(x1, y1)
                )
            } else {
                format!(
                    "M{} L{} L{} Q{} {} L{} Q{} {} Z",
                    p(x0, y0),
                    p(x1, y0),
                    p(x1, y1 - k),
                    p(x1, y1),
                    p(x1 - k, y1),
                    p(x0 + k, y1),
                    p(x0, y1),
                    p(x0, y1 - k)
                )
            };
            write!(out, r#"<path class="{class}" d="{d}"/>"#)
        }
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

/// A kind's mark for the legend: its shape and colours as the pictures draw them; `class` adds
/// a state (` closed`, ` t-passed`).
pub(crate) fn kind_swatch(kind: &str, class: &str) -> String {
    let kind = kind_of(kind);
    let r = Rect {
        x: 18.0,
        y: 10.0,
        w: 32.0,
        h: 16.0,
    };
    let mut out = format!(
        r#"<svg class="docair-pic sw-shape" viewBox="0 0 36 20" width="36" height="20" aria-hidden="true"><g class="part k-{kind}{class}">"#
    );
    outline(&mut out, shape_of(kind), &r, "sh");
    if class.contains("closed") {
        outline(&mut out, shape_of(kind), &r, "gap");
    }
    out.push_str("</g></svg>");
    out
}

/// A line's mark for the legend: the line style of its kind, `class` for a state (` bundle`, ` dangling`, ` reach-could`).
pub(crate) fn line_swatch(kind: EdgeKind, class: &str, width: f64) -> String {
    format!(
        r#"<svg class="docair-pic sw-line" viewBox="0 0 36 10" width="36" height="10" aria-hidden="true"><g class="line e-{}{class}"><path class="ln" d="M2,5 L34,5" stroke-width="{}"/></g></svg>"#,
        edge_kind(kind),
        num(width)
    )
}

/// A datum marked in a line's words, for the legend.
pub(crate) fn chip_swatch() -> String {
    r#"<svg class="docair-pic sw-chip" viewBox="0 0 36 14" width="36" height="14" aria-hidden="true"><g class="words"><rect class="chip" x="2" y="1" width="32" height="12" rx="3"/><text font-size="9"><tspan class="d" x="18" y="7">data</tspan></text></g></svg>"#.to_string()
}

/// Terraform's mark (its four slanted bars), in a 10.4 × 11.6 box.
const TF_MARK: &str = "M0 0L3.2 1.8V5.4L0 3.6Z M3.6 2L6.8 3.8V7.4L3.6 5.6Z M3.6 6.2L6.8 8V11.6L3.6 9.8Z M7.2 3.8L10.4 2V5.6L7.2 7.4Z";

/// The parts Terraform declares, by the file they come from (.tf), and every box that holds only such parts.
pub(crate) fn terraform_parts(graph: &Graph) -> std::collections::HashSet<&str> {
    let mut out: std::collections::HashSet<&str> = graph
        .nodes
        .iter()
        .filter(|n| n.source.as_ref().is_some_and(|s| s.file.ends_with(".tf")))
        .map(|n| n.id.as_str())
        .collect();
    // A box whose parts are all Terraform's is Terraform's too: one level at a time, a few levels up.
    for _ in 0..3 {
        let mut kids: HashMap<&str, Vec<&str>> = HashMap::new();
        for n in graph.nodes.iter().filter(|n| n.kind != "data") {
            if let Some(p) = n.part_of.as_deref() {
                kids.entry(p).or_default().push(n.id.as_str());
            }
        }
        let more: Vec<&str> = kids
            .iter()
            .filter(|(p, ks)| {
                **p != graph.system && !out.contains(*p) && ks.iter().all(|k| out.contains(k))
            })
            .map(|(p, _)| *p)
            .collect();
        if more.is_empty() {
            break;
        }
        out.extend(more);
    }
    out
}

/// Terraform's mark on a part's corner, as a badge.
fn tf_mark(out: &mut String, r: &Rect, sc: f64) {
    let (x, y) = (r.x - r.w / 2.0 - 4.0 * sc, r.y - r.h / 2.0 - 6.0 * sc);
    let _ = write!(
        out,
        r#"<g class="tf-mark" transform="translate({} {}) scale({})"><rect x="-1.8" y="-1.8" width="14" height="15.2" rx="3"/><path d="{TF_MARK}"/></g>"#,
        num(x),
        num(y),
        num((sc * 10.0).round() / 10.0)
    );
}

/// Terraform's mark for the legend.
pub(crate) fn tf_swatch() -> String {
    format!(
        r#"<svg class="docair-pic sw-mark" viewBox="-2 -2 15 16" width="18" height="19" aria-hidden="true"><g class="tf-mark"><rect x="-1.8" y="-1.8" width="14" height="15.2" rx="3"/><path d="{TF_MARK}"/></g></svg>"#
    )
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
/// in lower case, with the datum each names; the longest first.
fn carried(graph: &Graph, nodes: &HashMap<&str, &Node>, l: &LayerLine) -> Vec<(String, String)> {
    let mut words: Vec<(String, String)> = Vec::new();
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
                if !w.is_empty() && !words.iter().any(|(x, _)| *x == w) {
                    words.push((w, n.id.clone()));
                }
            }
        }
    }
    words.sort_by(|a, b| {
        b.0.chars()
            .count()
            .cmp(&a.0.chars().count())
            .then(a.0.cmp(&b.0))
    });
    words
}

/// Where data stand in one line of words, as docair finds them (glossary::occurs_word): one of a
/// datum's words, or its plural, as a whole word, in any case. Byte ranges and the datum, in order.
fn chips_in<'w>(line: &str, words: &'w [(String, String)]) -> Vec<(usize, usize, &'w str)> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|&(_, c)| c.to_lowercase().next().unwrap_or(c))
        .collect();
    let byte = |i: usize| chars.get(i).map_or(line.len(), |&(b, _)| b);
    let is_word =
        |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '-');
    let mut taken: Vec<(usize, usize, &str)> = Vec::new();
    for (w, id) in words {
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
                    && taken.iter().all(|&(a, b, _)| e <= a || s >= b)
                {
                    taken.push((s, e, id.as_str()));
                    s = e;
                    continue;
                }
            }
            s += 1;
        }
    }
    taken.sort_unstable();
    taken
        .into_iter()
        .map(|(s, e, id)| (byte(s), byte(e), id))
        .collect()
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
    /// The parts Terraform declares (a .tf file), and the boxes that hold only such parts.
    terraform: std::collections::HashSet<&'a str>,
    /// What this picture marks, as a change between two versions does.
    marks: Option<&'a ChangeMarks<'a>>,
}

/// Parts and lines a picture marks, each with its class, as a change between two versions does:
/// kept (`ch-same`), changed inside (`ch-changed`), only before (`ch-removed`), only after
/// (`ch-added`) (docair-zrr.7, docair-zrr.18; pictures.css draws them).
pub struct ChangeMarks<'a> {
    /// Each marked part's class, by its id.
    pub parts: &'a HashMap<String, &'a str>,
    /// Each marked graph line's class, by its id: a picture's line takes the class of the first
    /// of its lines that has one.
    pub lines: &'a HashMap<String, &'a str>,
    /// A word some parts and open boxes carry at their top right, by id: `new`, `gone`, `code`
    /// (docair-zrr.22).
    pub tags: &'a HashMap<String, &'a str>,
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
        let _ = write!(
            out,
            r#"<g class="box k-{kind}{}" data-id="{}">"#,
            self.mark(&p.id),
            esc(&p.id)
        );
        outline(out, PartShape::RoundRect, r, "sh");
        if kind == "container" {
            outline(out, PartShape::RoundRect, r, "gap");
        }
        let _ = write!(
            out,
            r#"<text class="bn" x="{}" y="{}" font-size="{}">{}</text>"#,
            num(r.x),
            num(r.y - r.h / 2.0 + BOX_NAME_AT * sc),
            num(BOX_FONT * sc),
            esc(&self.part_words[i])
        );
        if self.terraform.contains(p.id.as_str()) {
            tf_mark(out, r, sc);
        }
        self.tag(out, &p.id, r, sc);
        out.push_str("</g>");
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
        class.push_str(&self.mark(&p.id));
        let _ = write!(out, r#"<g class="{class}" data-id="{}">"#, esc(&p.id));
        outline(out, shape_of(kind), r, "sh");
        if p.closed {
            outline(out, shape_of(kind), r, "gap");
        }
        let _ = write!(
            out,
            r#"<text x="{}" y="{}" font-size="{}">{}</text>"#,
            num(r.x),
            num(r.y),
            num(PART_FONT * self.scale(i)),
            esc(&self.part_words[i])
        );
        if self.terraform.contains(p.id.as_str()) {
            tf_mark(out, r, self.scale(i));
        }
        self.tag(out, &p.id, r, self.scale(i));
        out.push_str("</g>");
    }

    /// A line: its path at right angles, rounded where it turns, and its arrow.
    fn draw_line(&self, out: &mut String, j: usize, k: usize) {
        let l = &self.pic.lines[k];
        let width = line_width(l);
        // Drawn as the page's move of a part draws it (docair-layout's draw, also in the WASM).
        let Some(drawn) = docair_layout::draw::line(&self.layout.routes.lines[j], width) else {
            return;
        };
        let mut class = format!("line e-{}", edge_kind(l.kind));
        if l.bundled {
            class.push_str(" bundle");
        }
        if l.dangling {
            class.push_str(" dangling");
        }
        class.push_str(reach_class(self.graph, l));
        if let Some(c) = self.marks.and_then(|m| {
            l.members
                .iter()
                .find_map(|&e| m.lines.get(&edge_key(self.graph, e)))
        }) {
            let _ = write!(class, " {c}");
        }
        // The graph's lines it stands for, by their ids (docair-svh); ids hold spaces, never a newline.
        let members: Vec<String> = l
            .members
            .iter()
            .map(|&m| esc(&edge_key(self.graph, m)))
            .collect();
        let _ = write!(
            out,
            r#"<g class="{class}" data-id="{k}" data-from="{}" data-to="{}" data-edges="{}"><path class="ln" d="{}" stroke-width="{}"/><path class="ah" d="{}"/></g>"#,
            esc(&l.from),
            esc(&l.to),
            members.join("&#10;"),
            drawn.d,
            num(width),
            drawn.arrow
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
        // Words that sit on another line are crowded: the page shows them once it is zoomed in
        // so far that they read, at READABLE px (docair-702.35); `data-z` is that zoom.
        let crowded = self.layout.routes.lines.iter().enumerate().any(|(o, r)| {
            o != j
                && r.windows(2).any(|s| {
                    s[0].0.min(s[1].0) < at.x + at.w / 2.0 - 1.0
                        && s[0].0.max(s[1].0) > at.x - at.w / 2.0 + 1.0
                        && s[0].1.min(s[1].1) < at.y + at.h / 2.0 - 1.0
                        && s[0].1.max(s[1].1) > at.y - at.h / 2.0 + 1.0
                })
        });
        let zoom = if crowded {
            format!(r#" data-z="{:.2}""#, READABLE / fs)
        } else {
            String::new()
        };
        let _ = write!(
            out,
            r#"<g class="words{}{}" data-line="{k}"{zoom}><rect x="{}" y="{}" width="{}" height="{}" rx="{}"/>"#,
            if l.bundled { " bundle" } else { "" },
            if crowded { " crowded" } else { "" },
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
            for &(s, e, id) in &chips {
                let x = left + columns(&words[..s]) as f64 * advance;
                let w = columns(&words[s..e]) as f64 * advance;
                let _ = write!(
                    out,
                    r#"<rect class="chip" data-data="{}" x="{}" y="{}" width="{}" height="{}" rx="{}"/>"#,
                    esc(id),
                    num(x - 2.0 * f),
                    num(cy - 0.65 * fs),
                    num(w + 4.0 * f),
                    num(1.3 * fs),
                    num(3.0 * f)
                );
            }
            let _ = write!(text, r#"<tspan x="{}" y="{}">"#, num(at.x), num(cy));
            let mut pos = 0;
            for &(s, e, id) in &chips {
                text.push_str(&esc(&words[pos..s]));
                let _ = write!(
                    text,
                    r#"<tspan class="d" data-data="{}">{}</tspan>"#,
                    esc(id),
                    esc(&words[s..e])
                );
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
            terraform: terraform_parts(graph),
            marks: None,
        }
    }

    /// A change's word on a part or an open box, at its top right, drawn at the part's own scale
    /// so it reads wherever the part does: `new`, `gone`, `code` (docair-zrr.22).
    fn tag(&self, out: &mut String, id: &str, r: &Rect, sc: f64) {
        let Some(word) = self.marks.and_then(|m| m.tags.get(id)) else {
            return;
        };
        let fs = 10.0 * sc;
        let w = columns(word) as f64 * Monospace::default().advance * fs + 8.0 * sc;
        let h = 13.0 * sc;
        let (x, y) = (r.x + r.w / 2.0 - w - 4.0 * sc, r.y - r.h / 2.0 - h / 2.0);
        let _ = write!(
            out,
            r#"<g class="ch-tag ch-tag-{}"><rect x="{}" y="{}" width="{}" height="{}" rx="{}"/><text x="{}" y="{}" font-size="{}">{}</text></g>"#,
            esc(word),
            num(x),
            num(y),
            num(w),
            num(h),
            num(h / 2.0),
            num(x + w / 2.0),
            num(y + h / 2.0),
            num(fs),
            esc(word)
        );
    }

    /// The class a marked part gets: ` ch-added`, or nothing.
    fn mark(&self, id: &str) -> String {
        self.marks
            .and_then(|m| m.parts.get(id))
            .map(|c| format!(" {c}"))
            .unwrap_or_default()
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

/// As `draw`, with some parts and lines marked: those a change added or removed (docair-zrr.7).
pub fn draw_marked(
    graph: &Graph,
    pic: &LayerPicture,
    layout: &Layout,
    marks: &ChangeMarks,
) -> String {
    let mut p = SvgPicture::new(graph, pic, layout);
    p.marks = Some(marks);
    p.draw(false)
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

    /// No two kinds look alike: their outlines differ, or their dashes (pictures.css) do.
    #[test]
    fn no_two_kinds_look_alike() {
        let dashes = |k: &str| match k {
            "external" | "namespace" => " dashed",
            "unresolved" => " dashed red",
            "unknown" => " dotted",
            _ => "",
        };
        let mut seen: HashMap<String, &str> = HashMap::new();
        for k in KINDS.iter().filter(|k| **k != "system") {
            let look = format!("{:?}{}", shape_of(k), dashes(k));
            if let Some(other) = seen.insert(look.clone(), k) {
                panic!("{k} and {other} look alike: {look}");
            }
        }
    }

    #[test]
    fn the_legend_draws_each_kind_in_its_shape() {
        for k in KINDS {
            let sw = kind_swatch(k, "");
            let mark = match shape_of(k) {
                PartShape::Rect | PartShape::RoundRect | PartShape::Pill => r#"<rect class="sh""#,
                PartShape::Ellipse => r#"<ellipse class="sh""#,
                PartShape::Barrel | PartShape::TopRound | PartShape::BottomRound => {
                    r#"<path class="sh""#
                }
                PartShape::Polygon(_) | PartShape::Cut | PartShape::CutTop | PartShape::Folder => {
                    r#"<polygon class="sh""#
                }
            };
            assert!(
                sw.contains(mark) && sw.contains(&format!("k-{k}")),
                "{k}: {sw}"
            );
        }
    }

    #[test]
    fn numbers_are_written_to_a_tenth() {
        assert_eq!(num(12.0), "12");
        assert_eq!(num(-0.04), "0");
        assert_eq!(num(12.345), "12.3");
        assert_eq!(num(-7.25), "-7.3");
    }

    #[test]
    fn data_are_found_as_docair_finds_them() {
        let words = vec![
            ("orderline".to_string(), "data.line".to_string()),
            ("order".to_string(), "data.order".to_string()),
        ];
        let line = "POST /api/orders (CreateOrder, OrderLine) → Order · order-id";
        let found: Vec<String> = chips_in(line, &words)
            .iter()
            .map(|&(s, e, id)| format!("{}={id}", &line[s..e]))
            .collect();
        assert_eq!(
            found,
            [
                "orders=data.order",
                "OrderLine=data.line",
                "Order=data.order"
            ]
        );
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

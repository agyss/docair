//! A layer's picture or a slice's, as input for docair-layout: the words of its parts and lines, their
//! sizes, and the hints that keep the order things happen in (docs/SLICES.md §5).
//!
//! The words are the ones viewer.js writes (`visibleElements`, `describeEdge`, `edgeText`,
//! `shortWords`, `viaWords`), set in one monospace font.

use std::collections::{HashMap, HashSet};

use docair_core::layer_view::{LayerLine, LayerPart, LayerPicture, route_path};
use docair_core::model::{Edge, EdgeKind, Graph, Node, Reach};
use docair_layout::{Input, Layout, Line, Monospace, Options, Part, Placement};

/// The most characters a line's words have in the picture.
const LABEL_MAX: usize = 48;
/// The size of a part's name, in px (a line's words: docair_layout::words_size).
const PART_FONT: f64 = 12.0;
/// The kinds of lines that say who depends on whom: the way things happen.
const AHEAD: &[EdgeKind] = &[
    EdgeKind::Call,
    EdgeKind::Route,
    EdgeKind::Select,
    EdgeKind::Write,
    EdgeKind::Create,
    EdgeKind::Emit,
    EdgeKind::Consume,
];

/// The graph, looked up by id.
pub(crate) struct GraphLookup<'g> {
    pub(crate) nodes: HashMap<&'g str, &'g Node>,
    pub(crate) edges: &'g [Edge],
}

impl<'g> GraphLookup<'g> {
    pub(crate) fn new(graph: &'g Graph) -> Self {
        GraphLookup {
            nodes: graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect(),
            edges: &graph.edges,
        }
    }

    pub(crate) fn name(&self, id: &str) -> Option<&'g str> {
        self.nodes.get(id).map(|n| n.name.as_str())
    }

    pub(crate) fn kind(&self, id: &str) -> &'g str {
        self.nodes.get(id).map_or("", |n| n.kind.as_str())
    }
}

fn verb(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Call => "calls",
        EdgeKind::Read => "reads",
        EdgeKind::Write => "writes",
        EdgeKind::Emit => "emits",
        EdgeKind::Consume => "consumes",
        EdgeKind::Data => "passes data",
        EdgeKind::Route => "routes",
        EdgeKind::Select => "selects",
        EdgeKind::Create => "creates",
        EdgeKind::Cover => "tests",
    }
}

/// `text` cut to `max` characters, with an ellipsis.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        let mut out: String = text.chars().take(max - 1).collect();
        out.push('…');
        out
    } else {
        text.to_string()
    }
}

/// At most `max` different items, the rest counted: `POST /api/ask · POST /api/word +3`.
pub(crate) fn summarize(items: &[String], max: usize) -> String {
    let mut unique: Vec<&str> = Vec::new();
    for i in items {
        if !unique.contains(&i.as_str()) {
            unique.push(i);
        }
    }
    if unique.len() > max {
        format!("{} +{}", unique[..max].join(" · "), unique.len() - max)
    } else {
        unique.join(" · ")
    }
}

/// What one line carries, in a few words: a route, a call, a payload, a file.
pub(crate) fn describe_edge(g: &GraphLookup, e: &Edge) -> String {
    if let Some(t) = &e.text {
        return t.clone();
    }
    if e.kind == EdgeKind::Cover {
        return format!("tests {}", g.name(&e.to).unwrap_or(&e.to));
    }
    let reply = e
        .reply
        .as_deref()
        .map(|r| format!(" → {r}"))
        .unwrap_or_default();
    if e.kind == EdgeKind::Call {
        if let Some(via) = &e.via {
            let via = via.strip_prefix("http:").unwrap_or(via);
            let payload = e
                .payload
                .as_deref()
                .map(|p| format!(" ({p})"))
                .unwrap_or_default();
            return format!("{via}{payload}{reply}");
        }
        if g.kind(&e.to) == "function" {
            return format!(
                "{}({}){reply}",
                g.name(&e.to).unwrap_or(&e.to),
                e.payload.as_deref().unwrap_or("")
            );
        }
        let what = e.payload.as_deref().or(g.name(&e.to)).unwrap_or(&e.to);
        return format!("{what}{reply}");
    }
    if let Some(l) = &e.label {
        return l.clone();
    }
    match e.kind {
        EdgeKind::Emit => g
            .name(&e.to)
            .filter(|n| !n.is_empty())
            .unwrap_or("emits")
            .to_string(),
        EdgeKind::Consume => g
            .name(&e.from)
            .filter(|n| !n.is_empty())
            .unwrap_or("consumes")
            .to_string(),
        EdgeKind::Create => format!("creates {}", g.name(&e.to).unwrap_or(&e.to)),
        k => verb(k).to_string(),
    }
}

/// The words of one line in the picture: a call to a function reads `(args) → result`, other hops say what travels.
pub(crate) fn edge_text(g: &GraphLookup, e: &Edge) -> String {
    if let Some(t) = &e.text {
        return t.clone();
    }
    if e.kind == EdgeKind::Call
        && g.kind(&e.to) != "function"
        && (e.payload.is_some() || e.reply.is_some())
    {
        let reply = e
            .reply
            .as_deref()
            .map(|r| format!(" → {r}"))
            .unwrap_or_default();
        return format!("{}{reply}", e.payload.as_deref().unwrap_or(""))
            .trim()
            .to_string();
    }
    e.label.clone().unwrap_or_else(|| verb(e.kind).to_string())
}

/// A line's words in a bundle: a request's method and path (`POST /api/ask`), else its words, kept short.
fn short_words(g: &GraphLookup, e: &Edge) -> String {
    if let Some(path) = route_path(e) {
        let said = format!(
            "{} {}",
            e.payload.as_deref().unwrap_or(""),
            e.label.as_deref().unwrap_or("")
        );
        let tokens: Vec<&str> = said
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .collect();
        let method = tokens
            .iter()
            .find(|t| ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"].contains(t))
            .or_else(|| tokens.iter().find(|t| **t == "HTTP"))
            .copied()
            .unwrap_or_default();
        return format!("{method} {path}").trim().to_string();
    }
    clip(&describe_edge(g, e), 30)
}

/// The parts a line passes, in a few words: its gateways when it has any, else every part.
fn via_words(g: &GraphLookup, ids: &[String]) -> String {
    let gates: Vec<&String> = ids.iter().filter(|id| g.kind(id) == "gateway").collect();
    let pick: Vec<&String> = if gates.is_empty() {
        ids.iter().collect()
    } else {
        gates
    };
    let names: Vec<&str> = pick.iter().map(|id| g.name(id).unwrap_or(id)).collect();
    if names.len() > 4 {
        let mut short = vec![names[0], "…"];
        short.extend(&names[names.len() - 2..]);
        short.join(" › ")
    } else {
        names.join(" › ")
    }
}

/// A box's name: `kind cluster`, `docair namespace`, `docair-core`.
pub(crate) fn group_label(n: &Node) -> String {
    let plain = n.name.contains(' ')
        || ![
            "namespace",
            "cluster",
            "network",
            "workload",
            "container",
            "host",
            "release",
        ]
        .contains(&n.kind.as_str());
    if plain {
        n.name.clone()
    } else {
        format!("{} {}", n.name, n.kind)
    }
}

/// A part's words in the picture: its name, a closed box's count, an open box's parts left out.
fn part_words(g: &GraphLookup, p: &LayerPart) -> String {
    let Some(n) = g.nodes.get(p.id.as_str()) else {
        return p.id.clone();
    };
    let group = p.open || p.closed;
    let mut name = if group {
        group_label(n)
    } else {
        n.name.clone()
    };
    if n.kind == "test" && !group {
        let icon = match n.test.as_ref().map(|t| t.status.as_str()) {
            Some("passed") => "✓",
            Some("failed") => "✗",
            Some("flaky") => "~",
            Some("skipped") => "–",
            _ => "○",
        };
        name = format!("{icon} {name}");
    }
    if p.closed {
        format!("{name} ({})", p.holds)
    } else if p.more > 0 {
        format!("{name} · {} more", p.more)
    } else {
        name
    }
}

/// How many parts a line's call reaches one of (docs/ADAPTERS.md §5.5): the lines of the graph
/// from the same call, each to one implementation; none unless every line it stands for is such a
/// call, and there are several.
fn one_of(g: &GraphLookup, edges: &[&Edge]) -> Option<usize> {
    let first = edges.first()?;
    if !edges.iter().all(|e| e.reach == Some(Reach::OneOf)) {
        return None;
    }
    let n = g
        .edges
        .iter()
        .filter(|e| {
            e.reach == Some(Reach::OneOf) && e.from == first.from && e.source == first.source
        })
        .count();
    (n > 1).then_some(n)
}

/// A line's words in the picture: what it carries, what it passes.
fn line_words(g: &GraphLookup, l: &LayerLine) -> String {
    let says = l.says.as_ref().unwrap_or(&l.members);
    let edges: Vec<&Edge> = l.members.iter().filter_map(|&i| g.edges.get(i)).collect();
    let tests = if !edges.is_empty() && edges.iter().all(|e| e.kind == EdgeKind::Cover) {
        edges
            .iter()
            .map(|e| e.from.as_str())
            .collect::<HashSet<_>>()
            .len()
    } else {
        0
    };
    // A line that stands for one test says what that test checks ("checks Cart"), or "1 test"
    // when its tag has no words; one for several tests counts them (docair-702.14).
    let what = if tests == 1 {
        edges
            .iter()
            .find_map(|e| e.text.clone().filter(|t| !t.trim().is_empty()))
            .unwrap_or_else(|| "1 test".to_string())
    } else if tests > 1 {
        format!("{tests} tests")
    } else if l.bundled {
        let items: Vec<String> = says
            .iter()
            .filter_map(|&i| g.edges.get(i))
            .map(|e| short_words(g, e))
            .collect();
        summarize(&items, 2)
    } else {
        let mut labels: Vec<String> = Vec::new();
        for e in says.iter().filter_map(|&i| g.edges.get(i)) {
            let t = edge_text(g, e);
            if !t.is_empty() && !labels.contains(&t) {
                labels.push(t);
            }
        }
        labels.join(", ")
    };
    // A call through a trait or an interface: exactly one of its implementations runs.
    let what = match one_of(g, &edges) {
        Some(n) if what == "calls" => format!("calls one of {n}"),
        Some(n) => format!("{what} · one of {n}"),
        None => what,
    };
    let what = clip(&what, LABEL_MAX);
    let all = if l.via.is_empty() {
        what
    } else {
        format!("{what} · via {}", via_words(g, &l.via))
    };
    // At most two lines in the picture; the whole words stay in the line's tip (docair-702.34).
    docair_layout::text::cut(&all, docair_layout::text::WORDS_COLUMNS, 2)
}

/// The size of a part of a kind, for its words.
fn part_size(font: &Monospace, kind: &str, label: &str) -> (f64, f64) {
    let extra = match kind {
        "topic" => 26.0,
        "endpoint" => 18.0,
        "workload" => 22.0,
        "config" => 20.0,
        "job" => 14.0,
        "store" => 8.0,
        "gateway" => 30.0,
        "network" => 22.0,
        "api" => 14.0,
        // Room for the words inside the outlines that cut or point in (svg.rs shape_of).
        "cluster" => 26.0,
        "release" => 18.0,
        "service" => 14.0,
        "data" => 14.0,
        "container" => 10.0,
        "module" => 6.0,
        _ => 0.0,
    };
    let w = (font.width(label, PART_FONT).ceil() + 26.0 + extra).max(56.0);
    (w, if kind == "store" { 40.0 } else { 30.0 })
}

/// For each line of the picture: whether it is laid out against its arrow (an answer along its call), and how much it points ahead.
fn hints(g: &GraphLookup, pic: &LayerPicture) -> Vec<(bool, u32)> {
    let parent: HashMap<&str, Option<&str>> = pic
        .parts
        .iter()
        .map(|p| (p.id.as_str(), p.parent.as_deref()))
        .collect();
    let up = |id: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = Some(id.to_string());
        while let Some(c) = cur {
            if out.contains(&c) {
                break;
            }
            cur = parent
                .get(c.as_str())
                .copied()
                .flatten()
                .map(str::to_string);
            out.push(c);
        }
        out
    };
    let mut ahead: HashSet<(String, String)> = HashSet::new();
    for l in &pic.lines {
        if !AHEAD.contains(&l.kind) {
            continue;
        }
        let (a_s, b_s) = (up(&l.from), up(&l.to));
        for a in &a_s {
            for b in &b_s {
                if !a_s.contains(b) && !b_s.contains(a) {
                    ahead.insert((a.clone(), b.clone()));
                }
            }
        }
    }
    // Clients: people and programs outside that start things.
    let clients: HashSet<&str> = pic
        .parts
        .iter()
        .map(|p| p.id.as_str())
        .filter(|id| {
            g.kind(id) == "external"
                && pic
                    .lines
                    .iter()
                    .any(|l| l.from == *id && AHEAD.contains(&l.kind))
        })
        .collect();
    pic.lines
        .iter()
        .map(|l| {
            let flip = matches!(l.kind, EdgeKind::Data | EdgeKind::Read) && {
                let (t_s, s_s) = (up(&l.to), up(&l.from));
                t_s.iter()
                    .any(|t| s_s.iter().any(|x| ahead.contains(&(t.clone(), x.clone()))))
            };
            let priority = if clients.contains(l.from.as_str()) {
                10
            } else if clients.contains(l.to.as_str()) {
                0
            } else {
                1
            };
            (flip, priority)
        })
        .collect()
}

/***
 * A picture as docair-layout's input: each part with its box and the size of its words, each line with its words and hints.
 ***/
pub fn input(graph: &Graph, pic: &LayerPicture, font: &Monospace) -> Input {
    input_with(graph, pic, font, &HashMap::new())
}

/***
 * As `input`, with how much some of the picture's lines point ahead given: a walk's lines, by the picture's line index.
 ***/
pub fn input_with(
    graph: &Graph,
    pic: &LayerPicture,
    font: &Monospace,
    ahead: &HashMap<usize, u32>,
) -> Input {
    let g = GraphLookup::new(graph);
    let index: HashMap<&str, usize> = pic
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.as_str(), i))
        .collect();
    let parts = pic
        .parts
        .iter()
        .map(|p| {
            let label = part_words(&g, p);
            let kind = g.kind(&p.id);
            let (width, height) = part_size(font, kind, &label);
            let name = g.name(&p.id).unwrap_or(&p.id);
            let min_width = font
                .width(if p.more > 0 { &label } else { name }, PART_FONT)
                .ceil()
                + 34.0;
            Part {
                id: p.id.clone(),
                parent: p.parent.as_deref().and_then(|q| index.get(q).copied()),
                width,
                height,
                min_width,
                // A box's name, in the band at its top: 11.5 px, a little bold.
                name_width: font.width(&label, 11.5) * 1.08,
            }
        })
        .collect();
    let hint = hints(&g, pic);
    let lines = pic
        .lines
        .iter()
        .enumerate()
        .filter_map(|(k, l)| {
            let (from, to) = (*index.get(l.from.as_str())?, *index.get(l.to.as_str())?);
            let words = line_words(&g, l);
            Some(Line {
                id: k.to_string(),
                from,
                to,
                flip: hint[k].0,
                ahead: ahead.get(&k).copied().unwrap_or(hint[k].1),
                words: docair_layout::words_size(font, &words),
            })
        })
        .collect();
    Input { parts, lines }
}

/***
 * The words of a picture's parts and lines, in the picture's order: the words `input` makes room for, and the drawing writes.
 ***/
pub(crate) fn words(graph: &Graph, pic: &LayerPicture) -> (Vec<String>, Vec<String>) {
    let g = GraphLookup::new(graph);
    (
        pic.parts.iter().map(|p| part_words(&g, p)).collect(),
        pic.lines.iter().map(|l| line_words(&g, l)).collect(),
    )
}

/***
 * Lay out a picture: its parts placed box by box, in the direction that fits the canvas best, and its lines routed.
 *
 * @calls docair-viewer.layout_input.input "turns the picture into layout input"
 * @calls docair-layout.lay_out "places the parts and routes the lines"
 ***/
pub fn lay_out(graph: &Graph, pic: &LayerPicture) -> Layout {
    lay_out_with(graph, pic, &HashMap::new())
}

/// As `lay_out`, with how much some of the picture's lines point ahead given.
pub fn lay_out_with(graph: &Graph, pic: &LayerPicture, ahead: &HashMap<usize, u32>) -> Layout {
    let font = Monospace::default();
    docair_layout::lay_out(&input_with(graph, pic, &font, ahead), &options_for(pic))
}

/***
 * The options a picture is laid out with. A layer or a section shrinks what each box holds to its size budget (top-down). A slice does not: every word keeps its size, the picture grows and the reader pans, so a walk through it stays readable (docair-702.27).
 ***/
pub fn options_for(pic: &LayerPicture) -> Options {
    if pic.slice.is_some() {
        Options {
            budget: f64::INFINITY,
            floor: 1.0,
            ..Options::default()
        }
    } else {
        Options::default()
    }
}

/// How often the picture's lines cross when drawn straight from part to part.
pub fn straight_crossings(input: &Input, placed: &Placement) -> usize {
    let segs: Vec<(usize, usize)> = input.lines.iter().map(|l| (l.from, l.to)).collect();
    let at = |i: usize| (placed.parts[i].x, placed.parts[i].y);
    let ccw = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
        (c.1 - a.1) * (b.0 - a.0) > (b.1 - a.1) * (c.0 - a.0)
    };
    let mut n = 0;
    for i in 0..segs.len() {
        for j in i + 1..segs.len() {
            let ((a, b), (c, d)) = (segs[i], segs[j]);
            if a == c || a == d || b == c || b == d {
                continue;
            }
            let (pa, pb, pc, pd) = (at(a), at(b), at(c), at(d));
            if ccw(pa, pc, pd) != ccw(pb, pc, pd) && ccw(pa, pb, pc) != ccw(pa, pb, pd) {
                n += 1;
            }
        }
    }
    n
}

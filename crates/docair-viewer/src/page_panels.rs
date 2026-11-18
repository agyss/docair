//! The panels around the pictures (docs/SLICES.md §4), written by the CLI: each part's details,
//! the tips of parts and lines, the tree, the search words, the legend, the diagnostics and the
//! tests switch. The page's script only shows them; it writes no words of its own.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write;

use docair_core::model::{Edge, EdgeKind, Graph, Node, Origin, Severity};
use serde_json::{Map, Value, json};

use crate::layout_input::{GraphLookup, describe_edge, edge_text, group_label, summarize};
use crate::{escape_text, markdown_html};

/// The mark of each kind in the panels (viewer.js `KINDS`); any other kind is `unknown`.
const GLYPHS: &[(&str, &str)] = &[
    ("system", "◎"),
    ("service", "◆"),
    ("module", "▤"),
    ("api", "⎘"),
    ("component", "▣"),
    ("function", "ƒ"),
    ("endpoint", "⊙"),
    ("store", "◯"),
    ("topic", "▱"),
    ("external", "☁"),
    ("job", "⏱"),
    ("workload", "⬡"),
    ("config", "⚙"),
    ("host", "◫"),
    ("cluster", "⎈"),
    ("namespace", "⬚"),
    ("network", "⇌"),
    ("gateway", "⇶"),
    ("container", "▦"),
    ("release", "⎔"),
    ("test", "✓"),
    ("data", "◇"),
    ("unresolved", "?"),
    ("unknown", "·"),
];

/// Tags docair knows; any other is marked in the Tags tab.
const KNOWN_TAGS: &[&str] = &[
    "id",
    "kind",
    "part-of",
    "doc",
    "deployed-as",
    "in",
    "out",
    "calls",
    "reads",
    "writes",
    "emits",
    "consumes",
    "http",
    "flow",
    "creates",
];

/// The line styles of the kinds of lines, for the legend (viewer.js `EDGE_LINE`).
fn line_style(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Emit | EdgeKind::Consume | EdgeKind::Cover => "dotted",
        EdgeKind::Route | EdgeKind::Select | EdgeKind::Create => "dashed",
        _ => "solid",
    }
}

fn kind_word(kind: EdgeKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn glyph(kind: &str) -> &'static str {
    GLYPHS
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or("·", |(_, g)| g)
}

/// The mark of a part's kind, as the panels show it.
pub(crate) fn glyph_of(kind: &str) -> &'static str {
    glyph(kind_of(kind))
}

fn kind_of(kind: &str) -> &str {
    if GLYPHS.iter().any(|(k, _)| *k == kind) {
        kind
    } else {
        "unknown"
    }
}

fn test_icon(status: &str) -> &'static str {
    match status {
        "passed" => "✓",
        "failed" => "✗",
        "flaky" => "~",
        "skipped" => "–",
        _ => "○",
    }
}

/// An address as a link's target: `#part/orders.create`, with what a URL cannot hold escaped.
pub(crate) fn href(addr: &str) -> String {
    let mut out = String::from("#");
    for b in addr.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~:/".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// What the panels are made of.
pub(crate) struct Panels {
    /// The tree, the legend and the diagnostics' list: in the page from the start.
    pub tree: String,
    pub legend: String,
    pub drawer: String,
    /// The diagnostics' button: its class, its words, its title.
    pub diag: (String, String, String),
    /// The tests switch, when there are tests: its words with tests shown, with them hidden, its title.
    pub tests: Option<(String, String, String)>,
    /// The glossary's and the data's buttons in the header.
    pub buttons: String,
    /// Packed with the pictures: each part's details and tip, each line's tip, the search words.
    pub data: Value,
}

/// The graph, looked up for the panels.
struct Look<'g> {
    graph: &'g Graph,
    g: GraphLookup<'g>,
    kids: HashMap<&'g str, Vec<&'g str>>,
    sections: &'g HashSet<String>,
}

impl<'g> Look<'g> {
    fn node(&self, id: &str) -> Option<&'g Node> {
        self.g.nodes.get(id).copied()
    }

    fn name(&self, id: &str) -> String {
        self.g.name(id).unwrap_or(id).to_string()
    }

    fn is_data(&self, id: &str) -> bool {
        self.g.kind(id) == "data"
    }

    /// A part's children, data left out.
    fn kids(&self, id: &str) -> &[&'g str] {
        self.kids.get(id).map_or(&[], Vec::as_slice)
    }

    /// A part and everything inside it.
    fn inside(&self, id: &str) -> HashSet<&'g str> {
        let mut out: HashSet<&str> = HashSet::new();
        let mut todo: Vec<&str> = vec![self.node(id).map_or("", |n| n.id.as_str())];
        while let Some(x) = todo.pop() {
            if out.insert(x) {
                todo.extend(
                    self.graph
                        .nodes
                        .iter()
                        .filter(|n| n.part_of.as_deref() == Some(x))
                        .map(|n| n.id.as_str()),
                );
            }
        }
        out
    }

    fn descendants(&self, id: &str) -> usize {
        self.kids(id).iter().map(|k| 1 + self.descendants(k)).sum()
    }

    /// How many boxes a part is in.
    fn depth(&self, id: &str) -> usize {
        let mut d = 0;
        let mut at = self.node(id).and_then(|n| n.part_of.as_deref());
        while let Some(p) = at.filter(|p| *p != self.graph.system) {
            d += 1;
            at = self.node(p).and_then(|n| n.part_of.as_deref());
            if d > 64 {
                break;
            }
        }
        d
    }

    /// The box around `id` at `depth`, or `id` itself when it is not that deep.
    fn ancestor_at(&self, id: &str, depth: usize) -> String {
        let mut cur = id.to_string();
        while self.depth(&cur) > depth {
            match self.node(&cur).and_then(|n| n.part_of.clone()) {
                Some(p) => cur = p,
                None => break,
            }
        }
        cur
    }

    fn link(&self, id: &str) -> String {
        format!(
            r#"<a class="link" href="{}" title="{}">{}</a>"#,
            href(&format!("part/{id}")),
            escape_text(id),
            escape_text(&self.name(id))
        )
    }

    fn data_chip(&self, id: &str) -> String {
        let n = self.node(id);
        let title = n.map_or(id, |n| {
            if n.summary.is_empty() {
                id
            } else {
                n.summary.as_str()
            }
        });
        format!(
            r#"<span class="data-ref" data-data="{}" tabindex="0" role="button" title="{}">◇ {}</span>"#,
            escape_text(id),
            escape_text(title),
            escape_text(&self.name(id))
        )
    }

    /***
     * A part's details: its name and kind, the Overview, Docs and Tags tabs, what it talks to, what tests it, its lines and the flows it is in.
     ***/
    fn details(&self, n: &Node) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            r#"<h2><span class="glyph">{}</span>{}</h2><div class="meta">{} · <span class="id">{}</span>"#,
            glyph(kind_of(&n.kind)),
            escape_text(&n.name),
            escape_text(&n.kind),
            escape_text(&n.id)
        );
        if n.origin != Origin::Authored {
            let origin = serde_json::to_value(n.origin)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let _ = write!(
                out,
                r#" <span class="badge">{}</span>"#,
                escape_text(&origin)
            );
        }
        let id = escape_text(&n.id);
        let _ = write!(
            out,
            r#" <a class="link hide-link" data-test="hide-section" data-hide="{id}" title="Leave this part out of the picture; show it again from the chips above the picture">Hide this section</a><a class="link hide-link" data-test="show-section" data-show="{id}" hidden title="Bring this part back; if its section is hidden, the section comes back with only this in it">Show this section</a></div>"#
        );
        let docs: Vec<String> = n
            .docs
            .iter()
            .map(|d| {
                let file = match &d.anchor {
                    Some(a) => format!("{}#{a}", d.file),
                    None => d.file.clone(),
                };
                format!(
                    r#"<div class="docfile">{}</div><div class="md">{}</div>"#,
                    escape_text(&file),
                    markdown_html(&d.markdown)
                )
            })
            .collect();
        let _ = write!(
            out,
            r#"<div class="tabs"><button class="on" data-tab="0">Overview</button><button data-tab="1">Docs ({})</button><button data-tab="2">Tags ({})</button></div>"#,
            n.docs.len(),
            n.tags.len()
        );
        let _ = write!(
            out,
            r#"<div class="tab" data-tab="0">{}</div>"#,
            self.overview(n)
        );
        let docs = if docs.is_empty() {
            r#"<p class="empty">No Markdown is attached to this node.</p>"#.to_string()
        } else {
            docs.concat()
        };
        let _ = write!(out, r#"<div class="tab" data-tab="1" hidden>{docs}</div>"#);
        let _ = write!(
            out,
            r#"<div class="tab" data-tab="2" hidden>{}</div>"#,
            self.tags(n)
        );
        out
    }

    fn overview(&self, n: &Node) -> String {
        let mut out = String::new();
        if !n.summary.is_empty() {
            let _ = write!(out, r#"<p class="summary">{}</p>"#, escape_text(&n.summary));
        }
        if !n.description.is_empty() {
            let _ = write!(
                out,
                r#"<div class="md">{}</div>"#,
                markdown_html(&n.description)
            );
        }
        let mut kv = String::new();
        let mut row = |k: &str, v: String| {
            if !v.is_empty() {
                let _ = write!(kv, r#"<div class="k">{k}</div><div>{v}</div>"#);
            }
        };
        let code = |s: &str| format!("<code>{}</code>", escape_text(s));
        row("symbol", n.symbol.as_deref().map(code).unwrap_or_default());
        row(
            "signature",
            n.signature
                .as_ref()
                .map(|s| {
                    code(&format!(
                        "({}){}",
                        s.params.join(", "),
                        s.returns
                            .as_deref()
                            .map(|r| format!(" → {r}"))
                            .unwrap_or_default()
                    ))
                })
                .unwrap_or_default(),
        );
        row(
            "source",
            n.source
                .as_ref()
                .map(|s| code(&format!("{}:{}", s.file, s.line)))
                .unwrap_or_default(),
        );
        row(
            "http",
            n.http
                .as_ref()
                .map(|h| code(&format!("{} {}", h.method, h.path)))
                .unwrap_or_default(),
        );
        row(
            "part of",
            n.part_of
                .as_deref()
                .filter(|p| *p != self.graph.system)
                .map(|p| self.link(p))
                .unwrap_or_default(),
        );
        row(
            "deployed as",
            n.deployed_as.as_deref().map(code).unwrap_or_default(),
        );
        row(
            "runs in",
            n.runs_in
                .as_deref()
                .map(|p| self.link(p))
                .unwrap_or_default(),
        );
        row(
            "code from",
            n.code_from
                .as_deref()
                .map(|p| self.link(p))
                .unwrap_or_default(),
        );
        let makes: Vec<String> = self
            .graph
            .edges
            .iter()
            .filter(|e| e.from == n.id && e.kind == EdgeKind::Create && self.is_data(&e.to))
            .map(|e| {
                let why = e
                    .text
                    .as_deref()
                    .map(|t| format!(r#" <span class="muted">{}</span>"#, escape_text(t)))
                    .unwrap_or_default();
                format!("{}{why}", self.data_chip(&e.to))
            })
            .collect();
        row("makes", makes.join(" "));
        let kids = self.kids(&n.id);
        if !kids.is_empty() {
            let open = if self.sections.contains(&n.id) {
                format!(
                    r#" · <a class="link" href="{}" data-test="zoom-in">zoom in</a>"#,
                    href(&format!("box/{}", n.id))
                )
            } else {
                String::new()
            };
            row(
                "contains",
                format!("{} nodes{open}", self.descendants(&n.id)),
            );
        }
        match &n.test {
            Some(t) => {
                let at =
                    t.at.as_deref()
                        .map(|a| {
                            format!(
                                " · {}",
                                a.chars().take(16).collect::<String>().replace('T', " ")
                            )
                        })
                        .unwrap_or_default();
                row(
                    "result",
                    format!(
                        r#"<span class="t-badge t-{}">{} {} · {:.1} s{}</span>"#,
                        escape_text(&t.status),
                        test_icon(&t.status),
                        escape_text(&t.status),
                        t.duration_ms as f64 / 1000.0,
                        escape_text(&at)
                    ),
                );
            }
            None if n.kind == "test" && kids.is_empty() => row(
                "result",
                r#"<span class="t-badge t-none">no result yet</span>"#.to_string(),
            ),
            None => {}
        }
        let _ = write!(out, r#"<div class="kv">{kv}</div>"#);
        if !kids.is_empty() {
            out.push_str(&self.talks_to(&n.id));
        }
        out.push_str(&self.tested_by(&n.id));
        let ins: Vec<&Edge> = self
            .graph
            .edges
            .iter()
            .filter(|e| e.to == n.id && e.kind != EdgeKind::Cover)
            .collect();
        let outs: Vec<&Edge> = self
            .graph
            .edges
            .iter()
            .filter(|e| e.from == n.id && (e.kind != EdgeKind::Cover || n.kind == "test"))
            .collect();
        for (title, es, other) in [("Incoming", &ins, true), ("Outgoing", &outs, false)] {
            if es.is_empty() {
                continue;
            }
            let _ = write!(
                out,
                r#"<h3 class="sec">{title} ({})</h3><ul class="edges">"#,
                es.len()
            );
            for e in es.iter() {
                let label = match (&e.text, &e.label) {
                    (Some(_), Some(l)) => {
                        format!(r#"<span class="badge code">{}</span>"#, escape_text(l))
                    }
                    _ => String::new(),
                };
                let _ = write!(
                    out,
                    r#"<li title="{}"><span class="ek">{}</span>{}<span class="badge">{}</span>{label}</li>"#,
                    escape_text(&format!("{} · {}:{}", e.tag, e.source.file, e.source.line)),
                    kind_word(e.kind),
                    self.link(if other { &e.from } else { &e.to }),
                    escape_text(&edge_text(&self.g, e))
                );
            }
            out.push_str("</ul>");
        }
        let mut flows = String::new();
        for f in &self.graph.flows {
            for s in f.steps.iter().filter(|s| s.nodes.contains(&n.id)) {
                let _ = write!(
                    flows,
                    r#"<li><span class="ek">{} {}</span><a class="link" href="{}">{}</a></li>"#,
                    escape_text(&f.name),
                    s.step,
                    href(&format!("slice/flow:{}", f.name)),
                    escape_text(s.label.as_deref().unwrap_or("play from here"))
                );
            }
        }
        if !flows.is_empty() {
            let _ = write!(
                out,
                r#"<h3 class="sec">Flows</h3><ul class="edges">{flows}</ul>"#
            );
        }
        out
    }

    /// For a box: every part outside it that something inside it talks to, and what they exchange.
    fn talks_to(&self, id: &str) -> String {
        let inside = self.inside(id);
        let depth = self.depth(id);
        let mut order: Vec<(bool, String)> = Vec::new();
        let mut peers: HashMap<(bool, String), (Vec<String>, bool)> = HashMap::new();
        for e in &self.graph.edges {
            if e.kind == EdgeKind::Cover {
                continue;
            }
            let from_inside = inside.contains(e.from.as_str());
            if from_inside == inside.contains(e.to.as_str()) {
                continue;
            }
            let peer = self.ancestor_at(if from_inside { &e.to } else { &e.from }, depth);
            let key = (from_inside, peer);
            let entry = peers.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                (Vec::new(), false)
            });
            entry.0.push(describe_edge(&self.g, e));
            if !matches!(e.kind, EdgeKind::Emit | EdgeKind::Consume) {
                entry.1 = true;
            }
        }
        if peers.is_empty() {
            return String::new();
        }
        let mut out = format!(
            r#"<h3 class="sec">Talks to ({})</h3><ul class="edges">"#,
            peers.len()
        );
        for key in &order {
            let (items, sync) = &peers[key];
            let (out_of, peer) = key;
            // Events are asynchronous: ⇢ and ⇠ instead of → and ←.
            let arrow = match (out_of, sync) {
                (true, true) => "→",
                (true, false) => "⇢",
                (false, true) => "←",
                (false, false) => "⇠",
            };
            let _ = write!(
                out,
                r#"<li title="{}"><span class="ek">{arrow}</span>{}<span class="badge">{}</span></li>"#,
                if *out_of {
                    "from inside to this part"
                } else {
                    "from this part to inside"
                },
                self.link(peer),
                escape_text(&summarize(items, 4))
            );
        }
        out.push_str("</ul>");
        out
    }

    /// The tests that cover a part or anything inside it.
    fn tested_by(&self, id: &str) -> String {
        let inside = self.inside(id);
        let mut tests: Vec<&str> = Vec::new();
        for e in &self.graph.edges {
            if e.kind == EdgeKind::Cover
                && inside.contains(e.to.as_str())
                && !inside.contains(e.from.as_str())
                && !tests.contains(&e.from.as_str())
            {
                tests.push(&e.from);
            }
        }
        if tests.is_empty() {
            return String::new();
        }
        let mut out = format!(
            r#"<h3 class="sec">Tested by ({})</h3><ul class="edges" data-test="tested-by">"#,
            tests.len()
        );
        for t in tests {
            let result = self.node(t).and_then(|n| n.test.as_ref());
            let status = result.map_or("none", |r| r.status.as_str());
            let badge = match result {
                Some(r) => format!("{} · {:.1} s", r.status, r.duration_ms as f64 / 1000.0),
                None => "no result".to_string(),
            };
            let _ = write!(
                out,
                r#"<li><span class="ek t-{}-text">{}</span>{}<span class="badge">{}</span></li>"#,
                escape_text(status),
                test_icon(status),
                self.link(t),
                escape_text(&badge)
            );
        }
        out.push_str("</ul>");
        out
    }

    fn tags(&self, n: &Node) -> String {
        if n.tags.is_empty() {
            return r#"<p class="empty">No tags.</p>"#.to_string();
        }
        let mut out = String::from(r#"<table class="tags">"#);
        for t in &n.tags {
            let inferred = if t.origin == Origin::Inferred {
                let why = t.reason.as_deref().unwrap_or("inferred");
                format!(
                    r#" <span class="badge inferred" title="{}">ⓘ {}</span>"#,
                    escape_text(t.reason.as_deref().unwrap_or("")),
                    escape_text(why)
                )
            } else {
                String::new()
            };
            let unknown = if KNOWN_TAGS.contains(&t.name.as_str()) {
                ""
            } else {
                r#" <span class="badge warn">⚠ unknown tag</span>"#
            };
            let _ = write!(
                out,
                "<tr><td>@{}</td><td>{}{inferred}{unknown}</td></tr>",
                escape_text(&t.name),
                escape_text(&t.value)
            );
        }
        out.push_str("</table>");
        out
    }

    /// A part's tip: its name and kind, what it is for.
    fn tip(&self, n: &Node) -> String {
        let summary = if n.summary.is_empty() {
            String::new()
        } else {
            format!("<div>{}</div>", escape_text(&n.summary))
        };
        format!(
            "<b>{}</b> · {}{summary}",
            escape_text(&n.name),
            escape_text(&n.kind)
        )
    }

    /// One line of the graph in a line's tip: who does what, what travels, where it is written.
    fn edge_tip(&self, e: &Edge) -> String {
        let mut out = String::from(r#"<div class="tip-edge">"#);
        let words = e.text.clone().unwrap_or_else(|| describe_edge(&self.g, e));
        let _ = write!(
            out,
            r#"<div class="tip-words">{} {}</div>"#,
            escape_text(&self.name(&e.from)),
            escape_text(&words)
        );
        if e.payload.is_some() || e.reply.is_some() {
            let code = format!(
                "{}{}",
                e.payload
                    .as_deref()
                    .map(|p| format!("({p})"))
                    .unwrap_or_default(),
                e.reply
                    .as_deref()
                    .map(|r| format!(" → {r}"))
                    .unwrap_or_default()
            );
            let from = if e.from_code {
                r#"<span class="where"> · from the code</span>"#.to_string()
            } else {
                String::new()
            };
            let api = e
                .from_api
                .as_deref()
                .map(|a| format!(r#"<span class="where"> · from {}</span>"#, escape_text(a)))
                .unwrap_or_default();
            let _ = write!(
                out,
                r#"<div class="mono">{}{from}{api}</div>"#,
                escape_text(&code)
            );
        }
        for (words, data) in [("carries", &e.carries), ("brings back", &e.carries_back)] {
            if !data.is_empty() {
                let names: Vec<String> = data
                    .iter()
                    .map(|d| format!("◇ {}", escape_text(&self.name(d))))
                    .collect();
                let _ = write!(out, "<div>{words} {}</div>", names.join(", "));
            }
        }
        let origin = serde_json::to_value(e.origin)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let via = e
            .via
            .as_deref()
            .map(|v| format!(" · via {v}"))
            .unwrap_or_default();
        let _ = write!(
            out,
            r#"<code>{}</code><div class="where mono">{}</div></div>"#,
            escape_text(&e.tag),
            escape_text(&format!(
                "{}:{} · {origin}{via}",
                e.source.file, e.source.line
            ))
        );
        out
    }

    /// The tree of every part, boxes first, then by name; the boxes below the top closed.
    fn tree(&self) -> String {
        let mut out = String::new();
        self.tree_level(&mut out, self.graph.system.as_str(), 0);
        out
    }

    fn sorted_kids(&self, id: &str) -> Vec<&'g str> {
        let mut kids: Vec<&str> = self.kids(id).to_vec();
        kids.sort_by(|a, b| {
            let (ga, gb) = (!self.kids(a).is_empty(), !self.kids(b).is_empty());
            gb.cmp(&ga).then_with(|| self.name(a).cmp(&self.name(b)))
        });
        kids
    }

    fn tree_level(&self, out: &mut String, id: &str, depth: usize) {
        let kids = self.sorted_kids(id);
        if kids.is_empty() {
            return;
        }
        out.push_str("<ul>");
        for k in kids {
            let Some(n) = self.node(k) else { continue };
            let has = !self.kids(k).is_empty();
            let closed = has && depth > 0;
            // One mark for a box; the stylesheet turns it while the box is closed.
            let caret = if has { "▾" } else { "" };
            let title = if n.summary.is_empty() {
                &n.id
            } else {
                &n.summary
            };
            let _ = write!(
                out,
                r#"<li data-id="{}"{}><div class="row" title="{}"><span class="caret">{caret}</span><span class="glyph">{}</span><span class="name">{}</span><button class="eye" title="Hide or show this section" aria-label="Hide or show {}" data-test="eye"></button></div>"#,
                escape_text(&n.id),
                if closed { r#" class="closed""# } else { "" },
                escape_text(title),
                glyph(kind_of(&n.kind)),
                escape_text(&n.name),
                escape_text(&n.name)
            );
            self.tree_level(out, k, depth + 1);
            out.push_str("</li>");
        }
        out.push_str("</ul>");
    }

    fn legend(&self) -> String {
        let kinds: BTreeSet<&str> = self
            .graph
            .nodes
            .iter()
            .filter(|n| n.id != self.graph.system)
            .map(|n| kind_of(&n.kind))
            .collect();
        let mut seen: Vec<EdgeKind> = Vec::new();
        for e in &self.graph.edges {
            if !seen.contains(&e.kind) {
                seen.push(e.kind);
            }
        }
        let mut out = String::from(r#"<h4>Nodes</h4><div class="grid">"#);
        for k in kinds {
            // Data has the data colours (docair-54p).
            let colours = if k == "data" {
                "background: var(--data-soft); border-color: var(--data)".to_string()
            } else {
                format!("background: var(--k-{k}-bg); border-color: var(--k-{k}-line)")
            };
            let _ = write!(
                out,
                r#"<span class="sw" style="{colours}"></span><span>{} {k}</span>"#,
                glyph(k)
            );
        }
        out.push_str(r#"</div><h4>Edges</h4><div class="grid">"#);
        for k in seen {
            let w = kind_word(k);
            let _ = write!(
                out,
                r#"<span class="ln" style="border-top-color: var(--e-{w}); border-top-style: {}"></span><span>{w}</span>"#,
                line_style(k)
            );
        }
        out.push_str("</div>");
        out
    }

    fn diagnostics(&self) -> ((String, String, String), String) {
        let d = &self.graph.diagnostics;
        let errors = d.iter().filter(|x| x.severity == Severity::Error).count();
        let warnings = d.len() - errors;
        let words = if d.is_empty() {
            "✓ 0 issues".to_string()
        } else {
            [
                if errors > 0 {
                    format!("✖ {errors}")
                } else {
                    String::new()
                },
                if warnings > 0 {
                    format!("⚠ {warnings}")
                } else {
                    String::new()
                },
            ]
            .join(" ")
            .trim()
            .to_string()
        };
        let title = if d.is_empty() {
            "docair check found nothing to report".to_string()
        } else {
            format!("{errors} errors, {warnings} warnings from docair check")
        };
        let class = if errors > 0 {
            "btn diag-error"
        } else if warnings > 0 {
            "btn diag-warning"
        } else {
            "btn diag-ok"
        };
        let mut drawer = String::new();
        if d.is_empty() {
            drawer.push_str(r#"<div class="none">docair check found nothing to report.</div>"#);
        }
        for x in d {
            let (sev, mark) = if x.severity == Severity::Error {
                ("error", "✖")
            } else {
                ("warning", "⚠")
            };
            let at = match (&x.file, x.line) {
                (Some(f), Some(l)) => format!("{f}:{l}"),
                (Some(f), None) => f.clone(),
                _ => String::new(),
            };
            let node = x
                .node
                .as_deref()
                .filter(|n| self.node(n).is_some())
                .map(|n| format!(r#" data-part="{}""#, escape_text(n)))
                .unwrap_or_default();
            let _ = write!(
                drawer,
                r#"<div class="row"{node}><span class="{sev}">{mark}</span><span class="{sev} mono">{}</span><span>{}</span><span class="where">{}</span></div>"#,
                escape_text(&x.code),
                escape_text(&x.message),
                escape_text(&at)
            );
        }
        ((class.to_string(), words, title), drawer)
    }

    /// The tests switch's words, with tests shown and hidden, and its title; none without tests.
    fn tests_switch(&self) -> Option<(String, String, String)> {
        let tests: Vec<&Node> = self
            .graph
            .nodes
            .iter()
            .filter(|n| n.kind == "test" && self.kids(&n.id).is_empty())
            .collect();
        if tests.is_empty() {
            return None;
        }
        let by = |st: &str| {
            tests
                .iter()
                .filter(|n| n.test.as_ref().is_some_and(|t| t.status == st))
                .count()
        };
        let (passed, failed, flaky) = (by("passed"), by("failed"), by("flaky"));
        let none = tests.iter().filter(|n| n.test.is_none()).count();
        let mut counts = String::new();
        if passed > 0 {
            let _ = write!(counts, r#"<span class="t-passed-text">✓{passed} </span>"#);
        }
        if failed > 0 {
            let _ = write!(counts, r#"<span class="t-failed-text">✗{failed} </span>"#);
        }
        if flaky > 0 {
            let _ = write!(counts, "<span>~{flaky} </span>");
        }
        if passed + failed + flaky == 0 {
            let _ = write!(counts, "{} ", tests.len());
        }
        let shown = format!(r#"Tests {counts}<span class="onoff">shown</span>"#);
        let hidden = format!(r#"Tests {counts}<span class="onoff">hidden</span>"#);
        let title = format!(
            "{} tests: {passed} passed, {failed} failed, {flaky} flaky, {none} without a result. Click to show or hide them.",
            tests.len()
        );
        Some((shown, hidden, title))
    }
}

/***
 * Write the panels: each part's details and tip, each line's tip, the tree, the search words, the legend, the diagnostics and the tests switch.
 ***/
pub(crate) fn panels(graph: &Graph, sections: &HashSet<String>) -> Panels {
    let mut kids: HashMap<&str, Vec<&str>> = HashMap::new();
    for n in &graph.nodes {
        if n.id == graph.system || n.kind == "data" {
            continue;
        }
        if let Some(p) = n.part_of.as_deref() {
            kids.entry(p).or_default().push(n.id.as_str());
        }
    }
    let look = Look {
        graph,
        g: GraphLookup::new(graph),
        kids,
        sections,
    };
    let linker = crate::page_glossary::Linker::new(graph);
    let glossary = crate::page_glossary::glossary(graph, &linker);
    let data_pages = glossary
        .data
        .get("data")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut details = Map::new();
    let mut tips = Map::new();
    let mut search = Map::new();
    let mut parents = Map::new();
    let mut names = Map::new();
    for n in &graph.nodes {
        if n.id == graph.system {
            continue;
        }
        // A datum's details are its page: who makes it, which lines carry it.
        let shown = match data_pages.get(&n.id) {
            Some(page) => page.clone(),
            None => Value::String(linker.link(&look.details(n), None)),
        };
        details.insert(n.id.clone(), shown);
        tips.insert(n.id.clone(), Value::String(look.tip(n)));
        search.insert(
            n.id.clone(),
            Value::String(format!("{} {} {}", n.id, n.name, n.summary).to_lowercase()),
        );
        names.insert(
            n.id.clone(),
            Value::String(if look.kids(&n.id).is_empty() {
                n.name.clone()
            } else {
                group_label(n)
            }),
        );
        if let Some(p) = n.part_of.as_deref().filter(|p| *p != graph.system) {
            parents.insert(n.id.clone(), Value::String(p.to_string()));
        }
    }
    let lines: Vec<Value> = graph
        .edges
        .iter()
        .map(|e| Value::String(look.edge_tip(e)))
        .collect();
    let (diag, drawer) = look.diagnostics();
    let mut data = json!({
        "details": details,
        "tips": tips,
        "lines": lines,
        "search": search,
        "names": names,
        "parents": parents,
    });
    for (k, v) in glossary.data {
        data[k] = v;
    }
    Panels {
        tree: look.tree(),
        legend: look.legend(),
        drawer,
        diag,
        tests: look.tests_switch(),
        buttons: glossary.buttons,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::href;

    #[test]
    fn an_address_is_escaped_for_a_link() {
        assert_eq!(href("part/orders.create"), "#part/orders.create");
        assert_eq!(href("slice/flow:check out"), "#slice/flow:check%20out");
        assert_eq!(href("box/a\"b"), "#box/a%22b");
    }
}

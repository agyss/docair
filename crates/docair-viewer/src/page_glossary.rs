//! The glossary and the data in the documentation page (SPEC §13, §3.6), written by the CLI: the
//! words the glossary explains are marked where the panels use them, code spans that name a part
//! link to it, the data a line's words name become chips, and each term, part and datum has its
//! popup; the glossary's list and entries, the data's list and pages, and the docs that "Read
//! more" opens. The page's script only opens them.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use docair_core::model::{EdgeKind, Graph, Node, Term};
use serde_json::{Map, Value};

use crate::{escape_text, markdown_html_in};

/// Words to find in text: whole words in any case, plurals too; a word written with a capital
/// counts only as written (SPEC §13.1); never inside an id or a path.
pub(crate) struct Words {
    /// Each word in lower case, what it names, and the word as written when it has a capital.
    words: Vec<(Vec<char>, String, Option<Vec<char>>)>,
}

impl Words {
    pub(crate) fn new(items: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut words: Vec<(Vec<char>, String, Option<Vec<char>>)> = Vec::new();
        for (w, id) in items {
            let w = w.trim();
            if w.chars().count() < 2 {
                continue;
            }
            let lower: Vec<char> = w.to_lowercase().chars().collect();
            if words.iter().any(|(l, ..)| *l == lower) {
                continue;
            }
            let exact = (w != w.to_lowercase()).then(|| w.chars().collect());
            words.push((lower, id, exact));
        }
        words.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
        Words { words }
    }

    /// Each word found in `text`, left to right: its byte range and what it names.
    pub(crate) fn find(&self, text: &str) -> Vec<(usize, usize, &str)> {
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let lower: Vec<char> = chars
            .iter()
            .map(|&(_, c)| c.to_lowercase().next().unwrap_or(c))
            .collect();
        let word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
        let byte = |i: usize| chars.get(i).map_or(text.len(), |&(b, _)| b);
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if i > 0 && (word(chars[i - 1].1) || chars[i - 1].1 == '.' || chars[i - 1].1 == '/') {
                i += 1;
                continue;
            }
            let mut hit = None;
            'words: for (w, id, exact) in &self.words {
                let n = w.len();
                if i + n > lower.len() || lower[i..i + n] != w[..] {
                    continue;
                }
                if let Some(ex) = exact
                    && !chars[i..i + n].iter().map(|c| c.1).eq(ex.iter().copied())
                {
                    continue;
                }
                for plural in [&['e', 's'][..], &['s'][..], &[][..]] {
                    let e = i + n + plural.len();
                    if lower[i + n..].starts_with(plural) && lower.get(e).is_none_or(|c| !word(*c))
                    {
                        hit = Some((e, id.as_str()));
                        break 'words;
                    }
                }
            }
            match hit {
                Some((e, id)) => {
                    out.push((byte(i), byte(e), id));
                    i = e;
                }
                None => i += 1,
            }
        }
        out
    }
}

/// Elements whose text names no term: links, code, headings, buttons and fields.
const NO_TERMS_TAGS: &[&str] = &[
    "a", "code", "pre", "button", "input", "textarea", "select", "h1", "h2", "h3", "option",
    "script", "style",
];
/// Classes whose text names no term: marks, ids, places, badges.
const NO_TERMS_CLASSES: &[&str] = &[
    "term",
    "no-terms",
    "badge",
    "ek",
    "id",
    "where",
    "docfile",
    "hop-carry",
    "empty",
    "data-ref",
    "node-ref",
];
/// Classes whose words name data: a line's words, what a step carries.
const DATA_IN: &[&str] = &["badge", "hop-carry", "carry", "hop-has"];
const VOID: &[&str] = &[
    "br", "hr", "img", "input", "meta", "link", "source", "wbr", "col", "area",
];

fn attr<'t>(tag: &'t str, name: &str) -> &'t str {
    let key = format!(" {name}=\"");
    tag.find(&key)
        .map(|at| {
            let rest = &tag[at + key.len()..];
            &rest[..rest.find('"').unwrap_or(rest.len())]
        })
        .unwrap_or("")
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// What the panels' words are linked to: the glossary's words, the data's words, the parts' names.
pub(crate) struct Linker<'g> {
    terms: Words,
    data: Words,
    ids: HashSet<&'g str>,
    /// A name only one part has, and that part.
    names: HashMap<&'g str, &'g str>,
    summaries: HashMap<&'g str, &'g str>,
}

impl<'g> Linker<'g> {
    pub(crate) fn new(graph: &'g Graph) -> Self {
        let terms = Words::new(graph.glossary.iter().flat_map(|t| {
            std::iter::once(t.term.clone())
                .chain(t.aliases.iter().cloned())
                .map(|w| (w, t.id.clone()))
        }));
        let data = Words::new(
            graph
                .nodes
                .iter()
                .filter(|n| n.kind == "data")
                .flat_map(|d| {
                    let mut own = vec![
                        d.name.clone(),
                        d.id.rsplit('.').next().unwrap_or(&d.id).to_string(),
                    ];
                    for t in d.tags_named("carried-as") {
                        own.extend(t.value.split(',').map(|w| w.trim().to_string()));
                    }
                    own.into_iter().map(|w| (w.to_lowercase(), d.id.clone()))
                }),
        );
        let mut count: HashMap<&str, usize> = HashMap::new();
        for n in &graph.nodes {
            *count.entry(n.name.as_str()).or_default() += 1;
        }
        let names = graph
            .nodes
            .iter()
            .filter(|n| n.name.chars().count() >= 3 && count[n.name.as_str()] == 1)
            .map(|n| (n.name.as_str(), n.id.as_str()))
            .collect();
        Linker {
            terms,
            data,
            ids: graph.nodes.iter().map(|n| n.id.as_str()).collect(),
            names,
            summaries: graph
                .nodes
                .iter()
                .map(|n| (n.id.as_str(), n.summary.as_str()))
                .collect(),
        }
    }

    /// A code span that names a part: its id, or its name when only one part has it.
    fn node_named(&self, text: &str) -> Option<&'g str> {
        self.ids
            .get(text)
            .copied()
            .or_else(|| self.names.get(text).copied())
    }

    /***
     * Mark the words of a panel: the first mention of each glossary term, the data a line's words name, and code spans that name a part.
     *
     * `except` is a term not to mark: the one the panel is about.
     ***/
    pub(crate) fn link(&self, html: &str, except: Option<&str>) -> String {
        // An open element: its name, whether its text names no term, whether its words name data, whether its code links no part.
        struct Open {
            name: String,
            no_terms: bool,
            data: bool,
            no_code: bool,
        }
        let mut out = String::with_capacity(html.len() + html.len() / 8);
        let mut stack: Vec<Open> = Vec::new();
        let mut done: HashSet<String> = except.map(str::to_string).into_iter().collect();
        let mut rest = html;
        while !rest.is_empty() {
            if rest.starts_with('<') {
                let end = rest.find('>').map_or(rest.len(), |e| e + 1);
                let tag = &rest[..end];
                rest = &rest[end..];
                if let Some(close) = tag.strip_prefix("</") {
                    let name = close.trim_end_matches('>').trim().to_ascii_lowercase();
                    while let Some(o) = stack.pop() {
                        if o.name == name {
                            break;
                        }
                    }
                    out.push_str(tag);
                    continue;
                }
                if tag.starts_with("<!") || tag.starts_with("<?") {
                    out.push_str(tag);
                    continue;
                }
                let name = tag[1..]
                    .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let class = attr(tag, "class");
                let classes: Vec<&str> = class.split_whitespace().collect();
                let up = stack.last();
                let no_terms = up.is_some_and(|o| o.no_terms)
                    || NO_TERMS_TAGS.contains(&name.as_str())
                    || classes.iter().any(|c| NO_TERMS_CLASSES.contains(c));
                let data = (up.is_some_and(|o| o.data)
                    || classes.iter().any(|c| DATA_IN.contains(c)))
                    && !classes.contains(&"data-ref");
                let no_code = up.is_some_and(|o| o.no_code)
                    || name == "pre"
                    || name == "a"
                    || classes.iter().any(|c| *c == "no-terms" || *c == "node-ref");
                let mut written = tag.to_string();
                if name == "code"
                    && !no_code
                    && !classes.contains(&"node-ref")
                    && let Some(close) = rest.find("</code>")
                    && !rest[..close].contains('<')
                    && let Some(id) = self.node_named(unescape(&rest[..close]).trim())
                {
                    let cls = if class.is_empty() {
                        "node-ref".to_string()
                    } else {
                        format!("{class} node-ref")
                    };
                    let rest_of_tag = if class.is_empty() {
                        tag[5..].to_string()
                    } else {
                        tag[5..].replace(&format!(" class=\"{class}\""), "")
                    };
                    written = format!(
                        r#"<code class="{cls}" data-node="{}" tabindex="0"{rest_of_tag}"#,
                        escape_text(id)
                    );
                }
                out.push_str(&written);
                if !tag.ends_with("/>") && !VOID.contains(&name.as_str()) {
                    stack.push(Open {
                        name,
                        no_terms,
                        data,
                        no_code,
                    });
                }
                continue;
            }
            let next = rest.find('<').unwrap_or(rest.len());
            let text = &rest[..next];
            rest = &rest[next..];
            let up = stack.last();
            if up.is_some_and(|o| o.data) {
                let mut at = 0;
                for (s, e, id) in self.data.find(text) {
                    out.push_str(&text[at..s]);
                    let _ = write!(
                        out,
                        r#"<span class="data-ref inline" data-data="{}" tabindex="0" role="button" title="{}">{}</span>"#,
                        escape_text(id),
                        escape_text(
                            self.summaries
                                .get(id)
                                .copied()
                                .filter(|s| !s.is_empty())
                                .unwrap_or(id)
                        ),
                        &text[s..e]
                    );
                    at = e;
                }
                out.push_str(&text[at..]);
            } else if up.is_none_or(|o| !o.no_terms) {
                let mut at = 0;
                for (s, e, id) in self.terms.find(text) {
                    if !done.insert(id.to_string()) {
                        continue;
                    }
                    out.push_str(&text[at..s]);
                    let _ = write!(
                        out,
                        r#"<span class="term" data-term="{}" tabindex="0" role="button">{}</span>"#,
                        escape_text(id),
                        &text[s..e]
                    );
                    at = e;
                }
                out.push_str(&text[at..]);
            } else {
                out.push_str(text);
            }
        }
        out
    }
}

fn link_to(graph_names: &HashMap<&str, &Node>, id: &str) -> String {
    let name = graph_names.get(id).map_or(id, |n| n.name.as_str());
    format!(
        r#"<a class="link" href="{}" title="{}">{}</a>"#,
        crate::page_panels::href(&format!("part/{id}")),
        escape_text(id),
        escape_text(name)
    )
}

/// A doc heading as a short name: `ADAPTERS.md §4`, else the file and the heading.
fn section_name(file: &str, title: &str) -> String {
    let file = file.rsplit('/').next().unwrap_or(file);
    let num: String = title
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let num = num.trim_end_matches('.');
    if !num.is_empty() && title[num.len()..].trim_start_matches('.').starts_with(' ') {
        format!("{file} §{num}")
    } else {
        format!("{file}: {title}")
    }
}

fn parent_dir(file: &str) -> String {
    file.rsplit_once('/')
        .map_or(String::new(), |(d, _)| d.to_string())
}

/// What the glossary and the data add to the page.
pub(crate) struct Glossary {
    /// The header's buttons: the glossary's and the data's, when there are any.
    pub buttons: String,
    /// Packed with the panels: popups, the glossary's list and entries, the data's list and pages, the docs.
    pub data: Map<String, Value>,
}

/***
 * Write what the glossary and the data add: each term's, part's and datum's popup, the glossary's list and its entries, the data's list and each datum's page, and the docs Read more opens.
 ***/
pub(crate) fn glossary(graph: &Graph, linker: &Linker) -> Glossary {
    let nodes: HashMap<&str, &Node> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let library: HashSet<String> = graph.library.iter().map(|d| d.file.clone()).collect();
    let md = |t: &Term, text: &str| markdown_html_in(text, &parent_dir(&t.source.file), &library);
    let mut pops = Map::new();
    let mut entries = Map::new();
    // Where a term's meaning comes from (docs/MEANINGS.md §2).
    let from = |t: &Term| -> String {
        let inner = match (t.from.as_str(), &t.node, &t.section) {
            ("code", Some(node), _) => {
                let mut s = format!(
                    "from {}",
                    if nodes.contains_key(node.as_str()) {
                        link_to(&nodes, node)
                    } else {
                        format!("<code>{}</code>", escape_text(node))
                    }
                );
                let own = nodes
                    .get(node.as_str())
                    .and_then(|n| n.source.as_ref())
                    .map(|s| s.file.as_str());
                if t.unchecked.is_some() {
                    s.push_str(" · the glossary is not checked against it");
                } else if !t.source.file.is_empty() && Some(t.source.file.as_str()) != own {
                    s.push_str(" · worded in the glossary");
                }
                s
            }
            ("doc", _, Some(sec)) => format!(
                r#"in <a class="link" href="docair-doc:{}#{}">{}</a>"#,
                escape_text(&sec.file),
                escape_text(&sec.anchor),
                escape_text(&section_name(&sec.file, &sec.title))
            ),
            _ => "in the glossary".to_string(),
        };
        format!(r#"<div class="pop-see" data-test="term-from">{inner}</div>"#)
    };
    // Read more: a doc in the page, a web page, the doc heading that defines it, or the part.
    let more = |t: &Term, body: &str| -> String {
        let link = body
            .split("href=\"")
            .skip(1)
            .map(|r| &r[..r.find('"').unwrap_or(r.len())])
            .find(|h| {
                h.starts_with("docair-doc:")
                    || h.starts_with("http://")
                    || h.starts_with("https://")
            })
            .map(|h| h.replace("&amp;", "&"))
            .or_else(|| {
                t.section
                    .as_ref()
                    .filter(|s| library.contains(&s.file))
                    .map(|s| format!("docair-doc:{}#{}", s.file, s.anchor))
            });
        match link {
            Some(h) => format!(
                r#"<a class="link" href="{}" data-test="pop-more">{}</a>"#,
                escape_text(&h),
                if h.starts_with("docair-doc:") {
                    "Read more"
                } else {
                    "Read more ↗"
                }
            ),
            None => match &t.node {
                Some(n) if t.from == "code" && nodes.contains_key(n.as_str()) => format!(
                    r#"<a class="link" href="{}" data-test="pop-more">Read more</a>"#,
                    crate::page_panels::href(&format!("part/{n}"))
                ),
                _ => String::new(),
            },
        }
    };
    let mentions = |t: &Term| -> Vec<&Node> {
        let words = Words::new(
            std::iter::once(t.term.clone())
                .chain(t.aliases.iter().cloned())
                .map(|w| (w, t.id.clone())),
        );
        graph
            .nodes
            .iter()
            .filter(|n| {
                !words
                    .find(&format!("{} {}", n.summary, n.description))
                    .is_empty()
            })
            .collect()
    };
    let mut terms: Vec<&Term> = graph.glossary.iter().collect();
    terms.sort_by_key(|t| t.term.to_lowercase());
    let mut list = String::new();
    for t in &terms {
        let summary = md(t, &t.summary);
        let body = md(t, &t.markdown);
        let see: Vec<String> = t
            .see
            .iter()
            .filter(|s| nodes.contains_key(s.as_str()) && Some(*s) != t.node.as_ref())
            .take(4)
            .map(|s| link_to(&nodes, s))
            .collect();
        let see = if see.is_empty() {
            String::new()
        } else {
            format!(r#"<div class="pop-see">see {}</div>"#, see.join(", "))
        };
        let read = more(t, &body);
        let read = if read.is_empty() {
            String::new()
        } else {
            format!(" · {read}")
        };
        pops.insert(
            format!("term:{}", t.id),
            Value::String(format!(
                r#"<div class="pop-title">{}</div><div class="md">{summary}</div>{}{see}<div class="pop-foot"><a class="link" data-term-open="{}" data-test="pop-open">In the glossary</a>{read}</div>"#,
                escape_text(&t.term),
                from(t),
                escape_text(&t.id)
            )),
        );
        // The entry in full.
        let mut e = format!(
            r#"<div class="gl-nav"><a class="link" data-glossary>← Glossary</a><a class="link" data-back hidden> · back</a></div><h2 data-test="term-title"><span class="glyph">¶</span>{}</h2>"#,
            escape_text(&t.term)
        );
        if !t.aliases.is_empty() {
            let _ = write!(
                e,
                r#"<div class="meta">also: {}</div>"#,
                escape_text(&t.aliases.join(", "))
            );
        }
        let _ = write!(e, r#"<div class="md">{body}</div>{}"#, from(t));
        if let Some(sec) = t
            .section
            .as_ref()
            .filter(|s| library.contains(&s.file) && t.from != "doc")
        {
            let _ = write!(
                e,
                r#"<div class="pop-see">more in <a class="link" href="docair-doc:{}#{}" data-test="term-section">{}</a></div>"#,
                escape_text(&sec.file),
                escape_text(&sec.anchor),
                escape_text(&section_name(&sec.file, &sec.title))
            );
        }
        if let Some(u) = &t.unchecked {
            let _ = write!(
                e,
                r#"<h3 class="sec">In the glossary, not checked against the code</h3><div class="md" data-test="term-unchecked">{}</div>"#,
                md(t, u)
            );
        }
        let see_all: Vec<&String> = t
            .see
            .iter()
            .filter(|s| nodes.contains_key(s.as_str()))
            .collect();
        if !see_all.is_empty() {
            e.push_str(r#"<h3 class="sec">See</h3><ul class="edges">"#);
            for s in see_all {
                let _ = write!(
                    e,
                    r#"<li>{}<span class="badge">{}</span></li>"#,
                    link_to(&nodes, s),
                    escape_text(&nodes[s.as_str()].kind)
                );
            }
            e.push_str("</ul>");
        }
        let used = mentions(t);
        if !used.is_empty() {
            let _ = write!(
                e,
                r#"<h3 class="sec">Used by ({})</h3><ul class="edges">"#,
                used.len()
            );
            for n in used.iter().take(20) {
                let _ = write!(
                    e,
                    r#"<li>{}<span class="badge">{}</span></li>"#,
                    link_to(&nodes, &n.id),
                    escape_text(&n.kind)
                );
            }
            e.push_str("</ul>");
        }
        let _ = write!(
            e,
            r#"<div class="docfile">{}:{}</div>"#,
            escape_text(&t.source.file),
            t.source.line
        );
        entries.insert(t.id.clone(), Value::String(linker.link(&e, Some(&t.id))));
        // Its line in the list.
        let badge = match t.from.as_str() {
            "code" => r#"<span class="badge" title="defined by the code">ƒ</span>"#,
            "doc" => r#"<span class="badge" title="defined by a doc heading">§</span>"#,
            _ => "",
        };
        let words = std::iter::once(t.term.as_str())
            .chain(t.aliases.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let _ = write!(
            list,
            r#"<dt data-words="{}"><a class="link" data-term-open="{}">{}</a>{badge}</dt><dd><div class="md">{summary}</div></dd>"#,
            escape_text(&words),
            escape_text(&t.id),
            escape_text(&t.term)
        );
    }
    let glossary = format!(
        r#"<h2><span class="glyph">¶</span>Glossary</h2><div class="meta">{} terms · what the words of these docs mean</div><input type="search" class="gl-search" placeholder="Find a term…" aria-label="Find a term" data-test="glossary-search"><dl class="gl-list" data-test="glossary">{list}</dl><a class="link" data-back hidden>← back</a>"#,
        terms.len()
    );
    // Parts named in code spans: their popup.
    for n in &graph.nodes {
        if n.id == graph.system {
            continue;
        }
        let summary = if n.summary.is_empty() {
            String::new()
        } else {
            format!("<p>{}</p>", escape_text(&n.summary))
        };
        pops.insert(
            format!("node:{}", n.id),
            Value::String(format!(
                r#"<div class="pop-title">{} <span class="badge">{}</span></div>{summary}<div class="pop-foot"><a class="link" href="{}">Show it</a></div>"#,
                escape_text(&n.name),
                escape_text(&n.kind),
                crate::page_panels::href(&format!("part/{}", n.id))
            )),
        );
    }
    // The data: each one's popup and page, and their list.
    let carried_by = |id: &str| {
        graph
            .edges
            .iter()
            .filter(|e| e.carries.iter().chain(&e.carries_back).any(|d| d == id))
            .collect::<Vec<_>>()
    };
    let mut data: Vec<&Node> = graph
        .nodes
        .iter()
        .filter(|n| {
            n.kind == "data"
                && !graph
                    .nodes
                    .iter()
                    .any(|k| k.part_of.as_deref() == Some(n.id.as_str()))
        })
        .collect();
    data.sort_by(|a, b| a.name.cmp(&b.name));
    let mut items = String::new();
    let mut pages = Map::new();
    let follow = |id: &str| crate::page_panels::href(&format!("slice/{id}"));
    for d in &data {
        let makers: Vec<&docair_core::model::Edge> = graph
            .edges
            .iter()
            .filter(|e| e.to == d.id && e.kind == EdgeKind::Create)
            .collect();
        let lines = carried_by(&d.id);
        let made = if makers.is_empty() {
            String::new()
        } else {
            format!(
                "made by {}",
                makers
                    .iter()
                    .map(|e| link_to(&nodes, &e.from))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let summary = if d.summary.is_empty() {
            String::new()
        } else {
            format!("<p>{}</p>", escape_text(&d.summary))
        };
        pops.insert(
            format!("data:{}", d.id),
            Value::String(format!(
                r#"<div class="pop-title">◇ {} <span class="badge">data</span></div>{summary}{}<div class="pop-foot"><a class="link" href="{}">Follow it</a> · <a class="link" data-about="{}">About it</a></div>"#,
                escape_text(&d.name),
                if made.is_empty() { String::new() } else { format!(r#"<div class="pop-see">{made}</div>"#) },
                follow(&d.id),
                escape_text(&d.id)
            )),
        );
        let _ = write!(
            items,
            r#"<li><div><a class="link" data-about="{}">◇ {}</a><span class="badge">{} lines</span>{}</div><div class="muted">{}</div>{}</li>"#,
            escape_text(&d.id),
            escape_text(&d.name),
            lines.len(),
            if lines.is_empty() {
                String::new()
            } else {
                format!(
                    r#"<a class="link follow" href="{}" data-test="follow">Follow</a>"#,
                    follow(&d.id)
                )
            },
            escape_text(&d.summary),
            if made.is_empty() {
                String::new()
            } else {
                format!(r#"<div class="made-by">{made}</div>"#)
            }
        );
        let mut page = format!(
            r#"<div class="gl-nav"><a class="link" data-data-list>← Data</a></div><h2 data-test="data-title"><span class="glyph">◇</span>{} <span class="badge">data</span></h2>"#,
            escape_text(&d.name)
        );
        if !d.summary.is_empty() {
            let _ = write!(
                page,
                r#"<p class="summary">{}</p>"#,
                escape_text(&d.summary)
            );
        }
        if !lines.is_empty() {
            let _ = write!(
                page,
                r#"<a class="btn" href="{}" data-test="follow-data">Follow it on the chart</a>"#,
                follow(&d.id)
            );
        }
        if !makers.is_empty() {
            page.push_str(r#"<h3 class="sec">Made by</h3><ul class="edges">"#);
            for e in &makers {
                let why = e
                    .text
                    .as_deref()
                    .map(|t| format!(r#"<span class="badge">{}</span>"#, escape_text(t)))
                    .unwrap_or_default();
                let _ = write!(page, "<li>{}{why}</li>", link_to(&nodes, &e.from));
            }
            page.push_str("</ul>");
        }
        if !lines.is_empty() {
            let _ = write!(
                page,
                r#"<h3 class="sec">Carried by ({})</h3><ul class="edges">"#,
                lines.len()
            );
            for e in &lines {
                let kind = if e.carries.contains(&d.id) {
                    serde_json::to_value(e.kind)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_default()
                } else {
                    "answer".to_string()
                };
                let words = crate::layout_input::edge_text(
                    &crate::layout_input::GraphLookup::new(graph),
                    e,
                );
                let _ = write!(
                    page,
                    r#"<li><span class="ek">{kind}</span>{} → {}<span class="badge">{}</span></li>"#,
                    link_to(&nodes, &e.from),
                    link_to(&nodes, &e.to),
                    escape_text(&words)
                );
            }
            page.push_str("</ul>");
        }
        pages.insert(d.id.clone(), Value::String(linker.link(&page, None)));
    }
    let data_list = format!(
        r#"<h2><span class="glyph">◇</span>Data</h2><div class="meta">What users and the code make and carry. The boxes of the chart are the working parts; data travels along their lines.</div><ul class="data-list" data-test="data-list">{items}</ul><a class="link" data-back hidden>← back</a>"#
    );
    // The docs Read more opens.
    let mut docs = Map::new();
    for d in &graph.library {
        let body = markdown_html_in(&d.markdown, &parent_dir(&d.file), &library);
        docs.insert(
            d.file.clone(),
            Value::String(format!(
                r#"<div class="reader-head"><b>{}</b><span class="spacer"></span><button class="btn" data-test="reader-close">Close</button></div><div class="reader-body md">{}</div>"#,
                escape_text(&d.file),
                linker.link(&body, None)
            )),
        );
    }
    let mut buttons = String::new();
    if !terms.is_empty() {
        let _ = write!(
            buttons,
            r#"<button class="btn" id="btn-glossary" data-test="glossary-button" title="What the words of these docs mean">Glossary ({})</button>"#,
            terms.len()
        );
    }
    if !data.is_empty() {
        let _ = write!(
            buttons,
            r#"<button class="btn" id="btn-data" data-test="data-button" title="What users and the code make and carry">◇ Data ({})</button>"#,
            data.len()
        );
    }
    let mut out = Map::new();
    out.insert("pops".into(), Value::Object(pops));
    out.insert("terms".into(), Value::Object(entries));
    out.insert(
        "glossary".into(),
        Value::String(linker.link(&glossary, None)),
    );
    out.insert("data".into(), Value::Object(pages));
    out.insert(
        "dataList".into(),
        Value::String(linker.link(&data_list, None)),
    );
    out.insert("docs".into(), Value::Object(docs));
    Glossary { buttons, data: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words() -> Words {
        Words::new([
            ("walkthrough".into(), "walkthrough".into()),
            ("Ingress".into(), "ingress".into()),
            ("doc block".into(), "doc-block".into()),
        ])
    }

    #[test]
    fn words_are_found_whole_with_their_plurals_and_as_written_when_capital() {
        let w = words();
        let found = |t: &str| {
            w.find(t)
                .iter()
                .map(|&(s, e, id)| format!("{}={id}", &t[s..e]))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            found("two walkthroughs and a Doc Block"),
            ["walkthroughs=walkthrough", "Doc Block=doc-block"]
        );
        assert_eq!(found("an Ingress, an ingress"), ["Ingress=ingress"]);
        assert!(
            found("docair-ai.walkthrough and docs/walkthrough and walkthroughish").is_empty(),
            "not inside an id, a path or a longer word"
        );
    }

    #[test]
    fn a_panel_marks_each_term_once_and_never_in_code_links_or_badges() {
        let w = words();
        let linker = Linker {
            terms: w,
            data: Words::new([("order".to_string(), "data.order".to_string())]),
            ids: ["orders.create"].into_iter().collect(),
            names: HashMap::new(),
            summaries: HashMap::new(),
        };
        let html = r#"<p>A walkthrough, another walkthrough.</p><code>walkthrough</code><a>walkthrough</a><span class="badge">POST orders</span><code>orders.create</code>"#;
        let out = linker.link(html, None);
        assert_eq!(out.matches(r#"class="term""#).count(), 1, "{out}");
        assert!(
            out.contains(r#"<span class="data-ref inline" data-data="data.order""#),
            "{out}"
        );
        assert!(out.contains(r#"<code class="node-ref" data-node="orders.create" tabindex="0">orders.create</code>"#), "{out}");
        assert!(
            out.contains("<code>walkthrough</code><a>walkthrough</a>"),
            "{out}"
        );
    }
}

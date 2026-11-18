//! docair-viewer: the graph as one self-contained HTML page (SPEC §1, §9).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use docair_core::model::Graph;
use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd, html};
use serde_json::json;

const TEMPLATE: &str = include_str!("../assets/viewer.html");
const CSS: &str = include_str!("../assets/viewer.css");
const APP: &str = include_str!("../assets/viewer.js");
const CYTOSCAPE: &str = include_str!("../vendor/cytoscape.min.js");
const ELK: &str = include_str!("../vendor/elk.bundled.js");

/***
 * Write `docair.html`: the viewer page with the graph inlined.
 *
 * @flow   scan 12 "write the outputs"
 * @in     graph: Graph from docair-core.check.check
 * @calls  docair-viewer.render_html "builds the self-contained page"
 * @writes html-file "writes the viewer page"
 ***/
pub fn write_html(graph: &Graph, path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, render_html(graph)).with_context(|| format!("writing {}", path.display()))
}

/***
 * Render the graph as one HTML page that inlines the viewer code, the libraries and the data.
 *
 * Markdown is rendered here, so the page needs no Markdown library. JSON
 * is embedded with `<` escaped, so no text from the docs can close a script tag.
 *
 * @calls docair-viewer.markdown_html "turns Markdown into safe HTML"
 * @creates data.page "renders the self-contained page"
 ***/
pub fn render_html(graph: &Graph) -> String {
    let mut rendered = BTreeMap::new();
    for n in &graph.nodes {
        let docs: Vec<_> = n
            .docs
            .iter()
            .map(|d| json!({ "file": d.file, "anchor": d.anchor, "html": markdown_html(&d.markdown) }))
            .collect();
        if n.description.is_empty() && docs.is_empty() {
            continue;
        }
        rendered.insert(
            n.id.clone(),
            json!({ "description": markdown_html(&n.description), "docs": docs }),
        );
    }
    // The glossary and the docs it links to (SPEC §13): links between them stay inside the page.
    let library: HashSet<String> = graph.library.iter().map(|d| d.file.clone()).collect();
    for t in &graph.glossary {
        let dir = parent_dir(&t.source.file);
        rendered.insert(
            format!("term:{}", t.id),
            json!({
                "summary": markdown_html_in(&t.summary, &dir, &library),
                "body": markdown_html_in(&t.markdown, &dir, &library),
                // The glossary's text while it is not checked against the code (docs/MEANINGS.md §2).
                "unchecked": t.unchecked.as_deref().map(|m| markdown_html_in(m, &dir, &library)),
            }),
        );
    }
    for d in &graph.library {
        rendered.insert(
            format!("doc:{}", d.file),
            json!({ "body": markdown_html_in(&d.markdown, &parent_dir(&d.file), &library) }),
        );
    }
    let graph_json = script_json(&serde_json::to_string(graph).expect("graph serializes"));
    let html_json = script_json(&serde_json::to_string(&rendered).expect("map serializes"));
    let title = escape_text(&format!("docair · {}", graph.system));

    fill(
        TEMPLATE,
        &[
            ("generator", &escape_text(&graph.generator)),
            ("title", &title),
            ("css", CSS),
            ("graph", &graph_json),
            ("html", &html_json),
            ("cytoscape", &script_js(CYTOSCAPE)),
            ("elk", &script_js(ELK)),
            ("app", &script_js(APP)),
        ],
    )
}

/***
 * Render Markdown to HTML, showing raw HTML as text and dropping script links.
 ***/
pub fn markdown_html(md: &str) -> String {
    if md.trim().is_empty() {
        return String::new();
    }
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    let events = Parser::new_ext(md, opts).map(|e| match e {
        Event::Html(s) | Event::InlineHtml(s) => Event::Text(s),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) if is_script_url(&dest_url) => Event::Start(Tag::Link {
            link_type,
            dest_url: CowStr::from("#"),
            title,
            id,
        }),
        e => e,
    });
    let mut out = String::new();
    html::push_html(&mut out, events);
    out
}

fn parent_dir(file: &str) -> String {
    file.rsplit_once('/')
        .map_or(String::new(), |(d, _)| d.to_string())
}

/// A heading's anchor as GitHub makes it: `8. Diagnostics (stage 6)` → `8-diagnostics-stage-6`.
pub fn heading_slug(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

/***
 * Render Markdown of the glossary or a library doc: headings get anchors, and links to docs
 * embedded in the page become `docair-doc:` links that the page opens itself.
 ***/
pub fn markdown_html_in(md: &str, dir: &str, library: &HashSet<String>) -> String {
    if md.trim().is_empty() {
        return String::new();
    }
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    let mut events: Vec<Event> = Parser::new_ext(md, opts)
        .map(|e| match e {
            Event::Html(s) | Event::InlineHtml(s) => Event::Text(s),
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => {
                let dest = if is_script_url(&dest_url) {
                    CowStr::from("#")
                } else {
                    local_doc(&dest_url, dir, library).map_or(dest_url, CowStr::from)
                };
                Event::Start(Tag::Link {
                    link_type,
                    dest_url: dest,
                    title,
                    id,
                })
            }
            e => e,
        })
        .collect();
    // Anchors for headings without one, from their text.
    let mut k = 0;
    while k < events.len() {
        if let Event::Start(Tag::Heading { id: None, .. }) = &events[k] {
            let mut text = String::new();
            let mut j = k + 1;
            while j < events.len() && !matches!(events[j], Event::End(TagEnd::Heading(_))) {
                if let Event::Text(t) | Event::Code(t) = &events[j] {
                    text.push_str(t);
                }
                j += 1;
            }
            if let Event::Start(Tag::Heading { id, .. }) = &mut events[k] {
                *id = Some(CowStr::from(heading_slug(&text)));
            }
        }
        k += 1;
    }
    let mut out = String::new();
    html::push_html(&mut out, events.into_iter());
    out
}

/// `SPEC.md#8-x` from `docs/` → `docair-doc:docs/SPEC.md#8-x`, when that doc is in the page.
fn local_doc(url: &str, dir: &str, library: &HashSet<String>) -> Option<String> {
    if url.contains("://") || url.starts_with('#') || url.starts_with("mailto:") {
        return None;
    }
    let (path, anchor) = url.split_once('#').unwrap_or((url, ""));
    let joined = if dir.is_empty() {
        std::path::PathBuf::from(path)
    } else {
        std::path::Path::new(dir).join(path)
    };
    let file = docair_core::glossary::normalize(&joined)?;
    library
        .contains(&file)
        .then(|| format!("docair-doc:{file}#{anchor}"))
}

fn is_script_url(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    u.starts_with("javascript:") || u.starts_with("vbscript:") || u.starts_with("data:")
}

/// Replace `{{docair:name}}` placeholders. Only the template is scanned, never
/// the inserted values, so data cannot inject placeholders.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    const OPEN: &str = "{{docair:";
    let mut out =
        String::with_capacity(template.len() + values.iter().map(|(_, v)| v.len()).sum::<usize>());
    let mut rest = template;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        let end = after.find("}}").expect("placeholder is closed");
        let name = &after[..end];
        let value = values
            .iter()
            .find(|(k, _)| *k == name)
            .unwrap_or_else(|| panic!("no value for {name}"))
            .1;
        out.push_str(value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// JSON inside `<script type="application/json">`: `<` only occurs inside strings, where `<` is equivalent.
fn script_json(json: &str) -> String {
    json.replace('<', "\\u003c")
}

fn script_js(js: &str) -> String {
    js.replace("</script", "<\\/script")
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_escapes_raw_html_and_script_links() {
        let html = markdown_html("Hi <script>alert(1)</script> [x](javascript:alert(1))");
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("href=\"#\""));
    }

    #[test]
    fn glossary_links_to_embedded_docs_stay_in_the_page() {
        let library: HashSet<String> = ["docs/SPEC.md".to_string()].into();
        let html = markdown_html_in(
            "## 8. Diagnostics (stage 6: check)\n\nSee [§8](SPEC.md#8-diagnostics), [other](OTHER.md), [k8s](https://k8s.io/).",
            "docs",
            &library,
        );
        assert!(
            html.contains("<h2 id=\"8-diagnostics-stage-6-check\">"),
            "{html}"
        );
        assert!(
            html.contains("href=\"docair-doc:docs/SPEC.md#8-diagnostics\""),
            "{html}"
        );
        assert!(
            html.contains("href=\"OTHER.md\""),
            "not embedded: left alone"
        );
        assert!(html.contains("href=\"https://k8s.io/\""));
    }

    #[test]
    fn placeholders_in_values_are_not_expanded() {
        let out = fill(
            "a {{docair:x}} b {{docair:y}}",
            &[("x", "{{docair:y}}"), ("y", "Y")],
        );
        assert_eq!(out, "a {{docair:y}} b Y");
    }

    #[test]
    fn json_cannot_close_the_script_tag() {
        let out = script_json(r#"{"a":"</script>"}"#);
        assert!(!out.contains('<'), "{out}");
        let back: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(back["a"], "</script>", "still the same JSON value");
    }
}

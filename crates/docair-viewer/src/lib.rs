//! docair-viewer: the graph as one self-contained HTML page (SPEC §1, §9).

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use docair_core::model::Graph;
use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd, html};

/***
 * A layer's picture or a slice's as layout input: the words of its parts and lines, their sizes, the hints.
 ***/
pub mod layout_input;
/***
 * Layouts kept on disk under their key, so a picture that did not change is not laid out again.
 ***/
pub mod layout_store;
/***
 * Each picture drawn as one SVG: parts in the shape and colour of their kind, lines with their arrows and words, colours from the page's CSS variables.
 ***/
pub mod svg;
/***
 * A picture drawn as text, for a comment on a pull request: the SVG's layout compressed to character cells, in box-drawing characters.
 ***/
pub mod text_pic;
/***
 * The documentation as one page: every picture a reader can reach, laid out, drawn and packed by the CLI, and a thin script that shows them by their address.
 ***/
pub mod doc_page;
/***
 * The panels around the pictures, written by the CLI: details, tips, the tree, the search words, the legend and the diagnostics.
 ***/
mod page_panels;
/***
 * The glossary and the data in the page, written by the CLI: marked words, popups, the glossary's list and entries, the data's pages, the docs Read more opens.
 ***/
mod page_glossary;
/***
 * Walks in the page, written by the CLI: each scenario, walkthrough and datum's way is one picture, its steps lit, framed and told.
 ***/
mod page_walks;
/***
 * The AI in the page: the panel to ask in, which the CLI writes, and each reply as HTML, written by docair serve.
 ***/
pub mod page_ai;
/***
 * Tidy in the page, written by the CLI: the header's button with the count of tags the code repeats, the banner, the panel that lists them, and their marks in the Tags tab.
 ***/
mod page_tidy;
/***
 * What changed between two versions, in the page, written by the CLI: the list of changed flows, and each flow's slice before and after, side by side.
 ***/
mod page_changes;
/***
 * docair's review on a pull request: a summary comment and one per change, each with its cut drawn as text, and the plan that brings the pull request's comments up to date.
 ***/
pub mod pr;

/// The page's stylesheet: its colours, light and dark, and its panels.
const CSS: &str = include_str!("../assets/viewer.css");

/***
 * Write `docair.html`: the documentation, every picture laid out and drawn by the CLI.
 *
 * @flow   scan 12 "write the outputs"
 * @in     graph: Graph from docair-core.check.check
 * @calls  docair-viewer.render_html "builds the self-contained page"
 * @writes html-file "writes the documentation page"
 ***/
pub fn write_html(graph: &Graph, path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, render_html(graph)).with_context(|| format!("writing {}", path.display()))
}

/***
 * Render the graph as one HTML page: every picture a reader can reach, laid out, drawn and packed by the CLI, with the panels and a thin script (docs/SLICES.md §4).
 *
 * Markdown is rendered here, so the page needs no Markdown library; text from the docs is
 * escaped, so none of it can close a script tag.
 *
 * @calls docair-viewer.doc_page.render_page "lays out, draws and packs every picture"
 * @creates data.page "renders the self-contained page"
 ***/
pub fn render_html(graph: &Graph) -> String {
    doc_page::render_page(graph)
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

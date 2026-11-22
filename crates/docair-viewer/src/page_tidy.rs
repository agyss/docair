//! Tidy in the page (docs/ADAPTERS.md §6.1): the header's button with the count of tags the code
//! repeats, a banner while there are any, the panel that lists them with the code that says the
//! same, and the mark on each in a part's Tags tab. The page's script tidies them through `docair
//! serve` (`POST /api/tidy`); a page opened as a file offers the command instead.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

use docair_core::model::Graph;
use docair_core::tidy::{RepeatedTag, repeated};

use crate::escape_text;
use crate::page_panels::href;

/// The command that tidies a project from the CLI.
const COMMAND: &str = "docair tidy --apply";

/// What the page shows of tidying.
pub(crate) struct Tidy {
    /// The header's button: `✂ Tidy · n`, or `✂ Tidy ✓` when nothing repeats.
    pub button: String,
    /// The banner under the header while tags repeat; empty when none do.
    pub banner: String,
    /// The panel the button opens.
    pub panel: String,
    /// Each repeated tag's mark for the Tags tab, by its part and the tag as written.
    marks: HashMap<(String, String), String>,
}

impl Tidy {
    /// The mark of a tag in the Tags tab, when the code repeats it.
    pub(crate) fn mark(&self, node: &str, tag: &str) -> Option<&str> {
        self.marks
            .get(&(node.to_string(), tag.to_string()))
            .map(String::as_str)
    }
}

fn repeat(n: usize) -> &'static str {
    if n == 1 { "tag repeats" } else { "tags repeat" }
}

/// Where the code says it: `src/lib.rs:12`.
fn code_at(t: &RepeatedTag) -> String {
    match &t.code_file {
        Some(f) if t.code_line > 0 => format!("{f}:{}", t.code_line),
        Some(f) => f.clone(),
        None => String::new(),
    }
}

/// The tag's checkbox or button carries what `POST /api/tidy` needs: its part and the tag.
fn tag_data(t: &RepeatedTag) -> String {
    format!(
        r#"data-node="{}" data-tag="{}""#,
        escape_text(&t.node),
        escape_text(&t.tag)
    )
}

/***
 * Tidy as the page shows it: the tags the code repeats, found the way `docair tidy` finds them, as the header's button, the banner, the panel by file and block, and a mark on each in the Tags tab.
 *
 * @calls docair-core.tidy.repeated "finds the tags the code repeats"
 ***/
pub(crate) fn tidy(graph: &Graph) -> Tidy {
    let tags = repeated(graph);
    let n = tags.len();
    if n == 0 {
        return Tidy {
            button: r#"<button class="btn tidy done" id="btn-tidy" data-test="tidy-button" title="No tag repeats what the code says">✂ Tidy ✓</button>"#.to_string(),
            banner: String::new(),
            panel: r#"<h2>✂ Tidy ✓</h2><p class="empty" data-test="tidy-none">No tag repeats what the code says.</p>"#.to_string(),
            marks: HashMap::new(),
        };
    }
    let button = format!(
        r#"<button class="btn tidy on" id="btn-tidy" data-test="tidy-button" title="{n} {} what the code says: review and tidy">✂ Tidy · {n}</button>"#,
        repeat(n)
    );
    let banner = format!(
        r#"<div id="tidy-banner" data-test="tidy-banner" data-count="{n}">✂ {n} {} what the code says. <a class="link" data-tidy-open data-test="tidy-review">Review and tidy</a> <button class="chip-x" type="button" data-tidy-close title="Hide this until the count changes" aria-label="Hide">×</button></div>"#,
        repeat(n)
    );
    // The tags by the file of their block, then by block.
    let mut by_file: BTreeMap<String, BTreeMap<String, Vec<&RepeatedTag>>> = BTreeMap::new();
    for t in &tags {
        let file = graph
            .nodes
            .iter()
            .find(|x| x.id == t.node)
            .and_then(|x| x.source.as_ref())
            .map(|s| s.file.clone())
            .unwrap_or_default();
        by_file
            .entry(file)
            .or_default()
            .entry(t.node.clone())
            .or_default()
            .push(t);
    }
    let mut panel = format!(
        r#"<h2>✂ Tidy</h2><div class="meta">{n} {} what the code says</div><p>Each of these tags says only what the code says already, with no words of its own. Tidying takes it out of its doc block; the line stays in the pictures, drawn from the code.</p><div class="tidy-list" data-test="tidy-list">"#,
        repeat(n)
    );
    for (file, blocks) in &by_file {
        let _ = write!(panel, r#"<h4 class="mono">{}</h4>"#, escape_text(file));
        for (node, ts) in blocks {
            let name = graph
                .nodes
                .iter()
                .find(|x| &x.id == node)
                .map_or(node.as_str(), |x| x.name.as_str());
            let _ = write!(
                panel,
                r#"<div class="tidy-block"><a class="link" href="{}">{}</a>"#,
                href(&format!("part/{node}")),
                escape_text(name)
            );
            for t in ts {
                let why = t
                    .reason
                    .as_deref()
                    .map(|r| format!(" ({})", escape_text(r)))
                    .unwrap_or_default();
                let _ = write!(
                    panel,
                    r#"<label class="tidy-tag"><input type="checkbox" checked {}> <code>{}</code> <span class="muted">line {}</span></label><div class="muted tidy-code">the code says <code>{}</code> · {}{why}</div>"#,
                    tag_data(t),
                    escape_text(&t.tag),
                    t.line,
                    escape_text(&t.code_says),
                    escape_text(&code_at(t))
                );
            }
            panel.push_str("</div>");
        }
    }
    let _ = write!(
        panel,
        r#"</div><div class="row tidy-actions"><button class="btn primary" type="button" data-test="tidy-apply" data-served hidden>✂ Tidy the ticked tags</button><button class="btn" type="button" data-test="tidy-patch" data-served hidden>⤓ Patch</button><button class="btn" type="button" data-test="tidy-copy" data-command="{COMMAND}" data-copied="Copied: {COMMAND}" title="Copy the command that tidies them all">Copy command</button> <code>{COMMAND}</code></div><pre class="diff" data-test="tidy-diff" hidden></pre>"#
    );
    let marks = tags
        .iter()
        .map(|t| {
            (
                (t.node.clone(), t.tag.clone()),
                format!(
                    r#" <span class="badge tidy" data-test="tidy-mark" title="the code says {}">✂ the code says this · {}</span> <button class="link" type="button" data-tidy-one {} data-served hidden>Tidy this tag</button>"#,
                    escape_text(&t.code_says),
                    escape_text(&code_at(t)),
                    tag_data(t)
                ),
            )
        })
        .collect();
    Tidy {
        button,
        banner,
        panel,
        marks,
    }
}

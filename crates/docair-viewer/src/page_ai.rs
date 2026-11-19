//! The AI in the documentation page (docs/SLICES.md §8): the panel to ask in, written by the CLI into
//! the page, and each reply as HTML, written by `docair serve` with `reply_html`. The page's script
//! sends the reader's questions to the server and shows what comes back; without a server the
//! page says so where the AI would be.

use std::fmt::Write;

use serde_json::Value;

use crate::escape_text;
use crate::page_panels::href;

/// What the panel offers before anything is asked.
const SUGGESTIONS: &[&str] = &[
    "Walk me through the main use case, from start to end.",
    "What is not documented yet? Document it.",
    "I don't care about the infrastructure.",
];

/// The header's button that opens the panel.
pub(crate) const BUTTON: &str = r#"<button class="btn" id="btn-ask" data-test="ask-button" title="Ask docair how something works, or what to change">✦ Ask</button>"#;

/// The panel to ask in: its head, the log with what it offers, the tools, and the field to ask in.
pub(crate) fn panel() -> String {
    let suggestions: String = SUGGESTIONS
        .iter()
        .map(|s| {
            format!(
                r#"<button class="suggest" type="button">{}</button>"#,
                escape_text(s)
            )
        })
        .collect();
    format!(
        r#"<aside id="chat" class="chat" data-test="chat" hidden><header class="chat-head"><span class="title">✦ Ask docair</span><span class="model" data-test="model"></span><button class="icon" type="button" data-chat-close title="Close the conversation" aria-label="Close">×</button></header><div class="log" data-test="log"><div class="intro"><p>Ask how something works, say what you don't care about, or tell docair what is wrong or missing. Follow-up questions keep the context.</p>{suggestions}</div></div><div class="msg ai typing" data-test="typing" hidden><span class="dot"></span><span class="dot"></span><span class="dot"></span> <span class="model-name"></span> is thinking</div><p class="chat-off" data-test="ai-off" hidden>The AI is off: docair serve has no model to ask (<code>docair ai</code> shows the settings).</p><p class="chat-off" data-test="no-server" hidden>The AI needs docair serve: run <code>docair serve</code> in the project and open the page it serves. Everything else works without it.</p><div class="tools"><button class="tool" type="button" data-test="word-lines" title="The model writes a few words on each line that says nothing yet; apply them to write them into the code">✦ Word the lines</button><button class="tool" type="button" data-test="explain-terms" title="The model explains the words of these docs that the glossary does not explain yet; apply them to keep them">✦ Explain the terms</button></div><form class="compose"><div class="context" data-test="context" hidden>about <span class="refs"></span></div><div class="input-row"><textarea rows="2" maxlength="500" placeholder="Ask, or tell docair what to change…" aria-label="Message" data-test="question"></textarea><button class="button primary send" type="submit" data-test="send" aria-label="Send">➤</button></div></form></aside>"#
    )
}

fn text<'v>(v: &'v Value, key: &str) -> &'v str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn list<'v>(v: &'v Value, key: &str) -> &'v [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// A unified diff, line by line, each marked for what it is.
fn diff_html(diff: &str) -> String {
    diff.split('\n')
        .map(|l| {
            let class = if l.starts_with("+++") || l.starts_with("---") || l.starts_with("diff ") {
                "file"
            } else if l.starts_with('+') {
                "add"
            } else if l.starts_with('-') {
                "rm"
            } else if l.starts_with("@@") {
                "hunk"
            } else {
                ""
            };
            format!(r#"<span class="{class}">{}</span>"#, escape_text(l)) + "\n"
        })
        .collect()
}

/***
 * A reply as the chat shows it: the answer, then what came with it (a walkthrough to play, words for lines, terms explained, a view change, edits), the edits waiting to be applied with their diff, and what was left out.
 *
 * `reply` is docair-ai's `Reply` as JSON; `patch` the waiting edits as a unified diff, `pending` how many there are.
 ***/
pub fn reply_html(reply: &Value, patch: &str, pending: usize) -> String {
    let mut out = format!(
        r#"<p class="answer">{}</p>"#,
        escape_text(text(reply, "answer"))
    );
    if let Some(w) = reply.get("walkthrough").filter(|w| w.is_object()) {
        let steps = list(w, "steps").len();
        let title = if text(w, "title").is_empty() {
            text(w, "id")
        } else {
            text(w, "title")
        };
        let _ = write!(
            out,
            r#"<div class="part walk"><div class="walk-title">✦ {}</div><div class="row"><a class="button small primary" href="{}" data-test="play">▶ Play · {steps} steps</a></div></div>"#,
            escape_text(title),
            href(&format!("slice/walkthrough:{}", text(w, "id")))
        );
    }
    let wording = list(reply, "wording");
    if !wording.is_empty() {
        let _ = write!(
            out,
            r#"<div class="part wording" data-test="wording"><div class="edits-head">✦ Words for {} lines, written into the code when applied</div><ul class="words">"#,
            wording.len()
        );
        for w in wording.iter().take(6) {
            let _ = write!(
                out,
                r#"<li><span class="muted">{} →</span> {}</li>"#,
                escape_text(text(w, "from")),
                escape_text(text(w, "text"))
            );
        }
        if wording.len() > 6 {
            let _ = write!(
                out,
                r#"<li class="muted">… {} more</li>"#,
                wording.len() - 6
            );
        }
        out.push_str("</ul></div>");
    }
    let terms = list(reply, "glossary");
    if !terms.is_empty() {
        let _ = write!(
            out,
            r#"<div class="part glossary" data-test="glossary-terms"><div class="edits-head">¶ {} terms explained, written into the glossary when applied</div><dl class="terms">"#,
            terms.len()
        );
        for t in terms.iter().take(5) {
            let _ = write!(
                out,
                "<dt>{}</dt><dd>{}</dd>",
                escape_text(text(t, "term")),
                escape_text(text(t, "definition"))
            );
        }
        out.push_str("</dl>");
        if terms.len() > 5 {
            let _ = write!(
                out,
                r#"<div class="muted">… {} more</div>"#,
                terms.len() - 5
            );
        }
        out.push_str("</div>");
    }
    // A view change hides and shows in the page at once, as the reader's own hiding, with Undo (docair-702.12).
    if let Some(v) = reply.get("view").filter(|v| v.is_object()) {
        let ids = |key: &str| {
            list(v, key)
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
        };
        let (hide, show) = (ids("hide"), ids("show"));
        let said = [
            if hide.is_empty() {
                String::new()
            } else {
                format!("hid {}", hide.join(", "))
            },
            if show.is_empty() {
                String::new()
            } else {
                format!("showed {}", show.join(", "))
            },
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
        if !said.is_empty() {
            let said = format!("{}{}.", said[..1].to_uppercase(), &said[1..]);
            let _ = write!(
                out,
                r#"<div class="part view" data-test="view-change" data-hide="{}" data-show="{}"><span>{}</span> <button class="link" type="button" data-test="undo">Undo</button><span class="muted" data-undone hidden>undone</span></div>"#,
                escape_text(&hide.join(" ")),
                escape_text(&show.join(" ")),
                escape_text(&said)
            );
        }
    }
    let changes = list(reply, "changes");
    if !changes.is_empty() {
        let _ = write!(
            out,
            r#"<div class="part edits" data-test="edits"><div class="edits-head">✎ {} proposed {} to the doc blocks</div>"#,
            changes.len(),
            if changes.len() == 1 { "edit" } else { "edits" }
        );
        for c in changes {
            let line = c
                .get("line")
                .and_then(Value::as_u64)
                .filter(|l| *l > 0)
                .map(|l| format!(":{l}"))
                .unwrap_or_default();
            let _ = write!(
                out,
                r#"<div class="edit"><div class="edit-node"><code>{}</code> <span class="where">{}{line}</span></div>"#,
                escape_text(text(c, "node")),
                escape_text(text(c, "file"))
            );
            if !text(c, "summary").is_empty() {
                let _ = write!(
                    out,
                    r#"<div class="add">+ {}</div>"#,
                    escape_text(text(c, "summary"))
                );
            }
            for a in list(c, "add").iter().filter_map(Value::as_str) {
                let _ = write!(out, r#"<div class="add">+ {}</div>"#, escape_text(a));
            }
            for r in list(c, "remove").iter().filter_map(Value::as_str) {
                let _ = write!(out, r#"<div class="rm">− {}</div>"#, escape_text(r));
            }
            if !text(c, "why").is_empty() {
                let _ = write!(
                    out,
                    r#"<div class="why">{}</div>"#,
                    escape_text(text(c, "why"))
                );
            }
            out.push_str("</div>");
        }
        out.push_str("</div>");
    }
    if pending > 0 && !patch.is_empty() {
        let _ = write!(
            out,
            r#"<div class="part apply" data-test="pending"><div class="muted">{pending} {} to apply</div><pre class="diff" data-test="diff" hidden>{}</pre><div class="row"><button class="button small" type="button" data-test="review"><span>Review diff</span><span hidden>Hide diff</span></button><button class="button small primary" type="button" data-test="apply"><span>Apply &amp; regenerate</span><span hidden>✓ Applied</span></button><button class="link" type="button" data-test="download-patch" title="Save docair.patch; apply it with git apply">⤓ Patch</button></div></div>"#,
            if pending == 1 {
                "edit waits"
            } else {
                "edits wait"
            },
            diff_html(patch)
        );
    }
    let problems = list(reply, "problems");
    if !problems.is_empty() {
        let _ = write!(
            out,
            r#"<details class="left-out"><summary>{} parts left out</summary><ul>"#,
            problems.len()
        );
        for p in problems.iter().filter_map(Value::as_str) {
            let _ = write!(out, "<li>{}</li>", escape_text(p));
        }
        out.push_str("</ul></details>");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_reply_shows_what_came_with_it() {
        let reply = json!({
            "answer": "Here is how an order moves.",
            "walkthrough": { "id": "buy", "title": "Buying", "steps": [{}, {}, {}] },
            "changes": [{ "node": "orders.list", "file": "api/src/lib.rs", "line": 4, "add": ["@reads orders-db -> Order"], "remove": [], "why": "it reads" }],
            "problems": ["ghost"],
        });
        let html = reply_html(
            &reply,
            "+++ b/api/src/lib.rs\n+ * @reads orders-db -> Order",
            1,
        );
        assert!(
            html.contains(r##"href="#slice/walkthrough:buy" data-test="play">▶ Play · 3 steps"##),
            "{html}"
        );
        assert!(
            html.contains("✎ 1 proposed edit to the doc blocks") && html.contains("orders.list")
        );
        assert!(
            html.contains(r#"<span class="add">+ * @reads orders-db -&gt; Order</span>"#),
            "{html}"
        );
        assert!(html.contains(r#"data-test="apply""#) && html.contains("1 parts left out"));
        assert!(
            !reply_html(&json!({ "answer": "<b>no</b>" }), "", 0).contains("<b>"),
            "the answer is text"
        );
        let hid = reply_html(
            &json!({ "answer": "Hidden.", "view": { "hide": ["infra"], "show": [] } }),
            "",
            0,
        );
        assert!(
            hid.contains(r#"data-hide="infra""#)
                && hid.contains("Hid infra.")
                && hid.contains(r#"data-test="undo""#),
            "{hid}"
        );
    }
}

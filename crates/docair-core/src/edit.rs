//! Edits of doc blocks, as proposed by the model or by hand (SPEC §10.6).
//!
//! An edit names a node; [`resolve`] finds where it goes (an existing block,
//! a new block above a code line, or a new fenced block in Markdown), and
//! [`patch`] applies a set of edits to file texts and returns the new texts
//! plus a unified diff that `git apply` takes.

use std::collections::{BTreeMap, HashSet};

use crate::check::KNOWN_TAGS;
use crate::extract::extract_text;
use crate::lang::{Family, language_for_file};
use crate::markdown::read_markdown_text;
use crate::model::{Change, Node, Origin};

/// Where new nodes without code are declared when the edit does not say.
pub const DEFAULT_APPEND_FILE: &str = "docs/docair.md";

/// One file before and after the edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEdit {
    pub path: String,
    /// `None` when the edits create the file.
    pub before: Option<String>,
    pub after: String,
}

#[derive(Debug, Clone, Default)]
pub struct PatchResult {
    pub files: Vec<FileEdit>,
    /// Unified diff with `a/` and `b/` paths, for `git apply`.
    pub diff: String,
    /// Indices of the edits that were applied.
    pub applied: Vec<usize>,
    pub problems: Vec<String>,
}

/// `@calls x` or `calls x` → `("calls", "x")`, if the tag is known.
fn parse_tag(line: &str) -> Result<(String, String), String> {
    let t = line.trim().trim_start_matches('@');
    let (name, value) = t.split_once(char::is_whitespace).unwrap_or((t, ""));
    if !KNOWN_TAGS.contains(&name) {
        return Err(format!("unknown tag `@{name}`"));
    }
    Ok((
        name.to_string(),
        value.split_whitespace().collect::<Vec<_>>().join(" "),
    ))
}

/***
 * Find where an edit goes: the node's block (`edit`), a new block above its code line (`insert`), or a new fenced block in Markdown (`append`).
 *
 * @id   docair-core.edit.resolve
 * @in   change: Change
 * @out  change: Change with mode, file and line
 ***/
pub fn resolve(change: &Change, nodes: &[Node]) -> Result<Change, String> {
    let mut c = change.clone();
    // Text for a Markdown file, such as new glossary entries (SPEC §13.4).
    if c.mode == "text" {
        if !c.file.ends_with(".md") || c.summary.as_deref().is_none_or(|t| t.trim().is_empty()) {
            return Err("a text edit needs a Markdown file and the text".into());
        }
        return Ok(c);
    }
    c.node = c.node.trim().to_string();
    for line in c.add.iter().chain(&c.remove) {
        parse_tag(line)?;
    }
    if c.add.is_empty() && c.remove.is_empty() && c.summary.is_none() {
        return Err(format!("the edit of `{}` changes nothing", c.node));
    }
    let node = nodes.iter().find(|n| n.id == c.node);
    match node {
        Some(n) if n.origin != Origin::Placeholder && n.source.is_some() => {
            let src = n.source.as_ref().expect("checked");
            if src.file.ends_with(".md") && n.origin != Origin::Authored {
                return Err(format!(
                    "`{}` comes from a Markdown file without a block; edit it by hand",
                    c.node
                ));
            }
            c.file = src.file.clone();
            c.line = src.line;
            c.mode = if n.origin == Origin::Authored {
                "edit".into()
            } else {
                "insert".into()
            };
        }
        _ => {
            let has_kind = c
                .add
                .iter()
                .any(|l| parse_tag(l).is_ok_and(|(n, _)| n == "kind"));
            if !has_kind {
                return Err(format!(
                    "`{}` is not in the graph; a new node needs `@kind` in `add`",
                    c.node
                ));
            }
            if !crate::ids::is_valid_id(&c.node) {
                return Err(format!("`{}` is not a valid id", c.node));
            }
            c.mode = "append".into();
            c.line = 0;
            if !c.file.ends_with(".md") {
                c.file = DEFAULT_APPEND_FILE.into();
            }
        }
    }
    Ok(c)
}

/***
 * Apply resolved edits to the files they name and return the new texts and a unified diff.
 *
 * Edits of one file are applied from the bottom up, so line numbers stay
 * valid. Each new text must still parse, and refs that an edit adds must
 * name a node (else the problem is reported, the edit is still made).
 *
 * @id    docair-core.edit.patch
 * @creates data.patch "writes the edits as a diff"
 * @flow  ask 8 "the proposed edits become a patch"
 * @in    changes: Change[]
 * @out   patch: new file texts + unified diff
 ***/
pub fn patch(
    changes: &[Change],
    read: &dyn Fn(&str) -> Option<String>,
    known_ids: &HashSet<String>,
) -> PatchResult {
    let mut out = PatchResult::default();
    let mut by_file: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, c) in changes.iter().enumerate() {
        if c.mode.is_empty() || c.file.is_empty() {
            out.problems
                .push(format!("edit of `{}` is not resolved", c.node));
            continue;
        }
        by_file.entry(c.file.clone()).or_default().push(i);
    }
    let new_ids: HashSet<String> = changes
        .iter()
        .filter(|c| c.mode == "append")
        .map(|c| c.node.clone())
        .collect();
    for (file, mut idx) in by_file {
        let before = read(&file);
        let mut text = before.clone().unwrap_or_default();
        if before.is_none()
            && idx
                .iter()
                .any(|&i| changes[i].mode != "append" && changes[i].mode != "text")
        {
            out.problems.push(format!("{file}: file not found"));
            continue;
        }
        // Bottom up; appends first (they touch only the end).
        idx.sort_by_key(|&i| {
            std::cmp::Reverse(
                if changes[i].mode == "append" || changes[i].mode == "text" {
                    usize::MAX
                } else {
                    changes[i].line
                },
            )
        });
        let mut changed = false;
        for i in idx {
            let c = &changes[i];
            let result = match c.mode.as_str() {
                "edit" => edit_block(&text, c),
                "insert" => insert_block(&text, c),
                "append" => Ok(append_block(&text, c, before.is_none())),
                "text" => Ok(append_text(&text, c, before.is_none())),
                other => Err(format!("unknown mode `{other}`")),
            };
            match result {
                Ok(new) => {
                    if let Err(e) = still_parses(&file, &new) {
                        out.problems
                            .push(format!("{file}: edit of `{}` breaks the file: {e}", c.node));
                        continue;
                    }
                    let unknown =
                        unknown_refs(c, &|r| known_ids.contains(r) || new_ids.contains(r));
                    if !unknown.is_empty() {
                        // The edit would add an unresolved ref (E001): leave it out.
                        out.problems.push(format!(
                            "{file}: edit of `{}` left out: {}",
                            c.node,
                            unknown.join("; ")
                        ));
                        continue;
                    }
                    text = new;
                    changed = true;
                    out.applied.push(i);
                }
                Err(e) => out
                    .problems
                    .push(format!("{file}: edit of `{}`: {e}", c.node)),
            }
        }
        if changed {
            out.diff.push_str(&unified(&file, before.as_deref(), &text));
            out.files.push(FileEdit {
                path: file,
                before,
                after: text,
            });
        }
    }
    out.applied.sort_unstable();
    out
}

/// The refs an edit adds that name no node, as messages. A sub-part ref
/// (`store.table`) resolves through its node, as in stage 5 (SPEC §4.3).
pub fn unknown_refs(change: &Change, known: &dyn Fn(&str) -> bool) -> Vec<String> {
    let resolves = |r: &str| {
        let segs: Vec<&str> = r.split('.').collect();
        (1..=segs.len()).any(|k| known(&segs[..k].join(".")))
    };
    change
        .add
        .iter()
        .filter_map(|line| parse_tag(line).ok())
        .filter_map(|(name, value)| {
            let r = ref_of(&name, &value)?;
            (!resolves(r))
                .then(|| format!("`@{name} {value}` names `{r}`, which is not in the graph"))
        })
        .collect()
}

/// The node a tag refers to, for tags that name one.
fn ref_of<'a>(name: &str, value: &'a str) -> Option<&'a str> {
    let first = || value.split_whitespace().next();
    let r = match name {
        "calls" | "reads" | "writes" | "creates" | "covers" | "part-of" | "deployed-as" => first(),
        "in" => value
            .split(" from ")
            .nth(1)
            .and_then(|v| v.split_whitespace().next()),
        "out" => value
            .split(" to ")
            .nth(1)
            .and_then(|v| v.split_whitespace().next()),
        _ => None,
    }?;
    // `http:` and `topic:` refs resolve by route or create their node.
    (!r.contains(':')).then_some(r)
}

fn still_parses(file: &str, text: &str) -> Result<(), String> {
    let mut diags = Vec::new();
    if file.ends_with(".md") {
        read_markdown_text(file, text, &mut diags);
    } else if let Some(lang) = language_for_file(file) {
        extract_text(file, text, lang.family, &mut diags);
    }
    match diags.iter().find(|d| d.code.starts_with('E')) {
        Some(d) => Err(format!("{} {}", d.code, d.message)),
        None => Ok(()),
    }
}

fn lines_of(text: &str) -> Vec<String> {
    text.lines().map(str::to_string).collect()
}

fn join(lines: &[String]) -> String {
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

/// The prefix of a content line up to its content, and the content.
fn split_content(line: &str, prefix: Option<char>) -> (String, String) {
    let indent = line.len() - line.trim_start().len();
    let mut rest = &line[indent..];
    let mut head = line[..indent].to_string();
    if let Some(p) = prefix
        && let Some(r) = rest.strip_prefix(p)
    {
        head.push(p);
        rest = r;
        let spaces = rest.len() - rest.trim_start().len();
        head.push_str(&rest[..spaces]);
        rest = &rest[spaces..];
    }
    (head, rest.to_string())
}

fn format_tag(name: &str, value: &str, width: usize) -> String {
    let tag = format!("@{name}");
    if value.is_empty() {
        tag
    } else {
        format!("{tag:<width$} {value}", width = width.max(tag.len()))
    }
}

/// Add, remove and re-summarize in the block that starts at `c.line`.
fn edit_block(text: &str, c: &Change) -> Result<String, String> {
    let is_md = c.file.ends_with(".md");
    let (start, end, prefix) = if is_md {
        let doc = read_markdown_text(&c.file, text, &mut Vec::new());
        let b = doc
            .blocks
            .iter()
            .find(|b| b.line == c.line)
            .ok_or("no docair block starts at that line")?;
        (b.line - 1, b.end_line - 1, None)
    } else {
        let lang = language_for_file(&c.file).ok_or("unknown language")?;
        let blocks = extract_text(&c.file, text, lang.family, &mut Vec::new());
        let b = blocks
            .iter()
            .find(|b| b.line == c.line)
            .ok_or("no doc block starts at that line")?;
        let marker = lang
            .family
            .markers()
            .iter()
            .find(|m| {
                text.lines()
                    .nth(b.line - 1)
                    .is_some_and(|l| l.trim_start().starts_with(m.open))
            })
            .ok_or("unknown block opener")?;
        (b.line - 1, b.end_line - 1, marker.prefix)
    };
    if start == end {
        return Err("a one-line block; edit it by hand".into());
    }
    let mut lines = lines_of(text);
    let content = |l: &str| split_content(l, prefix);

    // Replace: a removed tag and an added tag with the same name and ref are one
    // line that changes in place, so rewording a tag keeps its position.
    let first_word = |v: &str| v.split_whitespace().next().unwrap_or("").to_string();
    let mut removes: Vec<(String, String)> =
        c.remove.iter().filter_map(|l| parse_tag(l).ok()).collect();
    let mut adds: Vec<(String, String)> = c.add.iter().filter_map(|l| parse_tag(l).ok()).collect();
    adds.retain(|(name, value)| {
        let Some(r) = removes
            .iter()
            .position(|(n, v)| n == name && first_word(v) == first_word(value))
        else {
            return true;
        };
        let old = removes.remove(r);
        let Some(k) = (start + 1..end).find(|&k| {
            let (_, body) = content(&lines[k]);
            body.starts_with('@') && parse_tag(&body).is_ok_and(|t| t == old)
        }) else {
            removes.push(old);
            return true;
        };
        let (head, body) = content(&lines[k]);
        let name_len = body.find(char::is_whitespace).unwrap_or(body.len());
        let value_at = body[name_len..]
            .find(|ch: char| !ch.is_whitespace())
            .map_or(name_len, |v| name_len + v);
        lines[k] = format!(
            "{head}{}",
            format_tag(name, value, value_at.saturating_sub(1))
        )
        .trim_end()
        .to_string();
        false
    });

    // Remove.
    let wanted = removes;
    let mut end = end;
    let mut k = start + 1;
    while k < end {
        let (_, body) = content(&lines[k]);
        let hit = body.starts_with('@')
            && parse_tag(&body).is_ok_and(|t| {
                wanted
                    .iter()
                    .any(|w| *w == t || (w.0 == t.0 && w.1.is_empty()))
            });
        if hit {
            lines.remove(k);
            end -= 1;
        } else {
            k += 1;
        }
    }

    // Summary: the first paragraph.
    if let Some(summary) = &c.summary {
        let first = (start + 1..end).find(|&k| !content(&lines[k]).1.trim().is_empty());
        if let Some(first) = first.filter(|&k| !content(&lines[k]).1.starts_with('@')) {
            let mut last = first;
            while last + 1 < end && {
                let b = content(&lines[last + 1]).1;
                !b.trim().is_empty() && !b.starts_with('@')
            } {
                last += 1;
            }
            let (head, _) = content(&lines[first]);
            lines.splice(first..=last, [format!("{head}{}", summary.trim())]);
            end -= last - first;
        }
    }

    // Add after the last tag line, aligned like it.
    if !adds.is_empty() {
        let tag_lines: Vec<usize> = (start + 1..end)
            .filter(|&k| content(&lines[k]).1.starts_with('@'))
            .collect();
        let (head, width, at) = match tag_lines.last() {
            Some(&last) => {
                let (head, body) = content(&lines[last]);
                let name_len = body.find(char::is_whitespace).unwrap_or(body.len());
                let value_at = body[name_len..]
                    .find(|ch: char| !ch.is_whitespace())
                    .map_or(name_len, |v| name_len + v);
                (head, value_at.saturating_sub(1).max(name_len), last + 1)
            }
            None => {
                let sample = (start + 1..end).find(|&k| !lines[k].trim().is_empty());
                let head = match (sample, prefix) {
                    (Some(k), _) => content(&lines[k]).0,
                    (None, Some(p)) => format!("{} {p} ", indent_of(&lines[start])),
                    (None, None) => indent_of(&lines[start]),
                };
                let blank = head.trim_end().to_string();
                lines.insert(end, blank);
                end += 1;
                (head, 0, end)
            }
        };
        let new: Vec<String> = adds
            .iter()
            .map(|(n, v)| {
                format!("{head}{}", format_tag(n, v, width))
                    .trim_end()
                    .to_string()
            })
            .collect();
        lines.splice(at..at, new);
    }
    Ok(join(&lines))
}

fn indent_of(line: &str) -> String {
    line[..line.len() - line.trim_start().len()].to_string()
}

/// A new block above the code line `c.line`, in the file's comment syntax.
fn insert_block(text: &str, c: &Change) -> Result<String, String> {
    let lang = language_for_file(&c.file).ok_or("unknown language")?;
    let mut lines = lines_of(text);
    if c.line == 0 || c.line > lines.len() {
        return Err("the code line is outside the file".into());
    }
    // Above attributes and decorators that belong to the item.
    let mut at = c.line - 1;
    while at > 0 {
        let prev = lines[at - 1].trim_start();
        if prev.starts_with("#[") || (lang.name == "typescript" && prev.starts_with('@')) {
            at -= 1;
        } else {
            break;
        }
    }
    let indent = indent_of(&lines[at]);
    let summary = c
        .summary
        .clone()
        .unwrap_or_else(|| c.node.rsplit('.').next().unwrap_or(&c.node).to_string());
    let tags: Vec<(String, String)> = c.add.iter().filter_map(|l| parse_tag(l).ok()).collect();
    let width = tags.iter().map(|(n, _)| n.len() + 1).max().unwrap_or(0);
    let (open, mid, close) = match lang.family {
        Family::CStyle => ("/***".to_string(), " * ".to_string(), " ***/".to_string()),
        Family::Hash => ("#***".to_string(), "# ".to_string(), "#***".to_string()),
        Family::Markup => (
            "<!--***".to_string(),
            "  ".to_string(),
            "***-->".to_string(),
        ),
        Family::Markdown => return Err("Markdown nodes are edited with `append`".into()),
        Family::Data => return Err(format!("{} has no comments to hold a doc block", lang.name)),
    };
    let mut block = vec![
        format!("{indent}{open}"),
        format!("{indent}{mid}{}", summary.trim()),
    ];
    if !tags.is_empty() {
        block.push(format!("{indent}{}", mid.trim_end()));
        for (n, v) in &tags {
            block.push(format!("{indent}{mid}{}", format_tag(n, v, width)));
        }
    }
    block.push(format!("{indent}{close}"));
    lines.splice(at..at, block);
    Ok(join(&lines))
}

/// A fenced `docair` block at the end of a Markdown file.
fn append_block(text: &str, c: &Change, new_file: bool) -> String {
    let mut out = if new_file {
        "# Nodes without code\n\nParts of the system that docair cannot find in the code, declared here.\n".to_string()
    } else {
        text.to_string()
    };
    if !out.ends_with('\n') {
        out.push('\n');
    }
    let mut tags: Vec<(String, String)> = c.add.iter().filter_map(|l| parse_tag(l).ok()).collect();
    if !tags.iter().any(|(n, _)| n == "id") {
        tags.insert(0, ("id".into(), c.node.clone()));
    }
    let width = tags.iter().map(|(n, _)| n.len() + 1).max().unwrap_or(0);
    out.push_str("\n```docair\n");
    out.push_str(c.summary.as_deref().unwrap_or(&c.node).trim());
    out.push_str("\n\n");
    for (n, v) in &tags {
        out.push_str(&format_tag(n, v, width));
        out.push('\n');
    }
    out.push_str("```\n");
    out
}

/// Text at the end of a Markdown file; a new glossary gets a title first.
fn append_text(text: &str, c: &Change, new_file: bool) -> String {
    let mut out = if new_file {
        "# Glossary\n\nWhat the words of these docs mean.\n".to_string()
    } else {
        text.to_string()
    };
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(c.summary.as_deref().unwrap_or("").trim());
    out.push('\n');
    out
}

/// A unified diff of one file, as `git diff` writes it.
pub fn unified(path: &str, before: Option<&str>, after: &str) -> String {
    let old = before.unwrap_or("");
    let diff = similar::TextDiff::from_lines(old, after);
    let (a, b) = match before {
        Some(_) => (format!("a/{path}"), format!("b/{path}")),
        None => ("/dev/null".to_string(), format!("b/{path}")),
    };
    let body = diff
        .unified_diff()
        .context_radius(3)
        .header(&a, &b)
        .to_string();
    let mode = if before.is_none() {
        "new file mode 100644\n"
    } else {
        ""
    };
    format!("diff --git a/{path} b/{path}\n{mode}{body}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SourceRef;

    fn node(id: &str, file: &str, line: usize, origin: Origin) -> Node {
        Node {
            id: id.into(),
            kind: "function".into(),
            name: id.into(),
            summary: String::new(),
            description: String::new(),
            part_of: None,
            source: Some(SourceRef {
                file: file.into(),
                line,
            }),
            symbol: None,
            http: None,
            deployed_as: None,
            docs: vec![],
            tags: vec![],
            origin,
            test: None,
            signature: None,
            runs_in: None,
            code_from: None,
        }
    }

    fn change(node: &str, add: &[&str], remove: &[&str]) -> Change {
        Change {
            node: node.into(),
            mode: String::new(),
            file: String::new(),
            line: 0,
            add: add.iter().map(|s| s.to_string()).collect(),
            remove: remove.iter().map(|s| s.to_string()).collect(),
            summary: None,
            why: "because".into(),
        }
    }

    const RS: &str = "/***\n * Read the settings.\n *\n * @id      srv.config\n * @reads   cm -> A\n ***/\npub fn from_env() {}\n\npub fn plain() {}\n";

    fn ids() -> HashSet<String> {
        ["srv.config", "cm", "secret", "srv.plain"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn tags_are_added_aligned_and_removed() {
        let nodes = [node("srv.config", "src/lib.rs", 1, Origin::Authored)];
        let c = resolve(
            &change(
                "srv.config",
                &["@reads secret -> KEY", "calls srv.plain"],
                &["@reads cm -> A"],
            ),
            &nodes,
        )
        .unwrap();
        assert_eq!((c.mode.as_str(), c.line), ("edit", 1));
        let r = patch(&[c], &|_| Some(RS.to_string()), &ids());
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        let after = &r.files[0].after;
        assert!(
            after.contains(
                " * @id      srv.config\n * @reads   secret -> KEY\n * @calls   srv.plain\n ***/"
            ),
            "{after}"
        );
        assert!(!after.contains("cm -> A"));
        assert!(
            r.diff.starts_with(
                "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n"
            ),
            "{}",
            r.diff
        );
        assert!(r.diff.contains("+ * @reads   secret -> KEY"));
    }

    #[test]
    fn a_node_without_block_gets_one_above_its_code_and_attributes() {
        let text = "fn a() {}\n\n    #[inline]\n    pub fn plain() {}\n";
        let nodes = [node("srv.plain", "src/lib.rs", 4, Origin::Inferred)];
        let mut c = change("srv.plain", &["@calls srv.config"], &[]);
        c.summary = Some("Do the plain thing.".into());
        let c = resolve(&c, &nodes).unwrap();
        assert_eq!(c.mode, "insert");
        let r = patch(&[c], &|_| Some(text.to_string()), &ids());
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(
            r.files[0].after,
            "fn a() {}\n\n    /***\n     * Do the plain thing.\n     *\n     * @calls srv.config\n     ***/\n    #[inline]\n    pub fn plain() {}\n"
        );
    }

    #[test]
    fn new_nodes_are_appended_to_markdown() {
        let nodes = [node("srv.config", "src/lib.rs", 1, Origin::Authored)];
        assert!(
            resolve(&change("ghost", &["@calls srv.config"], &[]), &nodes).is_err(),
            "needs @kind"
        );
        let mut c = change("payments", &["@kind external", "@part-of srv.config"], &[]);
        c.summary = Some("The payment provider.".into());
        let c = resolve(&c, &nodes).unwrap();
        assert_eq!(
            (c.mode.as_str(), c.file.as_str()),
            ("append", DEFAULT_APPEND_FILE)
        );
        let r = patch(&[c], &|_| None, &ids());
        let after = &r.files[0].after;
        assert!(after.contains("```docair\nThe payment provider.\n\n@id      payments\n@kind    external\n@part-of srv.config\n```\n"), "{after}");
        assert!(
            r.diff
                .contains("new file mode 100644\n--- /dev/null\n+++ b/docs/docair.md"),
            "{}",
            r.diff
        );
        assert!(r.problems.is_empty(), "{:?}", r.problems);
    }

    #[test]
    fn edits_that_add_unresolved_refs_are_left_out() {
        let nodes = [node("srv.config", "src/lib.rs", 1, Origin::Authored)];
        // `to` introduces a ref in @out: "bound to the graph" names `the`.
        let bad = resolve(
            &change("srv.config", &["@out cfg: Config bound to the graph"], &[]),
            &nodes,
        )
        .unwrap();
        let good = resolve(
            &change("srv.config", &["@out cfg: Config to srv.plain"], &[]),
            &nodes,
        )
        .unwrap();
        let r = patch(&[bad, good], &|_| Some(RS.to_string()), &ids());
        assert_eq!(r.applied, vec![1]);
        assert_eq!(r.problems.len(), 1, "{:?}", r.problems);
        assert!(r.problems[0].contains("names `the`"), "{:?}", r.problems);
        assert!(
            r.files[0]
                .after
                .contains("@out     cfg: Config to srv.plain")
        );
        assert!(!r.files[0].after.contains("bound to the graph"));
    }

    #[test]
    fn several_edits_in_one_file_keep_their_lines() {
        let text = "/***\n * A.\n *\n * @id a\n ***/\nfn a() {}\n\nfn b() {}\n";
        let nodes = [
            node("a", "x.rs", 1, Origin::Authored),
            node("b", "x.rs", 8, Origin::Inferred),
        ];
        let cs: Vec<Change> = [
            change("a", &["@calls b"], &[]),
            change("b", &["@calls a"], &[]),
        ]
        .iter()
        .map(|c| resolve(c, &nodes).unwrap())
        .collect();
        let known: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let r = patch(&cs, &|_| Some(text.to_string()), &known);
        assert_eq!(r.applied, vec![0, 1]);
        let after = &r.files[0].after;
        assert!(
            after.contains("@id a\n * @calls b\n ***/\nfn a() {}"),
            "{after}"
        );
        assert!(
            after.contains("/***\n * b\n *\n * @calls a\n ***/\nfn b() {}"),
            "{after}"
        );
    }

    #[test]
    fn summaries_are_replaced_and_markdown_blocks_edited() {
        let md =
            "# X\n\n```docair\nOld summary\nthat wraps.\n\n@id      ext\n@kind    external\n```\n";
        let nodes = [node("ext", "README.md", 3, Origin::Authored)];
        let mut c = change("ext", &["@part-of sys"], &[]);
        c.summary = Some("New summary.".into());
        let c = resolve(&c, &nodes).unwrap();
        let known: HashSet<String> = ["ext", "sys"].iter().map(|s| s.to_string()).collect();
        let r = patch(&[c], &|_| Some(md.to_string()), &known);
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(
            r.files[0].after,
            "# X\n\n```docair\nNew summary.\n\n@id      ext\n@kind    external\n@part-of sys\n```\n"
        );
    }

    #[test]
    fn a_reworded_tag_changes_in_place() {
        let nodes = [node("srv.config", "src/lib.rs", 1, Origin::Authored)];
        let c = resolve(
            &change(
                "srv.config",
                &["@reads cm \"reads the limits\" -> A"],
                &["@reads cm -> A"],
            ),
            &nodes,
        )
        .unwrap();
        let r = patch(&[c], &|_| Some(RS.to_string()), &ids());
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(
            r.files[0].after,
            RS.replace(
                " * @reads   cm -> A",
                " * @reads   cm \"reads the limits\" -> A"
            )
        );
    }

    #[test]
    fn text_edits_append_to_markdown_and_make_new_glossaries() {
        let mut c = change("glossary", &[], &[]);
        c.mode = "text".into();
        c.file = "docs/GLOSSARY.md".into();
        c.summary = Some("## digest\n\nThe graph, shortened.".into());
        let c = resolve(&c, &[]).unwrap();
        let r = patch(std::slice::from_ref(&c), &|_| None, &ids());
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(
            r.files[0].after,
            "# Glossary\n\nWhat the words of these docs mean.\n\n## digest\n\nThe graph, shortened.\n"
        );
        let r = patch(&[c], &|_| Some("# G\n\n## a\n\nA.\n".into()), &ids());
        assert_eq!(
            r.files[0].after,
            "# G\n\n## a\n\nA.\n\n## digest\n\nThe graph, shortened.\n"
        );
        let mut bad = change("x", &[], &[]);
        bad.mode = "text".into();
        bad.file = "a.rs".into();
        assert!(resolve(&bad, &[]).is_err());
    }

    #[test]
    fn unknown_tags_are_refused() {
        let nodes = [node("a", "x.rs", 1, Origin::Authored)];
        assert!(
            resolve(&change("a", &["@owner me"], &[]), &nodes)
                .unwrap_err()
                .contains("unknown tag")
        );
    }
}

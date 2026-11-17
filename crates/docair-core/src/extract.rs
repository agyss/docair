//! Stage 2: find doc blocks by comment syntax (SPEC §2).

use crate::lang::{Family, Marker};
use crate::model::{Diagnostic, Following, Hints, Origin, RawBlock, SourceFile, Tag};

/***
 * Find the doc blocks in one source file and split each into description and tags.
 *
 * Generic: it knows only the markers of the file's comment family. An
 * opener must be the first text on its line, so markers inside string
 * literals are ignored. A marker followed by one more star is a banner,
 * not an opener.
 *
 * @flow scan 5 "find doc blocks in code"
 * @in   files: SourceFile[] from docair-core.discover.discover
 * @out  blocks: RawBlock[]
 ***/
pub fn extract(file: &SourceFile, diags: &mut Vec<Diagnostic>) -> Vec<RawBlock> {
    extract_text(&file.rel, &file.text, file.lang.family, diags)
}

/// `extract` on plain text, for tests and other callers.
pub fn extract_text(
    rel: &str,
    text: &str,
    family: Family,
    diags: &mut Vec<Diagnostic>,
) -> Vec<RawBlock> {
    let lines: Vec<&str> = text.lines().collect();
    let markers = family.markers();
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        let Some(marker) = opener(trimmed, markers) else {
            i += 1;
            continue;
        };
        let after = &trimmed[marker.open.len()..];
        // (file line, text) of each content line
        let mut body: Vec<(usize, String)> = Vec::new();
        let end = if let Some(pos) = after.find(marker.close) {
            body.push((i + 1, after[..pos].trim().to_string()));
            i
        } else {
            if !after.trim().is_empty() {
                body.push((i + 1, after.trim().to_string()));
            }
            let mut j = i + 1;
            let mut closed = None;
            while j < lines.len() {
                let lt = lines[j].trim_start();
                if let Some(pos) = find_close(lt, marker) {
                    let before = strip_prefix(&lt[..pos], marker.prefix);
                    if !before.trim().is_empty() {
                        body.push((j + 1, before));
                    }
                    closed = Some(j);
                    break;
                }
                body.push((j + 1, strip_prefix(lt, marker.prefix)));
                j += 1;
            }
            match closed {
                Some(j) => j,
                None => {
                    diags.push(Diagnostic::new("E005", "doc block is never closed").at(rel, i + 1));
                    break;
                }
            }
        };
        let (summary, description, tags) = parse_body(&body, rel, diags);
        blocks.push(RawBlock {
            file: rel.to_string(),
            line: i + 1,
            end_line: end + 1,
            summary,
            description,
            tags,
            following: following_line(&lines, end + 1, family),
            hints: Hints::default(),
            origin: Origin::Authored,
        });
        i = end + 1;
    }
    blocks
}

fn opener(trimmed: &str, markers: &'static [Marker]) -> Option<&'static Marker> {
    markers
        .iter()
        .find(|m| trimmed.starts_with(m.open) && !trimmed[m.open.len()..].starts_with('*'))
}

fn find_close(line: &str, marker: &Marker) -> Option<usize> {
    if marker.close == marker.open {
        // Hash family: the closing line is the marker alone.
        let is_close =
            line.starts_with(marker.close) && !line[marker.close.len()..].starts_with('*');
        is_close.then_some(0)
    } else {
        line.find(marker.close)
    }
}

/// Strip the comment prefix (`*`, `#`) and one following space.
fn strip_prefix(line: &str, prefix: Option<char>) -> String {
    let mut s = line;
    if let Some(p) = prefix
        && let Some(rest) = s.strip_prefix(p)
    {
        s = rest.strip_prefix(' ').unwrap_or(rest);
    }
    s.trim_end().to_string()
}

/// The code line a block attaches to: the next non-blank line, skipping
/// attributes, decorators and ordinary line comments. `None` = free block.
fn following_line(lines: &[&str], from: usize, family: Family) -> Option<Following> {
    let mut depth: i32 = 0;
    for (k, line) in lines.iter().enumerate().skip(from) {
        let t = line.trim();
        if depth > 0 {
            depth += bracket_balance(t);
            continue;
        }
        if t.is_empty() {
            continue;
        }
        if opener(t, family.markers()).is_some() {
            return None;
        }
        let is_attribute = t.starts_with("#[") || t.starts_with("#![");
        let is_decorator =
            t.starts_with('@') && t[1..].starts_with(|c: char| c.is_ascii_alphabetic());
        if is_attribute || is_decorator {
            depth = bracket_balance(t).max(0);
            continue;
        }
        let is_comment = match family {
            Family::CStyle => t.starts_with("//"),
            Family::Hash => t.starts_with('#'),
            _ => false,
        };
        if is_comment {
            continue;
        }
        return Some(Following {
            line: k + 1,
            text: t.to_string(),
        });
    }
    None
}

fn bracket_balance(s: &str) -> i32 {
    s.chars().fold(0, |n, c| match c {
        '(' | '[' | '{' => n + 1,
        ')' | ']' | '}' => n - 1,
        _ => n,
    })
}

/// A `@name value` line, if this is one.
fn tag_line(t: &str) -> Option<(&str, &str)> {
    let rest = t.strip_prefix('@')?;
    let name_len = rest
        .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
        .unwrap_or(rest.len());
    let (name, value) = rest.split_at(name_len);
    let starts_lower = name.starts_with(|c: char| c.is_ascii_lowercase());
    let ends_name = value.is_empty() || value.starts_with(char::is_whitespace);
    (starts_lower && ends_name).then(|| (name, value.trim()))
}

/***
 * Split a block body into summary, description and tags (SPEC §2.2).
 *
 * A tag value continues on the following non-tag, non-blank lines. Text after
 * a blank line that follows the tags is ignored with W106.
 *
 * @in body: (line, text)[] from docair-core.extract.extract
 ***/
pub fn parse_body(
    body: &[(usize, String)],
    rel: &str,
    diags: &mut Vec<Diagnostic>,
) -> (String, String, Vec<Tag>) {
    let mut desc: Vec<&str> = Vec::new();
    let mut tags: Vec<Tag> = Vec::new();
    let mut blank_since_tag = false;
    for (line, text) in body {
        let t = text.trim();
        if let Some((name, value)) = tag_line(t) {
            tags.push(Tag::authored(name, value, *line));
            blank_since_tag = false;
        } else if tags.is_empty() {
            desc.push(text);
        } else if t.is_empty() {
            blank_since_tag = true;
        } else if blank_since_tag {
            diags.push(Diagnostic::new("W106", "text after the tags is ignored").at(rel, *line));
        } else if let Some(last) = tags.last_mut() {
            if !last.value.is_empty() {
                last.value.push(' ');
            }
            last.value.push_str(t);
        }
    }
    let mut lines = desc
        .iter()
        .map(|s| s.trim_end())
        .skip_while(|s| s.is_empty());
    let summary = lines.next().unwrap_or("").trim().to_string();
    let description = lines
        .skip_while(|s| s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string();
    (summary, description, tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixtures write markers with `+++` so that docair does not read its own tests.
    fn fx(s: &str) -> String {
        s.replace("+++", "***")
    }

    fn run(src: &str, family: Family) -> (Vec<RawBlock>, Vec<Diagnostic>) {
        let mut d = Vec::new();
        let b = extract_text("t", &fx(src), family, &mut d);
        (b, d)
    }

    #[test]
    fn c_style_block_with_tags_and_continuation() {
        let (b, d) = run(
            "/+++\n * Persist a new order.\n *\n * Longer text.\n *\n * @id orders.create\n * @in  req: CreateOrder\n *      from shop-ui.cart\n +++/\n#[post(\"/x\")]\npub async fn create_order() {}\n",
            Family::CStyle,
        );
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(b.len(), 1);
        let b = &b[0];
        assert_eq!(b.summary, "Persist a new order.");
        assert_eq!(b.description, "Longer text.");
        assert_eq!(b.tags[0].name, "id");
        assert_eq!(b.tags[1].value, "req: CreateOrder from shop-ui.cart");
        assert_eq!(b.tags[1].line, 7);
        assert_eq!(
            b.following.as_ref().unwrap().text,
            "pub async fn create_order() {}"
        );
        assert_eq!(b.following.as_ref().unwrap().line, 11);
    }

    #[test]
    fn single_line_and_free_blocks() {
        let (b, _) = run(
            "/+++ @id a +++/\n/+++ @id b +++/\nfn x() {}\n",
            Family::CStyle,
        );
        assert_eq!(b.len(), 2);
        assert!(b[0].following.is_none(), "followed by another block");
        assert_eq!(b[1].following.as_ref().unwrap().line, 3);
    }

    #[test]
    fn opener_must_start_the_line_and_banners_are_ignored() {
        let (b, _) = run(
            "let open = \"/+++\";\n/+++++++++++++ banner +++++++++++/\nfn x() {}\n",
            Family::CStyle,
        );
        assert!(b.is_empty(), "{b:?}");
    }

    #[test]
    fn hash_family() {
        let (b, d) = run(
            "#+++\n# Orders API.\n# @kind workload\n#+++\n# comment\nkind: Deployment\n",
            Family::Hash,
        );
        assert!(d.is_empty());
        assert_eq!(b[0].summary, "Orders API.");
        assert_eq!(b[0].tags[0].value, "workload");
        assert_eq!(b[0].following.as_ref().unwrap().text, "kind: Deployment");
    }

    #[test]
    fn markup_family_and_multiline_decorator() {
        let (b, _) = run(
            "<!--+++\n  Cart page.\n  @kind component\n+++-->\n<div id=\"cart\">\n",
            Family::Markup,
        );
        assert_eq!(b[0].summary, "Cart page.");
        assert_eq!(b[0].following.as_ref().unwrap().text, "<div id=\"cart\">");

        let (b, _) = run(
            "/+++ Cart. +++/\n@Component({\n  selector: 'cart',\n})\nexport class Cart {}\n",
            Family::CStyle,
        );
        assert_eq!(
            b[0].following.as_ref().unwrap().text,
            "export class Cart {}"
        );
    }

    #[test]
    fn unclosed_block_and_text_after_tags() {
        let (_, d) = run("/+++\n * @id a\n", Family::CStyle);
        assert_eq!(d[0].code, "E005");
        let (_, d) = run(
            "/+++\n * @id a\n *\n * stray\n +++/\nfn x() {}\n",
            Family::CStyle,
        );
        assert_eq!(d[0].code, "W106");
    }

    #[test]
    fn decorator_names_in_text_are_not_tags() {
        let (b, _) = run(
            "/+++\n * Uses @Injectable services.\n * @kind service\n +++/\nclass A {}\n",
            Family::CStyle,
        );
        assert_eq!(b[0].summary, "Uses @Injectable services.");
        assert_eq!(b[0].tags.len(), 1);
    }
}

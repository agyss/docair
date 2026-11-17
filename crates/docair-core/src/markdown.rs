//! Stage 2 for Markdown: anchored sections, fenced `docair` blocks, and the
//! rest of the text (SPEC §5). Parsed with pulldown-cmark, whose heading
//! attributes extension reads the `{#node-id}` anchors.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag as MdTag, TagEnd};

use crate::extract::parse_body;
use crate::model::{Diagnostic, Hints, MdDoc, Origin, RawBlock, SourceFile};

/***
 * Read one Markdown file into anchored sections, declared nodes and the rest of the text.
 *
 * `## Heading {#node-id}` attaches the section below it to that node.
 * A fenced `docair` code block declares a node that has no code.
 *
 * @flow scan 5 "read Markdown next to the code"
 * @in   files: SourceFile[] (Markdown) from docair-core.discover.discover
 * @out  doc: MdDoc
 ***/
pub fn read_markdown(file: &SourceFile, diags: &mut Vec<Diagnostic>) -> MdDoc {
    read_markdown_text(&file.rel, &file.text, diags)
}

pub fn read_markdown_text(rel: &str, text: &str, diags: &mut Vec<Diagnostic>) -> MdDoc {
    let opts =
        Options::ENABLE_HEADING_ATTRIBUTES | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    let mut headings: Vec<(usize, HeadingLevel, Option<String>)> = Vec::new();
    let mut fence: Option<(usize, String)> = None;
    let mut blocks = Vec::new();

    for (event, range) in Parser::new_ext(text, opts).into_offset_iter() {
        match event {
            Event::Start(MdTag::Heading { level, id, .. }) => {
                headings.push((range.start, level, id.map(|s| s.to_string())));
            }
            Event::Start(MdTag::CodeBlock(CodeBlockKind::Fenced(info)))
                if info.split_whitespace().next() == Some("docair") =>
            {
                fence = Some((range.start, String::new()));
            }
            Event::Text(t) => {
                if let Some((_, content)) = fence.as_mut() {
                    content.push_str(&t);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((start, content)) = fence.take() {
                    let fence_line = line_of(text, start);
                    let body: Vec<(usize, String)> = content
                        .lines()
                        .enumerate()
                        .map(|(k, l)| (fence_line + 1 + k, l.trim().to_string()))
                        .collect();
                    let (summary, description, tags) = parse_body(&body, rel, diags);
                    blocks.push(RawBlock {
                        file: rel.to_string(),
                        line: fence_line,
                        end_line: fence_line + body.len() + 1,
                        summary,
                        description,
                        tags,
                        following: None,
                        hints: Hints::default(),
                        origin: Origin::Authored,
                    });
                }
            }
            _ => {}
        }
    }

    // A section runs from its heading to the next heading of the same or a higher level.
    let mut sections = Vec::new();
    let mut taken: Vec<(usize, usize)> = Vec::new();
    for (k, (start, level, id)) in headings.iter().enumerate() {
        let Some(id) = id else { continue };
        let end = headings[k + 1..]
            .iter()
            .find(|(_, l, _)| l <= level)
            .map_or(text.len(), |(s, _, _)| *s);
        sections.push((
            id.clone(),
            text[*start..end].trim_end().to_string(),
            line_of(text, *start),
        ));
        taken.push((*start, end));
    }

    let mut rest = String::new();
    let mut pos = 0;
    taken.sort();
    for (start, end) in taken {
        if start >= pos {
            rest.push_str(&text[pos..start]);
        }
        pos = pos.max(end);
    }
    rest.push_str(&text[pos.min(text.len())..]);

    MdDoc {
        file: rel.to_string(),
        rest: rest.trim().to_string(),
        sections,
        blocks,
    }
}

fn line_of(text: &str, offset: usize) -> usize {
    text[..offset].matches('\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_fences_and_rest() {
        let md = "# Orders\nIntro.\n\n## Creating {#orders.create}\nDetails.\n\n### Edge cases\nMore.\n\n## Other\nTail.\n\n```docair\n@id    payments\n@kind  external\n```\n";
        let mut d = Vec::new();
        let doc = read_markdown_text("x.md", md, &mut d);
        assert_eq!(doc.sections.len(), 1);
        let (id, text, line) = &doc.sections[0];
        assert_eq!(id, "orders.create");
        assert!(
            text.contains("Edge cases"),
            "subsections belong to the section"
        );
        assert!(!text.contains("Tail"));
        assert_eq!(*line, 4);
        assert!(doc.rest.starts_with("# Orders\nIntro."));
        assert!(doc.rest.contains("Tail."));
        assert!(!doc.rest.contains("Details."));
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0].tags[0].value, "payments");
        assert_eq!(doc.blocks[0].tags[0].line, 14);
    }

    #[test]
    fn fences_nested_in_other_fences_are_examples() {
        let md = "````md\n```docair\n@id x\n```\n````\n";
        let doc = read_markdown_text("x.md", md, &mut Vec::new());
        assert!(doc.blocks.is_empty());
    }
}

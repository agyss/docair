//! The glossary: what the words of the docs mean (SPEC §13).
//!
//! A glossary is Markdown: every `##` (or `###`) heading is a term, an
//! optional `Also:` line lists other spellings, an optional `See:` line names
//! nodes, and the rest is the definition. Its first paragraph is what a hover
//! shows. Relative links to Markdown files of the project put those files into
//! the page's library, so "read more" works offline.

use std::collections::{BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};

use crate::config::Config;
use crate::ids::slug;
use crate::model::{Diagnostic, LibraryDoc, Node, SourceRef, Term};

/// Where a new glossary goes when the project has none.
pub const DEFAULT_FILE: &str = "docs/GLOSSARY.md";

/// Whether a project file is a glossary: named `GLOSSARY.md`, or listed in `glossary`.
pub fn is_glossary(rel: &str, config: &Config) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.eq_ignore_ascii_case("glossary.md") || config.glossary.iter().any(|g| g == rel)
}

/***
 * Read the terms of a glossary file.
 *
 * @id    docair-core.glossary.parse
 * @flow  scan 5 "read the glossary"
 * @reads glossary-files -> Term[]
 ***/
pub fn parse(rel: &str, text: &str) -> Vec<Term> {
    let mut terms = Vec::new();
    let mut current: Option<(String, Option<String>, usize, Vec<String>)> = None;
    let mut fence = false;
    let flush = |cur: Option<(String, Option<String>, usize, Vec<String>)>, out: &mut Vec<Term>| {
        let Some((term, id, line, body)) = cur else {
            return;
        };
        let mut aliases = Vec::new();
        let mut see = Vec::new();
        let mut rest = Vec::new();
        for l in body {
            let t = l.trim();
            if rest.iter().all(|r: &String| r.trim().is_empty()) {
                if let Some(a) = t.strip_prefix("Also:") {
                    aliases.extend(
                        a.split(',')
                            .map(|x| x.trim().to_string())
                            .filter(|x| !x.is_empty()),
                    );
                    continue;
                }
                if let Some(s) = t.strip_prefix("See:") {
                    see.extend(
                        s.split([',', ' '])
                            .map(|x| x.trim().trim_matches('`').to_string())
                            .filter(|x| !x.is_empty()),
                    );
                    continue;
                }
            }
            rest.push(l);
        }
        let markdown = rest.join("\n").trim().to_string();
        let summary = markdown
            .split("\n\n")
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        out.push(Term {
            id: id.unwrap_or_else(|| slug(&term.to_lowercase())),
            term,
            aliases,
            summary,
            markdown,
            see,
            source: SourceRef {
                file: rel.to_string(),
                line,
            },
        });
    };
    for (k, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        let heading = (!fence)
            .then(|| {
                line.strip_prefix("### ")
                    .or_else(|| line.strip_prefix("## "))
            })
            .flatten();
        if let Some(h) = heading {
            flush(current.take(), &mut terms);
            let (text, id) = match h.trim().rsplit_once("{#") {
                Some((t, rest)) if rest.ends_with('}') => (
                    t.trim().to_string(),
                    Some(rest.trim_end_matches('}').to_string()),
                ),
                _ => (h.trim().to_string(), None),
            };
            current = Some((text, id, k + 1, Vec::new()));
        } else if let Some(cur) = current.as_mut() {
            cur.3.push(line.to_string());
        }
    }
    flush(current, &mut terms);
    terms
}

/// Duplicate terms, `See:` ids that name no node, empty definitions: W112.
pub fn check(terms: &[Term], nodes: &[Node], diags: &mut Vec<Diagnostic>) {
    let ids: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let mut seen = HashSet::new();
    for t in terms {
        let at = |d: Diagnostic| d.at(&t.source.file, t.source.line);
        for word in std::iter::once(&t.term).chain(&t.aliases) {
            if !seen.insert(word.to_lowercase()) {
                diags.push(at(Diagnostic::new(
                    "W112",
                    format!("glossary: `{word}` is defined twice"),
                )));
            }
        }
        if t.markdown.is_empty() {
            diags.push(at(Diagnostic::new(
                "W112",
                format!("glossary: `{}` has no definition", t.term),
            )));
        }
        for s in &t.see {
            if !ids.contains(s.as_str()) {
                diags.push(at(Diagnostic::new(
                    "W112",
                    format!(
                        "glossary: `{}` sees `{s}`, which is not in the graph",
                        t.term
                    ),
                )));
            }
        }
    }
}

/// Whether `word` (or its plural) occurs in `text` as a whole word; both lowercase.
pub fn occurs_word(word: &str, text: &str) -> bool {
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-');
    let mut from = 0;
    while let Some(i) = text[from..].find(word) {
        let start = from + i;
        let mut end = start + word.len();
        for plural in ["es", "s"] {
            if text[end..].starts_with(plural) {
                end += plural.len();
                break;
            }
        }
        if !is_word(text[..start].chars().next_back()) && !is_word(text[end..].chars().next()) {
            return true;
        }
        from = start + text[start..].chars().next().map_or(1, char::len_utf8);
        if from >= text.len() {
            return false;
        }
    }
    false
}

/// `docs/../SPEC.md` → `SPEC.md`; `None` when the path leaves the project.
pub fn normalize(path: &Path) -> Option<String> {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(out.to_string_lossy().replace('\\', "/"))
}

/// The project Markdown files a text links to, relative to the file it is in.
pub fn linked_docs(rel: &str, markdown: &str) -> Vec<String> {
    let dir = Path::new(rel).parent().unwrap_or(Path::new(""));
    let mut out = Vec::new();
    let mut rest = markdown;
    while let Some(i) = rest.find("](") {
        rest = &rest[i + 2..];
        let Some(end) = rest.find(')') else { break };
        let target = rest[..end].trim();
        let path = target.split('#').next().unwrap_or("");
        if !path.is_empty()
            && !target.contains("://")
            && !target.starts_with("mailto:")
            && path.ends_with(".md")
            && let Some(p) = normalize(&dir.join(path))
        {
            out.push(p);
        }
    }
    out
}

/***
 * Embed the project docs that glossary entries link to, so the page can open them.
 *
 * @id    docair-core.glossary.library
 * @reads project-files -> linked Markdown docs
 ***/
pub fn library(root: &Path, terms: &[Term]) -> Vec<LibraryDoc> {
    let files: BTreeSet<String> = terms
        .iter()
        .flat_map(|t| linked_docs(&t.source.file, &t.markdown))
        .collect();
    files
        .into_iter()
        .filter_map(|file| {
            let markdown = std::fs::read_to_string(root.join(&file)).ok()?;
            Some(LibraryDoc { file, markdown })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLOSSARY: &str = "# Glossary\n\nIntro text.\n\n## Diagnostics\n\nAlso: diagnostic, W104\nSee: docair-core.check\n\nWhat `docair check` reports.\nIt has codes.\n\nMore in [SPEC §8](SPEC.md#8-diagnostics) and [k8s](https://kubernetes.io/).\n\n```md\n## not a term\n```\n\n### Graph {#the-graph}\n\nNodes and edges.\n";

    #[test]
    fn terms_aliases_see_and_summary() {
        let t = parse("docs/GLOSSARY.md", GLOSSARY);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].term, "Diagnostics");
        assert_eq!(t[0].id, "diagnostics");
        assert_eq!(t[0].aliases, vec!["diagnostic", "W104"]);
        assert_eq!(t[0].see, vec!["docair-core.check"]);
        assert_eq!(t[0].summary, "What `docair check` reports.\nIt has codes.");
        assert!(
            t[0].markdown.contains("## not a term"),
            "fenced headings stay in the text"
        );
        assert_eq!(t[0].source.line, 5);
        assert_eq!(
            (t[1].id.as_str(), t[1].term.as_str()),
            ("the-graph", "Graph")
        );
        assert_eq!(
            linked_docs("docs/GLOSSARY.md", &t[0].markdown),
            vec!["docs/SPEC.md"]
        );
        assert_eq!(
            linked_docs("docs/G.md", "[x](../README.md) [y](../../etc.md)"),
            vec!["README.md"]
        );
    }

    #[test]
    fn problems_are_w112() {
        let mut t = parse(
            "g.md",
            "## A\n\nAlso: B\n\nText.\n\n## b\n\n## C\nSee: ghost\n\nText.\n",
        );
        t.truncate(3);
        let mut diags = vec![];
        check(&t, &[], &mut diags);
        let msgs: Vec<_> = diags.iter().map(|d| d.message.clone()).collect();
        assert_eq!(
            diags.iter().filter(|d| d.code == "W112").count(),
            3,
            "{msgs:?}"
        );
        assert!(msgs.iter().any(|m| m.contains("`b` is defined twice")));
        assert!(msgs.iter().any(|m| m.contains("`b` has no definition")));
        assert!(msgs.iter().any(|m| m.contains("`ghost`")));
    }
}

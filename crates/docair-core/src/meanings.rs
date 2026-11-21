//! Meanings: what the words of the docs mean, from the code, the glossary and
//! doc headings, the code first (docs/MEANINGS.md).
//!
//! A word that names a part (a module, a component, a data) means what that
//! part's docs say, once it is settled that the part defines it: by its own
//! summary ("Facts: …") or by a decision in the decisions file. Words that are
//! not settled are open (W120); a glossary entry not compared with the code's
//! current text is unchecked (W121).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::ids::slug;
use crate::model::{Diagnostic, Edge, Node, SourceFile, SourceRef, Term, TermSection, Walkthrough};

/// Where the decisions go when docair.toml names no other file.
pub const DEFAULT_FILE: &str = "docs/meanings.toml";

/// The kinds of parts whose names are words (docs/MEANINGS.md §2).
const WORD_KINDS: [&str; 3] = ["module", "component", "data"];

/// How many texts must use a word before it is open (docs/MEANINGS.md §3).
const MIN_USES: usize = 2;

/// What was decided about one word (docs/MEANINGS.md §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// `false`: an ordinary word here, never explained.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub term: bool,
    /// The part whose docs define the word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The part the word names where it is written as code: in backticks (`merged`,
    /// `change_steps::merged`), called (`merged()`). Decided apart from `term`, which is about its
    /// plain use: a word may be everyday in prose and still name a part as code (docair-8l6.88).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// The fingerprint of that part's text when the glossary entry was compared with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<String>,
    /// One short reason, for the reviewer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The model proposed the glossary entry: once the entry is gone from the glossary, the
    /// reviewer dropped it, and the word is not proposed again (docair-6sg).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub proposed: bool,
}

impl Default for Decision {
    fn default() -> Self {
        Decision {
            term: true,
            from: None,
            code: None,
            checked: None,
            why: None,
            proposed: false,
        }
    }
}

fn yes() -> bool {
    true
}

fn is_yes(b: &bool) -> bool {
    *b
}

/// A word that names parts and is used, but nothing settles what it means (W120).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenWord {
    pub word: String,
    /// The parts named so; one of them may define it.
    pub parts: Vec<String>,
    /// How many texts use it.
    pub uses: usize,
    /// A doc heading that defines it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<TermSection>,
}

/// A word the code defines whose glossary entry is not compared with the code's current text (W121).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unchecked {
    pub word: String,
    /// The part that defines it.
    pub node: String,
    /// The glossary entry's id.
    pub term: String,
    /// The part's fingerprint now: what `checked` becomes once they are compared.
    pub fingerprint: String,
}

/// The decisions and what is still to settle; the model's explain run reads it (docs/MEANINGS.md §6).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meanings {
    /// The decisions file, relative to the root.
    pub file: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub decisions: BTreeMap<String, Decision>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<OpenWord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unchecked: Vec<Unchecked>,
    /// Words several texts use that name no part, have no glossary entry and no decision
    /// (`NodePort`, `CIDR`): found by docair, explained by the model if a reader needs them (docair-6sg).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unnamed: Vec<String>,
}

impl Meanings {
    pub fn is_empty(&self) -> bool {
        self.decisions.is_empty() && self.open.is_empty() && self.unchecked.is_empty()
    }
}

/// The decisions file as read: the decisions and the line of each.
#[derive(Debug, Clone, Default)]
pub struct Decisions {
    pub file: String,
    pub exists: bool,
    pub words: BTreeMap<String, Decision>,
    lines: HashMap<String, usize>,
}

/// The decisions file this project uses.
pub fn file_of(config: &Config) -> String {
    config
        .meanings
        .clone()
        .unwrap_or_else(|| DEFAULT_FILE.to_string())
}

/***
 * Read the decisions about words: which part defines a word, which words are ordinary.
 *
 * @id    docair-core.meanings.load
 * @reads meanings-file -> decisions "reads what was decided about the words"
 ***/
pub fn load(root: &Path, config: &Config, diags: &mut Vec<Diagnostic>) -> Decisions {
    let file = file_of(config);
    let inside = crate::config::inside(root, &file).filter(|p| p.is_file());
    if inside.is_none() && root.join(&file).exists() {
        diags.push(
            Diagnostic::new(
                "W113",
                format!("the decisions about words, {file}, are no file inside the project"),
            )
            .at("docair.toml", 1),
        );
    }
    let Some(text) = inside.and_then(|p| std::fs::read_to_string(p).ok()) else {
        return Decisions {
            file,
            ..Decisions::default()
        };
    };
    match parse(&text) {
        Ok(words) => Decisions {
            lines: lines_of(&text, &words),
            file,
            exists: true,
            words,
        },
        Err(e) => {
            diags.push(
                Diagnostic::new(
                    "W113",
                    format!("the decisions about words cannot be read: {e}"),
                )
                .at(&file, 1),
            );
            Decisions {
                file,
                exists: true,
                ..Decisions::default()
            }
        }
    }
}

/// The decisions in a decisions file, keyed by the word in lower case.
pub fn parse(text: &str) -> Result<BTreeMap<String, Decision>, String> {
    let words: BTreeMap<String, Decision> =
        toml::from_str(text).map_err(|e| e.message().to_string())?;
    Ok(words
        .into_iter()
        .map(|(k, v)| (k.trim().to_lowercase(), v))
        .collect())
}

/// The line of each word's table, for diagnostics.
fn lines_of(text: &str, words: &BTreeMap<String, Decision>) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    for (k, line) in text.lines().enumerate() {
        let t = line.trim();
        if let Some(h) = t.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            let key = h.trim().trim_matches('"').to_lowercase();
            if words.contains_key(&key) {
                out.insert(key, k + 1);
            }
        }
    }
    out
}

/// The decisions file's text: a header, then one table per word in order.
pub fn to_toml(words: &BTreeMap<String, Decision>) -> String {
    let mut out = String::from(
        "# What the words of the docs mean: decisions written by `docair glossary`\n\
         # (docs/MEANINGS.md in docair). Review them like code.\n",
    );
    let quote = |s: &str| toml::Value::String(s.to_string()).to_string();
    for (word, d) in words {
        let bare = !word.is_empty()
            && word
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        out.push_str(&format!(
            "\n[{}]\n",
            if bare { word.clone() } else { quote(word) }
        ));
        if !d.term {
            out.push_str("term = false\n");
        }
        if let Some(f) = &d.from {
            out.push_str(&format!("from = {}\n", quote(f)));
        }
        if let Some(c) = &d.checked {
            out.push_str(&format!("checked = {}\n", quote(c)));
        }
        if let Some(w) = &d.why {
            out.push_str(&format!("why = {}\n", quote(w)));
        }
    }
    out
}

/// FNV-1a (64 bits) of a part's summary and description, whitespace collapsed: the first 8 hex digits.
pub fn fingerprint(node: &Node) -> String {
    let text = format!("{}\n\n{}", node.summary, node.description);
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in flat.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")[..8].to_string()
}

/// A word without its plural ending: `facts` → `fact`, `classes` → `class`.
pub fn stem(word: &str) -> String {
    let w = word.trim().to_lowercase();
    if w.ends_with("ss") || w.len() < 4 {
        return w;
    }
    if let Some(s) = w.strip_suffix("ies") {
        return format!("{s}y");
    }
    if let Some(s) = w.strip_suffix("es")
        && (s.ends_with('s') || s.ends_with('x') || s.ends_with("ch") || s.ends_with("sh"))
    {
        return s.to_string();
    }
    w.strip_suffix('s').map_or(w.clone(), str::to_string)
}

/// The words of a part's name, in lower case: `LibraryDoc` → `library doc`; `None` for names of things (`docair-core`, `index.html`).
pub fn name_words(name: &str) -> Option<String> {
    if name.is_empty()
        || name.contains(['-', '.', '/', ':', '@'])
        || !name.chars().any(char::is_alphabetic)
    {
        return None;
    }
    let words: Vec<String> = name
        .split_whitespace()
        .map(|w| slug(w).replace(['-', '_'], " "))
        .collect();
    let out = words
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (out.chars().filter(|c| c.is_alphanumeric()).count() >= 3).then_some(out)
}

/// Whether a part's summary defines its word: `Facts: …`, `A digest is …`.
pub fn defines_itself(node: &Node, word: &str) -> bool {
    let s = node.summary.trim().to_lowercase();
    let st = stem(word);
    let forms = [
        st.clone(),
        format!("{st}s"),
        format!("{st}es"),
        word.to_lowercase(),
    ];
    forms.iter().any(|f| {
        [
            format!("{f}:"),
            format!("{f} is "),
            format!("{f} are "),
            format!("a {f} is "),
            format!("an {f} is "),
            format!("the {f} is "),
            format!("the {f}:"),
            format!("the {f} are "),
        ]
        .iter()
        .any(|p| s.starts_with(p.as_str()))
    })
}

/// A text in lower case, whitespace collapsed, code spans left out.
fn plain(text: &str) -> String {
    let mut out = String::new();
    for (i, part) in text.split('`').enumerate() {
        if i % 2 == 0 {
            out.push_str(part);
        }
        out.push(' ');
    }
    out.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a plain text uses `word` (lower case, its plural too) as a word: not inside an id, a path or a file name.
pub fn uses_word(word: &str, text: &str) -> bool {
    match word.strip_suffix('y') {
        Some(s) if word.len() > 3 => used(word, text) || used(&format!("{s}ies"), text),
        _ => used(word, text),
    }
}

fn used(word: &str, text: &str) -> bool {
    let inner = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
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
        let before = text[..start].chars().next_back();
        let after: Vec<char> = text[end..].chars().take(2).collect();
        let joined = |c: Option<&char>| c.is_some_and(|&c| c == '.' || c == '/');
        let ok_before = !before.is_some_and(|c| inner(c) || "./#".contains(c));
        let ok_after = !after.first().is_some_and(|&c| inner(c))
            && !(joined(after.first()) && after.get(1).is_some_and(|&c| inner(c)));
        if ok_before && ok_after {
            return true;
        }
        from = start + text[start..].chars().next().map_or(1, char::len_utf8);
        if from >= text.len() {
            return false;
        }
    }
    false
}

/// GitHub's anchor of a heading: `4. Facts: the contract` → `4-facts-the-contract`.
pub fn anchor(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// A heading `Word: the rest` of a doc, with what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defined {
    pub word: String,
    pub rest: String,
    pub section: TermSection,
    pub line: usize,
}

/// The headings of Markdown files that define a word: `## 4. Facts: the contract between adapters and core`.
pub fn sections(files: &[SourceFile]) -> Vec<Defined> {
    let mut out = Vec::new();
    for f in files {
        let mut fence = false;
        for (k, line) in f.text.lines().enumerate() {
            if line.trim_start().starts_with("```") {
                fence = !fence;
            }
            if fence {
                continue;
            }
            let Some(h) = line
                .strip_prefix('#')
                .map(|l| l.trim_start_matches('#'))
                .filter(|l| l.starts_with(' '))
            else {
                continue;
            };
            let heading = h.trim();
            let unnumbered = heading
                .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.')
                .trim_start();
            let Some((word, rest)) = unnumbered.split_once(':') else {
                continue;
            };
            let word = word.trim();
            let words = word.split_whitespace().count();
            if !(1..=3).contains(&words)
                || !word
                    .chars()
                    .all(|c| c.is_alphabetic() || c == ' ' || c == '-')
                || rest.trim().is_empty()
            {
                continue;
            }
            out.push(Defined {
                word: word.to_string(),
                rest: rest.trim().to_string(),
                section: TermSection {
                    file: f.rel.clone(),
                    anchor: anchor(heading),
                    title: heading.to_string(),
                },
                line: k + 1,
            });
        }
    }
    out
}

/// The texts that use words: every part's summary and description, the words on lines, steps, glossary entries.
fn texts<'a>(
    nodes: &'a [Node],
    edges: &'a [Edge],
    walkthroughs: &'a [Walkthrough],
    terms: &'a [Term],
) -> Vec<(Option<&'a str>, String)> {
    let mut out: Vec<(Option<&str>, String)> = Vec::new();
    for n in nodes {
        let t = plain(&format!("{}\n{}", n.summary, n.description));
        if !t.is_empty() {
            out.push((Some(n.id.as_str()), t));
        }
    }
    for e in edges {
        if let Some(t) = &e.text {
            out.push((None, plain(t)));
        }
    }
    for w in walkthroughs {
        out.push((None, plain(&w.summary)));
        for s in &w.steps {
            out.push((None, plain(&format!("{} {}", s.title, s.text))));
        }
    }
    for t in terms {
        out.push((None, plain(&t.markdown)));
    }
    out
}

/// The parts of the graph, the glossary and the doc headings, put together.
pub struct Input<'a> {
    pub terms: Vec<Term>,
    pub nodes: &'a [Node],
    pub edges: &'a [Edge],
    pub walkthroughs: &'a [Walkthrough],
    pub docs: &'a [SourceFile],
    pub decisions: &'a Decisions,
}

/***
 * Settle what each word means: the part whose docs define it, else the glossary, else a doc heading; the open and unchecked words.
 *
 * @id      docair-core.meanings.settle
 * @flow    scan 11 "settle what the words mean"
 * @in      glossary terms, nodes, doc headings, decisions
 * @out     terms: Term[] with where each meaning comes from, and Meanings
 ***/
pub fn settle(input: Input, diags: &mut Vec<Diagnostic>) -> (Vec<Term>, Meanings) {
    let Input {
        terms,
        nodes,
        edges,
        walkthroughs,
        docs,
        decisions,
    } = input;
    let warn = !terms.is_empty() || decisions.exists;
    let by_id: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    // Glossary entries by the stems of their term and spellings.
    let mut glossary: HashMap<String, usize> = HashMap::new();
    for (i, t) in terms.iter().enumerate() {
        for w in std::iter::once(&t.term).chain(&t.aliases) {
            glossary.entry(stem(w)).or_insert(i);
        }
    }
    // Parts by the words of their names, in graph order.
    let mut named: BTreeMap<String, (String, Vec<&Node>)> = BTreeMap::new();
    for n in nodes {
        if !WORD_KINDS.contains(&n.kind.as_str()) {
            continue;
        }
        if let Some(w) = name_words(&n.name) {
            named
                .entry(stem(&w))
                .or_insert_with(|| (w, Vec::new()))
                .1
                .push(n);
        }
    }
    let mut headings: HashMap<String, &Defined> = HashMap::new();
    let defined = sections(docs);
    for d in &defined {
        headings.entry(stem(&d.word)).or_insert(d);
    }
    // `@defines word` on a block settles the word in the code: that part defines it. The
    // decisions file wins where both speak (docair-6sg).
    let mut words = decisions.words.clone();
    for n in nodes {
        for t in n
            .tags_named("defines")
            .filter(|t| t.origin == crate::model::Origin::Authored)
        {
            for w in t
                .value
                .split(',')
                .map(|w| w.trim().trim_matches('"').to_lowercase())
                .filter(|w| !w.is_empty())
            {
                words.entry(w).or_insert_with(|| Decision {
                    from: Some(n.id.clone()),
                    why: Some("@defines in its block".into()),
                    ..Decision::default()
                });
            }
        }
    }
    let decided: HashMap<String, (&String, &Decision)> =
        words.iter().map(|(k, d)| (stem(k), (k, d))).collect();
    let all_texts = texts(nodes, edges, walkthroughs, &terms);

    let mut meanings = Meanings {
        file: decisions.file.clone(),
        decisions: decisions.words.clone(),
        unnamed: unnamed_words(
            &cased_texts(nodes, edges, walkthroughs, &terms),
            &glossary,
            &named,
            &decided,
        ),
        ..Meanings::default()
    };
    // A decision may name any part, also one whose name is not its word (a function).
    for (st, (key, d)) in &decided {
        if let Some(n) = d.from.as_deref().and_then(|f| by_id.get(f))
            && !named.contains_key(st)
        {
            named.insert(st.clone(), ((*key).clone(), vec![*n]));
        }
    }
    // Glossary entries keep their place; a word the code defines replaces its entry.
    let mut settled: Vec<Term> = terms.clone();
    let mut used = vec![false; terms.len()];
    let mut out: Vec<Term> = Vec::new();

    for (st, (word, parts)) in &named {
        let decision = decided.get(st).map(|(_, d)| *d);
        let entry = glossary.get(st).copied();
        let heading = headings.get(st).copied();
        if decision.is_some_and(|d| !d.term) {
            continue;
        }
        // The part that defines the word: by a decision, or by its own summary.
        let mut code: Option<&Node> = None;
        if let Some(d) = decision
            && let Some(from) = &d.from
        {
            match by_id.get(from.as_str()) {
                Some(n) => code = Some(n),
                None => {
                    let (key, _) = decided[st];
                    let line = decisions.lines.get(key).copied().unwrap_or(1);
                    if warn {
                        diags.push(
                            Diagnostic::new(
                                "W121",
                                format!("`{key}`: the decisions say `{from}` defines it, which is not in the graph"),
                            )
                            .at(&decisions.file, line),
                        );
                    }
                    continue;
                }
            }
        } else if decision.is_none() && parts.len() == 1 && defines_itself(parts[0], word) {
            code = Some(parts[0]);
        }
        let code =
            code.filter(|n| !n.summary.trim().is_empty() || !n.description.trim().is_empty());

        if code.is_none() && decision.is_none() {
            let own: Vec<&str> = parts.iter().map(|n| n.id.as_str()).collect();
            let uses = all_texts
                .iter()
                .filter(|(owner, t)| !owner.is_some_and(|o| own.contains(&o)) && uses_word(word, t))
                .count();
            if uses >= MIN_USES {
                meanings.open.push(OpenWord {
                    word: word.clone(),
                    parts: own.iter().map(|s| s.to_string()).collect(),
                    uses,
                    section: heading.map(|h| h.section.clone()),
                });
                if warn {
                    let first = parts[0];
                    let what = if parts.len() == 1 {
                        if first.summary.trim().is_empty() {
                            format!("names `{}`, which has no docs", first.id)
                        } else {
                            format!("names `{}`", first.id)
                        }
                    } else {
                        format!(
                            "names {} parts ({})",
                            parts.len(),
                            own.iter()
                                .map(|s| format!("`{s}`"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    };
                    let mut d = Diagnostic::new(
                        "W120",
                        format!(
                            "`{word}` {what} and {uses} texts use it; what it means is not settled: `docair glossary` decides (docs/MEANINGS.md)"
                        ),
                    )
                    .node(&first.id);
                    if let Some(s) = &first.source {
                        d = d.at(&s.file, s.line);
                    }
                    diags.push(d);
                }
            }
        }

        let Some(n) = code else {
            // No part defines it: the glossary or a heading may, when a decision makes it a term.
            if entry.is_none()
                && decision.is_some_and(|d| d.term)
                && let Some(h) = heading
            {
                out.push(from_heading(word, h));
            }
            continue;
        };
        let source = n.source.clone().unwrap_or(SourceRef {
            file: String::new(),
            line: 0,
            end: None,
        });
        let code_summary = without_lead(&n.summary, word);
        let code_markdown = if n.description.trim().is_empty() {
            code_summary.clone()
        } else {
            format!("{code_summary}\n\n{}", n.description.trim())
        };
        let mut term = match entry {
            Some(i) => {
                used[i] = true;
                let g = &terms[i];
                let fp = fingerprint(n);
                if decision.and_then(|d| d.checked.as_deref()) == Some(fp.as_str()) {
                    Term { ..g.clone() }
                } else {
                    meanings.unchecked.push(Unchecked {
                        word: word.clone(),
                        node: n.id.clone(),
                        term: g.id.clone(),
                        fingerprint: fp,
                    });
                    if warn {
                        diags.push(
                            Diagnostic::new(
                                "W121",
                                format!(
                                    "glossary: `{}` is not checked against `{}`, whose docs define it: `docair glossary` compares them (docs/MEANINGS.md)",
                                    g.term, n.id
                                ),
                            )
                            .at(&g.source.file, g.source.line),
                        );
                    }
                    Term {
                        summary: code_summary.clone(),
                        markdown: code_markdown,
                        unchecked: Some(g.markdown.clone()),
                        ..g.clone()
                    }
                }
            }
            None => {
                let st_word = stem(word);
                Term {
                    id: slug(&word.replace(' ', "-")),
                    term: word.clone(),
                    aliases: plurals(word, &st_word),
                    summary: code_summary,
                    markdown: code_markdown,
                    see: vec![],
                    source,
                    from: String::new(),
                    node: None,
                    section: None,
                    unchecked: None,
                }
            }
        };
        term.from = "code".into();
        term.node = Some(n.id.clone());
        term.section = heading.map(|h| h.section.clone());
        match entry {
            Some(i) => settled[i] = term,
            None => out.push(term),
        }
    }

    // Glossary entries no part defines stay as they are, with a heading's section if one defines them.
    for (i, t) in settled.iter_mut().enumerate() {
        if !used[i] && t.section.is_none() {
            t.section = std::iter::once(&t.term)
                .chain(&t.aliases)
                .find_map(|w| headings.get(&stem(w)))
                .map(|h| h.section.clone());
        }
    }
    // A decision that names a word no part has: not a part's word, so nothing to check.
    for (st, (key, d)) in &decided {
        if named.contains_key(st) || glossary.contains_key(st) || !d.term {
            continue;
        }
        if let Some(from) = &d.from
            && !by_id.contains_key(from.as_str())
            && warn
        {
            let line = decisions.lines.get(*key).copied().unwrap_or(1);
            diags.push(
                Diagnostic::new(
                    "W121",
                    format!(
                        "`{key}`: the decisions say `{from}` defines it, which is not in the graph"
                    ),
                )
                .at(&decisions.file, line),
            );
        } else if d.from.is_none()
            && let Some(h) = headings.get(st)
        {
            out.push(from_heading(&h.word.to_lowercase(), h));
        }
    }
    out.sort_by_key(|t| t.term.to_lowercase());
    settled.extend(out);
    (settled, meanings)
}

/// The same texts as `texts`, with their case, code spans left out: for words written like names.
fn cased_texts(
    nodes: &[Node],
    edges: &[Edge],
    walkthroughs: &[Walkthrough],
    terms: &[Term],
) -> Vec<String> {
    let prose = |t: &str| -> String { t.split('`').step_by(2).collect::<Vec<_>>().join(" ") };
    let mut out: Vec<String> = Vec::new();
    for n in nodes {
        out.push(prose(&format!("{}\n{}", n.summary, n.description)));
    }
    out.extend(edges.iter().filter_map(|e| e.text.as_deref()).map(prose));
    for w in walkthroughs {
        out.push(prose(&w.summary));
        out.extend(
            w.steps
                .iter()
                .map(|s| prose(&format!("{} {}", s.title, s.text))),
        );
    }
    out.extend(terms.iter().map(|t| prose(&t.markdown)));
    out
}

/// Words that look like names of things (`NodePort`, `CIDR`, `ClusterIP`) and several texts use,
/// that no part is named after and nothing explains or settles: at most 20, the most used first.
fn unnamed_words(
    texts: &[String],
    glossary: &HashMap<String, usize>,
    named: &BTreeMap<String, (String, Vec<&Node>)>,
    decided: &HashMap<String, (&String, &Decision)>,
) -> Vec<String> {
    let looks_named = |w: &str| {
        let letters: Vec<char> = w.chars().collect();
        let upper = letters.iter().filter(|c| c.is_ascii_uppercase()).count();
        let acronym = (3..=6).contains(&letters.len()) && upper == letters.len();
        let camel = letters.len() >= 5
            && letters[0].is_ascii_uppercase()
            && letters[1..].iter().any(|c| c.is_ascii_lowercase())
            && letters[1..].iter().any(|c| c.is_ascii_uppercase());
        letters.iter().all(|c| c.is_ascii_alphabetic()) && (acronym || camel)
    };
    let mut uses: BTreeMap<String, usize> = BTreeMap::new();
    for text in texts {
        let mut here: Vec<&str> = text
            // `_` stays inside a word: `DOCAIR_MAX_FILES` is a variable, not three words.
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|w| {
                looks_named(w) && !["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"].contains(w)
            })
            .collect();
        here.sort_unstable();
        here.dedup();
        for w in here {
            *uses.entry(w.to_string()).or_default() += 1;
        }
    }
    let mut out: Vec<(usize, String)> = uses
        .into_iter()
        .filter(|(w, n)| {
            // `ScanResult` is the part named `scan result`.
            let words = name_words(w).map_or_else(|| stem(w), |x| stem(&x));
            let known = |st: &str| {
                glossary.contains_key(st) || named.contains_key(st) || decided.contains_key(st)
            };
            *n >= MIN_USES && !known(&stem(w)) && !known(&words)
        })
        .map(|(w, n)| (n, w))
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    out.into_iter().take(20).map(|(_, w)| w).collect()
}

/// A summary without the word it defines in front: `Facts: what adapters learn` → `What adapters learn`.
fn without_lead(summary: &str, word: &str) -> String {
    let s = summary.trim();
    let st = stem(word);
    let lead = s.split_once(':').filter(|(w, rest)| {
        let w = w.trim().to_lowercase();
        let w = w.strip_prefix("the ").unwrap_or(&w);
        stem(w) == st && !rest.trim().is_empty()
    });
    match lead {
        Some((_, rest)) => {
            let rest = rest.trim();
            let mut c = rest.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        }
        None => s.to_string(),
    }
}

/// The other spellings of a word from code: `facts` → `fact`; `entry` → `entries` (the page finds `-s` and `-es` itself).
fn plurals(word: &str, stem: &str) -> Vec<String> {
    let mut out = Vec::new();
    if stem != word {
        out.push(stem.to_string());
    }
    if let Some(s) = stem.strip_suffix('y')
        && stem.len() > 3
    {
        out.push(format!("{s}ies"));
    }
    out
}

/// A word defined only by a doc heading: the words after the colon, and a link to the section.
fn from_heading(word: &str, h: &Defined) -> Term {
    let name = h.section.file.rsplit('/').next().unwrap_or(&h.section.file);
    let mut rest = h.rest.trim_end_matches('.').to_string();
    if let Some(first) = rest.chars().next() {
        rest = first.to_uppercase().chain(rest.chars().skip(1)).collect();
    }
    let summary = format!("{rest}.");
    Term {
        id: slug(&word.replace(' ', "-")),
        term: word.to_string(),
        aliases: vec![],
        markdown: format!(
            "{summary}\n\n[{name}: {}]({name}#{})",
            h.section.title, h.section.anchor
        ),
        summary,
        see: vec![],
        source: SourceRef {
            file: h.section.file.clone(),
            line: h.line,
            end: None,
        },
        from: "doc".into(),
        node: None,
        section: Some(h.section.clone()),
        unchecked: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Origin;

    fn node(id: &str, kind: &str, name: &str, summary: &str) -> Node {
        Node {
            id: id.into(),
            kind: kind.into(),
            name: name.into(),
            summary: summary.into(),
            description: String::new(),
            part_of: None,
            source: Some(SourceRef {
                file: format!("src/{name}.rs"),
                line: 1,
                end: None,
            }),
            symbol: None,
            http: None,
            deployed_as: None,
            docs: vec![],
            tags: vec![],
            origin: Origin::Authored,
            test: None,
            signature: None,
            runs_in: None,
            code_from: None,
        }
    }

    fn term(t: &str, text: &str) -> Term {
        Term {
            id: slug(t),
            term: t.into(),
            aliases: vec![],
            summary: text.into(),
            markdown: text.into(),
            see: vec![],
            source: SourceRef {
                file: "docs/GLOSSARY.md".into(),
                line: 3,
                end: None,
            },
            from: String::new(),
            node: None,
            section: None,
            unchecked: None,
        }
    }

    fn doc(rel: &str, text: &str) -> SourceFile {
        SourceFile {
            path: rel.into(),
            rel: rel.into(),
            lang: crate::lang::language_for("md").expect("markdown"),
            text: text.into(),
        }
    }

    fn run(
        nodes: &[Node],
        terms: Vec<Term>,
        docs: &[SourceFile],
        decisions: &str,
    ) -> (Vec<Term>, Meanings, Vec<Diagnostic>) {
        let words = parse(decisions).expect("decisions parse");
        let d = Decisions {
            file: DEFAULT_FILE.into(),
            exists: !decisions.is_empty(),
            lines: lines_of(decisions, &words),
            words,
        };
        let mut diags = vec![];
        let (t, m) = settle(
            Input {
                terms,
                nodes,
                edges: &[],
                walkthroughs: &[],
                docs,
                decisions: &d,
            },
            &mut diags,
        );
        (t, m, diags)
    }

    #[test]
    fn words_of_names_stems_and_uses() {
        assert_eq!(name_words("LibraryDoc").as_deref(), Some("library doc"));
        assert_eq!(name_words("test_results").as_deref(), Some("test results"));
        assert_eq!(name_words("facts").as_deref(), Some("facts"));
        assert_eq!(name_words("docair-core"), None);
        assert_eq!(name_words("index.html"), None);
        assert_eq!(name_words("io"), None);
        assert_eq!(stem("facts"), "fact");
        assert_eq!(stem("classes"), "class");
        assert_eq!(stem("class"), "class");
        assert_eq!(stem("glossary"), "glossary");
        assert_eq!(stem("entries"), "entry");
        assert!(uses_word("entry", &plain("Two entries.")));
        let t = plain("Adapters report `facts` and facts. See docair-core.facts or facts.rs");
        assert!(uses_word("facts", &t));
        assert!(!uses_word(
            "facts",
            &plain("See docair-core.facts or facts.rs and `facts`")
        ));
        assert!(uses_word("fact", &plain("Each fact, and the facts.")));
        assert!(!uses_word("fact", &plain("factory")));
        assert_eq!(
            anchor("4. Facts: the contract between adapters and core"),
            "4-facts-the-contract-between-adapters-and-core"
        );
        assert_eq!(
            anchor("8. Diagnostics (stage 6: check)"),
            "8-diagnostics-stage-6-check"
        );
    }

    #[test]
    fn a_summary_that_defines_its_word_settles_it() {
        let nodes = [
            node(
                "core.facts",
                "module",
                "facts",
                "Facts: what adapters learn from the code.",
            ),
            node("core.lib", "module", "lib", "Adapters report facts."),
            node(
                "core.merge",
                "function",
                "merge",
                "Merges facts and blocks.",
            ),
        ];
        let (t, m, diags) = run(&nodes, vec![], &[], "");
        assert!(diags.is_empty(), "{diags:?}");
        assert!(m.open.is_empty());
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].term.as_str(), t[0].from.as_str()), ("facts", "code"));
        assert_eq!(
            t[0].summary, "What adapters learn from the code.",
            "the lead word goes"
        );
        assert_eq!(t[0].node.as_deref(), Some("core.facts"));
        assert_eq!(t[0].aliases, vec!["fact"]);
        assert_eq!(t[0].source.file, "src/facts.rs");
    }

    /// `@defines` settles a word in the code; the decisions file wins where both speak (docair-6sg).
    #[test]
    fn a_defines_tag_settles_a_word_in_the_code() {
        let mut ledger = node(
            "books.ledger",
            "module",
            "ledger",
            "The record of all payments.",
        );
        ledger.tags.push(crate::model::Tag {
            name: "defines".into(),
            value: "ledger".into(),
            line: 2,
            origin: Origin::Authored,
            reason: None,
            reach: None,
        });
        let nodes = [
            ledger,
            node(
                "books.audit",
                "function",
                "audit",
                "Compares the ledger with the bank.",
            ),
        ];
        let terms = vec![term("ledger", "The book of every payment ever made.")];
        let (_, m, diags) = run(&nodes, terms, &[], "");
        assert!(m.open.is_empty(), "{:?}", m.open);
        assert!(!diags.iter().any(|d| d.code == "W120"), "{diags:?}");
        assert_eq!(
            m.unchecked.len(),
            1,
            "settled from the code, so the entry is checked against it"
        );
        assert_eq!(m.unchecked[0].node, "books.ledger");
    }

    /// Words like names of things that no part, entry or decision explains are found for the explain run (docair-6sg).
    #[test]
    fn words_that_name_no_part_are_found() {
        let nodes = [
            node(
                "net.svc",
                "endpoint",
                "svc",
                "A NodePort Service on the CIDR of the cluster.",
            ),
            node(
                "net.ing",
                "endpoint",
                "ing",
                "Reaches the NodePort from outside; the CIDR is fixed.",
            ),
            node("net.pod", "workload", "pod", "Gets an IP from the CIDR."),
        ];
        let (_, m, _) = run(&nodes, vec![], &[], "");
        assert_eq!(
            m.unnamed,
            ["CIDR", "NodePort"],
            "most used first; IP is too short"
        );
        let (_, m, _) = run(&nodes, vec![term("CIDR", "A block of addresses.")], &[], "");
        assert_eq!(m.unnamed, ["NodePort"], "an entry explains it");
    }

    #[test]
    fn a_name_alone_is_open_and_warns_with_a_glossary() {
        let nodes = [
            node(
                "core.facts",
                "module",
                "facts",
                "What adapters learn from the code.",
            ),
            node("core.lib", "module", "lib", "Adapters report facts."),
            node(
                "core.merge",
                "function",
                "merge",
                "Merges facts and blocks.",
            ),
        ];
        let (t, m, diags) = run(&nodes, vec![], &[], "");
        assert!(t.is_empty());
        assert_eq!(m.open.len(), 1);
        assert_eq!((m.open[0].word.as_str(), m.open[0].uses), ("facts", 2));
        assert!(
            diags.is_empty(),
            "no glossary and no decisions: no warnings"
        );

        let (_, _, diags) = run(&nodes, vec![term("cart", "The cart.")], &[], "");
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, "W120");
        assert!(
            diags[0].message.contains("`facts` names `core.facts`"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn decisions_settle_words_and_an_entry_is_checked_against_the_code() {
        let nodes = [
            node(
                "core.facts",
                "module",
                "facts",
                "What adapters learn from the code.",
            ),
            node("core.check", "module", "check", "The checks."),
            node(
                "core.graph",
                "component",
                "Graph",
                "Everything docair knows.",
            ),
            node(
                "core.lib",
                "module",
                "lib",
                "Adapters report facts; check the graph.",
            ),
            node(
                "core.x",
                "function",
                "x",
                "Facts to check, a graph to draw.",
            ),
        ];
        let glossary = vec![term("graph", "The nodes and the edges.")];
        let decisions = "[facts]\nfrom = \"core.facts\"\n\n[check]\nterm = false\n\n[graph]\nfrom = \"core.graph\"\n";
        let (t, m, diags) = run(&nodes, glossary.clone(), &[], decisions);
        assert!(m.open.is_empty(), "{:?}", m.open);
        let codes: Vec<_> = diags.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, ["W121"], "{diags:?}");
        let graph = t.iter().find(|t| t.term == "graph").expect("graph");
        assert_eq!(
            graph.summary, "Everything docair knows.",
            "unchecked: the code's words"
        );
        assert_eq!(graph.unchecked.as_deref(), Some("The nodes and the edges."));
        assert_eq!(m.unchecked.len(), 1);
        assert!(t.iter().all(|t| t.term != "check"));

        // Once compared, the glossary's words are shown, and nothing warns.
        let fp = fingerprint(&nodes[2]);
        let decisions = decisions.replace(
            "from = \"core.graph\"\n",
            &format!("from = \"core.graph\"\nchecked = \"{fp}\"\n"),
        );
        let (t, m, diags) = run(&nodes, glossary.clone(), &[], &decisions);
        assert!(diags.is_empty(), "{diags:?}");
        assert!(m.unchecked.is_empty());
        let graph = t.iter().find(|t| t.term == "graph").expect("graph");
        assert_eq!(
            (graph.summary.as_str(), graph.from.as_str()),
            ("The nodes and the edges.", "code")
        );
        assert_eq!(graph.node.as_deref(), Some("core.graph"));

        // A changed text opens the comparison again.
        let mut changed = nodes.clone();
        changed[2].summary = "All docair knows about a system.".into();
        let (_, m, diags) = run(&changed, glossary, &[], &decisions);
        assert_eq!(m.unchecked.len(), 1);
        assert_eq!(diags[0].code, "W121");

        // A decision naming a part that is gone.
        let (_, _, diags) = run(&nodes, vec![], &[], "[facts]\nfrom = \"core.gone\"\n");
        let gone = diags.iter().find(|d| d.code == "W121").expect("W121");
        assert!(gone.message.contains("`core.gone`"), "{}", gone.message);
        assert_eq!(
            (gone.file.as_deref(), gone.line),
            (Some(DEFAULT_FILE), Some(1))
        );
    }

    #[test]
    fn headings_give_sections_and_define_decided_words() {
        let docs = [doc(
            "docs/ADAPTERS.md",
            "# Adapters\n\n## 4. Facts: the contract between adapters and core\n\n```md\n## Fenced: no\n```\n\n## 2. Doc blocks (stage 2: extract)\n\n## Output: `x`\n",
        )];
        let s = sections(&docs);
        assert_eq!(s.len(), 2, "{s:?}");
        assert_eq!(s[0].word, "Facts");
        assert_eq!(
            s[0].section.anchor,
            "4-facts-the-contract-between-adapters-and-core"
        );
        let nodes = [
            node(
                "core.facts",
                "module",
                "facts",
                "Facts: what adapters learn.",
            ),
            node("core.lib", "module", "lib", "Adapters report facts."),
        ];
        let (t, _, _) = run(&nodes, vec![], &docs, "");
        let facts = t.iter().find(|t| t.term == "facts").expect("facts");
        assert_eq!(
            facts.section.as_ref().map(|s| s.file.as_str()),
            Some("docs/ADAPTERS.md")
        );
        assert!(
            t.iter().all(|t| t.term != "output"),
            "a heading alone is not a term"
        );

        let (t, _, _) = run(&[], vec![], &docs, "[output]\nwhy = \"the graph file\"\n");
        let out = t.iter().find(|t| t.term == "output").expect("decided");
        assert_eq!((out.from.as_str(), out.summary.as_str()), ("doc", "`x`."));
    }

    #[test]
    fn the_file_round_trips() {
        let words = parse(
            "[facts]\nfrom = \"a.b\"\nwhy = \"it \\\"is\\\"\"\n\n[\"library doc\"]\nterm = false\n",
        )
        .expect("parses");
        let text = to_toml(&words);
        assert_eq!(parse(&text).expect("again"), words);
        assert!(text.contains("[\"library doc\"]\nterm = false\n"), "{text}");
    }
}

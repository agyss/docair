//! The model explains the words of the docs: new glossary entries, and what
//! the words that name parts mean, the code first (SPEC §13.4, docs/MEANINGS.md §6).

use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, bail};
use docair_core::glossary::DEFAULT_FILE;
use docair_core::meanings::{self, Decision, OpenWord, Unchecked, stem};
use docair_core::model::{Change, Graph, Term};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ask::Reply;
use crate::client::{Message, Usage};
use crate::config::AiConfig;

/// The short question about words written as code (docair-8l6.90).
const CODE_PROMPT: &str = r#"You read software documentation. Each word below is written as code in the sentences "as code" (in backticks, with :: or ()). Say which of its "parts" those uses name: its id, or "" when they name none of them. Reply with exactly one JSON object: {"code": [{"word": "...", "part": "<id or empty>"}]}"#;

/// The rules for the entries, the decisions and the checks.
pub const GLOSSARY_PROMPT: &str = r#"You write the glossary of a software project's documentation. You get the texts of the docs (summaries and descriptions of the parts of the system, and the words on the lines between them) and the terms the glossary already explains.

Pick the words a newcomer to this project would not know and that the glossary does not explain yet: words of this project ("digest", "walkthrough") and technologies ("Ingress", "NodePort", "tree-sitter"). Skip everyday words and names of single functions.

For each term:
- "term": as it is written in the texts, singular; keep the capitals of proper names (Ingress, Traefik).
- "aliases": other spellings in the texts (not plurals; those are found anyway).
- "definition": 1 to 3 short sentences in plain words. For a word of this project, what it is here. For a technology, what it is in general, then what this project does with it.
- "see": ids of the parts (from the texts) where the term is implemented or used, at most 3.
- "link": for a technology, the URL of its official documentation; else "".

The request may also list "Open words": words that name parts of the code, each with those parts' texts, and the glossary's entry when there is one. The code is the best source of what a word means. Decide each word once:
- "term": false when it is an everyday word here that a reader knows (a verb such as "check", a plain noun such as "message"). Look at "used", the sentences that use it: when they use it in its everyday sense ("the following", "placed", "a change", "the project"), it is an everyday word here, even though a part has that name. true only when a reader should get it explained.
- "from": the id of the part whose text says what the word means in those sentences; one of the ids listed with the word, or of "named so in the code"; "" when none of them does. A part that merely shares the name is not it: a parser's tree node does not define "node" when the sentences mean a part on the chart.
- "code": the id of the part the word names where it is written as code ("as code": in backticks such as `merged` or `change_steps::merged`, or called, merged()); one of "named so in the code", which may be a function; "" only when "as code" is empty or its uses mean none of them. Whenever an "as code" sentence names one of "named so in the code", set "code" to that id, also when "term" is false. Decide it apart from "term": a word can be everyday in prose ("edges are merged", term false) and still name a part where it is written as code. Judge each use on its own; when one is in doubt, decide it from the sentence around it.
- "definition": only with "from", and only when the part's text is too terse for a newcomer: that text in 1 to 3 plain sentences, never changing its meaning; else "".
- "agrees": only with "from" and a glossary entry: true when the entry means the same as the part's text; false when it does not, and then "definition" says it the way the code does.
- "why": one short reason, for the reviewer.

It may also list "Entries to check": glossary entries of words the code defines, with the code's text. The code is right. For each: "agrees": true when the entry means the same; else false, with "definition": the entry rewritten in 1 to 3 sentences to agree with the code.

Decide quickly: go through the texts once and take the terms that stand out; do not weigh every word. Write in the language of the texts. Reply with exactly one JSON object: {"terms": [{"term": "...", "aliases": [], "definition": "...", "see": [], "link": ""}], "decisions": [{"word": "...", "term": true, "from": "", "code": "", "definition": "", "agrees": true, "why": "..."}], "checks": [{"word": "...", "agrees": true, "definition": ""}]}"#;

/// One entry the model proposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermDraft {
    pub term: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub definition: String,
    #[serde(default)]
    pub see: Vec<String>,
    #[serde(default)]
    pub link: String,
}

/// What the model decided about an open word.
#[derive(Debug, Clone, Deserialize)]
struct DecisionDraft {
    word: String,
    #[serde(default = "yes")]
    term: bool,
    #[serde(default)]
    from: Option<String>,
    /// The part the word names where it is written as code (docair-8l6.88).
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    definition: Option<String>,
    #[serde(default)]
    agrees: Option<bool>,
    #[serde(default)]
    why: Option<String>,
}

/// Whether a glossary entry agrees with the code.
#[derive(Debug, Clone, Deserialize)]
struct CheckDraft {
    word: String,
    #[serde(default)]
    agrees: Option<bool>,
    #[serde(default)]
    definition: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    terms: Vec<TermDraft>,
    #[serde(default)]
    decisions: Vec<DecisionDraft>,
    #[serde(default)]
    checks: Vec<CheckDraft>,
}

/// How many open words and entries to check go into one request, and how many requests one run makes.
const OPEN_PER_REQUEST: usize = 20;
const CHECKS_PER_REQUEST: usize = 10;
const MAX_REQUESTS: usize = 6;

/// How much of a description goes to the model.
const TEXT_CHARS: usize = 160;

/// Whether `word` (or its plural) occurs in `text` as a word.
fn occurs(word: &str, text: &str) -> bool {
    let w = word.to_lowercase();
    let t = text.to_lowercase();
    let bytes = t.as_bytes();
    let is_word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-');
    let mut from = 0;
    while let Some(i) = t[from..].find(&w) {
        let start = from + i;
        let mut end = start + w.len();
        for plural in ["es", "s"] {
            if t[end..].starts_with(plural) {
                end += plural.len();
                break;
            }
        }
        let before = t[..start].chars().next_back();
        let after = t[end..].chars().next();
        if !is_word(before) && !is_word(after) {
            return true;
        }
        from = start + 1;
        if from >= bytes.len() {
            break;
        }
        while !t.is_char_boundary(from) {
            from += 1;
        }
    }
    false
}

/// A glossary section in the format of SPEC §13.1.
pub fn markdown_of(t: &TermDraft) -> String {
    let mut out = format!("## {}\n\n", t.term.trim());
    if !t.aliases.is_empty() {
        out.push_str(&format!("Also: {}\n", t.aliases.join(", ")));
    }
    if !t.see.is_empty() {
        out.push_str(&format!("See: {}\n", t.see.join(", ")));
    }
    if !t.aliases.is_empty() || !t.see.is_empty() {
        out.push('\n');
    }
    out.push_str(t.definition.trim());
    if !t.link.is_empty() {
        out.push_str(&format!(" [Read more]({})", t.link));
    }
    out.push('\n');
    out
}

/// A non-empty, trimmed string.
fn given(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// The glossary entry (not one from code or a doc heading) that spells this word.
fn entry_for<'a>(graph: &'a Graph, word: &str) -> Option<&'a Term> {
    let st = stem(word);
    graph.glossary.iter().find(|t| {
        t.from.is_empty()
            && std::iter::once(&t.term)
                .chain(&t.aliases)
                .any(|w| stem(w) == st)
    })
}

/// A part's text for the model.
fn part_json(graph: &Graph, id: &str) -> Value {
    match graph.nodes.iter().find(|n| n.id == id) {
        Some(n) => {
            let about: String = n.description.chars().take(TEXT_CHARS * 2).collect();
            json!({ "id": n.id, "kind": n.kind, "name": n.name, "summary": n.summary, "about": about })
        }
        None => json!({ "id": id }),
    }
}

/// Up to three sentences of other parts that use the word, so the model sees in which sense the docs use it.
fn used_in(graph: &Graph, word: &str, parts: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for n in &graph.nodes {
        if parts.contains(&n.id) {
            continue;
        }
        let text = format!("{} {}", n.summary, n.description);
        for sentence in text.split_inclusive(['.', '\n']) {
            let plain = sentence.split_whitespace().collect::<Vec<_>>().join(" ");
            if meanings::uses_word(word, &plain.to_lowercase()) {
                out.push(plain.chars().take(140).collect());
                break;
            }
        }
        if out.len() >= 3 {
            break;
        }
    }
    out
}

/// Up to three sentences of other parts that write the word as code (`merged`, `change_steps::merged`,
/// `merged()`), so the model sees what those uses name (docair-8l6.88).
fn as_code(graph: &Graph, word: &str, parts: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for n in graph.nodes.iter().filter(|n| !parts.contains(&n.id)) {
        let text = format!("{} {}", n.summary, n.description);
        for sentence in text.split_inclusive(['.', '\n']) {
            if code_spans(sentence).any(|c| names_word(c, word)) {
                out.push(
                    sentence
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .chars()
                        .take(140)
                        .collect(),
                );
                break;
            }
        }
        if out.len() >= 3 {
            break;
        }
    }
    out
}

/// Every part the word names, of any kind (a function `merged` too): what its uses as code may name.
fn named_so(graph: &Graph, word: &str) -> Vec<String> {
    graph
        .nodes
        .iter()
        .filter(|n| meanings::stem(&n.name.to_lowercase()) == meanings::stem(word))
        .map(|n| n.id.clone())
        .collect()
}

/// The code spans of a text: what stands between backticks.
fn code_spans(text: &str) -> impl Iterator<Item = &str> {
    text.split('`').skip(1).step_by(2)
}

/// Whether a code span writes the word: `merged`, `Merged`, `change_steps::merged`, `merged()`.
pub fn names_word(code: &str, word: &str) -> bool {
    let last = code
        .trim()
        .trim_end_matches("()")
        .rsplit(['.', ':', '/'])
        .next()
        .unwrap_or("");
    last.eq_ignore_ascii_case(word) || meanings::stem(&last.to_lowercase()) == meanings::stem(word)
}

/// How much the model reasons before it explains, unless the user says (docair-8l6.89).
pub const REASONING: &str = "low";

/// The reasoning a user asked for: `low`, `medium` or `high`, else the default; another word is an error.
pub fn reasoning_of(asked: Option<&str>) -> Result<&'static str> {
    match asked.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(REASONING),
        Some("low") => Ok("low"),
        Some("medium") => Ok("medium"),
        Some("high") => Ok("high"),
        Some(other) => anyhow::bail!("reasoning `{other}` is none of low, medium, high"),
    }
}

/// What happened while asking, said in the reply (docair-8l6.89).
#[derive(Default)]
struct HowItWent {
    requests: usize,
    short: usize,
}

/// Ask once, streamed, so a long input is not cut off by the time a whole answer may take
/// (docair-8l6.88, as the review asks); a model that still thinks until its budget is gone gets the
/// short version instead, and the reply says so.
fn ask(
    cfg: &AiConfig,
    full: &str,
    short: Option<&str>,
    reasoning: &str,
    how: &mut HowItWent,
) -> Result<crate::client::Reply> {
    let asked = |text: &str| {
        let messages = [Message::system(GLOSSARY_PROMPT), Message::user(text)];
        crate::client::chat_streamed(cfg, &messages, true, Some(reasoning), &|_| {})
    };
    how.requests += 1;
    match asked(full) {
        Err(e) if e.to_string().contains("max_tokens") && short.is_some() => {
            how.requests += 1;
            how.short += 1;
            asked(short.unwrap_or_default())
        }
        other => other,
    }
}

/***
 * Ask the model for glossary entries: the words of the docs a newcomer would not know and the glossary does not explain yet; and settle the words that name parts, the code first: which part defines each, which are ordinary, whether the glossary agrees with the code.
 *
 * @id      docair-ai.glossary.explain
 * @calls   docair-ai.client.chat (texts + known terms + open words + entries to check) -> entries, decisions, checks "asks the model for the missing terms and what the open words mean"
 * @out     reply: Reply with the entries and one edit: new and rewritten entries, the decisions file
 ***/
pub fn explain(cfg: &AiConfig, graph: &Graph, limit: usize, reasoning: &str) -> Result<Reply> {
    let mut how = HowItWent::default();
    let mut texts = Vec::new();
    let mut corpus = String::new();
    for n in &graph.nodes {
        if n.summary.is_empty() && n.description.is_empty() {
            continue;
        }
        let about: String = n.description.chars().take(TEXT_CHARS).collect();
        corpus.push_str(&format!("{} {} {}\n", n.name, n.summary, n.description));
        texts.push(json!({ "id": n.id, "kind": n.kind, "name": n.name, "summary": n.summary, "about": about }));
    }
    for e in &graph.edges {
        if let Some(t) = &e.text {
            corpus.push_str(t);
            corpus.push('\n');
        }
    }
    let words: Vec<&str> = graph
        .edges
        .iter()
        .filter_map(|e| e.text.as_deref())
        .collect();
    let known: Vec<String> = graph
        .glossary
        .iter()
        .flat_map(|t| std::iter::once(t.term.clone()).chain(t.aliases.iter().cloned()))
        .collect();
    let known_text = if known.is_empty() {
        "nothing".to_string()
    } else {
        known.join(", ")
    };

    // What is still to settle (docs/MEANINGS.md §3, §4), in batches.
    let m = graph.meanings.clone().unwrap_or_default();
    let open: Vec<&OpenWord> = m
        .open
        .iter()
        .take(OPEN_PER_REQUEST * MAX_REQUESTS)
        .collect();
    let unchecked: Vec<&Unchecked> = m
        .unchecked
        .iter()
        .take(CHECKS_PER_REQUEST * MAX_REQUESTS)
        .collect();
    let left = m.open.len() - open.len() + m.unchecked.len() - unchecked.len();
    let batches = open
        .len()
        .div_ceil(OPEN_PER_REQUEST)
        .max(unchecked.len().div_ceil(CHECKS_PER_REQUEST))
        .max(1);
    let mut answers = Vec::new();
    let mut problems_early: Vec<String> = Vec::new();
    let mut usage = Usage::default();
    let mut seconds = 0.0;
    for b in 0..batches {
        let mut tail = String::new();
        let open_here: Vec<Value> = open
            .iter()
            .skip(b * OPEN_PER_REQUEST)
            .take(OPEN_PER_REQUEST)
            .map(|o| {
                json!({
                    "word": o.word,
                    "parts": o.parts.iter().map(|id| part_json(graph, id)).collect::<Vec<_>>(),
                    "glossary": entry_for(graph, &o.word).map(|t| t.markdown.clone()),
                    "used": used_in(graph, &o.word, &o.parts),
                    "as code": as_code(graph, &o.word, &o.parts),
                    "named so in the code": named_so(graph, &o.word).iter().map(|id| part_json(graph, id)).collect::<Vec<_>>(),
                    "doc": o.section.as_ref().map(|s| format!("{}: {}", s.file, s.title)),
                })
            })
            .collect();
        if !open_here.is_empty() {
            tail.push_str(&format!(
                "\nOpen words:\n{}\n",
                serde_json::to_string(&open_here).expect("open words serialize")
            ));
        }
        let checks_here: Vec<Value> = unchecked
            .iter()
            .skip(b * CHECKS_PER_REQUEST)
            .take(CHECKS_PER_REQUEST)
            .map(|u| {
                let entry = graph.glossary.iter().find(|t| t.id == u.term);
                json!({
                    "word": u.word,
                    "code": part_json(graph, &u.node),
                    "glossary": entry.and_then(|t| t.unchecked.clone()),
                })
            })
            .collect();
        if !checks_here.is_empty() {
            tail.push_str(&format!(
                "\nEntries to check:\n{}\n",
                serde_json::to_string(&checks_here).expect("checks serialize")
            ));
        }
        // Words docair found that name no part and nothing explains: entries for those a reader needs.
        if b == 0 && !m.unnamed.is_empty() {
            tail.push_str(&format!(
                "\nWords without a part or an entry (write entries for those a reader of these docs needs):\n{}\n",
                serde_json::to_string(&m.unnamed).expect("words serialize")
            ));
        }
        // The texts go along only when new terms are wanted; `--limit 0` asks about the open words
        // and the entries to check alone (docair-8l6.88).
        let reply = if b == 0 && limit > 0 {
            let full = format!(
                "System `{}`. Explained already: {known_text}.\nAt most {limit} new terms.\nThe texts:\n{}\nThe words on the lines:\n{}{tail}",
                graph.system,
                serde_json::to_string(&texts).expect("texts serialize"),
                serde_json::to_string(&words).expect("words serialize"),
            );
            let short: Vec<Value> = texts
                .iter()
                .map(|t| json!({ "id": t["id"], "name": t["name"], "summary": t["summary"] }))
                .collect();
            let short = format!(
                "System `{}`. Explained already: {known_text}.\nAt most {limit} new terms.\nThe texts:\n{}{tail}",
                graph.system,
                serde_json::to_string(&short).expect("texts serialize"),
            );
            ask(cfg, &full, Some(&short), reasoning, &mut how)?
        } else {
            let full = format!(
                "System `{}`. Explained already: {known_text}.\nAt most 0 new terms.{tail}",
                graph.system
            );
            ask(cfg, &full, None, reasoning, &mut how)?
        };
        usage += reply.usage;
        seconds += reply.seconds;
        let content = &reply.content;
        let json = content
            .find('{')
            .zip(content.rfind('}'))
            .map(|(a, b)| &content[a..=b])
            .unwrap_or("{}");
        // A cut-off or unreadable answer costs only its own words: they stay open for the next run.
        match serde_json::from_str::<Answer>(json) {
            Ok(parsed) => answers.push(parsed),
            Err(e) if b > 0 || batches > 1 => problems_early.push(format!(
                "request {} of {batches}: the model's answer cannot be read ({e}); its words stay open",
                b + 1
            )),
            Err(e) => bail!("the model's entries cannot be read: {e}"),
        }
    }

    let ids: HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    // Words are the same word in any case and number: `fact` is `facts`.
    let mut taken: HashSet<String> = known.iter().map(|k| stem(k)).collect();
    // Open words get settled by decisions, not by new terms.
    let open_stems: HashSet<String> = m.open.iter().map(|o| stem(&o.word)).collect();
    if answers.is_empty() {
        bail!("{}", problems_early.join("; "));
    }
    let mut problems = problems_early;
    let mut terms = Vec::new();
    for t in answers
        .iter_mut()
        .flat_map(|a| std::mem::take(&mut a.terms))
    {
        let mut t = t;
        t.term = t.term.trim().to_string();
        t.definition = t.definition.trim().to_string();
        if t.term.is_empty() || t.term.chars().count() > 40 || t.definition.is_empty() {
            continue;
        }
        if terms.len() >= limit
            || open_stems.contains(&stem(&t.term))
            || !taken.insert(stem(&t.term))
        {
            continue;
        }
        // Proposed before and no longer in the glossary: the reviewer dropped it (docair-6sg).
        if m.decisions
            .iter()
            .any(|(w, d)| d.proposed && stem(w) == stem(&t.term))
        {
            problems.push(format!(
                "`{}` was dropped from the glossary before; left out",
                t.term
            ));
            continue;
        }
        if !occurs(&t.term, &corpus) && !t.aliases.iter().any(|a| occurs(a, &corpus)) {
            problems.push(format!("`{}` does not occur in the docs; left out", t.term));
            continue;
        }
        t.aliases
            .retain(|a| !a.trim().is_empty() && taken.insert(stem(a)));
        t.see.retain(|s| ids.contains(s.as_str()));
        t.see.truncate(3);
        if !(t.link.starts_with("https://") || t.link.starts_with("http://")) {
            t.link.clear();
        }
        let definition: String = t.definition.chars().take(600).collect();
        t.definition = definition;
        terms.push(t);
    }

    // Decisions and checks: the decisions file, entries built from the code, entries rewritten to agree with it.
    let mut decisions: BTreeMap<String, Decision> = m.decisions.clone();
    // Each new entry is marked as proposed, so a reviewer's dropping it later is remembered.
    for t in &terms {
        decisions
            .entry(t.term.to_lowercase())
            .or_insert_with(|| Decision {
                proposed: true,
                why: Some("the model proposed the entry".into()),
                ..Decision::default()
            });
    }
    let mut rewrites: Vec<Change> = Vec::new();
    let (mut settled, mut ordinary, mut checked, mut rewritten) = (0, 0, 0, 0);
    let mut seen: HashSet<String> = HashSet::new();
    let fingerprint = |id: &str| {
        graph
            .nodes
            .iter()
            .find(|n| n.id == id)
            .map(meanings::fingerprint)
    };
    let rewrite = |entry: &Term, definition: &str, node: &str, rewrites: &mut Vec<Change>| {
        let mut see = entry.see.clone();
        if !see.iter().any(|s| s == node) {
            see.push(node.to_string());
        }
        let text = markdown_of(&TermDraft {
            term: entry.term.clone(),
            aliases: entry.aliases.clone(),
            definition: definition.chars().take(600).collect(),
            see,
            link: String::new(),
        });
        rewrites.push(Change {
            node: "glossary".into(),
            mode: "section".into(),
            file: entry.source.file.clone(),
            line: entry.source.line,
            add: vec![],
            remove: vec![],
            summary: Some(text),
            why: format!("`{}` rewritten to agree with `{node}`.", entry.term),
        });
    };
    for d in answers.iter().flat_map(|a| &a.decisions) {
        let Some(o) = open.iter().find(|o| stem(&o.word) == stem(&d.word)) else {
            problems.push(format!(
                "`{}` was not an open word; left out",
                d.word.trim()
            ));
            continue;
        };
        if !seen.insert(o.word.clone()) {
            continue;
        }
        let why = given(&d.why);
        // Where the word is written as code, the part it names: one the word names (docair-8l6.88).
        let code = given(&d.code).filter(|c| named_so(graph, &o.word).contains(c));
        if !d.term {
            decisions.insert(
                o.word.to_lowercase(),
                Decision {
                    term: false,
                    code,
                    why,
                    ..Decision::default()
                },
            );
            settled += 1;
            ordinary += 1;
            continue;
        }
        let from = given(&d.from);
        // One of its parts, or any part of that name: a constant may say what a tag means (docair-8l6.90).
        if let Some(f) = &from
            && !o.parts.contains(f)
            && !named_so(graph, &o.word).contains(f)
        {
            problems.push(format!(
                "`{}`: `{f}` is not one of its parts; it stays open",
                o.word
            ));
            continue;
        }
        let mut dec = Decision {
            from: from.clone(),
            code,
            why,
            ..Decision::default()
        };
        if let Some(f) = &from {
            let definition = given(&d.definition);
            match entry_for(graph, &o.word) {
                Some(entry) => match (d.agrees, definition) {
                    (Some(true), _) => {
                        dec.checked = fingerprint(f);
                        checked += 1;
                    }
                    (Some(false), Some(def)) => {
                        rewrite(entry, &def, f, &mut rewrites);
                        dec.checked = fingerprint(f);
                        checked += 1;
                        rewritten += 1;
                    }
                    _ => {}
                },
                None => {
                    if let Some(def) = definition
                        && taken.insert(stem(&o.word))
                    {
                        terms.push(TermDraft {
                            term: o.word.clone(),
                            aliases: vec![],
                            definition: def.chars().take(600).collect(),
                            see: vec![f.clone()],
                            link: String::new(),
                        });
                        dec.checked = fingerprint(f);
                    }
                }
            }
        }
        decisions.insert(o.word.to_lowercase(), dec);
        settled += 1;
    }
    for c in answers.iter().flat_map(|a| &a.checks) {
        let Some(u) = unchecked.iter().find(|u| stem(&u.word) == stem(&c.word)) else {
            problems.push(format!(
                "`{}` was not an entry to check; left out",
                c.word.trim()
            ));
            continue;
        };
        if !seen.insert(format!("check:{}", u.word)) {
            continue;
        }
        let Some(entry) = graph.glossary.iter().find(|t| t.id == u.term) else {
            continue;
        };
        let key = u.word.to_lowercase();
        let mut dec = decisions.get(&key).cloned().unwrap_or_default();
        dec.from.get_or_insert_with(|| u.node.clone());
        match (c.agrees, given(&c.definition)) {
            (Some(true), _) => {}
            (Some(false), Some(def)) => {
                rewrite(entry, &def, &u.node, &mut rewrites);
                rewritten += 1;
            }
            _ => {
                problems.push(format!(
                    "`{}`: no answer whether the glossary agrees; it stays unchecked",
                    u.word
                ));
                continue;
            }
        }
        dec.checked = Some(u.fingerprint.clone());
        decisions.insert(key, dec);
        checked += 1;
    }
    // A word the docs also write as code, settled without saying which part those uses name: one
    // short question about those uses alone, since the model leaves that field out of the long
    // answer (docair-8l6.90).
    let unnamed_code: Vec<(String, Vec<String>, Vec<String>)> = open
        .iter()
        .filter(|o| {
            decisions
                .get(&o.word.to_lowercase())
                .is_some_and(|d| d.code.is_none())
        })
        .filter_map(|o| {
            let uses = as_code(graph, &o.word, &o.parts);
            let parts = named_so(graph, &o.word);
            (!uses.is_empty() && !parts.is_empty()).then(|| (o.word.clone(), uses, parts))
        })
        .collect();
    if !unnamed_code.is_empty() {
        let asked: Vec<Value> = unnamed_code
            .iter()
            .map(|(word, uses, parts)| {
                json!({
                    "word": word,
                    "as code": uses,
                    "parts": parts.iter().map(|id| part_json(graph, id)).collect::<Vec<_>>(),
                })
            })
            .collect();
        let messages = [
            Message::system(CODE_PROMPT),
            Message::user(serde_json::to_string(&asked).expect("words serialize")),
        ];
        how.requests += 1;
        match crate::client::chat_streamed(cfg, &messages, true, Some(reasoning), &|_| {}) {
            Ok(r) => {
                usage += r.usage;
                seconds += r.seconds;
                let content = &r.content;
                let json = content
                    .find('{')
                    .zip(content.rfind('}'))
                    .map_or("{}", |(a, b)| &content[a..=b]);
                let named: Vec<Value> = serde_json::from_str::<Value>(json)
                    .ok()
                    .and_then(|v| v.get("code").and_then(Value::as_array).cloned())
                    .unwrap_or_default();
                for n in named {
                    let word = n["word"].as_str().unwrap_or("").trim();
                    let part = n["part"].as_str().unwrap_or("").trim();
                    let Some((w, _, parts)) =
                        unnamed_code.iter().find(|(w, ..)| stem(w) == stem(word))
                    else {
                        continue;
                    };
                    if parts.iter().any(|p| p == part)
                        && let Some(d) = decisions.get_mut(&w.to_lowercase())
                    {
                        d.code = Some(part.to_string());
                    }
                }
            }
            Err(e) => problems.push(format!(
                "asking which parts the words written as code name failed: {e:#}"
            )),
        }
    }
    if terms.is_empty() && settled == 0 && checked == 0 && !problems.is_empty() {
        bail!("the model proposed no term that the docs use, and settled no word");
    }

    let file = graph
        .glossary
        .iter()
        .find(|t| t.source.file.ends_with(".md") && t.from != "doc")
        .map_or(DEFAULT_FILE.to_string(), |t| t.source.file.clone());
    let mut changes = Vec::new();
    if !terms.is_empty() {
        changes.push(Change {
            node: "glossary".into(),
            mode: "text".into(),
            file: file.clone(),
            line: 0,
            add: vec![],
            remove: vec![],
            summary: Some(terms.iter().map(markdown_of).collect::<Vec<_>>().join("\n")),
            why: format!("{} new terms for {file}.", terms.len()),
        });
    }
    changes.extend(rewrites);
    if decisions != m.decisions {
        let meanings_file = if m.file.is_empty() {
            meanings::DEFAULT_FILE.to_string()
        } else {
            m.file.clone()
        };
        changes.push(Change {
            node: "meanings".into(),
            mode: "file".into(),
            file: meanings_file.clone(),
            line: 0,
            add: vec![],
            remove: vec![],
            summary: Some(meanings::to_toml(&decisions)),
            why: format!(
                "{settled} words settled, {checked} glossary entries checked, in {meanings_file}."
            ),
        });
    }

    let mut answer = if terms.is_empty() {
        "The glossary already explains every word the model would add.".to_string()
    } else {
        format!(
            "Explained {} terms: {}.",
            terms.len(),
            terms
                .iter()
                .map(|t| t.term.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    if settled > 0 {
        answer.push_str(&format!(
            " Settled {settled} words that name parts ({ordinary} everyday words)."
        ));
    }
    if checked > 0 {
        answer.push_str(&format!(
            " Checked {checked} glossary entries against the code ({rewritten} rewritten to agree with it)."
        ));
    }
    if left > 0 {
        answer.push_str(&format!(" {left} more are left for the next run."));
    }
    // What happened, said plainly (docair-8l6.89).
    let texts = if limit > 0 {
        "the docs' texts went along to find new terms"
    } else {
        "no new terms were wanted (--limit 0), so only the open words and the entries to check went, not the docs' texts"
    };
    let mut went = format!(
        " How it went: {} asked {} time{}, streamed, reasoning {reasoning}; {texts}; {:.0} s.",
        cfg.model,
        how.requests,
        if how.requests == 1 { "" } else { "s" },
        seconds
    );
    if how.short > 0 {
        went.push_str(&format!(
            " The model used up max_tokens before it answered {} time{}, and got the short version (the texts' summaries only) instead.",
            how.short,
            if how.short == 1 { "" } else { "s" }
        ));
    }
    answer.push_str(&went);
    Ok(Reply {
        answer,
        walkthrough: None,
        view: None,
        changes,
        wording: vec![],
        glossary: terms,
        problems,
        usage,
        seconds,
        requests: u8::try_from(batches).unwrap_or(u8::MAX),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_occur_with_plurals_but_not_inside_other_words() {
        assert!(occurs("digest", "the digest of the graph"));
        assert!(occurs("Ingress", "two ingresses route it"));
        assert!(occurs("NodePort", "a NodePort service"));
        assert!(!occurs("port", "the NodePort service"));
        assert!(!occurs("tag", "tagged"));
        assert!(occurs("über", "das Über-Ich ist über"));
    }

    #[test]
    fn entries_are_glossary_markdown() {
        let t = TermDraft {
            term: "Ingress".into(),
            aliases: vec!["ingress rule".into()],
            definition: "Routes HTTP into the cluster.".into(),
            see: vec!["k8s.ingress.docair".into()],
            link: "https://kubernetes.io/docs/concepts/services-networking/ingress/".into(),
        };
        let md = markdown_of(&t);
        assert_eq!(
            md,
            "## Ingress\n\nAlso: ingress rule\nSee: k8s.ingress.docair\n\nRoutes HTTP into the cluster. [Read more](https://kubernetes.io/docs/concepts/services-networking/ingress/)\n"
        );
        let parsed = docair_core::glossary::parse("g.md", &md);
        assert_eq!(parsed[0].aliases, vec!["ingress rule"]);
        assert_eq!(parsed[0].see, vec!["k8s.ingress.docair"]);
    }

    /// A model that answers one request with `content`; the request's body comes back through the channel.
    fn stub(content: &str) -> (AiConfig, std::sync::mpsc::Receiver<String>) {
        stub_all(&[content])
    }

    /// A model that answers one request after another with these contents.
    fn stub_all(contents: &[&str]) -> (AiConfig, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let bodies: Vec<String> = contents
            .iter()
            .map(|content| {
                json!({
                    "choices": [{ "message": { "content": content }, "finish_reason": "stop" }],
                    "usage": { "prompt_tokens": 10, "completion_tokens": 5 },
                })
                .to_string()
            })
            .collect();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for body in bodies {
                let (mut sock, _) = listener.accept().expect("accept");
                let mut reader = BufReader::new(sock.try_clone().expect("clone"));
                let mut len = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).expect("header");
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut req = vec![0; len];
                reader.read_exact(&mut req).expect("body");
                tx.send(String::from_utf8_lossy(&req).to_string()).ok();
                write!(
                sock,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("reply");
            }
        });
        let cfg = AiConfig {
            endpoint: format!("http://{addr}/v1"),
            model: "stub".into(),
            api_key: Some("test".into()),
            key_var: "DOCAIR_AI_API_KEY".into(),
            key_source: None,
            max_tokens: 1000,
        };
        (cfg, rx)
    }

    fn graph() -> Graph {
        let part = |id: &str, kind: &str, name: &str, summary: &str| {
            json!({ "id": id, "kind": kind, "name": name, "summary": summary, "description": "",
                    "source": { "file": "src/lib.rs", "line": 1 }, "docs": [], "tags": [], "origin": "authored" })
        };
        serde_json::from_value(json!({
            "schema": "x", "system": "books", "generator": "test", "edges": [], "flows": [], "diagnostics": [],
            "nodes": [
                part("books.ledger", "module", "ledger", "Ledger: the record of all payments, newest first."),
                part("books.payment", "component", "Payment", "One payment someone made."),
                part("books.audit", "module", "audit", "Checks the ledger every night."),
                part("books.record", "function", "record", "Record a payment in the ledger, then audit it."),
            ],
            "glossary": [{
                "id": "ledger", "term": "ledger", "summary": "The record of all payments, newest first.",
                "markdown": "The record of all payments, newest first.",
                "source": { "file": "GLOSSARY.md", "line": 5 }, "from": "code", "node": "books.ledger",
                "unchecked": "The book of every payment, kept on paper."
            }],
            "meanings": {
                "file": "docs/meanings.toml",
                "decisions": { "old": { "term": false } },
                "open": [
                    { "word": "payment", "parts": ["books.payment"], "uses": 3 },
                    { "word": "audit", "parts": ["books.audit"], "uses": 2 }
                ],
                "unchecked": [{ "word": "ledger", "node": "books.ledger", "term": "ledger", "fingerprint": "0badcafe" }]
            }
        }))
        .expect("graph")
    }

    #[test]
    fn a_word_written_as_code_gets_the_part_it_names_asked_apart() {
        let mut g = graph();
        // `audit()` in a text, and a function named audit beside the module.
        g.nodes[3].summary = "Record a payment in the ledger, then `audit()` it.".into();
        let mut f = g.nodes[3].clone();
        f.id = "books.check.audit".into();
        f.name = "audit".into();
        f.summary = "Audit one payment.".into();
        g.nodes.push(f);
        let long = json!({ "terms": [], "decisions": [{ "word": "audit", "term": false, "why": "the verb" }], "checks": [] }).to_string();
        let short =
            json!({ "code": [{ "word": "audit", "part": "books.check.audit" }] }).to_string();
        let (cfg, sent) = stub_all(&[&long, &short]);
        let reply = explain(&cfg, &g, 0, "low").expect("explain");
        let _first = sent.recv().expect("the long request");
        let second = sent.recv().expect("the question about code");
        assert!(
            second.contains("written as code") && second.contains("audit()"),
            "{second}"
        );
        let toml = reply
            .changes
            .iter()
            .find_map(|c| c.summary.clone().filter(|s| s.contains("[audit]")))
            .expect("the decisions file");
        assert!(
            toml.contains("term = false") && toml.contains(r#"code = "books.check.audit""#),
            "{toml} {:?} {}",
            reply.problems,
            reply.answer
        );
        assert!(
            reply.answer.contains("asked 2 times"),
            "{} {:?}",
            reply.answer,
            reply.problems
        );
    }

    #[test]
    fn explain_settles_open_words_and_checks_the_glossary_against_the_code() {
        let answer = json!({
            "terms": [],
            "decisions": [
                { "word": "payment", "term": true, "from": "books.payment", "definition": "A payment: money someone paid, recorded in the ledger.", "why": "the struct says it" },
                { "word": "audit", "term": false, "why": "the nightly check" },
                { "word": "ghost", "term": false }
            ],
            "checks": [{ "word": "ledger", "agrees": false, "definition": "The record of all payments, newest first." }]
        })
        .to_string();
        let (cfg, sent) = stub(&answer);
        let g = graph();
        let reply = explain(&cfg, &g, 5, "medium").expect("explain");
        let request = sent.recv().expect("request");
        assert!(
            request.contains(r#""reasoning_effort": "medium""#),
            "the reasoning asked for goes along: {request}"
        );
        assert!(
            reply.answer.contains("How it went:") && reply.answer.contains("reasoning medium"),
            "the reply says how it went (docair-8l6.89): {}",
            reply.answer
        );
        assert!(request.contains("Open words:"), "{request}");
        assert!(request.contains("Entries to check:"), "{request}");
        assert!(request.contains("The book of every payment, kept on paper."));

        assert_eq!(
            reply.glossary.len(),
            1,
            "the entry built from the code is shown at once"
        );
        assert_eq!(reply.glossary[0].see, vec!["books.payment"]);
        assert!(
            reply.problems.iter().any(|p| p.contains("`ghost`")),
            "{:?}",
            reply.problems
        );
        let modes: Vec<_> = reply.changes.iter().map(|c| c.mode.as_str()).collect();
        assert_eq!(modes, ["text", "section", "file"]);
        let section = &reply.changes[1];
        assert_eq!((section.file.as_str(), section.line), ("GLOSSARY.md", 5));
        assert!(
            section
                .summary
                .as_deref()
                .unwrap()
                .starts_with("## ledger\n\nSee: books.ledger\n\nThe record")
        );
        let toml = reply.changes[2].summary.as_deref().unwrap();
        let decided = meanings::parse(toml).expect("the decisions parse");
        assert!(!decided["old"].term, "earlier decisions stay");
        assert!(!decided["audit"].term);
        assert_eq!(decided["payment"].from.as_deref(), Some("books.payment"));
        assert!(
            decided["payment"].checked.is_some(),
            "the new entry was written from the code"
        );
        assert_eq!(decided["ledger"].checked.as_deref(), Some("0badcafe"));
        assert_eq!(decided["ledger"].from.as_deref(), Some("books.ledger"));
        assert!(reply.answer.contains("Settled 2 words"), "{}", reply.answer);
        assert!(reply.answer.contains("1 rewritten"), "{}", reply.answer);
    }
}

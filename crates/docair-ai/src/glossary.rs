//! The model explains the words of the docs: new glossary entries (SPEC §13.4).

use std::collections::HashSet;

use anyhow::{Result, bail};
use docair_core::glossary::DEFAULT_FILE;
use docair_core::model::{Change, Graph};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ask::Reply;
use crate::client::{Message, Usage, chat};
use crate::config::AiConfig;

/// The rules for the entries.
pub const GLOSSARY_PROMPT: &str = r#"You write the glossary of a software project's documentation. You get the texts of the docs (summaries and descriptions of the parts of the system, and the words on the lines between them) and the terms the glossary already explains.

Pick the words a newcomer to this project would not know and that the glossary does not explain yet: words of this project ("digest", "walkthrough") and technologies ("Ingress", "NodePort", "tree-sitter"). Skip everyday words and names of single functions.

For each term:
- "term": as it is written in the texts, singular; keep the capitals of proper names (Ingress, Traefik).
- "aliases": other spellings in the texts (not plurals; those are found anyway).
- "definition": 1 to 3 short sentences in plain words. For a word of this project, what it is here. For a technology, what it is in general, then what this project does with it.
- "see": ids of the parts (from the texts) where the term is implemented or used, at most 3.
- "link": for a technology, the URL of its official documentation; else "".

Decide quickly: go through the texts once and take the terms that stand out; do not weigh every word. Write in the language of the texts. Reply with exactly one JSON object: {"terms": [{"term": "...", "aliases": [], "definition": "...", "see": [], "link": ""}]}"#;

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

#[derive(Deserialize)]
struct Terms {
    #[serde(default)]
    terms: Vec<TermDraft>,
}

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

/***
 * Ask the model for glossary entries: the words of the docs a newcomer would not know and the glossary does not explain yet.
 *
 * @id      docair-ai.glossary.explain
 * @calls   docair-ai.client.chat (texts + known terms) -> new entries "asks the model for the missing terms"
 * @out     reply: Reply with the entries and one text edit
 ***/
pub fn explain(cfg: &AiConfig, graph: &Graph, limit: usize) -> Result<Reply> {
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
    let messages = [
        Message::system(GLOSSARY_PROMPT),
        Message::user(format!(
            "System `{}`. Explained already: {}.\nAt most {limit} new terms.\nThe texts:\n{}\nThe words on the lines:\n{}",
            graph.system,
            if known.is_empty() {
                "nothing".to_string()
            } else {
                known.join(", ")
            },
            serde_json::to_string(&texts).expect("texts serialize"),
            serde_json::to_string(&words).expect("words serialize"),
        )),
    ];
    // A long text can make a reasoning model think until its budget is gone:
    // then once more with the summaries only.
    let reply = match chat(cfg, &messages, true) {
        Err(e) if e.to_string().contains("max_tokens") => {
            let short: Vec<Value> = texts
                .iter()
                .map(|t| json!({ "id": t["id"], "name": t["name"], "summary": t["summary"] }))
                .collect();
            let retry = [
                Message::system(GLOSSARY_PROMPT),
                Message::user(format!(
                    "System `{}`. Explained already: {}.\nAt most {limit} new terms.\nThe texts:\n{}",
                    graph.system,
                    if known.is_empty() {
                        "nothing".to_string()
                    } else {
                        known.join(", ")
                    },
                    serde_json::to_string(&short).expect("texts serialize"),
                )),
            ];
            chat(cfg, &retry, true)?
        }
        other => other?,
    };
    let content = &reply.content;
    let json = content
        .find('{')
        .zip(content.rfind('}'))
        .map(|(a, b)| &content[a..=b])
        .unwrap_or("{}");
    let parsed: Terms = serde_json::from_str(json)
        .map_err(|e| anyhow::anyhow!("the model's entries cannot be read: {e}"))?;

    let ids: HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    let mut taken: HashSet<String> = known.iter().map(|k| k.to_lowercase()).collect();
    let mut problems = Vec::new();
    let mut terms = Vec::new();
    for mut t in parsed.terms {
        t.term = t.term.trim().to_string();
        t.definition = t.definition.trim().to_string();
        if t.term.is_empty() || t.term.chars().count() > 40 || t.definition.is_empty() {
            continue;
        }
        if !taken.insert(t.term.to_lowercase()) {
            continue;
        }
        if !occurs(&t.term, &corpus) && !t.aliases.iter().any(|a| occurs(a, &corpus)) {
            problems.push(format!("`{}` does not occur in the docs; left out", t.term));
            continue;
        }
        t.aliases
            .retain(|a| !a.trim().is_empty() && taken.insert(a.to_lowercase()));
        t.see.retain(|s| ids.contains(s.as_str()));
        t.see.truncate(3);
        if !(t.link.starts_with("https://") || t.link.starts_with("http://")) {
            t.link.clear();
        }
        let definition: String = t.definition.chars().take(600).collect();
        t.definition = definition;
        terms.push(t);
        if terms.len() >= limit {
            break;
        }
    }
    if terms.is_empty() && !problems.is_empty() {
        bail!("the model proposed no term that the docs use");
    }
    let file = graph
        .glossary
        .first()
        .map_or(DEFAULT_FILE.to_string(), |t| t.source.file.clone());
    let changes = if terms.is_empty() {
        vec![]
    } else {
        vec![Change {
            node: "glossary".into(),
            mode: "text".into(),
            file: file.clone(),
            line: 0,
            add: vec![],
            remove: vec![],
            summary: Some(terms.iter().map(markdown_of).collect::<Vec<_>>().join("\n")),
            why: format!("{} new terms for {file}.", terms.len()),
        }]
    };
    Ok(Reply {
        answer: if terms.is_empty() {
            "The glossary already explains every word the model would add.".into()
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
        },
        walkthrough: None,
        view: None,
        changes,
        wording: vec![],
        glossary: terms,
        problems,
        usage: Usage {
            input: reply.usage.input,
            output: reply.usage.output,
        },
        seconds: reply.seconds,
        requests: 1,
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
}

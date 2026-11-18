# Meanings: where the page's explanations come from

Status: **built 2026-11-18** (beads epic `docair-xkj`). SPEC §13.4 and
§13.5 describe what docair does; this file explains why it is built this way.

The page explains a word when you hover it (SPEC §13.2). Until now it knew
only the words of `GLOSSARY.md`. Everything else the project says about its
words stayed out of reach. Example: a reader looks at *facts* and cannot tell
what a fact is, although the project defines it three times:

| Where | What it says |
|-------|--------------|
| the doc block of `docair-core.facts` | What adapters learn from the code: symbols, the routes they serve, their relations. |
| `crates/docair-core/src/facts.rs` | `//! Facts: what adapters learn from the code (docs/ADAPTERS.md §4).` |
| `docs/ADAPTERS.md` | `## 4. Facts: the contract between adapters and core` |

Even the glossary's own entry for *adapter* says "reports facts" and leaves
the word unexplained.

---

## 1. Decisions

| Decided (2026-11-17) | Why |
|----------------------|-----|
| **The code is the best source, always.** A word that a part of the code defines means what that part's docs say. | The code is what runs; its docs sit next to it and change with it. |
| **The glossary is checked against the code.** Where a glossary entry contradicts the code, the code wins. | Two definitions of one word drift apart unless something compares them. |
| **The model may expand a definition or word it better**, starting from the code's. | A doc comment is often terse; the hover is for newcomers. |
| **Where a word is not clear, the model decides, and the decision is kept** in the repository. | Deciding once per word is a one-off effort; the next run does not ask again. |
| **`docair check` warns** about a word that is used and not settled. | The work is visible and does not rot. |
| **Explain builds entries from the definitions the project has**, instead of writing new ones from nothing. | The project's own words are better than the model's guesses. |

## 2. Where a word is defined

Three sources, best first:

| # | Source | Which words | Example |
|---|--------|-------------|---------|
| 1 | **code**: the docs of a part (its doc block, or `///` docs the adapter read) | the name of a part of kind `module`, `component` or `data`, split into words (`LibraryDoc` → "library doc", `test_results` → "test results"); names with `-` or `.` are names of things, not words | `docair-core.facts` defines *facts* |
| 2 | **glossary**: an entry of a `GLOSSARY.md` (SPEC §13.1) | the entry's term and its `Also:` spellings | *Ingress*, *Helm*: words with no code of their own |
| 3 | **doc heading** of a Markdown file that is not a glossary | a heading `Word: the rest`, with numbering stripped and one to three words before the colon | `## 4. Facts: the contract between adapters and core` |

A word and its plural are the same word (*fact*, *facts*). A glossary entry
whose term or alias is the word belongs to it.

A doc heading alone does not make a word a term: `## 9. Output: …` and
`### 1.1 Configuration: …` are titles, and *output* is an everyday word. A
heading adds *Read more* to a word that the code or the glossary settles, and
it is the definition of a word a decision makes a term without naming a part.

**When a part defines its word.** A part's name alone does not make its docs
the word's definition: a module named `check` holds the checks, but the docs
use *check* as a verb. A part defines its word

* **by itself** when it is the only part of those kinds with that name and
  its summary says so: it starts with `Facts:`, `A digest is …`, `Facts are …`
  (the word or its plural, any case);
* **by a decision** (§3) in every other case.

**What the page shows.** Each word has one entry; the code decides what it
means.

| The project has | The hover shows | Under it |
|-----------------|-----------------|----------|
| code | the part's summary; the whole entry adds its description | from `docair-core.facts` · `src/lib.rs:46` |
| code, and a glossary entry checked against this code (§4) | the glossary's words | from `docair-core.facts` · worded in the glossary |
| code, and a glossary entry not checked against the code's current text | the code's words; the glossary's text follows in the whole entry, marked *not checked against the code* | W121 |
| a glossary entry only | the entry, as before | in the glossary |
| a doc heading only | the words after the colon, then the section's first paragraph | in ADAPTERS.md §4 |

A doc heading that defines the same word also gives every entry a *Read
more* that opens the section inside the page (SPEC §13.3). *Read more* of a
word from code opens the part's details.

## 3. Words that are not clear: decisions

A word is **open** when

* a part of kind `module`, `component` or `data` has it as its name (three
  letters or more),
* at least two texts use it as a plain word: summaries and descriptions of
  other parts, words on lines, steps of walkthroughs, glossary entries (code
  spans and ids do not count),
* and nothing settles it: no part defines it by itself (§2), no decision
  names it.

Examples on docair itself:

| Word | Why it is open | What a decision says |
|------|----------------|----------------------|
| *facts* | `docair-core.facts` says "What adapters learn from the code: …", not "Facts: …" | `from = "docair-core.facts"` |
| *glossary* | two modules: `docair-core.glossary` and `docair-ai.glossary` | `from = "docair-core.glossary"` |
| *check* | the module `docair-core.check`; the texts mean the verb or `docair check` | `term = false` |
| *graph* | the struct `Graph`, and a glossary entry *graph* | `from = "docair-core.model.Graph"`, then the entry is checked (§4) |

**The decisions file** is `docs/meanings.toml` (`meanings = "…"` in
docair.toml names another). It is written by `docair glossary` and reviewed
like code. One table per word, keyed by the word in lower case:

```toml
# What the words of the docs mean: decisions written by `docair glossary`
# (docs/MEANINGS.md). Review them like code.

[facts]
from = "docair-core.facts"
why = "the module that holds what adapters report"

[graph]
from = "docair-core.model.Graph"
checked = "4be1c07a"
why = "the struct the whole page is built from"

[check]
term = false
why = "the verb and the command, not the module"
```

| Key | Meaning |
|-----|---------|
| `term` | `false`: an ordinary word here, never underlined. Default `true`. |
| `from` | the part whose docs define the word |
| `checked` | the fingerprint of that part's text when the word's glossary entry was last compared with it (§4) |
| `why` | one short reason, for the reviewer |

The **fingerprint** is FNV-1a (64 bits) of the part's summary and
description with whitespace collapsed, written as its first 8 hex digits.
Stable across Rust versions and platforms, unlike `DefaultHasher`.

A decision is **opened again** when `from` names no part any more, or when
`checked` differs from the current fingerprint (§5). `term = false` stays
until a person changes it.

## 4. Contradictions

Whether two texts mean the same thing is a question of meaning, so the model
answers it. A pair is **unchecked** when a word is defined by code and has a
glossary entry, and its decision has no `checked`, or one that differs from
the current fingerprint.

Explain sends each unchecked pair. The model answers that they agree, or
that they do not and gives a glossary text that agrees with the code. Both
answers record `checked`; a disagreement also rewrites the entry. When the
code's docs are the wrong ones, fix them: the fingerprint changes, and the
pair is compared again.

## 5. docair check

| Code | When |
|------|------|
| `W120` | a word is open (§3): it names a part and the docs use it, but nothing settles what it means |
| `W121` | a word's settling is out of date: a decision's `from` names no part, or the glossary entry of a word the code defines is not checked against the code's current text |

`docair check` runs these only in a project that explains its words: one
with a glossary or a decisions file. It finds only words that name parts;
other words (*Ingress*) come from the model's explain run (§6). W120 stands
at the part that has the name; W121 at the decision or the glossary entry.

Codes W116 to W118 belong to the adapters epic (`docair-4wk.6`); W119 stays
free for it.

## 6. Explain builds from the definitions

`docair glossary` and **✦ Explain the terms** (SPEC §13.4) send one request
with three parts:

1. the **open words**, each with the texts of the parts named so;
2. the **unchecked pairs**: the code's text and the glossary's text;
3. the **texts of the docs** and the terms already explained, as before, to
   find the words that name no part.

The model answers with one JSON object:

```json
{
  "decisions": [
    { "word": "facts", "term": true, "from": "docair-core.facts", "why": "…",
      "definition": "What adapters learn from the code: the symbols, the routes they serve and their relations. The core merges them with the doc blocks." },
    { "word": "check", "term": false, "why": "…" }
  ],
  "checks": [
    { "word": "graph", "agrees": true },
    { "word": "node", "agrees": false, "definition": "…" }
  ],
  "terms": [ { "term": "Ingress", "definition": "…", "see": [], "link": "…" } ]
}
```

* A decision's `definition` is optional: the model writes one only when it
  words the code's text better or expands it, never changing its meaning. It
  becomes a glossary entry with `See:` naming the part, and the decision gets
  `checked`, since the text was written from the code.
* `from` must name one of the parts the request offered; anything else is
  dropped, and the word stays open.
* Everything becomes **one edit**: new glossary entries, rewritten ones, and
  the decisions file. Review the diff, apply it, or download the patch
  (SPEC §10.6).
* A second run on an unchanged tree sends no open words and no unchecked
  pairs.

## 7. The graph and the page

`Term` in `docair.graph.json` gets four fields, all left out when empty:

| Field | Meaning |
|-------|---------|
| `from` | where the meaning comes from: `code` or `doc`; left out for a glossary entry |
| `node` | the part whose docs define the word |
| `section` | `{ file, anchor, title }`: a doc heading that defines the word; *Read more* opens it |
| `unchecked` | the glossary's text, when it is not checked against the code (Markdown) |

`source` is the glossary entry when the word has one (also while it is
unchecked: that is where it gets fixed), else the part's block or the
heading. Words from code and doc headings join `glossary` in
the graph, so the viewer underlines them like any term. The files of their
sections join `library`.

The graph also gets `meanings`, left out when it is empty: the decisions
file's path, its decisions, the open words (with the parts named so and how
many texts use them) and the unchecked words (with the part's fingerprint).
The explain run reads it; the page can list what is still to settle.

The hover and the whole entry name the source: the part (a link to its
details), the doc section, or the glossary. The **Glossary** list shows every
word, with a mark for words from code (`ƒ`) and from docs (`§`).

## 8. Measured on ourselves

On docair itself (2026-11-18, GLM-5.3-Flash) there were 97 open words and
8 glossary entries to check. One run, reviewed and applied, settled them:

| | Count |
|-|-------|
| words that name parts, settled | 98 (51 everyday words, 47 terms) |
| words explained from the code | 42 |
| glossary entries compared with the code | 8, none contradicted it |
| new glossary entries, written from the code's text | 18 |
| decisions corrected by review | 3 (*lines*, *http*, *problem*: the docs use the everyday sense) |

The first try sent 40 words to a request, and the answer was cut off; now
20. It also made terms of *following*, *placed* and *out*, and took a YAML
parser's tree node for *node*. Since then the request shows the model up to
three sentences that use each word, and the prompt says that a part which
only shares the name is not the definition. With that, *node* stays the
glossary's (core's `Node` has no docs), and the everyday words are marked as
such.

## 9. Steps

| # | Step | Beads |
|---|------|-------|
| 1 | this design; SPEC §13 and §8 follow | `docair-xkj.1` |
| 2 | core: definitions from code, glossary and doc headings, code first; `Term.from`, `node`, `section`, `unchecked` | `docair-xkj.2` |
| 3 | viewer: the hover and the entry name the source; *Read more* opens it | `docair-xkj.3` |
| 4 | the decisions file, read by core; fingerprints | `docair-xkj.4` |
| 5 | explain: open words, unchecked pairs, decisions and checks in one edit | `docair-xkj.5` |
| 6 | `docair check`: W120, W121 | `docair-xkj.6` |
| 7 | docair's own words settled; *facts* explained from the code | `docair-xkj.7` |

## 10. Not yet

* **`//!` docs.** The Rust adapter reads `///` docs of an item, not the inner
  `//!` docs of a module file. `facts.rs` says "Facts: what adapters learn
  from the code", which would define *facts* by itself.
* **A tag for authors**, such as `@defines facts`, to settle a word in the
  code instead of the decisions file.
* **Words that name no part**, found without the model (*NodePort*,
  *CIDR*): today only the model finds them.
* **A rejected entry comes back**: when the reviewer drops an entry the
  model proposed for a word that names no part, the next run may propose it
  again.

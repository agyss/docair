//! The data that flows between pipeline stages, and the `docair.graph.json`
//! schema (SPEC §3.3, §4.2, §7, §8, §9, §10.3).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::lang::LanguageSpec;

/// Version tag written into every graph file.
pub const SCHEMA: &str = "docair.graph/0";

/// Where a value came from (SPEC §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// Written by a human in a doc block or Markdown.
    #[default]
    Authored,
    /// Filled in by an adapter from the code.
    Inferred,
    /// Created by an adapter from code that has no block.
    Discovered,
    /// Created by docair for a ref that does not resolve.
    Placeholder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    pub file: String,
    pub line: usize,
    /// The part's last line, when known: its code's end, or its block's (docair-zrr.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub value: String,
    pub line: usize,
    pub origin: Origin,
    /// Why an adapter inferred this tag, e.g. `tree-sitter: function_item`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// How sure the code is about where this line goes; none means certain (docs/ADAPTERS.md §5.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach: Option<Reach>,
}

/// How sure the code is about where a line goes (docs/ADAPTERS.md §5.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reach {
    /// Exactly this target.
    Certain,
    /// Exactly one of several targets runs, decided at runtime: a trait method and its implementations.
    OneOf,
    /// Any of several targets, or something outside the project.
    Could,
}

impl Tag {
    pub fn authored(name: &str, value: &str, line: usize) -> Self {
        Tag {
            name: name.to_string(),
            value: value.to_string(),
            line,
            origin: Origin::Authored,
            reason: None,
            reach: None,
        }
    }

    pub fn inferred(name: &str, value: &str, line: usize, reason: &str) -> Self {
        Tag {
            name: name.to_string(),
            value: value.to_string(),
            line,
            origin: Origin::Inferred,
            reason: Some(reason.to_string()),
            reach: None,
        }
    }
}

/// One file picked by stage 1, with its contents.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: PathBuf,
    /// Path relative to the project root, always with `/`.
    pub rel: String,
    pub lang: &'static LanguageSpec,
    pub text: String,
}

/// The code line a block attaches to (SPEC §2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Following {
    pub line: usize,
    pub text: String,
}

/// What an adapter learned from the code around a block (stage 3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hints {
    /// e.g. `fn create_order`
    pub symbol: Option<String>,
    /// e.g. `create_order`
    pub name: Option<String>,
    /// Default id when the block has no `@id` (e.g. Rust module path).
    pub id: Option<String>,
    /// Default `@part-of` candidates, best first; the first existing node wins.
    pub part_of: Vec<String>,
    /// Line of the enclosing item (impl, trait, inline mod) in the same file.
    pub parent_line: Option<usize>,
    /// The file this item stands for, e.g. Rust `mod extract;` → `src/extract.rs`.
    pub declares_file: Option<String>,
    /// Functions: parameter names and the simplified return type, from the code.
    pub signature: Option<Signature>,
    /// The symbol's last line, from the adapter.
    pub end: Option<usize>,
}

/// What a function takes and gives back, as the code declares it (SPEC §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    /// Parameter names, without `self`.
    pub params: Vec<String>,
    /// The return type without wrappers such as `Result`, `Option` or `Promise`; none for `()` and `void`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returns: Option<String>,
}

/// A doc block as extracted (stage 2) and enriched (stage 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBlock {
    pub file: String,
    pub line: usize,
    pub end_line: usize,
    pub summary: String,
    pub description: String,
    pub tags: Vec<Tag>,
    pub following: Option<Following>,
    pub hints: Hints,
    pub origin: Origin,
}

impl RawBlock {
    /// The authored value of a single-valued tag, else the inferred one.
    pub fn tag(&self, name: &str) -> Option<&Tag> {
        let mut tags = self.tags.iter().filter(|t| t.name == name);
        let first = tags.clone().find(|t| t.origin == Origin::Authored);
        first.or_else(|| tags.next())
    }
}

/// A Markdown file as read by stage 2 (SPEC §5).
#[derive(Debug, Clone, Default)]
pub struct MdDoc {
    pub file: String,
    /// Text outside anchored sections.
    pub rest: String,
    /// `## Heading {#node-id}` sections: (node id, markdown, line).
    pub sections: Vec<(String, String, usize)>,
    /// Nodes declared in fenced `docair` blocks.
    pub blocks: Vec<RawBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Http {
    pub method: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Doc {
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub summary: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_of: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<Http>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployed_as: Option<String>,
    pub docs: Vec<Doc>,
    pub tags: Vec<Tag>,
    pub origin: Origin,
    /// Test nodes: the result of the last run (SPEC §12.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<TestResult>,
    /// Functions: what they take and give back, from the code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<Signature>,
    /// `@runs-in`: where the code runs; `part_of` is then that place (SPEC §3.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_in: Option<String>,
    /// With `@runs-in`: where the code comes from (its `@part-of`), e.g. the container that serves it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_from: Option<String>,
}

/// The last result of a test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestResult {
    /// `passed`, `failed`, `flaky` or `skipped`.
    pub status: String,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// What the page shows when it opens (SPEC §11, §12.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    /// Sections left out: these nodes and everything inside them.
    #[serde(default)]
    pub hide: Vec<String>,
    /// Parts shown inside hidden sections: the section comes back with only them in it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub show: Vec<String>,
    /// Whether test nodes are shown.
    #[serde(default = "yes")]
    pub tests: bool,
}

fn yes() -> bool {
    true
}

impl Default for View {
    fn default() -> Self {
        View {
            hide: Vec::new(),
            show: Vec::new(),
            tests: true,
        }
    }
}

/// One proposed edit of a doc block (SPEC §10.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// The node whose block changes, or the `@id` of a new one.
    pub node: String,
    /// `edit`, `insert` or `append`; resolved from the graph.
    #[serde(default)]
    pub mode: String,
    /// The file the edit goes into; resolved from the graph.
    #[serde(default)]
    pub file: String,
    /// `edit`: the block's first line. `insert`: the code line the new block goes above.
    #[serde(default)]
    pub line: usize,
    /// Tag lines to add, e.g. `@calls docair-core.scan (folder) -> Graph`.
    #[serde(default)]
    pub add: Vec<String>,
    /// Tag lines to remove.
    #[serde(default)]
    pub remove: Vec<String>,
    /// A new summary: the block's first paragraph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Why the edit is right, in one sentence.
    #[serde(default)]
    pub why: String,
}

/// Where a line's words come from (docs/ADAPTERS.md §3): which source, and where in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineEvidence {
    /// `code`: the callee's signature; `api`: an API description.
    pub source: String,
    /// The file: the callee's, or the description's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// The rule that took the words: `signature`, `described`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
}

impl LineEvidence {
    /// The words come from the callee's signature, at its place in the code.
    pub fn code(at: Option<&SourceRef>) -> LineEvidence {
        LineEvidence {
            source: "code".into(),
            file: at.map(|s| s.file.clone()),
            line: at.map(|s| s.line),
            rule: Some("signature".into()),
        }
    }

    /// The words come from an API description.
    pub fn api(file: String) -> LineEvidence {
        LineEvidence {
            source: "api".into(),
            file: Some(file),
            line: None,
            rule: Some("described".into()),
        }
    }
}

impl Edge {
    /// The payload and reply were taken from the callee's signature, not from the tag.
    pub fn words_from_code(&self) -> bool {
        self.evidence.as_ref().is_some_and(|e| e.source == "code")
    }

    /// The API description the payload and reply were taken from.
    pub fn words_from_api(&self) -> Option<&str> {
        self.evidence
            .as_ref()
            .filter(|e| e.source == "api")
            .and_then(|e| e.file.as_deref())
    }
}

impl Node {
    pub fn tags_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Tag> + 'a {
        self.tags.iter().filter(move |t| t.name == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    Data,
    Call,
    Read,
    Write,
    Emit,
    Consume,
    Route,
    Select,
    /// Brings the target into existence when provisioning: installs, creates, builds.
    Create,
    /// A test exercises the target (SPEC §12).
    Cover,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    /// Unique and stable across scans: `<from> -<kind>-> <to>`, with its label or `#2` only when
    /// two lines would share it (docair-svh). Given once the scan's lines are final.
    #[serde(default)]
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// What travels along the edge: call arguments, data read or written, an event (SPEC §4.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    /// Calls only: what comes back to the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    /// What happens along the edge, in words: the tag's `"…"` (SPEC §4.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Data nodes the edge carries on the way there: their names occur in its payload,
    /// or in its label outside a call's reply (SPEC §3.6).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carries: Vec<String>,
    /// Calls only: data nodes the reply carries back: their names occur in the reply (SPEC §3.6).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carries_back: Vec<String>,
    /// Where the payload and reply come from when the tag does not say them: the callee's signature
    /// (SPEC §4.2) or an API description (docs/IMPORTS.md); none when the tag says them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<LineEvidence>,
    /// How sure the code is about where this line goes; none means certain (docs/ADAPTERS.md §5.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach: Option<Reach>,
    /// The tag that created the edge.
    pub tag: String,
    pub source: SourceRef,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowStep {
    pub step: u32,
    pub nodes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Indices into `Graph::edges` leading from the previous step to this one.
    pub path: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    pub name: String,
    pub steps: Vec<FlowStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
}

impl Diagnostic {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        let severity = if code.starts_with('E') {
            Severity::Error
        } else {
            Severity::Warning
        };
        Diagnostic {
            code: code.to_string(),
            severity,
            message: message.into(),
            file: None,
            line: None,
            node: None,
        }
    }

    pub fn at(mut self, file: &str, line: usize) -> Self {
        self.file = Some(file.to_string());
        self.line = Some(line);
        self
    }

    pub fn node(mut self, id: &str) -> Self {
        self.node = Some(id.to_string());
        self
    }
}

/// Version tag of a walkthrough file (SPEC §10.3).
pub const WALKTHROUGH_SCHEMA: &str = "docair.walkthrough/0";

fn walkthrough_schema() -> String {
    WALKTHROUGH_SCHEMA.to_string()
}

/// A question answered as steps through the graph, recorded as JSON (SPEC §10.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Walkthrough {
    #[serde(default = "walkthrough_schema")]
    pub schema: String,
    /// The file name without `.json`: the slug of the question.
    pub id: String,
    pub question: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded: Option<Recorded>,
    pub steps: Vec<WalkStep>,
    /// Where the walkthrough was read from; set by the scan, never stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// Who wrote a walkthrough, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recorded {
    /// RFC 3339, UTC.
    pub at: String,
    /// `ai` or `human`.
    pub by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalkStep {
    pub title: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub nodes: Vec<String>,
    #[serde(default)]
    pub edges: Vec<EdgeRef>,
    /// Indices into `Graph::edges` that the edge refs bound to; set by the scan, never stored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<usize>,
}

/// An edge named by its ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeRef {
    pub from: String,
    pub to: String,
}

/// A term of the glossary: a word the docs use, and what it means (SPEC §13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Term {
    /// Slug of the term, e.g. `diagnostics`.
    pub id: String,
    pub term: String,
    /// Other spellings that mean the same, e.g. `diagnostic`, `W104`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// The first paragraph of the definition: what a hover shows (Markdown).
    pub summary: String,
    /// The whole definition (Markdown).
    pub markdown: String,
    /// Nodes where the term is implemented or used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub see: Vec<String>,
    pub source: SourceRef,
    /// Where the meaning comes from: `code` or `doc`; empty for a glossary entry (docs/MEANINGS.md §7).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from: String,
    /// The part whose docs define the word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// A doc heading that defines the word: "read more" opens it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<TermSection>,
    /// The glossary's text while it is not checked against the code that defines the word (Markdown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unchecked: Option<String>,
}

/// A heading of a doc that defines a word: `## 4. Facts: the contract between adapters and core`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermSection {
    pub file: String,
    /// GitHub's anchor of the heading.
    pub anchor: String,
    pub title: String,
}

/// A Markdown file of the project that a glossary entry links to; the page embeds it (SPEC §13.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryDoc {
    pub file: String,
    pub markdown: String,
}

/// The whole model: the only contract between backend and viewer (SPEC §9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Graph {
    pub schema: String,
    pub system: String,
    pub generator: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub flows: Vec<Flow>,
    #[serde(default)]
    pub walkthroughs: Vec<Walkthrough>,
    #[serde(default)]
    pub view: View,
    /// What the words of the docs mean (SPEC §13).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glossary: Vec<Term>,
    /// Docs the glossary links to, embedded so the page can show them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub library: Vec<LibraryDoc>,
    /// The decisions about words and what is still to settle (docs/MEANINGS.md).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meanings: Option<crate::meanings::Meanings>,
    pub diagnostics: Vec<Diagnostic>,
}

//! Facts: what adapters learn from the code (docs/ADAPTERS.md §4).
//!
//! Adapters report facts and never touch the graph. The core merges them with
//! the doc blocks (`merge`) and builds nodes and edges from both.

pub use crate::model::Reach;
use crate::model::{Diagnostic, Http, Signature};

/// Where a fact was read, and why, in words the viewer shows (`ⓘ inferred from …`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    pub line: usize,
    pub why: String,
}

impl Evidence {
    pub fn new(line: usize, why: impl Into<String>) -> Self {
        Evidence {
            line,
            why: why.into(),
        }
    }
}

/// A value an adapter read from the code, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inferred {
    pub value: String,
    pub why: String,
}

impl Inferred {
    pub fn new(value: impl Into<String>, why: impl Into<String>) -> Self {
        Inferred {
            value: value.into(),
            why: why.into(),
        }
    }
}

/// When a symbol becomes a node (docs/ADAPTERS.md §8).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Draw {
    /// Only when a block documents it; then it is that block's node.
    #[default]
    Documented,
    /// Always: the node of the block that documents it, else a node of its own (`origin: discovered`).
    Always,
    /// Always as a node of its own; no block attaches to it (what a Terraform resource creates).
    Alone,
}

/// Something that exists in the code: a function, a type, a Terraform resource, a Kubernetes object.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Symbol {
    /// Unique within one scan; serves and relations name their symbol by it.
    pub key: String,
    pub file: String,
    /// The symbol's own line, after attributes and decorators.
    pub line: usize,
    /// The last line of the symbol's code, when the adapter knows it (docair-zrr.2).
    pub end: Option<usize>,
    /// The first code line a block may stand on and still document this symbol; usually `line`.
    pub attach_from: usize,
    pub draw: Draw,
    /// e.g. `fn create_order`, `Deployment docair-server`.
    pub display: Option<String>,
    pub name: Option<String>,
    /// The default id, e.g. from the Rust module path.
    pub id: Option<String>,
    pub kind: Option<Inferred>,
    /// Candidate parents (node ids), best first; the first that exists wins.
    pub parents: Vec<String>,
    /// Line of the enclosing item (impl, trait, inline mod) in the same file.
    pub parent_line: Option<usize>,
    /// The file this symbol stands for, e.g. Rust `mod extract;` → `src/extract.rs`.
    pub declares_file: Option<String>,
    pub signature: Option<Signature>,
    /// What the code's own comments say (`///` in Rust): the first paragraph is the summary.
    /// A block's text wins over it.
    pub docs: Option<String>,
}

impl Symbol {
    /// A symbol at `line` that a block documents only from that line.
    pub fn at(key: impl Into<String>, file: &str, line: usize) -> Self {
        Symbol {
            key: key.into(),
            file: file.to_string(),
            line,
            attach_from: line,
            ..Symbol::default()
        }
    }

    /// Whether the symbol says what it is. One that doesn't only brings relations to a block.
    pub fn has_identity(&self) -> bool {
        self.display.is_some()
            || self.name.is_some()
            || self.id.is_some()
            || self.kind.is_some()
            || self.signature.is_some()
    }
}

/// A rendezvous point: where two sides meet without knowing each other (docs/ADAPTERS.md §4, §7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Meet {
    Http(Http),
    /// A message topic or queue, by its address: `order.created`.
    Topic(String),
    /// An environment variable: the code reads it, a Deployment's env or a ConfigMap sets it.
    Env(String),
}

impl std::fmt::Display for Meet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Meet::Http(h) => write!(f, "{} {}", h.method, h.path),
            Meet::Topic(t) => write!(f, "topic:{t}"),
            Meet::Env(name) => write!(f, "env:{name}"),
        }
    }
}

/// A symbol is where a rendezvous point leads: the code that runs there, e.g. the handler of a route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Serve {
    /// The key of the symbol that serves.
    pub by: String,
    pub at: Meet,
    pub evidence: Evidence,
}

/// An API description says a rendezvous point exists, with what travels there; it does not serve it.
///
/// An OpenAPI operation declares `POST /api/orders` taking `CreateOrder` and
/// answering `Order`; an AsyncAPI document declares the topics an application
/// sends to and receives from. The core matches declarations with the code
/// that serves and requests the same point (docs/IMPORTS.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declare {
    /// The key of the symbol that declares: the description file.
    pub by: String,
    pub at: Meet,
    /// HTTP: what the request sends. Topics: the message.
    pub payload: Option<String>,
    /// HTTP: what comes back.
    pub reply: Option<String>,
    /// The description's own name for it, e.g. the OpenAPI `operationId`.
    pub name: Option<String>,
    pub role: Role,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub evidence: Evidence,
}

/// A symbol asks a rendezvous point for something: a client's HTTP request.
///
/// Requests become lines to what serves the same point in step 6
/// (docs/ADAPTERS.md §7); until then they show as an inferred `@requests` tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The key of the symbol that sends the request.
    pub by: String,
    pub at: Meet,
    /// What the request sends.
    pub payload: Option<String>,
    /// What comes back.
    pub reply: Option<String>,
    pub evidence: Evidence,
}

/// What the described application does at a declared rendezvous point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// It answers requests there: an OpenAPI operation.
    Serves,
    /// It sends messages there: AsyncAPI `send` (2.x `subscribe`).
    Sends,
    /// It receives messages from there: AsyncAPI `receive` (2.x `publish`).
    Receives,
}

/// What a relation says, named like the tag an author would write (SPEC §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    In,
    Out,
    Calls,
    Reads,
    Writes,
    Emits,
    Consumes,
    Creates,
    Routes,
    Selects,
    Covers,
    /// A part uses a type: makes, takes, returns, holds, matches it, or reads its fields (docs/ADAPTERS.md §5.8).
    Uses,
}

impl Verb {
    pub fn tag(self) -> &'static str {
        match self {
            Verb::In => "in",
            Verb::Out => "out",
            Verb::Calls => "calls",
            Verb::Reads => "reads",
            Verb::Writes => "writes",
            Verb::Emits => "emits",
            Verb::Consumes => "consumes",
            Verb::Creates => "creates",
            Verb::Routes => "routes",
            Verb::Selects => "selects",
            Verb::Covers => "covers",
            Verb::Uses => "uses",
        }
    }
}

/// What a relation points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A node id or a ref as tags write it (SPEC §4.3): `tf:helm_release.x`, `k8s:Service/web`.
    Ref(String),
    /// A symbol by its key; the merge turns it into the id of that symbol's node.
    Symbol(String),
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Ref(r) | Target::Symbol(r) => f.write_str(r),
        }
    }
}

/// A symbol uses, makes or receives something: a call, data, a route, a selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    /// The key of the symbol the relation belongs to.
    pub from: String,
    pub verb: Verb,
    pub to: Target,
    /// `in` and `out`: what travels, written before `from` / `to`.
    pub label: Option<String>,
    /// What travels or how, written in parentheses after the target.
    pub payload: Option<String>,
    /// How sure the code is that this is where it goes (docs/ADAPTERS.md §5.5).
    pub reach: Reach,
    pub evidence: Evidence,
}

impl Relation {
    pub fn new(verb: Verb, to: Target, evidence: Evidence) -> Self {
        Relation {
            from: String::new(),
            verb,
            to,
            label: None,
            payload: None,
            reach: Reach::Certain,
            evidence,
        }
    }

    pub fn reach(mut self, reach: Reach) -> Self {
        self.reach = reach;
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn payload(mut self, payload: Option<String>) -> Self {
        self.payload = payload;
        self
    }

    /// The relation as a tag value, e.g. `uses from tf:kind_cluster.dev` or `k8s:Service/api (port 80)`.
    pub fn value(&self) -> String {
        self.value_to(&self.to.to_string())
    }

    /// The tag value with the target written as `to`, e.g. the id a symbol target became.
    pub fn value_to(&self, to: &str) -> String {
        let label = self.label.as_deref().unwrap_or("data");
        match (self.verb, &self.payload) {
            (Verb::In, _) => format!("{label} from {to}"),
            (Verb::Out, _) => format!("{label} to {to}"),
            (_, Some(p)) => format!("{to} ({p})"),
            (_, None) => to.to_string(),
        }
    }
}

/// Everything one adapter learned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub symbols: Vec<Symbol>,
    pub serves: Vec<Serve>,
    /// What API descriptions declare (docs/IMPORTS.md).
    pub declares: Vec<Declare>,
    /// What clients ask rendezvous points for.
    pub requests: Vec<Request>,
    pub relations: Vec<Relation>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Facts {
    pub fn extend(&mut self, other: Facts) {
        self.symbols.extend(other.symbols);
        self.serves.extend(other.serves);
        self.declares.extend(other.declares);
        self.requests.extend(other.requests);
        self.relations.extend(other.relations);
        self.diagnostics.extend(other.diagnostics);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relations_read_like_the_tags_authors_write() {
        let e = Evidence::new(1, "test");
        let r = |v, t: &str| Relation::new(v, Target::Ref(t.into()), e.clone());
        assert_eq!(
            r(Verb::In, "tf:kind_cluster.dev")
                .label("uses, runs after")
                .value(),
            "uses, runs after from tf:kind_cluster.dev"
        );
        assert_eq!(
            r(Verb::Creates, "k8s.release.shop")
                .payload(Some("helm install: chart shop".into()))
                .value(),
            "k8s.release.shop (helm install: chart shop)"
        );
        assert_eq!(
            r(Verb::Selects, "k8s:Deployment/web").value(),
            "k8s:Deployment/web"
        );
    }
}

//! Framework rules as data (docs/ADAPTERS.md §5.2).
//!
//! A rule is a YAML file: an [ast-grep](https://ast-grep.github.io/) rule
//! (`rule`, `constraints`, `utils`, `transform`, the same keys the `sg` tool
//! reads) plus `facts`, which say what a match means. Language adapters load
//! their rules into a [`RuleSet`] and turn what it finds into facts.
//!
//! ```yaml
//! id: rust.actix.route
//! language: rust
//! rule:
//!   pattern: '#[$METHOD($PATH)]'
//! constraints:
//!   METHOD: { regex: "^(get|post)$" }
//! facts:
//!   - serves: http
//!     method: $METHOD
//!     path: $PATH
//!     by: next          # the item the attribute belongs to
//! ```

use std::borrow::Cow;
use std::path::Path;

use ast_grep_config::{DeserializeEnv, RuleCore, SerializableRuleCore};
use ast_grep_core::matcher::{Pattern, PatternBuilder, PatternError};
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::tree_sitter::{LanguageExt, StrDoc, TSLanguage};
use ast_grep_core::{Language, Node};
use docair_core::facts::Meet;
use docair_core::model::Http;
use serde::Deserialize;

/// A tree-sitter grammar, as ast-grep needs it.
#[derive(Clone)]
pub struct Grammar {
    name: &'static str,
    ts: TSLanguage,
    /// Stands for `$` in patterns of languages where `$` can't start a name (Rust: `µ`).
    expando: char,
}

impl Grammar {
    /// `name` is what rule files write as `language`.
    pub fn new(name: &'static str, ts: TSLanguage, expando: char) -> Self {
        Grammar { name, ts, expando }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}

impl Language for Grammar {
    fn pre_process_pattern<'q>(&self, query: &'q str) -> Cow<'q, str> {
        if self.expando == '$' {
            return Cow::Borrowed(query);
        }
        // `$X` and `$$$` become `µX` and `µµµ`; a `$` before anything else stays.
        let mut out = String::with_capacity(query.len());
        let mut dollars = 0;
        for c in query.chars() {
            if c == '$' {
                dollars += 1;
                continue;
            }
            let sigil = if c.is_ascii_uppercase() || c == '_' || dollars == 3 {
                self.expando
            } else {
                '$'
            };
            out.extend(std::iter::repeat_n(sigil, dollars));
            dollars = 0;
            out.push(c);
        }
        let sigil = if dollars == 3 { self.expando } else { '$' };
        out.extend(std::iter::repeat_n(sigil, dollars));
        Cow::Owned(out)
    }

    fn expando_char(&self) -> char {
        self.expando
    }

    fn kind_to_id(&self, kind: &str) -> u16 {
        self.ts.id_for_node_kind(kind, true)
    }

    fn field_to_id(&self, field: &str) -> Option<u16> {
        self.ts.field_id_for_name(field).map(|f| f.get())
    }

    fn build_pattern(&self, builder: &PatternBuilder) -> Result<Pattern, PatternError> {
        builder.build(|src| StrDoc::try_new(src, self.clone()))
    }
}

impl LanguageExt for Grammar {
    fn get_ts_language(&self) -> TSLanguage {
        self.ts.clone()
    }
}

/// Which symbol a fact belongs to, seen from the match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum By {
    /// The matched node itself.
    #[default]
    Match,
    /// The next sibling that is something else: the item an attribute belongs to.
    Next,
    /// The node the match sits in: the method a decorator belongs to.
    Parent,
    /// The symbol the match is written in: the function that registers a route.
    Enclosing,
    /// The symbol a captured name names, such as the handler in `get(health)`.
    Name(String),
}

impl<'de> Deserialize<'de> for By {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(match s.as_str() {
            "match" => By::Match,
            "next" => By::Next,
            "parent" => By::Parent,
            "enclosing" => By::Enclosing,
            _ if s.starts_with('$') => By::Name(s),
            _ => {
                return Err(serde::de::Error::custom(format!(
                    "by: `{s}` is none of match, next, parent, enclosing or a $NAME"
                )));
            }
        })
    }
}

/// Where the adapter finds the symbol a fact belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum About {
    /// The symbol that starts on this line (1-based).
    Line(usize),
    /// The innermost symbol whose code holds this line.
    Within(usize),
    /// The symbol with this name, as the code writes it (`health`, `handlers::create`).
    Name(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MeetKind {
    Http,
}

/// One entry of a rule's `facts`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct FactSpec {
    /// `serves: http`: the symbol `by` names serves an HTTP route.
    serves: Option<MeetKind>,
    method: Option<String>,
    path: Option<String>,
    /// `calls: $NAME`: the symbol `by` names calls the symbol this name names.
    calls: Option<String>,
    /// What travels along a call, e.g. `$VERB $PATH`.
    payload: Option<String>,
    #[serde(default)]
    by: By,
}

impl FactSpec {
    /// Every template the fact uses.
    fn templates(&self) -> Vec<&str> {
        let by = match &self.by {
            By::Name(n) => Some(n.as_str()),
            _ => None,
        };
        [&self.method, &self.path, &self.calls, &self.payload]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .chain(by)
            .collect()
    }
}

#[derive(Deserialize)]
struct RuleFile {
    id: String,
    language: String,
    #[serde(flatten)]
    core: SerializableRuleCore,
    facts: Vec<FactSpec>,
}

/// A loaded rule: its matcher and what a match means.
pub struct Rule {
    pub id: String,
    matcher: RuleCore,
    facts: Vec<FactSpec>,
}

/// What a match means, before the adapter names the symbols.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    Serves(Meet),
    /// A call to the symbol a name names, with what travels.
    Calls {
        to: String,
        payload: Option<String>,
    },
}

/// A fact one rule found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub rule: String,
    /// The symbol the fact belongs to.
    pub about: About,
    pub fact: Fact,
    /// The line of the match (1-based).
    pub line: usize,
    /// The matched code, for the viewer's "inferred from …".
    pub why: String,
}

/// The `$NAME`s a template uses.
fn vars(template: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(i) = rest.find('$') {
        let after = &rest[i + 1..];
        let end = after
            .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            .unwrap_or(after.len());
        if end > 0 {
            out.push(&after[..end]);
        }
        rest = &after[end..];
    }
    out
}

/// A string literal's content; anything else as written.
fn unquote(text: &str) -> &str {
    let t = text.trim();
    let mut chars = t.chars();
    match (chars.next(), chars.next_back()) {
        (Some(a), Some(b)) if a == b && matches!(a, '"' | '\'' | '`') && t.len() >= 2 => {
            &t[1..t.len() - 1]
        }
        _ => t,
    }
}

/// A template with its variables filled in; `None` when the match left one of them unbound.
fn fill(template: &str, env: &MetaVarEnv<StrDoc<Grammar>>) -> Option<String> {
    let mut out = template.to_string();
    let mut names = vars(template);
    // Longest first, so `$PATH_X` is not cut by `$PATH`.
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    for name in names {
        let text = match env.get_match(name) {
            Some(n) => unquote(&n.text()).to_string(),
            None => String::from_utf8_lossy(env.get_transformed(name)?).into_owned(),
        };
        out = out.replace(&format!("${name}"), &text);
    }
    Some(out)
}

impl Rule {
    /// Parse and check a rule file written for `grammar`.
    pub fn parse(yaml: &str, grammar: &Grammar) -> Result<Rule, String> {
        let file: RuleFile = ast_grep_config::from_str(yaml).map_err(|e| e.to_string())?;
        if file.language != grammar.name {
            return Err(format!(
                "rule {} is for `{}`, not `{}`",
                file.id, file.language, grammar.name
            ));
        }
        let matcher = file
            .core
            .get_matcher(DeserializeEnv::new(grammar.clone()))
            .map_err(|e| format!("rule {}: {e}", file.id))?;
        if file.facts.is_empty() {
            return Err(format!("rule {} says no facts", file.id));
        }
        let defined = matcher.defined_vars();
        for f in &file.facts {
            match (f.serves, &f.calls) {
                (Some(MeetKind::Http), None) if f.method.is_some() && f.path.is_some() => {}
                (Some(MeetKind::Http), None) => {
                    return Err(format!(
                        "rule {}: `serves: http` needs method and path",
                        file.id
                    ));
                }
                (None, Some(_)) => {}
                _ => {
                    return Err(format!(
                        "rule {}: a fact needs either `serves` or `calls`",
                        file.id
                    ));
                }
            }
            for t in f.templates() {
                if let Some(v) = vars(t).into_iter().find(|v| !defined.contains(v)) {
                    return Err(format!("rule {}: `${v}` is not in the rule", file.id));
                }
            }
        }
        Ok(Rule {
            id: file.id,
            matcher,
            facts: file.facts,
        })
    }

    fn about(
        node: &Node<StrDoc<Grammar>>,
        by: &By,
        env: &MetaVarEnv<StrDoc<Grammar>>,
    ) -> Option<About> {
        let line = |n: Node<StrDoc<Grammar>>| About::Line(n.start_pos().line() + 1);
        match by {
            By::Match => Some(line(node.clone())),
            By::Next => node
                .next_all()
                .find(|n| n.is_named() && !n.is_extra() && n.kind() != node.kind())
                .map(line),
            By::Parent => node.parent().map(line),
            By::Enclosing => Some(About::Within(node.start_pos().line() + 1)),
            By::Name(t) => fill(t, env).map(About::Name),
        }
    }
}

/// The rules of one language.
pub struct RuleSet {
    grammar: Grammar,
    rules: Vec<Rule>,
}

impl RuleSet {
    /// Load rule files given as (file name, YAML); an error names the file.
    pub fn load(grammar: Grammar, files: &[(&str, &str)]) -> Result<RuleSet, String> {
        let rules = files
            .iter()
            .map(|(name, yaml)| Rule::parse(yaml, &grammar).map_err(|e| format!("{name}: {e}")))
            .collect::<Result<_, _>>()?;
        Ok(RuleSet { grammar, rules })
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.rules.iter().map(|r| r.id.as_str())
    }

    /***
     * Run every rule on one file's source and return what they found, rule by rule, in source order.
     ***/
    pub fn apply(&self, src: &str) -> Vec<Found> {
        let root = self.grammar.ast_grep(src);
        let mut out = Vec::new();
        for rule in &self.rules {
            for m in root.root().find_all(&rule.matcher) {
                let env = m.get_env();
                let why = m.text().lines().next().unwrap_or("").trim().to_string();
                for f in &rule.facts {
                    // A fact whose variables this match left unbound is not about this match.
                    let Some(about) = Rule::about(&m, &f.by, env) else {
                        continue;
                    };
                    let fact = match (f.serves, &f.calls) {
                        (Some(MeetKind::Http), _) => {
                            let (Some(method), Some(path)) = (
                                fill(f.method.as_deref().unwrap_or(""), env),
                                fill(f.path.as_deref().unwrap_or(""), env),
                            ) else {
                                continue;
                            };
                            Fact::Serves(Meet::Http(Http {
                                method: method.to_uppercase(),
                                path,
                            }))
                        }
                        (None, Some(to)) => {
                            let Some(to) = fill(to, env) else {
                                continue;
                            };
                            let payload = match &f.payload {
                                Some(p) => match fill(p, env) {
                                    Some(p) => Some(p),
                                    None => continue,
                                },
                                None => None,
                            };
                            Fact::Calls { to, payload }
                        }
                        (None, None) => continue,
                    };
                    out.push(Found {
                        rule: rule.id.clone(),
                        about,
                        fact,
                        line: m.start_pos().line() + 1,
                        why: why.clone(),
                    });
                }
            }
        }
        out
    }
}

/// Check every rule file in `dir` against its fixture (docs/ADAPTERS.md §9.1).
///
/// `<name>.yaml` needs `<name>.fixture.<ext>` beside it. The fixture's lines
/// that start with `expect` (e.g. `//>`) list the facts the rule must find,
/// and no others. `facts_of(rule_yaml, fixture_source)` runs one rule on the
/// fixture and renders the facts as lines. Returns one message per failure.
pub fn check_fixtures(
    dir: &Path,
    ext: &str,
    expect: &str,
    facts_of: impl Fn(&str, &str) -> Result<Vec<String>, String>,
) -> Vec<String> {
    let mut failures = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .map(|d| d.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default();
    names.sort();
    for path in names
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
    {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let fixture = dir.join(format!("{stem}.fixture.{ext}"));
        let (Ok(yaml), Ok(src)) = (
            std::fs::read_to_string(path),
            std::fs::read_to_string(&fixture),
        ) else {
            failures.push(format!("{stem}.yaml has no fixture {}", fixture.display()));
            continue;
        };
        let mut expected: Vec<String> = src
            .lines()
            .filter_map(|l| l.trim().strip_prefix(expect))
            .map(|l| l.trim().to_string())
            .collect();
        if expected.is_empty() {
            failures.push(format!(
                "{} expects nothing (`{expect}` lines)",
                fixture.display()
            ));
        }
        match facts_of(&yaml, &src) {
            Err(e) => failures.push(format!("{stem}.yaml: {e}")),
            Ok(mut actual) => {
                expected.sort();
                actual.sort();
                if actual != expected {
                    failures.push(format!(
                        "{stem}.yaml on its fixture\n  expected: {expected:#?}\n  found:    {actual:#?}"
                    ));
                }
            }
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust() -> Grammar {
        Grammar::new("rust", tree_sitter_rust::LANGUAGE.into(), 'µ')
    }

    #[test]
    fn templates_and_literals() {
        assert_eq!(vars("$METHOD /api$PATH"), ["METHOD", "PATH"]);
        assert_eq!(unquote("\"/api/orders\""), "/api/orders");
        assert_eq!(unquote("post"), "post");
        assert_eq!(rust().pre_process_pattern("#[$M($$$A)] $"), "#[µM(µµµA)] $");
    }

    #[test]
    fn rules_are_checked_when_they_load() {
        let bad = |yaml: &str| Rule::parse(yaml, &rust()).err().unwrap_or_default();
        assert!(bad("id: x\nlanguage: ts\nrule: {pattern: a}\nfacts: [{serves: http, method: GET, path: /}]").contains("not `rust`"));
        assert!(bad("id: x\nlanguage: rust\nrule: {pattern: a}\nfacts: []").contains("no facts"));
        assert!(bad("id: x\nlanguage: rust\nrule: {pattern: a}\nfacts: [{serves: http, method: $M, path: /}]").contains("`$M` is not in the rule"));
        assert!(bad("id: x\nlanguage: rust\nrule: {pattern: a}\nfacts: [{serves: http, method: GET, path: /, by: x}]").contains("none of match"));
        assert!(
            bad("id: x\nlanguage: rust\nrule: {pattern: a}\nfacts: [{calls: $H, by: enclosing}]")
                .contains("`$H` is not in the rule")
        );
        assert!(
            bad("id: x\nlanguage: rust\nrule: {pattern: a}\nfacts: [{method: GET}]")
                .contains("either `serves` or `calls`")
        );
    }

    #[test]
    fn a_rule_without_a_fixture_or_expectations_fails() {
        let dir = std::env::temp_dir().join(format!("docair-rules-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lonely.yaml"), "id: x").unwrap();
        std::fs::write(dir.join("quiet.yaml"), "id: y").unwrap();
        std::fs::write(dir.join("quiet.fixture.rs"), "fn a() {}\n").unwrap();
        let failures = check_fixtures(&dir, "rs", "//>", |_, _| Ok(vec![]));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(failures.len(), 2, "{failures:?}");
        assert!(failures[0].contains("lonely.yaml has no fixture"));
        assert!(failures[1].contains("expects nothing"));
    }

    #[test]
    fn a_route_attribute_serves_from_the_item_it_belongs_to() {
        let yaml = "id: t\nlanguage: rust\nrule:\n  pattern: '#[$METHOD($PATH)]'\nconstraints:\n  METHOD: {regex: '^(get|post)$'}\nfacts:\n  - serves: http\n    method: $METHOD\n    path: $PATH\n    by: next\n";
        let set = RuleSet::load(rust(), &[("t.yaml", yaml)]).unwrap();
        let src =
            "#[post(\"/api/orders\")]\npub async fn create() {}\n\n#[cfg(test)]\nfn other() {}\n";
        let found = set.apply(src);
        assert_eq!(
            found,
            [Found {
                rule: "t".into(),
                about: About::Line(2),
                line: 1,
                fact: Fact::Serves(Meet::Http(Http {
                    method: "POST".into(),
                    path: "/api/orders".into()
                })),
                why: "#[post(\"/api/orders\")]".into(),
            }]
        );
    }
}

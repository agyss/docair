//! Rust adapter (SPEC §6, docs/ADAPTERS.md): tree-sitter finds every item, the rules what frameworks mean.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use docair_core::adapter::{Adapter, Project};
use docair_core::facts::{Draw, Evidence, Facts, Inferred, Relation, Serve, Symbol, Target, Verb};
use docair_core::ids::slug;
use docair_core::model::{Diagnostic, Signature, SourceFile};
use docair_core::signature::simplify_type;
use docair_rules::{About, Fact, Found, Grammar, RuleSet};
use tree_sitter::{Node, Parser};

/// The built-in rules (docs/ADAPTERS.md §5.2). Each needs a fixture beside it in `rules/`.
const RULES: &[(&str, &str)] = &[
    (
        "actix.route.yaml",
        include_str!("../rules/actix.route.yaml"),
    ),
    ("axum.route.yaml", include_str!("../rules/axum.route.yaml")),
];

/// Rust as ast-grep sees it: `$` can't start a name, so patterns use `µ` for it.
pub fn grammar() -> Grammar {
    Grammar::new("rust", tree_sitter_rust::LANGUAGE.into(), 'µ')
}

fn rules() -> &'static RuleSet {
    static SET: OnceLock<RuleSet> = OnceLock::new();
    SET.get_or_init(|| {
        RuleSet::load(grammar(), RULES).expect("the built-in rules load (their fixtures test them)")
    })
}

/***
 * The Rust adapter: every item of a Rust file, found with tree-sitter, with ids and parents from the module path.
 *
 * Reports symbol and kind (fn, struct, enum, trait, impl, mod, const), a
 * module-path id, the enclosing impl or module as the default parent, and
 * the `///` docs. Every item is drawn (docs/ADAPTERS.md §8) except impl
 * blocks (their methods sit inside the type), unit tests and items inside
 * function bodies, which are drawn only when a block documents them. The
 * rules in `rules/` add what frameworks mean: the routes of actix and axum
 * handlers, and the router's calls to them.
 ***/
pub struct RustAdapter;

impl Adapter for RustAdapter {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn handles(&self, file: &SourceFile) -> bool {
        file.lang.name == "rust"
    }

    /***
     * Read every Rust file, then link the names the rules found (a handler in `get(health)`) across the files of a crate.
     *
     * @flow  scan 7 "tree-sitter parses Rust"
     * @calls docair-lang-rust.read_file "reads the items and runs the rules"
     * @calls docair-lang-rust.link "links handler names to their functions"
     ***/
    fn project(&self, project: &Project) -> Facts {
        let read: Vec<FileRead> = project
            .files
            .iter()
            .filter(|f| self.handles(f))
            .map(|f| read_file(f, rules()))
            .collect();
        link(read)
    }
}

/// A function or other item the rules may name, with where its code is.
struct Site {
    key: String,
    name: String,
    is_fn: bool,
    start: usize,
    end: usize,
}

/// One Rust file, read: its facts so far, its items, and what the rules found.
pub struct FileRead {
    facts: Facts,
    krate: Option<String>,
    items: Vec<Site>,
    found: Vec<Found>,
}

/***
 * Parse a Rust file once with tree-sitter, report every item in it, and run the rules on it.
 *
 * @calls docair-lang-rust.items_by_row "indexes the syntax tree by line"
 * @calls docair-lang-rust.crate_of "finds which package owns the file"
 * @calls docair-rules.rule-set.apply "finds what frameworks mean, such as routes"
 ***/
pub fn read_file(file: &SourceFile, rules: &RuleSet) -> FileRead {
    let mut read = FileRead {
        facts: Facts::default(),
        krate: None,
        items: Vec::new(),
        found: Vec::new(),
    };
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("tree-sitter-rust matches the tree-sitter ABI");
    let Some(tree) = parser.parse(&file.text, None) else {
        read.facts.diagnostics.push(
            Diagnostic::new("W103", "tree-sitter could not parse this file").at(&file.rel, 1),
        );
        return read;
    };
    let src = file.text.as_bytes();
    let mut items: Vec<(usize, Node)> = items_by_row(tree.root_node()).into_iter().collect();
    items.sort_by_key(|(row, _)| *row);
    let krate = crate_of(&file.path);
    let file_mods = file_modules(&file.rel, &file.path, krate.as_ref());
    let test_file = is_test_file(&file.rel);
    read.krate = krate.as_ref().map(|k| k.name.clone());

    for (row, item) in items {
        let Some(it) = describe(item, src) else {
            continue;
        };
        let line = row + 1;
        let key = format!("rust:{}:{line}", file.rel);

        // Module path: the file's modules + inline `mod x { }` ancestors.
        let mut inline = Vec::new();
        let mut owner: Option<String> = None;
        let mut parent_line = None;
        let mut cur = item.parent();
        while let Some(n) = cur {
            match n.kind() {
                "mod_item" => {
                    inline.push(text_of(n.child_by_field_name("name"), src));
                    parent_line.get_or_insert(n.start_position().row + 1);
                }
                "impl_item" | "trait_item" if owner.is_none() && inline.is_empty() => {
                    let field = if n.kind() == "impl_item" {
                        "type"
                    } else {
                        "name"
                    };
                    owner = Some(base_type(&text_of(n.child_by_field_name(field), src)));
                    parent_line.get_or_insert(n.start_position().row + 1);
                }
                _ => {}
            }
            cur = n.parent();
        }
        let mut prefix: Vec<String> = krate.iter().map(|k| slug(&k.name)).collect();
        prefix.extend(file_mods.iter().chain(inline.iter().rev()).map(|m| slug(m)));
        if prefix.is_empty() {
            prefix.push(slug(&file_stem(&file.rel)));
        }

        // Parent candidates, best first: the owning type, then the module path outwards.
        let mut candidates = Vec::new();
        let module = prefix.join(".");
        if let Some(o) = &owner {
            candidates.push(format!("{module}.{}", slug(o)));
        }
        for k in (1..=prefix.len()).rev() {
            candidates.push(prefix[..k].join("."));
        }
        let id = match &owner {
            Some(o) => format!("{module}.{}.{}", slug(o), slug(&it.name)),
            None => format!("{module}.{}", slug(&it.name)),
        };
        let declares_file = (it.keyword == "mod" && item.child_by_field_name("body").is_none())
            .then(|| module_file(&file.rel, &file.path, &it.name))
            .flatten();
        let hidden = it.keyword == "impl" || in_function(item) || test_file || is_test(item, src);

        read.items.push(Site {
            key: key.clone(),
            name: it.name.clone(),
            is_fn: it.keyword == "fn",
            start: line,
            end: item.end_position().row + 1,
        });
        read.facts.symbols.push(Symbol {
            draw: if hidden {
                Draw::Documented
            } else {
                Draw::Always
            },
            display: Some(it.symbol),
            name: Some(it.name),
            id: Some(id),
            kind: Some(Inferred::new(
                it.kind,
                format!("tree-sitter: {}", item.kind()),
            )),
            parents: candidates,
            parent_line,
            declares_file,
            signature: signature_of(item, src),
            docs: docs_of(item, src),
            ..Symbol::at(key, &file.rel, line)
        });
    }
    read.found = rules.apply(&file.text);
    read
}

/// The facts of one file on its own, names linked inside it only (fixtures, tests).
pub fn facts_with(file: &SourceFile, rules: &RuleSet) -> Facts {
    link(vec![read_file(file, rules)])
}

impl FileRead {
    /// The item a rule's fact belongs to, in this file.
    fn at(&self, about: &About) -> Option<&Site> {
        match about {
            About::Line(line) => self.items.iter().find(|i| i.start == *line),
            About::Within(line) => self
                .items
                .iter()
                .filter(|i| i.start <= *line && *line <= i.end)
                .min_by_key(|i| i.end - i.start),
            About::Name(_) => None,
        }
    }
}

/// The function a name in the code names: in the same file, else the only one of that name in the crate.
fn named<'a>(files: &'a [FileRead], at: usize, name: &str) -> Option<&'a Site> {
    let name = name.rsplit("::").next().unwrap_or(name).trim();
    let fns = |f: &'a FileRead| f.items.iter().filter(move |i| i.is_fn && i.name == name);
    let mut here = fns(&files[at]);
    if let (Some(one), None) = (here.next(), here.next()) {
        return Some(one);
    }
    let krate = files[at].krate.as_ref()?;
    let mut there = files
        .iter()
        .filter(|f| f.krate.as_ref() == Some(krate))
        .flat_map(fns);
    match (there.next(), there.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    }
}

/***
 * Turn what the rules found into facts: each fact on the symbol it belongs to, names linked to their functions.
 *
 * A name that names no function, or more than one in the crate, gives no fact.
 ***/
pub fn link(files: Vec<FileRead>) -> Facts {
    let mut out = Facts::default();
    for (at, file) in files.iter().enumerate() {
        for found in &file.found {
            let item = match &found.about {
                About::Name(n) => named(&files, at, n),
                about => file.at(about),
            };
            let Some(item) = item else {
                continue;
            };
            match &found.fact {
                Fact::Serves(meet) => out.serves.push(Serve {
                    by: item.key.clone(),
                    at: meet.clone(),
                    evidence: Evidence::new(item.start, found.why.clone()),
                }),
                Fact::Calls { to, payload } => {
                    let Some(callee) = named(&files, at, to) else {
                        continue;
                    };
                    out.relations.push(Relation {
                        from: item.key.clone(),
                        ..Relation::new(
                            Verb::Calls,
                            Target::Symbol(callee.key.clone()),
                            Evidence::new(found.line, found.why.clone()),
                        )
                        .payload(payload.clone())
                    });
                }
            }
        }
    }
    for file in files {
        out.extend(file.facts);
    }
    out
}

/// Files of integration tests and benchmarks.
fn is_test_file(rel: &str) -> bool {
    rel.split('/').any(|seg| seg == "tests" || seg == "benches")
}

/// Whether an item is written inside a function or closure, not at module level.
fn in_function(item: Node) -> bool {
    let mut cur = item.parent();
    while let Some(n) = cur {
        if matches!(n.kind(), "function_item" | "closure_expression") {
            return true;
        }
        cur = n.parent();
    }
    false
}

/// The attributes written right above a node.
fn attributes_of<'a>(node: Node, src: &'a [u8]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut sib = node.prev_named_sibling();
    while let Some(s) = sib {
        match s.kind() {
            "attribute_item" => out.push(s.utf8_text(src).unwrap_or("")),
            "line_comment" | "block_comment" => {}
            _ => break,
        }
        sib = s.prev_named_sibling();
    }
    out
}

/// Unit tests: an item marked `#[test]` (or `#[tokio::test]`, …), or inside a `#[cfg(test)]` module.
fn is_test(item: Node, src: &[u8]) -> bool {
    let mut cur = Some(item);
    while let Some(n) = cur {
        if attributes_of(n, src).iter().any(|a| {
            let a = a.split_whitespace().collect::<String>();
            a == "#[test]" || a == "#[cfg(test)]" || a.contains("::test")
        }) {
            return true;
        }
        cur = n.parent();
    }
    false
}

/// The item's `///` (or `/** */`) docs, without the comment marks; docair blocks (`/***`) are not docs.
fn docs_of(item: Node, src: &[u8]) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut sib = item.prev_named_sibling();
    while let Some(s) = sib {
        let text = s.utf8_text(src).unwrap_or("");
        match s.kind() {
            "attribute_item" => {}
            "line_comment" if text.starts_with("///") && !text.starts_with("////") => {
                let t = text.trim_end_matches(['\n', '\r']);
                let t = t[3..].strip_prefix(' ').unwrap_or(&t[3..]);
                lines.push(t.to_string());
            }
            "block_comment"
                if text.starts_with("/**") && !text.starts_with("/***") && text != "/**/" =>
            {
                let body = text[3..].trim_end_matches("*/");
                for l in body.lines().rev() {
                    let l = l.trim();
                    let l = l.strip_prefix('*').unwrap_or(l);
                    lines.push(l.strip_prefix(' ').unwrap_or(l).to_string());
                }
            }
            "block_comment" if text.starts_with("/***") => {}
            _ => break,
        }
        sib = s.prev_named_sibling();
    }
    lines.reverse();
    let docs = lines.join("\n").trim().to_string();
    (!docs.is_empty()).then_some(docs)
}

/***
 * Index the items of a syntax tree by the row they start on, outermost first.
 ***/
pub fn items_by_row(root: Node) -> HashMap<usize, Node> {
    const ITEMS: &[&str] = &[
        "function_item",
        "function_signature_item",
        "struct_item",
        "enum_item",
        "union_item",
        "trait_item",
        "impl_item",
        "mod_item",
        "const_item",
        "static_item",
        "type_item",
        "macro_definition",
    ];
    let mut out = HashMap::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if ITEMS.contains(&n.kind()) {
            out.entry(n.start_position().row).or_insert(n);
        }
        let mut cursor = n.walk();
        let children: Vec<Node> = n.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    out
}

struct Item {
    keyword: &'static str,
    name: String,
    symbol: String,
    kind: &'static str,
}

/// Parameter names and the simplified return type of a function item.
fn signature_of(item: Node, src: &[u8]) -> Option<Signature> {
    if !matches!(item.kind(), "function_item" | "function_signature_item") {
        return None;
    }
    let params = item
        .child_by_field_name("parameters")
        .map_or_else(Vec::new, |ps| {
            let mut cursor = ps.walk();
            ps.named_children(&mut cursor)
                .filter(|p| p.kind() == "parameter")
                .map(|p| {
                    text_of(p.child_by_field_name("pattern"), src)
                        .trim_start_matches("mut ")
                        .to_string()
                })
                .filter(|n| !n.is_empty())
                .collect()
        });
    let returns = item
        .child_by_field_name("return_type")
        .and_then(|r| simplify_type(&text_of(Some(r), src)));
    Some(Signature { params, returns })
}

/// Keyword, name, symbol and docair kind of an item.
fn describe(item: Node, src: &[u8]) -> Option<Item> {
    let named = |keyword: &'static str, kind: &'static str| {
        let name = text_of(item.child_by_field_name("name"), src);
        Item {
            keyword,
            symbol: format!("{keyword} {name}"),
            name,
            kind,
        }
    };
    Some(match item.kind() {
        "function_item" | "function_signature_item" => named("fn", "function"),
        "struct_item" => named("struct", "component"),
        "enum_item" => named("enum", "component"),
        "union_item" => named("union", "component"),
        "trait_item" => named("trait", "component"),
        "type_item" => named("type", "component"),
        "mod_item" => named("mod", "module"),
        "const_item" => named("const", "config"),
        "static_item" => named("static", "config"),
        "macro_definition" => named("macro_rules!", "function"),
        "impl_item" => {
            let ty = base_type(&text_of(item.child_by_field_name("type"), src));
            let (symbol, name) = match item.child_by_field_name("trait") {
                Some(t) => {
                    let tr = base_type(&text_of(Some(t), src));
                    (format!("impl {tr} for {ty}"), format!("{ty} as {tr}"))
                }
                None => (format!("impl {ty}"), format!("{ty} impl")),
            };
            Item {
                keyword: "impl",
                name,
                symbol,
                kind: "component",
            }
        }
        _ => return None,
    })
}

pub struct CrateInfo {
    pub name: String,
    pub root: PathBuf,
}

/***
 * Find the crate a file belongs to: the nearest `Cargo.toml` with a `[package]` name.
 *
 * @reads project-files.Cargo.toml "reads Cargo.toml files to locate crates"
 ***/
pub fn crate_of(path: &Path) -> Option<CrateInfo> {
    let mut dir = path.parent();
    while let Some(d) = dir {
        let manifest = d.join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&manifest)
            && let Ok(value) = text.parse::<toml::Table>()
            && let Some(name) = value
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
        {
            return Some(CrateInfo {
                name: name.to_string(),
                root: d.to_path_buf(),
            });
        }
        dir = d.parent();
    }
    None
}

/// Module segments of a file: `src/a/b.rs` → `[a, b]`; `lib.rs`, `main.rs`, `mod.rs` add none.
fn file_modules(rel: &str, path: &Path, krate: Option<&CrateInfo>) -> Vec<String> {
    let in_crate: Vec<String> = match krate.and_then(|k| path.strip_prefix(&k.root).ok()) {
        Some(p) => p.iter().map(|c| c.to_string_lossy().into_owned()).collect(),
        None => {
            let parts: Vec<&str> = rel.split('/').collect();
            let start = parts.iter().rposition(|p| *p == "src").unwrap_or(0);
            parts[start..].iter().map(|s| s.to_string()).collect()
        }
    };
    let mut mods: Vec<String> = in_crate
        .iter()
        .skip_while(|c| *c == "src")
        .map(|p| p.trim_end_matches(".rs").to_string())
        .collect();
    if mods
        .last()
        .is_some_and(|m| m == "lib" || m == "main" || m == "mod")
    {
        mods.pop();
    }
    mods
}

/// The file a `mod name;` declaration loads.
fn module_file(rel: &str, path: &Path, name: &str) -> Option<String> {
    let dir_rel = rel.rsplit_once('/').map_or("", |(d, _)| d);
    let stem = file_stem(rel);
    let (sub_rel, sub_dir) = if ["lib", "main", "mod"].contains(&stem.as_str()) {
        (dir_rel.to_string(), path.parent()?.to_path_buf())
    } else {
        (format!("{dir_rel}/{stem}"), path.parent()?.join(&stem))
    };
    let join = |a: &str, b: &str| {
        if a.is_empty() {
            b.to_string()
        } else {
            format!("{a}/{b}")
        }
    };
    if sub_dir.join(format!("{name}.rs")).exists() {
        Some(join(&sub_rel, &format!("{name}.rs")))
    } else if sub_dir.join(name).join("mod.rs").exists() {
        Some(join(&sub_rel, &format!("{name}/mod.rs")))
    } else {
        None
    }
}

fn text_of(node: Option<Node>, src: &[u8]) -> String {
    node.and_then(|n| n.utf8_text(src).ok())
        .unwrap_or("")
        .to_string()
}

/// `Vec<T>` → `Vec`, `crate::a::B` → `B`.
fn base_type(t: &str) -> String {
    let t = t.split('<').next().unwrap_or(t);
    t.rsplit("::").next().unwrap_or(t).trim().to_string()
}

fn file_stem(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.strip_suffix(".rs").unwrap_or(name).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use docair_core::extract::extract_text;
    use docair_core::facts::Meet;
    use docair_core::lang::{Family, language_for};
    use docair_core::model::{Origin, RawBlock, Tag};

    fn rules_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("rules")
    }

    /// Every rule in `rules/` finds exactly what its fixture expects (docs/ADAPTERS.md §9.1).
    #[test]
    fn every_rule_passes_its_fixture() {
        let failures = docair_rules::check_fixtures(&rules_dir(), "rs", "//>", |yaml, src| {
            let set = RuleSet::load(grammar(), &[("rule.yaml", yaml)])?;
            let file = SourceFile {
                path: PathBuf::from("/nonexistent/fixture.rs"),
                rel: "fixture.rs".into(),
                lang: language_for("rs").unwrap(),
                text: src.to_string(),
            };
            let facts = facts_with(&file, &set);
            let shown = |key: &str| {
                facts
                    .symbols
                    .iter()
                    .find(|s| s.key == key)
                    .and_then(|s| s.display.clone())
                    .unwrap_or_default()
            };
            let serves = facts.serves.iter().map(|s| match &s.at {
                Meet::Http(_) => format!("serves http {} by {}", s.at, shown(&s.by)),
                Meet::Topic(_) => format!("serves {} by {}", s.at, shown(&s.by)),
            });
            let relations = facts.relations.iter().map(|r| {
                let to = match &r.to {
                    Target::Symbol(k) => shown(k),
                    Target::Ref(id) => id.clone(),
                };
                let payload = r
                    .payload
                    .as_deref()
                    .map(|p| format!(" ({p})"))
                    .unwrap_or_default();
                format!("{} {to} from {}{payload}", r.verb.tag(), shown(&r.from))
            });
            Ok(serves.chain(relations).collect())
        });
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// A rule file in `rules/` that the adapter doesn't load would be tested but never run.
    #[test]
    fn every_rule_file_is_built_in() {
        let mut on_disk: Vec<String> = std::fs::read_dir(rules_dir())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".yaml"))
            .collect();
        on_disk.sort();
        let mut built_in: Vec<String> = RULES.iter().map(|(n, _)| n.to_string()).collect();
        built_in.sort();
        assert_eq!(on_disk, built_in);
        assert_eq!(rules().ids().count(), RULES.len());
    }

    /// Every item is drawn, except impl blocks, unit tests and items inside functions (docs/ADAPTERS.md §8).
    #[test]
    fn every_item_is_drawn_except_impls_tests_and_nested_ones() {
        let src = "/// Orders.\n///\n/// More about them.\n#[derive(Debug)]\npub struct Order;\n\nimpl Order {\n    /// Save it.\n    pub fn save(&self) {\n        fn inner() {}\n    }\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn saves() {}\n}\n\n/+++ Documented by a block. +++/\nfn helper() {}\n";
        let blocks = enrich("src/orders.rs", src);
        let drawn: Vec<_> = blocks
            .iter()
            .map(|b| {
                (
                    b.hints.name.clone().unwrap_or_default(),
                    b.origin,
                    b.summary.as_str(),
                    b.description.as_str(),
                )
            })
            .collect();
        assert_eq!(
            drawn,
            [
                (
                    "helper".into(),
                    Origin::Authored,
                    "Documented by a block.",
                    ""
                ),
                (
                    "Order".into(),
                    Origin::Discovered,
                    "Orders.",
                    "More about them."
                ),
                ("save".into(), Origin::Discovered, "Save it.", ""),
            ]
        );
    }

    fn enrich(rel: &str, src: &str) -> Vec<RawBlock> {
        let text = src.replace("+++", "***");
        let mut d = Vec::new();
        let blocks = extract_text(rel, &text, Family::CStyle, &mut d);
        let file = SourceFile {
            path: PathBuf::from("/nonexistent").join(rel),
            rel: rel.to_string(),
            lang: language_for("rs").unwrap(),
            text,
        };
        let facts = facts_with(&file, rules());
        let blocks = docair_core::merge::merge(&[file], vec![blocks], &[facts], &mut d);
        assert!(d.is_empty(), "{d:?}");
        blocks.into_iter().flatten().collect()
    }

    fn inferred<'a>(b: &'a RawBlock, name: &str) -> Option<&'a Tag> {
        b.tags
            .iter()
            .find(|t| t.name == name && t.origin == Origin::Inferred)
    }

    #[test]
    fn functions_methods_routes_and_modules() {
        let src = "/+++ Orders. +++/\npub mod orders;\n\n/+++ A repo. +++/\npub struct OrdersRepo;\n\nimpl OrdersRepo {\n    /+++ Save one. +++/\n    #[post(\"/api/orders\")]\n    pub async fn save(&self) {}\n}\n\nmod inner {\n    /+++ Helper. +++/\n    fn helper() {}\n}\n";
        let b = enrich("src/api/handlers.rs", src);

        assert_eq!(b[0].hints.symbol.as_deref(), Some("mod orders"));
        assert_eq!(inferred(&b[0], "kind").unwrap().value, "module");
        assert_eq!(b[0].hints.id.as_deref(), Some("api.handlers.orders"));

        assert_eq!(inferred(&b[1], "kind").unwrap().value, "component");
        assert_eq!(b[1].hints.id.as_deref(), Some("api.handlers.orders-repo"));

        let save = &b[2];
        assert_eq!(save.hints.symbol.as_deref(), Some("fn save"));
        assert_eq!(
            save.hints.id.as_deref(),
            Some("api.handlers.orders-repo.save")
        );
        assert_eq!(save.hints.part_of[0], "api.handlers.orders-repo");
        assert_eq!(save.hints.parent_line, Some(7));
        let http = inferred(save, "http").unwrap();
        assert_eq!(http.value, "POST /api/orders");
        assert_eq!(http.reason.as_deref(), Some("#[post(\"/api/orders\")]"));

        assert_eq!(b[3].hints.id.as_deref(), Some("api.handlers.inner.helper"));
        assert_eq!(b[3].hints.part_of[0], "api.handlers.inner");
    }
}

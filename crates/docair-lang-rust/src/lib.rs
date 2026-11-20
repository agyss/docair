//! Rust adapter (SPEC §6, docs/ADAPTERS.md): tree-sitter finds every item, the rules what frameworks mean.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use docair_core::adapter::{Adapter, Project};
use docair_core::config::CandidateLimit;
use docair_core::facts::{
    Draw, Evidence, Facts, Inferred, Meet, Reach, Relation, Request, Serve, Symbol, Target, Verb,
};
use docair_core::ids::slug;
use docair_core::model::{Diagnostic, Signature, SourceFile};
use docair_core::signature::simplify_type;
use docair_core::type_uses::{TypeUse as How, TypeUses};
use docair_rules::{About, Fact, Found, Grammar, RuleSet};
use docair_scip::Index;
use tree_sitter::{Node, Parser};

/// The built-in rules (docs/ADAPTERS.md §5.2). Each needs a fixture beside it in `rules/`.
const RULES: &[(&str, &str)] = &[
    (
        "actix.route.yaml",
        include_str!("../rules/actix.route.yaml"),
    ),
    ("axum.route.yaml", include_str!("../rules/axum.route.yaml")),
    ("std.env.yaml", include_str!("../rules/std.env.yaml")),
    (
        "std.fs-read.yaml",
        include_str!("../rules/std.fs-read.yaml"),
    ),
    (
        "std.fs-write.yaml",
        include_str!("../rules/std.fs-write.yaml"),
    ),
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
     * @calls docair-lang-rust.link_with "links handler names to their functions, and finds the calls"
     ***/
    fn project(&self, project: &Project) -> Facts {
        let read: Vec<FileRead> = project
            .files
            .iter()
            .filter(|f| self.handles(f))
            .map(|f| read_file(f, rules()))
            .collect();
        let mut diags = Vec::new();
        let index = index_of(project, &mut diags);
        let guesses = CandidateLimit::of(project.config.code.as_ref());
        let mut facts = link_with(read, index.as_ref(), guesses);
        facts.diagnostics.extend(diags);
        facts
    }
}

/***
 * The code index of the project's Rust files: the files `[code] scip` names, else, with `[code] run_indexers`, what `rust-analyzer scip` writes.
 *
 * No index when docair.toml asks for none or rust-analyzer isn't installed;
 * W113 for an index file that can't be read, W119 when rust-analyzer fails.
 *
 * @calls docair-scip.run_rust_analyzer "indexes the Cargo project"
 * @calls docair-scip.index.read "reads the index"
 ***/
fn index_of(project: &Project, diags: &mut Vec<Diagnostic>) -> Option<Index> {
    let code = project.config.code.as_ref()?;
    let first = project.files.iter().find(|f| f.lang.name == "rust")?;
    let root = first.path.ancestors().nth(first.rel.split('/').count())?;
    let mut index: Option<Index> = None;
    for rel in &code.scip {
        let Some(path) = docair_core::config::inside(root, rel).filter(|p| p.is_file()) else {
            diags.push(
                Diagnostic::new(
                    "W113",
                    format!("the code index {rel} is no file inside the project"),
                )
                .at("docair.toml", 1),
            );
            continue;
        };
        match Index::read(&path) {
            Ok(i) => match index.as_mut() {
                Some(all) => all.extend(i),
                None => index = Some(i),
            },
            Err(e) => diags.push(
                Diagnostic::new("W113", format!("cannot read the code index: {e}")).at(rel, 1),
            ),
        }
    }
    if index.is_none()
        && code.run_indexers
        && root.join("Cargo.toml").exists()
        && docair_scip::has_rust_analyzer()
    {
        // The index of the same sources is kept: rust-analyzer runs only when the code changed.
        let sources: Vec<(&str, &str)> = project
            .files
            .iter()
            .filter(|f| f.rel.ends_with(".rs") || f.rel.ends_with("Cargo.toml"))
            .map(|f| (f.rel.as_str(), f.text.as_str()))
            .collect();
        match docair_scip::kept_rust_index(root, &sources, &docair_scip::cache_dir()) {
            Ok(path) => index = Index::read(&path).ok(),
            Err(e) => diags.push(
                Diagnostic::new("W119", format!("no code index for the calls: {e}"))
                    .at("Cargo.toml", 1),
            ),
        }
    }
    index
}

/// A function or other item the rules may name, with where its code is.
struct Site {
    key: String,
    id: String,
    name: String,
    /// The type of an impl or trait the item is written in.
    owner: Option<String>,
    is_fn: bool,
    /// A struct, enum, union, trait or type alias.
    is_type: bool,
    /// A function's return type as written (`Self`, `Result<Index, String>`); empty without one.
    returns: String,
    start: usize,
    end: usize,
}

/// What a call names, as the syntax writes it.
#[derive(Clone)]
enum Callee {
    /// `load(…)`
    Name(String),
    /// `config::load(…)`, `Self::new(…)`
    Path(Vec<String>),
    /// `self.save(…)`
    SelfMethod(String),
    /// `x.save(…)` on a value whose type the syntax does not say.
    Method(String),
}

/// One Rust file, read: its facts so far, its items, what the rules found, and the calls it writes.
pub struct FileRead {
    rel: String,
    facts: Facts,
    krate: Option<String>,
    items: Vec<Site>,
    found: Vec<Found>,
    calls: Vec<(usize, Callee, String)>,
    flows: Vec<Flow>,
    /// axum `.nest("/prefix", …)` in this file (docair-4wk.9).
    nests: Vec<Nest>,
    /// The names of types, variants and fields, and how the code uses each (docs/ADAPTERS.md §5.8).
    mentions: Vec<TypeMention>,
}

/// A name of a type, a variant or a field in the code, and how the code around it uses it.
#[derive(Debug, Clone)]
struct TypeMention {
    line: usize,
    col: usize,
    /// The name as written: `Cli`, `Build`, `command`.
    text: String,
    /// The type the syntax takes it for: `Command` of `Command::Build`; empty for a field.
    ty: String,
    how: How,
    /// `Type::f(…)`: the function called, where its name is (line, column), and its name.
    via: Option<(usize, usize, String)>,
    /// `impl Trait for Type`: where the type's name is, and the name.
    implementor: Option<(usize, usize, String)>,
}

/// An axum router nested under a prefix: `.nest("/api", api())` or `.nest("/admin", admin)`.
#[derive(Debug, Clone)]
struct Nest {
    /// The line of the `nest` call.
    line: usize,
    prefix: String,
    /// What is nested: a function that builds the router, or a router in a variable of the same function.
    nested: NestTarget,
}

/// What a `nest` puts under its prefix.
#[derive(Debug, Clone)]
enum NestTarget {
    /// A function's name: every route it builds gets the prefix.
    Built(String),
    /// The lines of the `let` that builds the router in a variable: the routes on them get the prefix.
    Lines(usize, usize),
}

/// A call as written: where its name is (line, byte column), what it names, and its code.
#[derive(Clone)]
struct CallAt {
    line: usize,
    col: usize,
    name: String,
    callee: Option<Callee>,
    text: String,
}

/// A value one call makes and a later call in the same function takes: `let blocks = extract(…); collect(&blocks)`.
struct Flow {
    /// The variable, or none when the call is written inside the other (`g(f(x))`).
    var: Option<String>,
    /// `let x: T = …`
    ty: Option<String>,
    /// The maker changed the value (`f(&mut x)`): what it returns is not the value.
    changed: bool,
    from: CallAt,
    to: CallAt,
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
        rel: file.rel.clone(),
        facts: Facts::default(),
        krate: None,
        items: Vec::new(),
        found: Vec::new(),
        calls: Vec::new(),
        flows: Vec::new(),
        nests: Vec::new(),
        mentions: Vec::new(),
    };
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("tree-sitter-rust matches the tree-sitter ABI");
    let Some(tree) = parser.parse(&file.text, None) else {
        read.facts.diagnostics.push(
            Diagnostic::new("W113", "tree-sitter could not parse this file").at(&file.rel, 1),
        );
        return read;
    };
    let src = file.text.as_bytes();
    let mut items: Vec<(usize, Node)> = items_by_row(tree.root_node()).into_iter().collect();
    items.sort_by_key(|(row, _)| *row);
    // Items inside an item that starts on the same line (`impl A { fn f() {} }`), after it (docair-4wk.16).
    let firsts: Vec<Node> = items.iter().map(|(_, n)| *n).collect();
    let mut inner = items_within_rows(&firsts);
    items.append(&mut inner);
    items.sort_by_key(|(row, n)| (*row, n.start_byte()));
    let krate = crate_of(&file.path);
    let file_mods = file_modules(&file.rel, &file.path, krate.as_ref());
    let test_file = is_test_file(&file.rel);
    read.krate = krate.as_ref().map(|k| k.name.clone());

    for (row, item) in items {
        let Some(it) = describe(item, src) else {
            continue;
        };
        let line = row + 1;
        // The first item on a line has the line's key; one inside it on that line, its column too.
        let key = if firsts.iter().any(|f| f.id() == item.id()) {
            format!("rust:{}:{line}", file.rel)
        } else {
            format!(
                "rust:{}:{line}:{}",
                file.rel,
                item.start_position().column + 1
            )
        };

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
        // A `mod x;` without docs of its own: the module file's `//!` docs (docair-c6y).
        let module_docs = docs_of(item, src).or_else(|| {
            let module = declares_file.as_deref()?;
            let root = file.path.ancestors().nth(file.rel.split('/').count())?;
            inner_docs(&std::fs::read_to_string(root.join(module)).ok()?)
        });

        read.items.push(Site {
            key: key.clone(),
            id: id.clone(),
            name: it.name.clone(),
            owner: owner.clone(),
            is_fn: it.keyword == "fn",
            is_type: matches!(it.keyword, "struct" | "enum" | "union" | "trait" | "type"),
            returns: text_of(item.child_by_field_name("return_type"), src),
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
            // A `mod x;` without docs of its own: the module file's `//!` docs (docair-c6y).
            docs: module_docs,
            end: Some(item.end_position().row + 1),
            ..Symbol::at(key, &file.rel, line)
        });
    }
    read.found = rules.apply(&file.text);
    read.found.extend(env_through(tree.root_node(), src));
    read.calls = syntax_calls(tree.root_node(), src);
    read.nests = nests_of(tree.root_node(), src);
    read.flows = data_flows(tree.root_node(), src);
    read.mentions = type_mentions(tree.root_node(), src);
    read
}

/// Whether a name is a type's: it starts with a capital letter.
fn capital(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

/// The type's name in a type written out: `Vec` of `Vec<T>`, `Graph` of `model::Graph`.
fn type_name_node(n: Node) -> Node {
    match n.kind() {
        "generic_type" => n.child_by_field_name("type").map_or(n, type_name_node),
        "scoped_type_identifier" => n.child_by_field_name("name").unwrap_or(n),
        "reference_type" => n.child_by_field_name("type").map_or(n, type_name_node),
        _ => n,
    }
}

/***
 * Every name of a type, a variant or a field in a file, and how the code around it uses it (docs/ADAPTERS.md §5.8).
 *
 * The nearest telling place around the name decides: a pattern matches, a
 * struct expression or `Type::f(…)` makes, a parameter takes, the return
 * type returns, a field's type holds, `impl Trait for T` implements, a field
 * after a dot reads or, left of `=`, sets. `use` lines, attributes, comments
 * and the type's own definition name it without using it.
 ***/
fn type_mentions(root: Node, src: &[u8]) -> Vec<TypeMention> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        let mut cursor = n.walk();
        let children: Vec<Node> = n.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
        let text = text_of(Some(n), src);
        let field = match n.kind() {
            "field_identifier" | "shorthand_field_identifier" => true,
            "identifier"
                if n.parent()
                    .is_some_and(|p| p.kind() == "shorthand_field_initializer") =>
            {
                true
            }
            "type_identifier" | "identifier" if capital(&text) => false,
            _ => continue,
        };
        let Some(how) = how_of(n, field, src) else {
            continue;
        };
        // `Command::Build`: the variant's type is the path before it.
        let ty = if field {
            String::new()
        } else {
            n.parent()
                .filter(|p| {
                    matches!(p.kind(), "scoped_identifier" | "scoped_type_identifier")
                        && p.child_by_field_name("name")
                            .is_some_and(|c| c.id() == n.id())
                })
                .and_then(|p| p.child_by_field_name("path"))
                .map(|path| {
                    let last = match path.kind() {
                        "scoped_identifier" => path.child_by_field_name("name").unwrap_or(path),
                        _ => path,
                    };
                    text_of(Some(last), src)
                })
                .filter(|t| capital(t))
                .unwrap_or_else(|| text.clone())
        };
        // `Type::f(…)`: f decides whether it makes the type.
        let via = (how == How::Makes && !field)
            .then(|| {
                let p = n.parent()?;
                (p.kind() == "scoped_identifier"
                    && p.child_by_field_name("path")
                        .is_some_and(|c| c.id() == n.id()))
                .then_some(())?;
                let call = p.parent().filter(|c| c.kind() == "call_expression")?;
                call.child_by_field_name("function")
                    .is_some_and(|f| f.id() == p.id())
                    .then_some(())?;
                let f = p.child_by_field_name("name")?;
                (!capital(&text_of(Some(f), src))).then(|| {
                    (
                        f.start_position().row + 1,
                        f.start_position().column,
                        text_of(Some(f), src),
                    )
                })
            })
            .flatten();
        let implementor = (how == How::Implements)
            .then(|| {
                let mut p = n.parent();
                while let Some(q) = p {
                    if q.kind() == "impl_item" {
                        let t = type_name_node(q.child_by_field_name("type")?);
                        return Some((
                            t.start_position().row + 1,
                            t.start_position().column,
                            text_of(Some(t), src),
                        ));
                    }
                    p = q.parent();
                }
                None
            })
            .flatten();
        out.push(TypeMention {
            line: n.start_position().row + 1,
            col: n.start_position().column,
            text,
            ty,
            how,
            via,
            implementor,
        });
    }
    out
}

/// How the code around a name uses it; none where it only names it (a `use`, the definition).
fn how_of(n: Node, field: bool, src: &[u8]) -> Option<How> {
    let mut child = n;
    let mut cur = n.parent();
    while let Some(p) = cur {
        let is = |name: &str| {
            p.child_by_field_name(name)
                .is_some_and(|c| c.id() == child.id())
        };
        // A path used as a value: `Reach::Certain`, `CandidateLimit::DEFAULT`.
        if child.kind() == "scoped_identifier"
            && !matches!(
                p.kind(),
                "scoped_identifier" | "use_declaration" | "scoped_use_list" | "use_as_clause"
            )
            && !(p.kind() == "call_expression" && is("function"))
            && !p.kind().ends_with("pattern")
            && !(p.kind() == "binary_expression" && is_comparison(p, src))
        {
            return Some(How::Makes);
        }
        match p.kind() {
            "use_declaration"
            | "attribute_item"
            | "inner_attribute_item"
            | "macro_definition"
            | "mod_item" => return None,
            "token_tree" => return Some(How::Uses),
            k if k.ends_with("pattern") => return Some(How::Matches),
            "binary_expression" if is_comparison(p, src) => return Some(How::Matches),
            "struct_expression" | "field_initializer" | "shorthand_field_initializer" => {
                return Some(How::Makes);
            }
            "call_expression" if is("function") => return Some(How::Makes),
            "generic_function" if is("type_arguments") => return Some(How::Uses),
            "field_expression" if field && is("field") => {
                let left = p.parent().is_some_and(|q| {
                    matches!(
                        q.kind(),
                        "assignment_expression" | "compound_assignment_expr"
                    ) && q
                        .child_by_field_name("left")
                        .is_some_and(|l| l.id() == p.id())
                });
                return Some(if left { How::Sets } else { How::Reads });
            }
            "parameter" => {
                let closure = p.parent().is_some_and(|q| q.kind() == "closure_parameters");
                return Some(if closure { How::Uses } else { How::Takes });
            }
            "self_parameter" => return None,
            "function_item" | "function_signature_item" => {
                if is("return_type") {
                    return Some(How::Returns);
                }
                if is("type_parameters") || child.kind() == "where_clause" {
                    return Some(How::Takes);
                }
                return if is("name") { None } else { Some(How::Uses) };
            }
            "let_declaration" if is("type") => return Some(How::Uses),
            "field_declaration" => return if is("name") { None } else { Some(How::Holds) },
            "ordered_field_declaration_list" => return Some(How::Holds),
            "enum_variant" if is("name") => return None,
            "type_item" | "struct_item" | "enum_item" | "union_item" | "trait_item" => {
                return if is("name") { None } else { Some(How::Holds) };
            }
            "const_item" | "static_item" => {
                return match () {
                    _ if is("type") => Some(How::Holds),
                    _ if is("value") => Some(How::Makes),
                    _ => None,
                };
            }
            "impl_item" => {
                return match () {
                    _ if is("trait") => {
                        // The trait itself; a type in its arguments (`From<X>`) is only used.
                        let t = type_name_node(p.child_by_field_name("trait")?);
                        Some(if t.id() == n.id() {
                            How::Implements
                        } else {
                            How::Uses
                        })
                    }
                    _ if is("type") => None,
                    _ => Some(How::Uses),
                };
            }
            "source_file" | "declaration_list" | "block" => return Some(How::Uses),
            _ => {}
        }
        child = p;
        cur = p.parent();
    }
    Some(How::Uses)
}

/// `a == b`, `a != b`.
fn is_comparison(binary: Node, src: &[u8]) -> bool {
    binary
        .child_by_field_name("operator")
        .is_some_and(|o| matches!(text_of(Some(o), src).as_str(), "==" | "!="))
}

/// HTTP client crates and the methods that send a request.
const HTTP_CLIENTS: &[&str] = &["ureq", "reqwest", "isahc", "surf", "attohttpc"];
const HTTP_METHODS: &[&str] = &["get", "post", "put", "patch", "delete", "head", "request"];

/// The method of a request an HTTP client crate sends, from a code-index symbol: `rust-analyzer cargo ureq 3.1.2 impl#[Agent]post().` → POST.
fn http_request(symbol: &str) -> Option<String> {
    let package = symbol.split(' ').nth(2)?;
    if !HTTP_CLIENTS.contains(&package) {
        return None;
    }
    let name = docair_scip::short(symbol);
    let method = name.rsplit("::").next()?.to_string();
    HTTP_METHODS
        .contains(&method.as_str())
        .then(|| method.to_uppercase())
}

/// The functions that read an environment variable.
const ENV_READS: &[&str] = &[
    "std::env::var",
    "env::var",
    "std::env::var_os",
    "env::var_os",
];

/// The name a call uses, its code, and whether it is a method call: `num` of `num(…)`, `get` of `env.get(…)`, `var` of `std::env::var(…)`.
fn callee_name(call: Node, src: &[u8]) -> Option<(String, String, bool)> {
    let mut f = call.child_by_field_name("function")?;
    if f.kind() == "generic_function" {
        f = f.child_by_field_name("function")?;
    }
    let full = text_of(Some(f), src);
    let name = match f.kind() {
        "identifier" => full.clone(),
        "field_expression" => text_of(f.child_by_field_name("field"), src),
        "scoped_identifier" => text_of(f.child_by_field_name("name"), src),
        _ => return None,
    };
    Some((name, full, f.kind() == "field_expression"))
}

/// A closure or function that hands a parameter on to `env::var`: its name, the parameter's place, and whether callers write `x.name(…)` (a method).
type Reader = (String, usize, bool);

/// The closure or function a variable is a parameter of, and its place: `|key| …` bound by `let num` → (num, 0, false).
fn param_of(at: Node, name: &str, src: &[u8]) -> Option<Reader> {
    let mut up = at.parent();
    while let Some(n) = up {
        match n.kind() {
            "closure_expression" => {
                if let Some(i) = closure_params(n, src).iter().position(|p| p == name) {
                    let owner = n
                        .parent()
                        .filter(|l| l.kind() == "let_declaration")
                        .and_then(|l| l.child_by_field_name("pattern"))
                        .filter(|p| p.kind() == "identifier")
                        .map(|p| text_of(Some(p), src))?;
                    return Some((owner, i, false));
                }
            }
            "function_item" => {
                let params = n.child_by_field_name("parameters")?;
                let mut cursor = params.walk();
                let all: Vec<Node> = params.named_children(&mut cursor).collect();
                let method = all.iter().any(|p| p.kind() == "self_parameter");
                let names: Vec<String> = all
                    .iter()
                    .filter(|p| p.kind() == "parameter")
                    .map(|p| text_of(p.child_by_field_name("pattern"), src))
                    .collect();
                let i = names.iter().position(|p| p == name)?;
                return Some((text_of(n.child_by_field_name("name"), src), i, method));
            }
            _ => {}
        }
        up = n.parent();
    }
    None
}

/***
 * Reads of environment variables whose name a caller passes in: `let num = |key| std::env::var(key); num("DOCAIR_MAX_FILES")`.
 *
 * A closure or a function that hands a parameter to `env::var` reads the
 * variable each call names, as a string or a `const` of this file; a
 * parameter it hands on again makes that closure or function a reader too.
 * Within one file, by name. `std::env::var("X")` itself is the rule's
 * (`rules/std.env.yaml`).
 ***/
fn env_through(root: Node, src: &[u8]) -> Vec<Found> {
    let mut calls = Vec::new();
    let mut consts: HashMap<String, String> = HashMap::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "call_expression" => calls.push(n),
            "const_item" | "static_item" => {
                if let Some(v) = n
                    .child_by_field_name("value")
                    .filter(|v| v.kind() == "string_literal")
                {
                    consts.insert(
                        text_of(n.child_by_field_name("name"), src),
                        unquoted(&text_of(Some(v), src)),
                    );
                }
            }
            _ => {}
        }
        let mut cursor = n.walk();
        stack.extend(n.named_children(&mut cursor));
    }
    // Every closure and function that reads what it gets.
    let mut readers: Vec<Reader> = Vec::new();
    let reads = |c: Node, readers: &[Reader]| -> Option<usize> {
        let (name, full, method) = callee_name(c, src)?;
        if ENV_READS.contains(&full.as_str()) {
            return Some(0);
        }
        readers
            .iter()
            .find(|(n, _, m)| *n == name && *m == method)
            .map(|(_, i, _)| *i)
    };
    loop {
        let before = readers.len();
        for &c in &calls {
            let Some(a) = reads(c, &readers)
                .and_then(|i| nth_arg(c, i))
                .filter(|a| a.kind() == "identifier")
            else {
                continue;
            };
            let name = text_of(Some(a), src);
            if consts.contains_key(&name) {
                continue;
            }
            if let Some(r) = param_of(c, &name, src)
                && !readers.contains(&r)
            {
                readers.push(r);
            }
        }
        if readers.len() == before {
            break;
        }
    }
    let mut out = Vec::new();
    for &c in &calls {
        let Some(a) = reads(c, &readers).and_then(|i| nth_arg(c, i)) else {
            continue;
        };
        let direct = callee_name(c, src).is_some_and(|(_, f, _)| ENV_READS.contains(&f.as_str()));
        let names: Vec<String> = match a.kind() {
            // `std::env::var("X")` is the rule's.
            "string_literal" if !direct => vec![unquoted(&text_of(Some(a), src))],
            "identifier" => {
                let name = text_of(Some(a), src);
                match consts.get(&name) {
                    Some(v) => vec![v.clone()],
                    // `let key = … DEFAULT_KEY …; env.get(&key)`: the constants its `let` names.
                    None => consts_bound(c, &name, &consts, src),
                }
            }
            _ => continue,
        };
        let line = c.start_position().row + 1;
        let why = text_of(Some(c), src)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        for name in names {
            out.push(Found {
                rule: "rust.env-through".into(),
                about: About::Within(line),
                fact: Fact::Requests {
                    at: Meet::Env(name),
                    reply: None,
                },
                line,
                why: why.clone(),
                requires: Vec::new(),
            });
        }
    }
    out
}

/// The `i`th argument of a call, without a leading `&`.
fn nth_arg(call: Node<'_>, i: usize) -> Option<Node<'_>> {
    let args = call.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    let a = args.named_children(&mut cursor).nth(i)?;
    Some(match a.kind() {
        "reference_expression" => a.child_by_field_name("value").unwrap_or(a),
        _ => a,
    })
}

/// The string constants the nearest `let name = …` before `at`, in the same function, names.
fn consts_bound(at: Node, name: &str, consts: &HashMap<String, String>, src: &[u8]) -> Vec<String> {
    let mut f = at.parent();
    while let Some(n) = f.filter(|n| n.kind() != "function_item") {
        f = n.parent();
    }
    let Some(f) = f else {
        return Vec::new();
    };
    let mut nearest: Option<Node> = None;
    let mut stack = vec![f];
    while let Some(n) = stack.pop() {
        if n.kind() == "let_declaration"
            && n.end_byte() <= at.start_byte()
            && n.child_by_field_name("pattern")
                .is_some_and(|p| bound(p, src) == [name])
            && nearest.is_none_or(|m| m.start_byte() < n.start_byte())
        {
            nearest = Some(n);
        }
        let mut cursor = n.walk();
        stack.extend(n.named_children(&mut cursor));
    }
    let Some(value) = nearest.and_then(|l| l.child_by_field_name("value")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![value];
    while let Some(n) = stack.pop() {
        if n.kind() == "identifier"
            && let Some(v) = consts.get(&text_of(Some(n), src))
            && !out.contains(v)
        {
            out.push(v.clone());
        }
        let mut cursor = n.walk();
        stack.extend(n.named_children(&mut cursor));
    }
    out
}

/// `"DOCAIR_BIND"` → `DOCAIR_BIND`.
fn unquoted(s: &str) -> String {
    s.trim_matches('"').to_string()
}

/// Methods that hand on the value they are called on: `extract(…)?.into_iter().collect()` is still extract's value.
const HANDS_ON: &[&str] = &[
    "unwrap",
    "expect",
    "unwrap_or",
    "unwrap_or_default",
    "unwrap_or_else",
    "ok",
    "ok_or",
    "ok_or_else",
    "map_err",
    "context",
    "with_context",
    "clone",
    "cloned",
    "to_owned",
    "to_string",
    "into",
    "into_iter",
    "iter",
    "iter_mut",
    "collect",
    "as_ref",
    "as_deref",
    "as_slice",
    "to_vec",
    "copied",
    "flatten",
    "partition",
    "filter",
    "rev",
    "take",
    "skip",
    "chain",
    "zip",
    "enumerate",
    "peekable",
    "first",
    "last",
    "get",
    "find",
    "drain",
];

/// Methods whose value is what their closure returns: `files.iter().map(|f| extract(f))` is extract's value.
const MAPS: &[&str] = &[
    "map",
    "flat_map",
    "filter_map",
    "and_then",
    "find_map",
    "map_or_else",
];

/// Where a value comes from: a call that makes it, or variables that hold it.
enum Supply<'t> {
    Call(Node<'t>),
    Var(String),
}

/// What an expression's value comes from: `extract(&files)?` → that call, `files.iter().cloned()` → `files`, `Project { blocks: &blocks, .. }` → `blocks`.
fn sources<'t>(e: Node<'t>, src: &[u8]) -> Vec<Supply<'t>> {
    match e.kind() {
        "try_expression" | "await_expression" | "parenthesized_expression" => {
            e.named_child(0).map_or_else(Vec::new, |c| sources(c, src))
        }
        "reference_expression" => e
            .child_by_field_name("value")
            .map_or_else(Vec::new, |c| sources(c, src)),
        "identifier" => vec![Supply::Var(text_of(Some(e), src))],
        "field_expression" => match e.child_by_field_name("value") {
            Some(v) if v.kind() != "self" => sources(v, src),
            _ => Vec::new(),
        },
        "tuple_expression"
        | "array_expression"
        | "struct_expression"
        | "field_initializer_list" => {
            let mut cursor = e.walk();
            let parts: Vec<Node> = e.named_children(&mut cursor).collect();
            parts.into_iter().flat_map(|c| sources(c, src)).collect()
        }
        "field_initializer" => e
            .child_by_field_name("value")
            .map_or_else(Vec::new, |c| sources(c, src)),
        "shorthand_field_initializer" => {
            e.named_child(0).map_or_else(Vec::new, |c| sources(c, src))
        }
        "call_expression" => {
            let Some(mut f) = e.child_by_field_name("function") else {
                return Vec::new();
            };
            // `.collect::<Vec<_>>()`
            if f.kind() == "generic_function" {
                f = f.child_by_field_name("function").unwrap_or(f);
            }
            if f.kind() != "field_expression" {
                return vec![Supply::Call(e)];
            }
            let method = text_of(f.child_by_field_name("field"), src);
            let on = f.child_by_field_name("value");
            if MAPS.contains(&method.as_str())
                && let Some(body) = closure_arg(e).and_then(|c| c.child_by_field_name("body"))
            {
                let made = sources(body, src);
                if !made.is_empty() {
                    return made;
                }
            }
            match on {
                Some(v)
                    if HANDS_ON.contains(&method.as_str()) || MAPS.contains(&method.as_str()) =>
                {
                    sources(v, src)
                }
                _ => vec![Supply::Call(e)],
            }
        }
        // `{ f(x) }`: a block's value is its last expression.
        "block" => {
            let mut cursor = e.walk();
            let last = e.named_children(&mut cursor).last();
            match last {
                Some(l)
                    if !l.kind().ends_with("statement") && !l.kind().ends_with("declaration") =>
                {
                    sources(l, src)
                }
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

/// The closure a call is given: `map(|f| extract(f))`.
fn closure_arg(call: Node) -> Option<Node> {
    let args = call.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    args.named_children(&mut cursor)
        .find(|a| a.kind() == "closure_expression")
}

/// The variables a closure's parameters bind: `|f|`, `|(a, b)|`, `|f: &SourceFile|`.
fn closure_params(closure: Node, src: &[u8]) -> Vec<String> {
    let Some(params) = closure.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut cursor = params.walk();
    let parts: Vec<Node> = params.named_children(&mut cursor).collect();
    parts
        .into_iter()
        .flat_map(|p| match p.kind() {
            "parameter" => p
                .child_by_field_name("pattern")
                .map_or_else(Vec::new, |x| bound(x, src)),
            _ => bound(p, src),
        })
        .collect()
}

/// Where a call's name is, and what the syntax says it names.
fn call_at(call: Node, src: &[u8]) -> Option<CallAt> {
    let mut f = call.child_by_field_name("function")?;
    if f.kind() == "generic_function" {
        f = f.child_by_field_name("function")?;
    }
    let (name_node, callee) = match f.kind() {
        "identifier" => (f, Some(Callee::Name(text_of(Some(f), src)))),
        "scoped_identifier" => (
            f.child_by_field_name("name")?,
            Some(Callee::Path(
                text_of(Some(f), src)
                    .split("::")
                    .map(|s| s.trim().to_string())
                    .collect(),
            )),
        ),
        "field_expression" => {
            let field = f.child_by_field_name("field")?;
            let on_self = f
                .child_by_field_name("value")
                .is_some_and(|v| v.kind() == "self");
            let callee = on_self.then(|| Callee::SelfMethod(text_of(Some(field), src)));
            (field, callee)
        }
        _ => return None,
    };
    Some(CallAt {
        line: name_node.start_position().row + 1,
        col: name_node.start_position().column,
        name: text_of(Some(name_node), src),
        callee,
        text: text_of(Some(f), src),
    })
}

/// The variables a pattern binds: `x`, `mut x`, `(a, b)`.
fn bound(pattern: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![pattern];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "identifier" => out.push(text_of(Some(n), src)),
            "tuple_pattern" | "mut_pattern" | "ref_pattern" | "reference_pattern" => {
                let mut cursor = n.walk();
                stack.extend(n.named_children(&mut cursor));
            }
            _ => {}
        }
    }
    out
}

/// What a call takes: the variables in its arguments and its receiver, the calls written as its arguments, and the variables it may change (`&mut x`).
fn taken<'t>(call: Node<'t>, src: &[u8]) -> (Vec<String>, Vec<Node<'t>>, Vec<String>) {
    let (mut vars, mut calls, mut changed) = (Vec::new(), Vec::new(), Vec::new());
    let mut parts = Vec::new();
    if let Some(f) = call.child_by_field_name("function")
        && f.kind() == "field_expression"
        && let Some(v) = f.child_by_field_name("value")
    {
        parts.push(v);
    }
    if let Some(args) = call.child_by_field_name("arguments") {
        let mut cursor = args.walk();
        for a in args.named_children(&mut cursor) {
            if a.kind() == "reference_expression"
                && a.child(1).is_some_and(|m| m.kind() == "mutable_specifier")
                && let Some(v) = a.child_by_field_name("value")
                && v.kind() == "identifier"
            {
                changed.push(text_of(Some(v), src));
            }
            if a.kind() != "closure_expression" {
                parts.push(a);
            }
        }
    }
    for p in parts {
        for from in sources(p, src) {
            match from {
                Supply::Var(v) => vars.push(v),
                Supply::Call(c) if c != call => calls.push(c),
                Supply::Call(_) => {}
            }
        }
    }
    (vars, calls, changed)
}

/***
 * The values each function hands from one call to another: `let blocks = extract(…); collect(&blocks)`, `g(f(x))`, `resolve(&mut nodes); check(&nodes)`.
 *
 * Inside one function, in the order the code is written: a `let` (or an
 * assignment) names the value of its call, `&mut x` makes the call the
 * value's last maker, and a later call that takes the variable takes that
 * value. Methods like `?`, `.unwrap()` or `.collect()` hand the value on.
 * Blocks, loops and branches are not told apart.
 ***/
fn data_flows(root: Node, src: &[u8]) -> Vec<Flow> {
    let mut out = Vec::new();
    let mut fns = vec![root];
    let mut bodies = Vec::new();
    while let Some(n) = fns.pop() {
        if n.kind() == "function_item"
            && let Some(b) = n.child_by_field_name("body")
        {
            bodies.push(b);
        }
        let mut cursor = n.walk();
        fns.extend(n.named_children(&mut cursor));
    }
    /// A call that made a value, and the name and type the code gave it there.
    #[derive(Clone)]
    struct Maker {
        at: CallAt,
        var: String,
        ty: Option<String>,
        changed: bool,
    }
    enum Event<'t> {
        /// Variables get the value of these sources (`let`, an assignment, `for x in xs`, a closure's parameters).
        Bind(Vec<String>, Option<String>, Vec<Supply<'t>>),
        Call(Node<'t>),
    }
    for body in bodies {
        let mut events: Vec<(usize, Event)> = Vec::new();
        let mut stack = vec![body];
        while let Some(n) = stack.pop() {
            match n.kind() {
                // A function inside is a function of its own.
                "function_item" => continue,
                "let_declaration" => {
                    if let (Some(p), Some(v)) = (
                        n.child_by_field_name("pattern"),
                        n.child_by_field_name("value"),
                    ) {
                        let ty = n.child_by_field_name("type").map(|t| text_of(Some(t), src));
                        events.push((
                            n.end_byte(),
                            Event::Bind(bound(p, src), ty, sources(v, src)),
                        ));
                    }
                }
                "assignment_expression" => {
                    if let (Some(l), Some(r)) = (
                        n.child_by_field_name("left"),
                        n.child_by_field_name("right"),
                    ) && l.kind() == "identifier"
                    {
                        let vars = vec![text_of(Some(l), src)];
                        events.push((n.end_byte(), Event::Bind(vars, None, sources(r, src))));
                    }
                }
                "for_expression" => {
                    if let (Some(p), Some(v), Some(b)) = (
                        n.child_by_field_name("pattern"),
                        n.child_by_field_name("value"),
                        n.child_by_field_name("body"),
                    ) {
                        events.push((
                            b.start_byte(),
                            Event::Bind(bound(p, src), None, sources(v, src)),
                        ));
                    }
                }
                "call_expression" => {
                    events.push((n.start_byte(), Event::Call(n)));
                    // The closure's parameters hold what the call iterates over.
                    if let Some(c) = closure_arg(n)
                        && let Some(on) = n
                            .child_by_field_name("function")
                            .filter(|f| f.kind() == "field_expression")
                            .and_then(|f| f.child_by_field_name("value"))
                    {
                        events.push((
                            c.start_byte(),
                            Event::Bind(closure_params(c, src), None, sources(on, src)),
                        ));
                    }
                }
                _ => {}
            }
            let mut cursor = n.walk();
            stack.extend(n.named_children(&mut cursor));
        }
        events.sort_by_key(|(at, _)| *at);
        // The calls that made each variable's value last.
        let mut made: HashMap<String, Vec<Maker>> = HashMap::new();
        for (_, e) in events {
            match e {
                Event::Bind(vars, ty, from) => {
                    for v in vars {
                        let mut makers = Vec::new();
                        for f in &from {
                            match f {
                                Supply::Call(c) => {
                                    makers.extend(call_at(*c, src).map(|at| Maker {
                                        at,
                                        var: v.clone(),
                                        ty: ty.clone(),
                                        changed: false,
                                    }))
                                }
                                Supply::Var(w) => {
                                    makers.extend(made.get(w).cloned().unwrap_or_default())
                                }
                            }
                        }
                        if makers.is_empty() {
                            made.remove(&v);
                        } else {
                            made.insert(v, makers);
                        }
                    }
                }
                Event::Call(c) => {
                    let Some(to) = call_at(c, src) else {
                        continue;
                    };
                    let (vars, inner, changed) = taken(c, src);
                    let mut seen = HashSet::new();
                    for v in vars {
                        for m in made.get(&v).into_iter().flatten() {
                            if seen.insert((m.at.line, m.at.col, m.var.clone())) {
                                out.push(Flow {
                                    var: Some(m.var.clone()),
                                    ty: m.ty.clone(),
                                    changed: m.changed,
                                    from: m.at.clone(),
                                    to: to.clone(),
                                });
                            }
                        }
                    }
                    for i in inner {
                        if let Some(from) = call_at(i, src) {
                            out.push(Flow {
                                var: None,
                                ty: None,
                                changed: false,
                                from,
                                to: to.clone(),
                            });
                        }
                    }
                    for v in changed {
                        made.insert(
                            v.clone(),
                            vec![Maker {
                                at: to.clone(),
                                var: v,
                                ty: None,
                                changed: true,
                            }],
                        );
                    }
                }
            }
        }
    }
    out
}

/// The calls a file writes, with their line and the code: `config::load(root)`, `self.save(…)`.
/***
 * The axum routers nested under a prefix: `.nest("/api", api())` names the function that builds the nested router; `.nest("/admin", admin)` a variable whose `let` builds it in the same function.
 ***/
fn nests_of(root: Node, src: &[u8]) -> Vec<Nest> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind() == "call_expression"
            && let Some(f) = n.child_by_field_name("function")
            && f.kind() == "field_expression"
            && text_of(f.child_by_field_name("field"), src) == "nest"
            && let Some(args) = n.child_by_field_name("arguments")
        {
            let mut c = args.walk();
            let list: Vec<Node> = args.named_children(&mut c).collect();
            if let [path, what] = list.as_slice()
                && path.kind() == "string_literal"
            {
                let prefix = text_of(Some(*path), src).trim_matches('"').to_string();
                let nested = match what.kind() {
                    "call_expression" => {
                        let name = text_of(what.child_by_field_name("function"), src);
                        Some(NestTarget::Built(
                            name.rsplit("::").next().unwrap_or(&name).to_string(),
                        ))
                    }
                    "identifier" => {
                        let name = text_of(Some(*what), src);
                        let_lines(n, &name, src).map(|(a, b)| NestTarget::Lines(a, b))
                    }
                    _ => None,
                };
                if let Some(nested) = nested {
                    out.push(Nest {
                        line: n.start_position().row + 1,
                        prefix,
                        nested,
                    });
                }
            }
        }
        let mut cursor = n.walk();
        let children: Vec<Node> = n.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    out
}

/// The lines of the `let NAME = …;` in the function around `at`, before it.
fn let_lines(at: Node, name: &str, src: &[u8]) -> Option<(usize, usize)> {
    let mut f = at.parent();
    while let Some(n) = f {
        if n.kind() == "function_item" {
            break;
        }
        f = n.parent();
    }
    let mut stack = vec![f?];
    let mut found = None;
    while let Some(n) = stack.pop() {
        if n.kind() == "let_declaration"
            && n.start_byte() < at.start_byte()
            && text_of(n.child_by_field_name("pattern"), src) == name
        {
            found = Some((n.start_position().row + 1, n.end_position().row + 1));
        }
        let mut cursor = n.walk();
        stack.extend(n.named_children(&mut cursor));
    }
    found
}

/// Route prefixes: by the key of the function that builds the routes, and by (file, first line, last line) of a nested router's `let`.
type RoutePrefixes = (HashMap<String, String>, Vec<(usize, usize, usize, String)>);

/// The prefix each route of the crate gets from the routers nested around it: by the function
/// that builds the routes, and by the lines of a `let` that builds a nested router.
fn route_prefixes(files: &[FileRead]) -> RoutePrefixes {
    // Who nests what: the nested function's key, under (the nesting function's key, prefix).
    let mut under: HashMap<String, (String, String)> = HashMap::new();
    let mut lines: Vec<(usize, usize, usize, String, String)> = Vec::new();
    for (at, file) in files.iter().enumerate() {
        for nest in &file.nests {
            let Some(outer) = file.at(&About::Within(nest.line)) else {
                continue;
            };
            match &nest.nested {
                NestTarget::Built(name) => {
                    if let Some(inner) = named(files, at, name) {
                        under
                            .entry(inner.key.clone())
                            .or_insert((outer.key.clone(), nest.prefix.clone()));
                    }
                }
                NestTarget::Lines(a, b) => {
                    lines.push((at, *a, *b, outer.key.clone(), nest.prefix.clone()))
                }
            }
        }
    }
    // A prefix is the nesting function's prefix and the nest's own: `/api` then `/v1` is `/api/v1`.
    let full = |key: &str| {
        let mut parts = Vec::new();
        let mut cur = key.to_string();
        for _ in 0..16 {
            let Some((outer, p)) = under.get(&cur) else {
                break;
            };
            parts.push(p.clone());
            cur = outer.clone();
        }
        parts
            .iter()
            .rev()
            .fold(String::new(), |acc, p| docair_core::route::join(&acc, p))
    };
    let by_fn: HashMap<String, String> = under.keys().map(|k| (k.clone(), full(k))).collect();
    let by_lines = lines
        .into_iter()
        .map(|(at, a, b, outer, p)| (at, a, b, docair_core::route::join(&full(&outer), &p)))
        .collect();
    (by_fn, by_lines)
}

fn syntax_calls(root: Node, src: &[u8]) -> Vec<(usize, Callee, String)> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind() == "call_expression"
            && let Some(mut f) = n.child_by_field_name("function")
        {
            if f.kind() == "generic_function" {
                f = f.child_by_field_name("function").unwrap_or(f);
            }
            let text = text_of(Some(f), src);
            let callee = match f.kind() {
                "identifier" => Some(Callee::Name(text.clone())),
                "scoped_identifier" => Some(Callee::Path(
                    text.split("::").map(|s| s.trim().to_string()).collect(),
                )),
                "field_expression"
                    if f.child_by_field_name("value")
                        .is_some_and(|v| v.kind() == "self") =>
                {
                    Some(Callee::SelfMethod(text_of(
                        f.child_by_field_name("field"),
                        src,
                    )))
                }
                "field_expression" => {
                    Some(Callee::Method(text_of(f.child_by_field_name("field"), src)))
                }
                _ => None,
            };
            if let Some(c) = callee {
                out.push((n.start_position().row + 1, c, text));
            }
        }
        let mut cursor = n.walk();
        let children: Vec<Node> = n.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    out
}

/// The facts of one file on its own, names linked inside it only (fixtures, tests).
pub fn facts_with(file: &SourceFile, rules: &RuleSet) -> Facts {
    link_with(vec![read_file(file, rules)], None, CandidateLimit::DEFAULT)
}

impl FileRead {
    /// The item a rule's fact belongs to, in this file.
    fn at(&self, about: &About) -> Option<&Site> {
        match about {
            About::Line(line) => self.items.iter().find(|i| i.start == *line),
            // The smallest item around the line; of items on the same lines, the innermost (listed last).
            About::Within(line) => self
                .items
                .iter()
                .rev()
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

/// The one item an iterator gives, or none when it gives no item or several.
fn only<'a>(mut it: impl Iterator<Item = &'a Site>) -> Option<&'a Site> {
    match (it.next(), it.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    }
}

/// Method names the standard library's types have too: a call by such a name is far more often
/// theirs (docair: 166 of 167 `.is_empty()` calls), so it gets no could line.
const STD_METHODS: &[&str] = &[
    "as_bytes",
    "as_deref",
    "as_mut",
    "as_path",
    "as_ref",
    "as_slice",
    "as_str",
    "and_then",
    "borrow",
    "borrow_mut",
    "bytes",
    "chars",
    "clear",
    "clone",
    "cloned",
    "cmp",
    "collect",
    "contains",
    "contains_key",
    "copied",
    "count",
    "dedup",
    "default",
    "display",
    "drain",
    "ends_with",
    "entry",
    "eq",
    "err",
    "exists",
    "expect",
    "extend",
    "extension",
    "file_name",
    "filter",
    "filter_map",
    "find",
    "first",
    "flat_map",
    "flatten",
    "flush",
    "fmt",
    "fold",
    "from",
    "get",
    "get_mut",
    "hash",
    "insert",
    "into",
    "into_iter",
    "is_dir",
    "is_empty",
    "is_err",
    "is_file",
    "is_none",
    "is_ok",
    "is_some",
    "iter",
    "iter_mut",
    "join",
    "keys",
    "last",
    "len",
    "lines",
    "lock",
    "map",
    "map_err",
    "map_or",
    "max",
    "min",
    "next",
    "ok",
    "ok_or",
    "ok_or_else",
    "or_default",
    "or_else",
    "or_insert",
    "or_insert_with",
    "parent",
    "parse",
    "partial_cmp",
    "pop",
    "position",
    "push",
    "push_str",
    "read",
    "read_to_string",
    "recv",
    "remove",
    "replace",
    "retain",
    "rev",
    "send",
    "sort",
    "sort_by",
    "sort_by_key",
    "split",
    "split_once",
    "starts_with",
    "strip_prefix",
    "strip_suffix",
    "sum",
    "take",
    "to_lowercase",
    "to_owned",
    "to_string",
    "to_uppercase",
    "to_vec",
    "trim",
    "trim_end",
    "trim_start",
    "try_into",
    "unwrap",
    "unwrap_or",
    "unwrap_or_default",
    "unwrap_or_else",
    "values",
    "windows",
    "with_capacity",
    "write",
    "write_all",
    "zip",
];

/// The methods a call `x.m(…)` could reach when the type of `x` is unknown: methods (never free
/// functions) of that name in the same crate, unless standard types have a method of that name.
fn could_methods<'a>(files: &'a [FileRead], at: usize, m: &str) -> Vec<&'a Site> {
    if STD_METHODS.contains(&m) {
        return Vec::new();
    }
    let krate = &files[at].krate;
    files
        .iter()
        .filter(|f| &f.krate == krate)
        .flat_map(|f| f.items.iter())
        .filter(|i| i.is_fn && i.owner.is_some() && i.name == m)
        .collect()
}

fn called<'a>(
    files: &'a [FileRead],
    at: usize,
    caller: &Site,
    callee: &Callee,
) -> Option<&'a Site> {
    let fns = || {
        files
            .iter()
            .flat_map(|f| f.items.iter().filter(|i| i.is_fn).map(move |i| (f, i)))
    };
    match callee {
        // `x.save(…)`: the type is unknown, so no one function; could lines come from `could_methods`.
        Callee::Method(_) => None,
        // `load(…)`: the free function of that name in the same file.
        Callee::Name(n) => only(
            files[at]
                .items
                .iter()
                .filter(|i| i.is_fn && i.owner.is_none() && &i.name == n),
        ),
        // `self.save(…)`: the method of the same type, in the crate.
        Callee::SelfMethod(m) => {
            let owner = caller.owner.as_deref()?;
            let krate = &files[at].krate;
            only(
                fns()
                    .filter(|(f, i)| {
                        &f.krate == krate && i.owner.as_deref() == Some(owner) && &i.name == m
                    })
                    .map(|(_, i)| i),
            )
        }
        // `config::load(…)`: the one function whose id ends with the path; `Self::new(…)`: the caller's type.
        Callee::Path(segs) => {
            let segs: Vec<String> = segs
                .iter()
                .filter(|s| !matches!(s.as_str(), "crate" | "self" | "super"))
                .map(|s| match (s.as_str(), &caller.owner) {
                    ("Self", Some(o)) => slug(o),
                    _ => slug(s),
                })
                .collect();
            if segs.len() < 2 {
                return None;
            }
            let tail = segs.join(".");
            let dotted = format!(".{tail}");
            only(
                fns()
                    .filter(|(_, i)| i.id == tail || i.id.ends_with(&dotted))
                    .map(|(_, i)| i),
            )
        }
    }
}

/// The calls of one file: from the code index when it read the file, else those the syntax makes certain.
fn calls_of(
    files: &[FileRead],
    at: usize,
    index: Option<&Index>,
    keys: &HashMap<(&str, usize), &str>,
    guesses: CandidateLimit,
) -> (Vec<Relation>, Vec<Diagnostic>) {
    let file = &files[at];
    let mut out = Vec::new();
    let mut diags = Vec::new();
    let mut net = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |from: &str, to: &str, reach: Reach, line: usize, why: String| {
        if from != to && seen.insert((from.to_string(), to.to_string())) {
            out.push(Relation {
                from: from.to_string(),
                ..Relation::new(
                    Verb::Calls,
                    Target::Symbol(to.to_string()),
                    Evidence::new(line, why),
                )
                .reach(reach)
            });
        }
    };
    // The calls the index resolves, by line and name; the syntax reads the rest (docair-4wk.37).
    let mut resolved: HashSet<(usize, String)> = HashSet::new();
    let indexed = index.filter(|i| i.covers(&file.rel)).is_some();
    if let Some(index) = index.filter(|i| i.covers(&file.rel)) {
        {
            let key_of = |symbol: &str| {
                index
                    .definition(symbol)
                    .and_then(|(path, line)| keys.get(&(path, line)).copied())
            };
            for r in index.calls(&file.rel) {
                resolved.insert((r.line, symbol_name(&r.symbol)));
                let Some(caller) = file.at(&About::Within(r.line)) else {
                    continue;
                };
                let why = format!("rust-analyzer: {}", docair_scip::short(&r.symbol));
                let impls: Vec<&str> = index
                    .implementations(&r.symbol)
                    .iter()
                    .filter_map(|s| key_of(s))
                    .collect();
                match impls.as_slice() {
                    // A call through a trait: one of its implementations runs.
                    [_, _, ..] => {
                        for to in &impls {
                            push(&caller.key, to, Reach::OneOf, r.line, why.clone());
                        }
                    }
                    [one] => push(&caller.key, one, Reach::Certain, r.line, why),
                    [] => {
                        if let Some(to) = key_of(&r.symbol) {
                            push(&caller.key, to, Reach::Certain, r.line, why);
                        } else if let Some(method) = http_request(&r.symbol)
                            && net.iter().all(|n: &Relation| n.from != caller.key)
                        {
                            // A request to a host the code computes: the block names the system (`net:`).
                            net.push(Relation {
                                from: caller.key.clone(),
                                ..Relation::new(
                                    Verb::Calls,
                                    Target::Ref("net:".into()),
                                    Evidence::new(r.line, why),
                                )
                                .payload(Some(method))
                            });
                        }
                    }
                }
            }
        }
    }
    // The calls the syntax writes: all of them without an index, else those the index does not
    // resolve on their line, so they still get their could lines (docair-4wk.37).
    for (line, callee, text) in &file.calls {
        if indexed && resolved.contains(&(*line, called_name(callee).to_string())) {
            continue;
        }
        let Some(caller) = file.at(&About::Within(*line)) else {
            continue;
        };
        if !indexed && let Some(to) = called(files, at, caller, callee) {
            let to = to.key.clone();
            push(
                &caller.key,
                &to,
                Reach::Certain,
                *line,
                format!("tree-sitter: {text}(…)"),
            );
        } else if let Callee::Method(m) = callee {
            // The type is unknown: each method of that name could be the one (docs/ADAPTERS.md §5.5),
            // as many as [code] max_candidates allows (docair-4wk.36).
            let could = could_methods(files, at, m);
            let draw = match guesses {
                CandidateLimit::Off => false,
                CandidateLimit::All => true,
                CandidateLimit::AtMost(n) if could.len() > n => {
                    diags.push(CandidateLimit::too_many(
                        &file.rel,
                        *line,
                        text,
                        m,
                        could.len(),
                        n,
                    ));
                    false
                }
                CandidateLimit::AtMost(_) => true,
            };
            if draw {
                for to in could {
                    push(
                        &caller.key,
                        &to.key,
                        Reach::Could,
                        *line,
                        format!("tree-sitter: {text}(…), by its name"),
                    );
                }
            }
        }
    }
    out.extend(net);
    (out, diags)
}

/// The name a code index's symbol calls: `…/Type#save().` → `save`.
fn symbol_name(symbol: &str) -> String {
    let short = docair_scip::short(symbol);
    short
        .rsplit([':', '#', '.', '/'])
        .find(|w| !w.is_empty())
        .unwrap_or(&short)
        .to_string()
}

/// The name a call writes: `load`, `save`, the last of a path.
fn called_name(c: &Callee) -> &str {
    match c {
        Callee::Name(n) | Callee::SelfMethod(n) | Callee::Method(n) => n,
        Callee::Path(p) => p.last().map_or("", String::as_str),
    }
}

/// The function a call reaches: from the code index (the reference on that line with that name, the nearest one), else the syntax. None through a trait with several implementations.
fn reached(
    files: &[FileRead],
    at: usize,
    index: Option<&Index>,
    keys: &HashMap<(&str, usize), &str>,
    call: &CallAt,
) -> Option<String> {
    let file = &files[at];
    if let Some(index) = index.filter(|i| i.covers(&file.rel)) {
        let suffix = format!("::{}", call.name);
        let r = index
            .calls(&file.rel)
            .iter()
            .filter(|r| r.line == call.line)
            .filter(|r| {
                let s = docair_scip::short(&r.symbol);
                s == call.name || s.ends_with(&suffix)
            })
            .min_by_key(|r| r.col.abs_diff(call.col))?;
        let key_of = |symbol: &str| {
            index
                .definition(symbol)
                .and_then(|(path, line)| keys.get(&(path, line)).map(|k| k.to_string()))
        };
        let impls: Vec<String> = index
            .implementations(&r.symbol)
            .iter()
            .filter_map(|s| key_of(s))
            .collect();
        return match impls.as_slice() {
            [one] => Some(one.clone()),
            [] => key_of(&r.symbol),
            _ => None,
        };
    }
    let caller = file.at(&About::Within(call.line))?;
    called(files, at, caller, call.callee.as_ref()?).map(|s| s.key.clone())
}

/***
 * The data lines of one file: a value one call makes and a later call in the same function takes, as `@in name: Type from maker` on the taker.
 *
 * Both calls must reach functions of the project. The type is what the
 * `let` says, else what the maker returns. Values through a trait with
 * several implementations give no line.
 ***/
fn data_of(
    files: &[FileRead],
    at: usize,
    index: Option<&Index>,
    keys: &HashMap<(&str, usize), &str>,
) -> Vec<Relation> {
    let returns: HashMap<&str, &str> = files
        .iter()
        .flat_map(|f| &f.facts.symbols)
        .filter_map(|s| Some((s.key.as_str(), s.signature.as_ref()?.returns.as_deref()?)))
        .collect();
    // (taker, maker) → the values, in order, and where the first is taken.
    let mut lines: Vec<((String, String), Vec<String>, Evidence)> = Vec::new();
    for flow in &files[at].flows {
        let (Some(from), Some(to)) = (
            reached(files, at, index, keys, &flow.from),
            reached(files, at, index, keys, &flow.to),
        ) else {
            continue;
        };
        if from == to {
            continue;
        }
        let ty = flow.ty.clone().or_else(|| {
            (!flow.changed)
                .then(|| returns.get(from.as_str()).map(|t| t.to_string()))
                .flatten()
        });
        let (label, why) = match &flow.var {
            Some(v) => (
                match &ty {
                    Some(t) => format!("{v}: {t}"),
                    None => v.clone(),
                },
                format!(
                    "tree-sitter: let {v} = {}(…); {}(…{v}…)",
                    flow.from.text, flow.to.text
                ),
            ),
            None => (
                ty.unwrap_or_else(|| "value".to_string()),
                format!("tree-sitter: {}({}(…))", flow.to.text, flow.from.text),
            ),
        };
        let pair = (to, from);
        match lines.iter_mut().find(|(p, _, _)| *p == pair) {
            Some((_, labels, _)) => {
                if !labels.contains(&label) {
                    labels.push(label);
                }
            }
            None => lines.push((pair, vec![label], Evidence::new(flow.to.line, why))),
        }
    }
    lines
        .into_iter()
        .map(|((to, from), labels, evidence)| Relation {
            from: to,
            label: Some(labels.join(", ")),
            ..Relation::new(Verb::In, Target::Symbol(from), evidence)
        })
        .collect()
}

/// The type a name in the code names, without a code index: in the same file, else the only one of that name in the crate.
fn named_type<'a>(files: &'a [FileRead], at: usize, name: &str) -> Option<&'a Site> {
    let types = |f: &'a FileRead| f.items.iter().filter(move |i| i.is_type && i.name == name);
    if let Some(one) = only(types(&files[at])) {
        return Some(one);
    }
    let krate = files[at].krate.as_ref()?;
    only(
        files
            .iter()
            .filter(|f| f.krate.as_ref() == Some(krate))
            .flat_map(types),
    )
}

/// The name a code index's symbol of a type, a variant or a field is written with: `Cli`, `Build`, `command`.
fn written_name(symbol: &str) -> &str {
    let s = symbol.trim_end_matches(['#', '.']);
    s.rsplit(['#', '/', ']', ' ']).next().unwrap_or(s)
}

/***
 * The uses of types in one file: who makes, takes, returns, holds, matches or implements each, and which fields it reads and sets, as `@uses type (makes, reads command)` (docs/ADAPTERS.md §5.8).
 *
 * With a code index, every name the index gives a type of the project
 * counts, and the syntax there says how; without one, a type's name
 * is matched in the file, else in the crate, and fields are left out. A
 * type's own code, and `Type::f(…)` calling a function of the project that
 * returns something else, count nothing.
 ***/
fn uses_of(
    files: &[FileRead],
    at: usize,
    index: Option<&Index>,
    keys: &HashMap<(&str, usize), &str>,
    sites: &HashMap<&str, &Site>,
) -> Vec<Relation> {
    let file = &files[at];
    // (user, type) → how it uses it, and where it is first used.
    let mut uses: Vec<((String, String), TypeUses, Evidence)> = Vec::new();
    let mut add = |user: &Site, ty: &Site, m: &TypeMention, why: String| {
        if user.key == ty.key || user.id.starts_with(&format!("{}.", ty.id)) {
            return;
        }
        let pair = (user.key.clone(), ty.key.clone());
        let i = match uses.iter().position(|(p, ..)| *p == pair) {
            Some(i) => i,
            None => {
                uses.push((pair, TypeUses::default(), Evidence::new(m.line, why)));
                uses.len() - 1
            }
        };
        uses[i].1.add(m.how, &m.text);
    };
    // `Type::f(…)` makes the type unless f is the project's and returns something else.
    let makes = |m: &TypeMention, ty: &Site| {
        let Some((line, col, f)) = &m.via else {
            return true;
        };
        let call = CallAt {
            line: *line,
            col: *col,
            name: f.clone(),
            callee: Some(Callee::Path(vec![m.ty.clone(), f.clone()])),
            text: format!("{}::{f}", m.ty),
        };
        match reached(files, at, index, keys, &call) {
            Some(key) => sites.get(key.as_str()).is_none_or(|f| {
                f.returns
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|w| w == "Self" || w == ty.name)
            }),
            None => true,
        }
    };
    let user_of = |m: &TypeMention, ty: Option<&Site>| -> Option<&Site> {
        match &m.implementor {
            Some((line, col, name)) => match index.filter(|i| i.covers(&file.rel)) {
                Some(index) => index
                    .uses(&file.rel)
                    .iter()
                    .filter(|r| r.line == *line)
                    .min_by_key(|r| r.col.abs_diff(*col))
                    .and_then(|r| docair_scip::type_of(&r.symbol))
                    .and_then(|(t, _)| index.definition(&t))
                    .and_then(|(p, l)| keys.get(&(p, l)))
                    .and_then(|k| sites.get(k).copied()),
                None => named_type(files, at, name),
            },
            None => file
                .at(&About::Within(m.line))
                .filter(|u| ty.is_none_or(|t| u.key != t.key)),
        }
    };
    match index.filter(|i| i.covers(&file.rel)) {
        Some(index) => {
            for r in index.uses(&file.rel) {
                let Some((ty, member)) = docair_scip::type_of(&r.symbol) else {
                    continue;
                };
                let Some(ty) = index
                    .definition(&ty)
                    .and_then(|(p, l)| keys.get(&(p, l)))
                    .and_then(|k| sites.get(k).copied())
                    .filter(|s| s.is_type)
                else {
                    continue;
                };
                // The syntax at the name: the one at its column, else the nearest of its name on its line.
                let name = written_name(&r.symbol);
                let Some(m) = file
                    .mentions
                    .iter()
                    .find(|m| m.line == r.line && m.col == r.col)
                    .or_else(|| {
                        file.mentions
                            .iter()
                            .filter(|m| m.line == r.line && m.text == name)
                            .min_by_key(|m| m.col.abs_diff(r.col))
                    })
                else {
                    continue;
                };
                // A field read or set is named by the field; a variant's field counts as using the type.
                let plain_field = member.as_deref().is_some_and(|f| !f.contains('#'));
                if matches!(m.how, How::Reads | How::Sets) && !plain_field {
                    continue;
                }
                if m.how == How::Makes && !makes(m, ty) {
                    continue;
                }
                let Some(user) = user_of(m, Some(ty)) else {
                    continue;
                };
                add(
                    user,
                    ty,
                    m,
                    format!("rust-analyzer: {}", docair_scip::short(&r.symbol)),
                );
            }
        }
        None => {
            for m in &file.mentions {
                if matches!(m.how, How::Reads | How::Sets) || m.ty.is_empty() {
                    continue;
                }
                let Some(ty) = named_type(files, at, &m.ty) else {
                    continue;
                };
                if m.how == How::Makes && !makes(m, ty) {
                    continue;
                }
                let Some(user) = user_of(m, Some(ty)) else {
                    continue;
                };
                add(user, ty, m, format!("tree-sitter: {}", m.text));
            }
        }
    }
    uses.into_iter()
        .map(|((user, ty), ways, evidence)| Relation {
            from: user,
            ..Relation::new(Verb::Uses, Target::Symbol(ty), evidence).payload(Some(ways.words()))
        })
        .collect()
}

/***
 * Turn what the rules found and the calls into facts: each fact on the symbol it belongs to, names linked to their functions.
 *
 * A rule's name that names no function, or more than one in the crate, gives
 * no fact. Calls come from the code index when there is one, else from the
 * calls the syntax makes certain (`self.save()`, `load()` in the same file,
 * `config::load()`).
 *
 * @calls docair-lang-rust.calls_of "finds the calls between functions"
 * @calls docair-lang-rust.uses_of "finds who uses each type"
 ***/
pub fn link_with(files: Vec<FileRead>, index: Option<&Index>, guesses: CandidateLimit) -> Facts {
    let mut out = Facts::default();
    let keys: HashMap<(&str, usize), &str> = files
        .iter()
        .flat_map(|f| {
            f.items
                .iter()
                .map(move |i| ((f.rel.as_str(), i.start), i.key.as_str()))
        })
        .collect();
    for at in 0..files.len() {
        let (relations, diags) = calls_of(&files, at, index, &keys, guesses);
        out.relations.extend(relations);
        out.diagnostics.extend(diags);
    }
    for at in 0..files.len() {
        out.relations.extend(data_of(&files, at, index, &keys));
    }
    let sites: HashMap<&str, &Site> = files
        .iter()
        .flat_map(|f| f.items.iter().map(|i| (i.key.as_str(), i)))
        .collect();
    for at in 0..files.len() {
        out.relations
            .extend(uses_of(&files, at, index, &keys, &sites));
    }
    let (prefix_by_fn, prefix_by_lines) = route_prefixes(&files);
    for (at, file) in files.iter().enumerate() {
        // The prefix of a route found on this line: a nested router's `let`, else the function building it.
        let prefix_at = |line: usize| -> Option<&str> {
            prefix_by_lines
                .iter()
                .find(|(f, a, b, _)| *f == at && *a <= line && line <= *b)
                .map(|(_, _, _, p)| p.as_str())
                .or_else(|| {
                    let builder = file.at(&About::Within(line))?;
                    prefix_by_fn.get(&builder.key).map(String::as_str)
                })
        };
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
                    at: match (meet, prefix_at(found.line)) {
                        (Meet::Http(h), Some(p)) => Meet::Http(docair_core::model::Http {
                            method: h.method.clone(),
                            path: docair_core::route::join(p, &h.path),
                        }),
                        _ => meet.clone(),
                    },
                    evidence: Evidence::new(item.start, found.why.clone()),
                }),
                // `std::env::var("X")`: the variable meets what sets it (env::link).
                Fact::Requests { at, reply } => out.requests.push(Request {
                    by: item.key.clone(),
                    at: at.clone(),
                    payload: None,
                    reply: reply.clone(),
                    evidence: Evidence::new(found.line, found.why.clone()),
                }),
                // A file: the block names the store (`file:` binds to its `@reads`/`@writes`, docs/ADAPTERS.md §5.7).
                Fact::Access { write, path } => {
                    let verb = if *write { Verb::Writes } else { Verb::Reads };
                    let at = path
                        .as_deref()
                        .filter(|p| found.why.contains(&format!("\"{p}\"")))
                        .unwrap_or("");
                    out.relations.push(Relation {
                        from: item.key.clone(),
                        ..Relation::new(
                            verb,
                            Target::Ref(format!("file:{at}")),
                            Evidence::new(found.line, found.why.clone()),
                        )
                    });
                }
                Fact::Calls { to, payload } => {
                    let Some(callee) = named(&files, at, to) else {
                        continue;
                    };
                    // `GET /orders` under a nest is `GET /api/orders`.
                    let payload = match (payload, prefix_at(found.line)) {
                        (Some(pl), Some(p)) => Some(match pl.split_once(' ') {
                            Some((m, path)) if path.starts_with('/') => {
                                format!("{m} {}", docair_core::route::join(p, path))
                            }
                            _ => pl.clone(),
                        }),
                        (pl, _) => pl.clone(),
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

/// A module file's inner docs: its first `//!` lines (or a `/*! */` comment), before any item;
/// `#![…]` attributes and blank lines in between are skipped.
fn inner_docs(text: &str) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut rest = text.lines();
    while let Some(l) = rest.next() {
        let t = l.trim_start();
        if let Some(d) = t.strip_prefix("//!") {
            lines.push(d.strip_prefix(' ').unwrap_or(d).trim_end().to_string());
        } else if let Some(open) = t.strip_prefix("/*!") {
            let mut body = open.to_string();
            while !body.contains("*/") {
                let Some(next) = rest.next() else { break };
                body.push('\n');
                body.push_str(next);
            }
            let body = body.split("*/").next().unwrap_or("");
            for b in body.lines() {
                let b = b.trim();
                let b = b.strip_prefix('*').unwrap_or(b);
                lines.push(b.strip_prefix(' ').unwrap_or(b).to_string());
            }
        } else if t.is_empty() || t.starts_with("#![") {
            if !lines.is_empty() && t.is_empty() {
                lines.push(String::new());
            }
        } else {
            break;
        }
    }
    let docs = lines.join("\n").trim().to_string();
    (!docs.is_empty()).then_some(docs)
}

/// Items that start on the same line as an item around them, which `items_by_row` leaves out:
/// `fn file` in `impl Adapter for A { fn file(&self) {} }`, by the row they start on.
fn items_within_rows<'t>(firsts: &[Node<'t>]) -> Vec<(usize, Node<'t>)> {
    let mut out = Vec::new();
    for top in firsts {
        let row = top.start_position().row;
        let mut stack: Vec<Node> = {
            let mut c = top.walk();
            top.named_children(&mut c).collect()
        };
        while let Some(n) = stack.pop() {
            // Only what starts on the item's own line can be left out; what starts later has its own row.
            if n.start_position().row > row {
                continue;
            }
            if ITEM_KINDS.contains(&n.kind()) {
                out.push((row, n));
            }
            let mut c = n.walk();
            stack.extend(n.named_children(&mut c));
        }
    }
    out
}

/// The syntax kinds docair takes as items.
const ITEM_KINDS: &[&str] = &[
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

    #[test]
    fn a_module_files_inner_docs_are_read() {
        let text = "#![allow(dead_code)]\n//! Facts: what adapters learn.\n//!\n//! More.\n\nuse x;\n//! not this\n";
        assert_eq!(
            inner_docs(text).as_deref(),
            Some("Facts: what adapters learn.\n\nMore.")
        );
        assert_eq!(
            inner_docs("/*! Block docs.\n * second\n */\nfn a() {}").as_deref(),
            Some("Block docs.\nsecond")
        );
        assert_eq!(inner_docs("use x;\n//! late"), None);
    }

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
                Meet::Topic(_) | Meet::Env(_) => format!("serves {} by {}", s.at, shown(&s.by)),
            });
            let requests = facts
                .requests
                .iter()
                .map(|r| format!("requests {} by {}", r.at, shown(&r.by)));
            // A fixture tests its rule; who uses which type is the adapter's, tested on its own.
            let relations = facts
                .relations
                .iter()
                .filter(|r| r.verb != Verb::Uses)
                .map(|r| {
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
            Ok(serves.chain(requests).chain(relations).collect())
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

    fn rust_file(rel: &str, text: &str) -> SourceFile {
        SourceFile {
            path: PathBuf::from("/nonexistent").join(rel),
            rel: rel.to_string(),
            lang: language_for("rs").unwrap(),
            text: text.to_string(),
        }
    }

    fn calls(facts: &Facts) -> Vec<(String, String, Reach)> {
        let shown = |key: &str| {
            facts
                .symbols
                .iter()
                .find(|s| s.key == key)
                .and_then(|s| s.name.clone())
                .unwrap_or_default()
        };
        let mut out: Vec<_> = facts
            .relations
            .iter()
            .filter(|r| r.verb == Verb::Calls)
            .map(|r| {
                let Target::Symbol(k) = &r.to else {
                    panic!("{r:?}")
                };
                (shown(&r.from), shown(k), r.reach)
            })
            .collect();
        out.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        out
    }

    /// Who uses a type and how, read by the syntax without an index (docs/ADAPTERS.md §5.8).
    #[test]
    fn the_syntax_says_who_uses_a_type() {
        let src = "struct Cli {\n    command: Command,\n}\nenum Command {\n    Build { root: String },\n    Check,\n}\ntrait Adapter {}\nstruct RustAdapter;\nimpl Adapter for RustAdapter {}\nstruct Graph;\nimpl Graph {\n    fn new() -> Self { Graph }\n    fn size(&self) -> usize { 0 }\n}\nfn draw(g: &Graph) -> Option<Cli> {\n    let _ = Graph::size(g);\n    None\n}\nfn main() {\n    let cli = Cli::parse();\n    match cli.command {\n        Command::Build { root } => {}\n        Command::Check => {}\n    }\n    let g = Graph::new();\n}\nuse self::Graph as G;\n";
        let facts = facts_with(&rust_file("src/lib.rs", src), rules());
        let shown = |key: &str| {
            facts
                .symbols
                .iter()
                .find(|s| s.key == key)
                .and_then(|s| s.name.clone())
                .unwrap_or_default()
        };
        let mut uses: Vec<String> = facts
            .relations
            .iter()
            .filter(|r| r.verb == Verb::Uses)
            .map(|r| {
                let Target::Symbol(k) = &r.to else {
                    panic!("{r:?}")
                };
                format!(
                    "{} {} ({})",
                    shown(&r.from),
                    shown(k),
                    r.payload.clone().unwrap_or_default()
                )
            })
            .collect();
        uses.sort();
        assert_eq!(
            uses,
            [
                "Cli Command (holds)",
                "RustAdapter Adapter (implements)",
                "draw Cli (returns)",
                "draw Graph (takes)",
                "main Cli (makes)",
                "main Command (matches)",
                "main Graph (makes)",
            ],
            "Graph::size returns no Graph: that is a call, not a use; Graph's own methods and the `use` count nothing; fields need the index"
        );
    }

    /// Without an index, only the calls the syntax makes certain (docs/ADAPTERS.md §5.4).
    #[test]
    fn calls_the_syntax_makes_certain() {
        let src = "mod config {\n    pub fn load() {}\n}\nfn parse() {}\nfn helper() {}\nstruct Repo;\nimpl Repo {\n    fn new() -> Self { Repo }\n    fn save(&self) {}\n    fn run(&self) {\n        self.save();\n        let _ = Self::new();\n        helper();\n        config::load();\n        let mut parser = Parser::new();\n        parser.parse(1);\n        Vec::<u8>::new();\n    }\n}\n";
        let facts = facts_with(&rust_file("src/lib.rs", src), rules());
        assert_eq!(
            calls(&facts),
            [
                ("run".into(), "helper".into(), Reach::Certain),
                ("run".into(), "load".into(), Reach::Certain),
                ("run".into(), "new".into(), Reach::Certain),
                ("run".into(), "save".into(), Reach::Certain),
            ],
            "`parser.parse(1)` is a method of a type outside the project: no line to `fn parse`"
        );
    }

    /// Environment variables whose names callers pass in: through a closure, a function, a `const`.
    #[test]
    fn env_names_the_callers_pass() {
        let src = "const KEY: &str = \"DOCAIR_AI_API_KEY\";\nconst OTHER: &str = \"DOCAIR_OTHER_KEY\";\nfn get(key: &str) -> Option<String> {\n    std::env::var(key).ok()\n}\nfn from_env() {\n    let num = |key: &str| std::env::var(key).ok();\n    num(\"DOCAIR_MAX_FILES\");\n    get(KEY);\n    let pick = |var: &str| get(var);\n    pick(\"DOCAIR_AI_MODEL\");\n    let key_var = None.unwrap_or_else(|| OTHER.to_string());\n    get(&key_var);\n    std::env::var(\"DOCAIR_BIND\").ok();\n    let m = std::collections::HashMap::<String, String>::new();\n    m.get(\"not-an-env\");\n}\n";
        let facts = facts_with(&rust_file("src/lib.rs", src), rules());
        let mut names: Vec<String> = facts.requests.iter().map(|r| r.at.to_string()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "env:DOCAIR_AI_API_KEY",
                "env:DOCAIR_AI_MODEL",
                "env:DOCAIR_BIND",
                "env:DOCAIR_MAX_FILES",
                "env:DOCAIR_OTHER_KEY"
            ]
        );
    }

    #[test]
    fn requests_of_http_client_crates() {
        let sym = |s: &str| format!("rust-analyzer cargo {s}");
        assert_eq!(
            http_request(&sym("ureq 3.1.2 impl#[Agent]post().")).as_deref(),
            Some("POST")
        );
        assert_eq!(
            http_request(&sym("reqwest 0.12.0 impl#[Client]get().")).as_deref(),
            Some("GET")
        );
        assert_eq!(http_request(&sym("ureq 3.1.2 impl#[Agent]config().")), None);
        assert_eq!(
            http_request(&sym("tree-sitter 0.27.1 impl#[Parser]parse().")),
            None
        );
    }

    /// Data lines: what one call makes and a later call takes, in the order the function writes them.
    #[test]
    fn data_the_code_hands_on() {
        let src = "pub struct Graph;\nfn discover(root: &str) -> Vec<String> {\n    vec![]\n}\nfn extract(files: &[String]) -> Vec<u8> {\n    vec![]\n}\nfn build(blocks: &[u8]) -> Vec<u8> {\n    vec![]\n}\nfn resolve(nodes: &mut Vec<u8>) {}\nfn check(nodes: &[u8]) -> Graph {\n    Graph\n}\nfn render(g: Graph) {}\nfn scan(root: &str) -> Graph {\n    let files = discover(root);\n    let blocks = extract(&files).into_iter().collect::<Vec<_>>();\n    let mut nodes = build(&blocks);\n    resolve(&mut nodes);\n    check(&nodes)\n}\nfn main() {\n    render(scan(\"/\"));\n    let n = 1;\n    render(Graph);\n}\n";
        let facts = facts_with(&rust_file("src/lib.rs", src), rules());
        let name = |key: &str| {
            facts
                .symbols
                .iter()
                .find(|s| s.key == key)
                .and_then(|s| s.name.clone())
                .unwrap_or_default()
        };
        let mut data: Vec<String> = facts
            .relations
            .iter()
            .filter(|r| r.verb == Verb::In)
            .map(|r| {
                let Target::Symbol(to) = &r.to else {
                    return String::new();
                };
                format!(
                    "{} <- {}: {}",
                    name(&r.from),
                    name(to),
                    r.label.clone().unwrap_or_default()
                )
            })
            .collect();
        data.sort();
        assert_eq!(
            data,
            [
                "build <- extract: blocks: u8[]",
                // `resolve(&mut nodes)` made the value check takes.
                "check <- resolve: nodes",
                "extract <- discover: files: String[]",
                // A call written inside another.
                "render <- scan: Graph",
                "resolve <- build: nodes: u8[]",
            ]
        );
    }

    /// Data through closures, `for`, `partition` and struct literals.
    #[test]
    fn data_through_closures_and_structs() {
        let src = "pub struct Project<'a> {\n    blocks: &'a [u8],\n}\nfn discover(root: &str) -> Vec<String> {\n    vec![]\n}\nfn one(f: &String) -> u8 {\n    0\n}\nfn two(f: &String) {}\nfn collect(p: &Project) {}\nfn pipeline(root: &str) {\n    let files = discover(root);\n    let (_md, code) = files.iter().cloned().partition::<Vec<_>, _>(|f| f.is_empty());\n    let blocks: Vec<u8> = code.iter().map(|f| one(f)).collect();\n    let project = Project { blocks: &blocks };\n    collect(&project);\n    for f in &code {\n        two(f);\n    }\n}\n";
        let facts = facts_with(&rust_file("src/lib.rs", src), rules());
        let name = |key: &str| {
            facts
                .symbols
                .iter()
                .find(|s| s.key == key)
                .and_then(|s| s.name.clone())
                .unwrap_or_default()
        };
        let mut data: Vec<String> = facts
            .relations
            .iter()
            .filter(|r| r.verb == Verb::In)
            .filter_map(|r| match &r.to {
                Target::Symbol(to) => Some(format!(
                    "{} <- {}: {}",
                    name(&r.from),
                    name(to),
                    r.label.clone().unwrap_or_default()
                )),
                Target::Ref(_) => None,
            })
            .collect();
        data.sort();
        assert_eq!(
            data,
            [
                "collect <- one: blocks: Vec<u8>",
                "one <- discover: files: String[]",
                "two <- discover: files: String[]",
            ]
        );
    }

    /// With an index: calls through a trait reach one of its implementations; outside methods reach
    /// nothing; a call the index does not resolve is read by the syntax, its could lines by name.
    #[test]
    fn calls_from_the_code_index() {
        use protobuf::Message;
        // One item per line: the adapter knows an item by the line it starts on.
        let src = "pub trait Adapter {\n    fn file(&self);\n}\npub struct A;\nimpl Adapter for A {\n    fn file(&self) {}\n}\npub struct B;\nimpl Adapter for B {\n    fn file(&self) {}\n}\npub fn parse() {}\npub fn collect(a: &dyn Adapter, p: &mut Parser) {\n    a.file();\n    p.parse();\n    parse();\n    a.tick();\n}\npub struct C;\nimpl C {\n    fn tick(&self) {}\n}\npub struct D;\nimpl D {\n    fn tick(&self) {}\n}\n";
        let sym = |d: &str| format!("rust-analyzer cargo k 0.1.0 {d}");
        let occ = |line: i32, symbol: String, def: bool| {
            let mut o = scip::types::Occurrence::new();
            o.range = vec![line, 0, 1];
            o.symbol = symbol;
            o.symbol_roles = i32::from(def);
            o
        };
        let mut doc = scip::types::Document::new();
        doc.relative_path = "src/lib.rs".into();
        doc.occurrences = vec![
            occ(1, sym("Adapter#file()."), true),
            occ(5, sym("impl#[A][Adapter]file()."), true),
            occ(9, sym("impl#[B][Adapter]file()."), true),
            occ(11, sym("parse()."), true),
            occ(13, sym("Adapter#file()."), false),
            // tree-sitter's own `Parser::parse`: outside the project.
            occ(
                14,
                "rust-analyzer cargo tree-sitter 0.27.1 impl#[Parser]parse().".into(),
                false,
            ),
            occ(15, sym("parse()."), false),
        ];
        let mut index = scip::types::Index::new();
        index.documents = vec![doc];
        let index = Index::parse(&index.write_to_bytes().unwrap()).unwrap();
        let read = vec![read_file(&rust_file("src/lib.rs", src), rules())];
        let facts = link_with(read, Some(&index), CandidateLimit::DEFAULT);
        assert_eq!(
            calls(&facts),
            [
                ("collect".into(), "file".into(), Reach::OneOf),
                ("collect".into(), "file".into(), Reach::OneOf),
                ("collect".into(), "parse".into(), Reach::Certain),
                // docair-4wk.37: a call the index does not resolve still gets could lines, by its name.
                ("collect".into(), "tick".into(), Reach::Could),
                ("collect".into(), "tick".into(), Reach::Could),
            ]
        );
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

    /// Items inside an item on the same line get parts of their own, and calls from them start there (docair-4wk.16).
    #[test]
    fn items_on_one_line_are_parts_of_their_own() {
        let src = "mod config { pub fn load() { parse(); } }\n\npub fn parse() {}\n\npub struct A;\nimpl A { pub fn file(&self) { parse(); } }\n";
        let blocks = enrich("src/lib.rs", src);
        let names: Vec<String> = blocks.iter().filter_map(|b| b.hints.name.clone()).collect();
        for want in ["config", "load", "parse", "A", "file"] {
            assert!(names.iter().any(|n| n == want), "{want} in {names:?}");
        }
        let file = SourceFile {
            path: PathBuf::from("/nonexistent/src/lib.rs"),
            rel: "src/lib.rs".to_string(),
            lang: language_for("rs").unwrap(),
            text: src.to_string(),
        };
        let facts = facts_with(&file, rules());
        let key_of = |name: &str| {
            facts
                .symbols
                .iter()
                .find(|s| s.name.as_deref() == Some(name))
                .map(|s| s.key.clone())
                .unwrap()
        };
        assert_eq!(key_of("config"), "rust:src/lib.rs:1");
        assert_eq!(key_of("load"), "rust:src/lib.rs:1:14");
        let calls: Vec<(String, String)> = facts
            .relations
            .iter()
            .filter(|r| r.verb == Verb::Calls)
            .map(|r| (r.from.clone(), format!("{:?}", r.to)))
            .collect();
        let from_load = calls
            .iter()
            .any(|(f, t)| *f == key_of("load") && t.contains(&key_of("parse")));
        let from_file = calls
            .iter()
            .any(|(f, t)| *f == key_of("file") && t.contains(&key_of("parse")));
        assert!(from_load && from_file, "{calls:?}");
    }

    /// A router nested under a prefix serves its routes there: by a function, a nest in a nest, a variable (docair-4wk.9).
    #[test]
    fn nested_routers_put_their_prefix_in_front() {
        let src = r#"use axum::Router;
use axum::routing::get;

pub fn app() -> Router {
    let admin = Router::new().route("/users", get(users));
    Router::new()
        .route("/healthz", get(health))
        .nest("/api", api())
        .nest("/admin", admin)
}

pub fn api() -> Router {
    Router::new().route("/orders", get(list)).nest("/v1", v1())
}

pub fn v1() -> Router {
    Router::new().route("/items/{id}", get(items))
}

async fn health() {}
async fn list() {}
async fn items() {}
async fn users() {}
"#;
        let file = SourceFile {
            path: PathBuf::from("/nonexistent/src/lib.rs"),
            rel: "src/lib.rs".to_string(),
            lang: language_for("rs").unwrap(),
            text: src.to_string(),
        };
        let facts = facts_with(&file, rules());
        let name_of = |key: &str| {
            facts
                .symbols
                .iter()
                .find(|s| s.key == key)
                .and_then(|s| s.name.clone())
                .unwrap_or_default()
        };
        let mut served: Vec<String> = facts
            .serves
            .iter()
            .map(|s| format!("{} {}", s.at, name_of(&s.by)))
            .collect();
        served.sort();
        assert_eq!(
            served,
            [
                "GET /admin/users users",
                "GET /api/orders list",
                "GET /api/v1/items/{id} items",
                "GET /healthz health",
            ]
        );
        let payloads: Vec<String> = facts
            .relations
            .iter()
            .filter(|r| r.verb == Verb::Calls)
            .filter_map(|r| r.payload.clone())
            .collect();
        assert!(
            payloads.contains(&"GET /api/v1/items/{id}".to_string()),
            "{payloads:?}"
        );
    }

    /// A method called on a value of unknown type could be each method of that name; std names and too many give none (docair-4wk.13).
    #[test]
    fn a_method_of_unknown_type_could_be_each_of_that_name() {
        let mut src = String::from(
            "pub struct A;\nimpl A { pub fn save(&self) {} }\npub struct B;\nimpl B { pub fn save(&self) {} }\n\npub fn run(x: Box<dyn Any>) {\n    x.save();\n    x.is_empty();\n    x.tick();\n}\n",
        );
        for t in ["C", "D", "E", "F", "G", "H"] {
            src.push_str(&format!(
                "pub struct {t};\nimpl {t} {{ pub fn tick(&self) {{}} }}\n"
            ));
        }
        let file = SourceFile {
            path: PathBuf::from("/nonexistent/src/lib.rs"),
            rel: "src/lib.rs".to_string(),
            lang: language_for("rs").unwrap(),
            text: src,
        };
        let facts = facts_with(&file, rules());
        let name_of = |key: &str| {
            facts
                .symbols
                .iter()
                .find(|s| s.key == key)
                .map(|s| {
                    format!(
                        "{}.{}",
                        s.id.clone().unwrap_or_default(),
                        s.name.clone().unwrap_or_default()
                    )
                })
                .unwrap_or_default()
        };
        let from_run: Vec<(String, Reach)> = facts
            .relations
            .iter()
            .filter(|r| r.verb == Verb::Calls && name_of(&r.from).ends_with(".run"))
            .map(|r| match &r.to {
                Target::Symbol(k) => (name_of(k), r.reach),
                t => (format!("{t:?}"), r.reach),
            })
            .collect();
        assert_eq!(
            from_run.len(),
            2,
            "save of A and B, not is_empty, not tick with 6: {from_run:?}"
        );
        assert!(
            from_run
                .iter()
                .all(|(n, reach)| n.ends_with(".save") && *reach == Reach::Could),
            "{from_run:?}"
        );
        // docair-4wk.36: the call with more than the limit is an error, not left out silently.
        let e: Vec<&Diagnostic> = facts
            .diagnostics
            .iter()
            .filter(|d| d.code == "E007")
            .collect();
        assert_eq!(e.len(), 1, "{:?}", facts.diagnostics);
        assert!(
            e[0].message
                .contains("could go to 6 functions named `tick`")
                && e[0].message.contains("max_candidates (5)")
                && e[0].line == Some(9),
            "{:?}",
            e[0]
        );
        // 0 draws them all, -1 none.
        let could = |limit: CandidateLimit| {
            let facts = link_with(vec![read_file(&file, rules())], None, limit);
            let n = facts
                .relations
                .iter()
                .filter(|r| r.reach == Reach::Could)
                .count();
            (n, facts.diagnostics.iter().any(|d| d.code == "E007"))
        };
        assert_eq!(
            could(CandidateLimit::All),
            (8, false),
            "save twice, tick six times"
        );
        assert_eq!(could(CandidateLimit::Off), (0, false), "no guesses");
        assert_eq!(could(CandidateLimit::AtMost(6)), (8, false));
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

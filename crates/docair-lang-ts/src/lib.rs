//! TypeScript and JavaScript adapter (docs/ADAPTERS.md, step 5): tree-sitter
//! finds every declaration, and the rules in `rules/` say what frameworks mean
//! (Angular's HttpClient, `fetch`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use docair_core::adapter::{Adapter, Project};
use docair_core::facts::{
    Draw, Evidence, Facts, Inferred, Reach, Relation, Request, Serve, Symbol, Target, Verb,
};
use docair_core::glossary::normalize;
use docair_core::ids::slug;
use docair_core::model::{Diagnostic, SourceFile};
use docair_core::signature::ts_signature;
use docair_rules::{About, Fact, Grammar, RuleSet};
use tree_sitter::{Node, Parser};

/// The built-in rules (docs/ADAPTERS.md §5.2). Each needs a fixture beside it in `rules/`.
const RULES: &[(&str, &str)] = &[
    (
        "angular.http-client.yaml",
        include_str!("../rules/angular.http-client.yaml"),
    ),
    ("fetch.yaml", include_str!("../rules/fetch.yaml")),
    ("fetch-get.yaml", include_str!("../rules/fetch-get.yaml")),
    (
        "angular.bootstrap.yaml",
        include_str!("../rules/angular.bootstrap.yaml"),
    ),
    ("node.env.yaml", include_str!("../rules/node.env.yaml")),
    (
        "node.fs-read.yaml",
        include_str!("../rules/node.fs-read.yaml"),
    ),
    (
        "node.fs-write.yaml",
        include_str!("../rules/node.fs-write.yaml"),
    ),
];

/// `.tsx` and `.jsx` files use the TSX grammar.
fn is_tsx(rel: &str) -> bool {
    rel.ends_with(".tsx") || rel.ends_with(".jsx")
}

fn language(tsx: bool) -> tree_sitter::Language {
    if tsx {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    }
}

/// TypeScript as ast-grep sees it; rules say `language: typescript` for TSX too.
pub fn grammar(tsx: bool) -> Grammar {
    Grammar::new("typescript", language(tsx), '$')
}

fn rules(tsx: bool) -> &'static RuleSet {
    static TS: OnceLock<RuleSet> = OnceLock::new();
    static TSX: OnceLock<RuleSet> = OnceLock::new();
    let cell = if tsx { &TSX } else { &TS };
    cell.get_or_init(|| {
        RuleSet::load(grammar(tsx), RULES)
            .expect("the built-in rules load (their fixtures test them)")
    })
}

/***
 * The TypeScript and JavaScript adapter: every declaration of a file, its tests, and the requests it sends.
 *
 * Every file is a module, the box its undocumented declarations sit in.
 * Classes, functions, methods, constants, interfaces, types and enums at
 * module or class level are drawn (docs/ADAPTERS.md §8), and so are the
 * Playwright tests (`test('…')`, `test.describe('…')`). The rules in
 * `rules/` find what frameworks mean: the HTTP requests a method sends
 * through Angular's HttpClient or `fetch`, the component
 * `bootstrapApplication` starts. Across the files it finds the calls: on an
 * injected service, from an Angular output to the parent's handler that the
 * parent's template binds, and through `postMessage` to the function a
 * `message` listener calls for that type.
 ***/
pub struct TsAdapter;

impl Adapter for TsAdapter {
    fn name(&self) -> &'static str {
        "typescript"
    }

    fn handles(&self, file: &SourceFile) -> bool {
        file.lang.name == "typescript"
    }

    /***
     * Read every TypeScript and JavaScript file, then find the calls across them.
     *
     * The templates are the project's HTML files, by their path: nothing is read from disk.
     *
     * @calls docair-lang-ts.read_file "reads the declarations and runs the rules"
     * @calls docair-lang-ts.link "finds the calls across the files"
     ***/
    fn project(&self, project: &Project) -> Facts {
        let read: Vec<FileRead> = project
            .files
            .iter()
            .filter(|f| self.handles(f))
            .map(|f| read_file(f, rules(is_tsx(&f.rel))))
            .collect();
        let html: HashMap<String, String> = project
            .files
            .iter()
            .filter(|f| f.lang.name == "html")
            .map(|f| (f.rel.clone(), f.text.clone()))
            .collect();
        let mut diags = Vec::new();
        let index = ts_index(project, &mut diags);
        let mut facts = link_with_index(read, &html, index.as_ref());
        facts.diagnostics.extend(diags);
        facts
    }
}

/***
 * The code index of the project's TypeScript: with `[code] run_indexers`, what scip-typescript writes for each folder with a `tsconfig.json` that has it installed (`node_modules/.bin`), kept until the sources change. Paths are made the project's.
 *
 * @calls docair-scip.kept_ts_index "runs scip-typescript, or takes the kept index"
 ***/
fn ts_index(
    project: &Project,
    diags: &mut Vec<docair_core::model::Diagnostic>,
) -> Option<docair_scip::Index> {
    let code = project.config.code.as_ref()?;
    if !code.run_indexers {
        return None;
    }
    let first = project.files.first()?;
    let root = first.path.ancestors().nth(first.rel.split('/').count())?;
    let mut all: Option<docair_scip::Index> = None;
    // The folders with a tsconfig.json above the TypeScript files (it is not a file docair reads).
    let mut dirs: Vec<String> = Vec::new();
    for f in project.files.iter().filter(|f| f.lang.name == "typescript") {
        let mut d = f.rel.as_str();
        while let Some((up, _)) = d.rsplit_once('/') {
            if root.join(up).join("tsconfig.json").is_file() {
                if !dirs.iter().any(|x| x == up) {
                    dirs.push(up.to_string());
                }
                break;
            }
            d = up;
        }
    }
    for rel_dir in &dirs {
        let rel_dir = rel_dir.as_str();
        let dir = root.join(rel_dir);
        if docair_scip::scip_typescript_in(&dir).is_none() {
            continue;
        }
        let cfg_rel = format!("{rel_dir}/tsconfig.json");
        let under = |rel: &str| rel_dir.is_empty() || rel.starts_with(&format!("{rel_dir}/"));
        let sources: Vec<(&str, &str)> = project
            .files
            .iter()
            .filter(|f| under(&f.rel))
            .filter(|f| {
                [".ts", ".tsx", ".js", ".mjs", ".json"]
                    .iter()
                    .any(|e| f.rel.ends_with(e))
            })
            .map(|f| (f.rel.as_str(), f.text.as_str()))
            .collect();
        match docair_scip::kept_ts_index(&dir, &sources, &docair_scip::cache_dir()) {
            Ok(path) => match docair_scip::Index::read(&path) {
                Ok(i) => {
                    let i = i.under(rel_dir);
                    match all.as_mut() {
                        Some(a) => a.extend(i),
                        None => all = Some(i),
                    }
                }
                Err(e) => diags.push(
                    docair_core::model::Diagnostic::new(
                        "W113",
                        format!("cannot read the code index: {e}"),
                    )
                    .at(&cfg_rel, 1),
                ),
            },
            Err(e) => diags.push(
                docair_core::model::Diagnostic::new(
                    "W119",
                    format!("no TypeScript code index for the calls: {e}"),
                )
                .at(&cfg_rel, 1),
            ),
        }
    }
    all
}

/// Where a declaration's code is, so that what a rule finds can be put on it.
struct Site {
    key: String,
    start: usize,
    end: usize,
    name: String,
    /// The class a method is written in (its id).
    class: Option<String>,
    /// A function or method another one can call.
    callable: bool,
}

/// A class: where it is, the types of its fields (`http = inject(HttpClient)` makes `http` an HttpClient), and what Angular says about it.
struct Class {
    key: String,
    id: String,
    name: String,
    line: usize,
    start: usize,
    end: usize,
    fields: HashMap<String, String>,
    /// Angular outputs: `closed = output<void>()`, `@Output() closed = new EventEmitter()`.
    outputs: HashSet<String>,
    /// `@Component({ selector: 'app-chat-panel' })`.
    selector: Option<String>,
    template: Option<TemplateSource>,
}

/// A component's template: `templateUrl` as a path from the root, or the inline `template`.
enum TemplateSource {
    File(String),
    Inline(String),
}

/// `this.scanner.scan(…)`, where `scanner` is a ScanService.
struct MemberCall {
    caller: String,
    ty: String,
    method: String,
    line: usize,
    text: String,
}

/// `this.closed.emit(…)`: `by` emits the output of the class.
struct Emit {
    by: String,
    class: String,
    output: String,
    line: usize,
}

/// `postMessage({ type: 'docair:wording', … })`, also through a function that posts what it gets.
struct PostedMessage {
    by: String,
    ty: String,
    line: usize,
}

/// A `message` listener calls `handler` for messages of type `ty`.
struct MessageHandler {
    handler: String,
    name: String,
    ty: String,
}

/// A name a rule says is called: `bootstrapApplication(App)` calls App.
struct NamedCall {
    by: String,
    name: String,
    line: usize,
    why: String,
}

/// What one file says, kept until every file is read and the calls across them are found.
pub struct FileRead {
    rel: String,
    facts: Facts,
    sites: Vec<Site>,
    classes: Vec<Class>,
    /// `import { App } from './app/app'`: App → `./app/app`.
    imports: HashMap<String, String>,
    member_calls: Vec<MemberCall>,
    emits: Vec<Emit>,
    posts: Vec<PostedMessage>,
    listens: Vec<MessageHandler>,
    named: Vec<NamedCall>,
    /// `x.m(…)` on a value of no known type: (line, method, code) (docs/ADAPTERS.md §5.5).
    could: Vec<(usize, String, String)>,
}

struct Reader<'a> {
    file: &'a SourceFile,
    src: &'a [u8],
    lines: Vec<&'a str>,
    module: String,
    package: Option<String>,
    facts: Facts,
    sites: Vec<Site>,
    classes: Vec<Class>,
    imports: HashMap<String, String>,
    member_calls: Vec<MemberCall>,
    emits: Vec<Emit>,
    posts: Vec<PostedMessage>,
    listens: Vec<MessageHandler>,
    named: Vec<NamedCall>,
    could: Vec<(usize, String, String)>,
}

/// The facts of one file on its own (tests, fixtures): calls across files need [`link`] over all of them.
pub fn facts_with(file: &SourceFile, rules: &RuleSet) -> Facts {
    link(vec![read_file(file, rules)], &HashMap::new())
}

/***
 * Parse a TypeScript or JavaScript file with tree-sitter, report its declarations and tests, and run the rules on it.
 *
 * @calls docair-lang-ts.package_of "finds which package owns the file"
 * @calls docair-rules.rule-set.apply "finds the requests the code sends"
 ***/
pub fn read_file(file: &SourceFile, rules: &RuleSet) -> FileRead {
    let mut parser = Parser::new();
    parser
        .set_language(&language(is_tsx(&file.rel)))
        .expect("tree-sitter-typescript matches the tree-sitter ABI");
    let Some(tree) = parser.parse(&file.text, None) else {
        let mut facts = Facts::default();
        facts.diagnostics.push(
            Diagnostic::new("W113", "tree-sitter could not parse this file").at(&file.rel, 1),
        );
        return FileRead {
            rel: file.rel.clone(),
            facts,
            sites: Vec::new(),
            classes: Vec::new(),
            imports: HashMap::new(),
            member_calls: Vec::new(),
            emits: Vec::new(),
            posts: Vec::new(),
            listens: Vec::new(),
            named: Vec::new(),
            could: Vec::new(),
        };
    };
    // The project's root: the package of a file is never looked for above it.
    let root = file
        .path
        .ancestors()
        .nth(file.rel.split('/').count())
        .unwrap_or(Path::new("/"));
    let package = package_of(&file.path, root);
    let module = module_id(&file.rel, &file.path, package.as_ref());
    let mut r = Reader {
        file,
        src: file.text.as_bytes(),
        lines: file.text.lines().collect(),
        module: module.clone(),
        package: package.map(|(name, _)| slug(&name)),
        facts: Facts::default(),
        sites: Vec::new(),
        classes: Vec::new(),
        imports: HashMap::new(),
        member_calls: Vec::new(),
        emits: Vec::new(),
        posts: Vec::new(),
        listens: Vec::new(),
        named: Vec::new(),
        could: Vec::new(),
    };
    // The file is a module: its undocumented declarations sit in it. When
    // nothing ends up in it, the build leaves it out.
    let file_name = file.rel.rsplit('/').next().unwrap_or(&file.rel);
    r.facts.symbols.push(Symbol {
        draw: Draw::Alone,
        display: Some(file.rel.clone()),
        name: Some(file_name.to_string()),
        id: Some(module),
        kind: Some(Inferred::new("module", "a TypeScript or JavaScript file")),
        parents: r.package.iter().cloned().collect(),
        ..Symbol::at(format!("ts:{}", file.rel), &file.rel, 1)
    });
    r.statements(tree.root_node());
    r.found(rules);
    r.calls(tree.root_node());
    r.messages(tree.root_node());
    FileRead {
        rel: file.rel.clone(),
        facts: r.facts,
        sites: r.sites,
        classes: r.classes,
        imports: r.imports,
        member_calls: r.member_calls,
        emits: r.emits,
        posts: r.posts,
        listens: r.listens,
        named: r.named,
        could: r.could,
    }
}

impl Reader<'_> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

    fn line_text(&self, line: usize) -> &str {
        self.lines.get(line - 1).copied().unwrap_or("")
    }

    fn statements(&mut self, parent: Node) {
        let mut cursor = parent.walk();
        let children: Vec<Node> = parent.named_children(&mut cursor).collect();
        for n in children {
            self.statement(n);
        }
    }

    fn statement(&mut self, n: Node) {
        match n.kind() {
            "export_statement" => {
                let mut cursor = n.walk();
                let decorators: Vec<Node> = n
                    .named_children(&mut cursor)
                    .filter(|c| c.kind() == "decorator")
                    .collect();
                if let Some(d) = n.child_by_field_name("declaration") {
                    self.declaration(d, n, &decorators);
                }
            }
            "expression_statement" => {
                if !self.test(n) {
                    self.call(n);
                }
            }
            "import_statement" => self.import(n),
            _ => self.declaration(n, n, &[]),
        }
    }

    /// A symbol for a declaration named by `name`; `anchor` is where its docs stand.
    #[allow(clippy::too_many_arguments)]
    fn symbol(
        &mut self,
        name_node: Node,
        node: Node,
        anchor: Node,
        display: String,
        id: String,
        kind: Inferred,
        parents: Vec<String>,
        parent_line: Option<usize>,
        signature: bool,
    ) -> usize {
        let line = name_node.start_position().row + 1;
        let key = format!("ts:{}:{line}", self.file.rel);
        let name = self.text(name_node).to_string();
        self.sites.push(Site {
            key: key.clone(),
            start: node.start_position().row + 1,
            end: node.end_position().row + 1,
            name: name.clone(),
            class: parent_line.and(parents.first().cloned()),
            callable: kind.value == "function",
        });
        self.facts.symbols.push(Symbol {
            draw: Draw::Always,
            display: Some(display),
            name: Some(name),
            id: Some(id),
            kind: Some(kind),
            parents,
            parent_line,
            signature: signature
                .then(|| ts_signature(self.line_text(line)))
                .flatten(),
            docs: docs_of(anchor, self.src),
            end: Some(node.end_position().row + 1),
            ..Symbol::at(key, &self.file.rel, line)
        });
        line
    }

    fn up(&self) -> Vec<String> {
        std::iter::once(self.module.clone())
            .chain(self.package.clone())
            .collect()
    }

    fn declaration(&mut self, d: Node, anchor: Node, decorators: &[Node]) {
        let kind = d.kind();
        let why = format!("tree-sitter: {kind}");
        match kind {
            "class_declaration" | "abstract_class_declaration" => {
                let Some(name) = d.child_by_field_name("name") else {
                    return;
                };
                let id = format!("{}.{}", self.module, slug(self.text(name)));
                let mut cursor = d.walk();
                let own: Vec<Node> = d
                    .named_children(&mut cursor)
                    .filter(|c| c.kind() == "decorator")
                    .collect();
                let angular = decorators
                    .iter()
                    .chain(&own)
                    .find_map(|dec| angular_decorator(self.text(*dec)).map(|a| (a, *dec)));
                let props = angular
                    .map(|(_, dec)| decorator_props(dec, self.src))
                    .unwrap_or_default();
                let why = angular.map_or(why, |(a, _)| format!("Angular @{a}"));
                let display = format!("class {}", self.text(name));
                let up = self.up();
                let line = self.symbol(
                    name,
                    d,
                    anchor,
                    display,
                    id.clone(),
                    Inferred::new("component", why),
                    up,
                    None,
                    false,
                );
                if let Some(body) = d.child_by_field_name("body") {
                    self.class_body(body, &id, line);
                    let dir = self.file.rel.rsplit_once('/').map_or("", |(d, _)| d);
                    let template = match (props.get("templateUrl"), props.get("template")) {
                        (Some(url), _) => {
                            normalize(&Path::new(dir).join(url)).map(TemplateSource::File)
                        }
                        (None, Some(inline)) => Some(TemplateSource::Inline(inline.clone())),
                        (None, None) => None,
                    };
                    let class_name = self.text(name).to_string();
                    if let Some(c) = self.classes.last_mut() {
                        c.name = class_name;
                        c.selector = props.get("selector").cloned();
                        c.template = template;
                    }
                }
            }
            "function_declaration" | "generator_function_declaration" => {
                let Some(name) = d.child_by_field_name("name") else {
                    return;
                };
                let id = format!("{}.{}", self.module, slug(self.text(name)));
                let display = format!("function {}", self.text(name));
                let up = self.up();
                self.symbol(
                    name,
                    d,
                    anchor,
                    display,
                    id,
                    Inferred::new("function", why),
                    up,
                    None,
                    true,
                );
            }
            "lexical_declaration" | "variable_declaration" => {
                let keyword = self
                    .text(d)
                    .split_whitespace()
                    .next()
                    .unwrap_or("const")
                    .to_string();
                let mut cursor = d.walk();
                let declarators: Vec<Node> = d
                    .named_children(&mut cursor)
                    .filter(|c| c.kind() == "variable_declarator")
                    .collect();
                for v in declarators {
                    let Some(name) = v.child_by_field_name("name") else {
                        continue;
                    };
                    if name.kind() != "identifier" {
                        continue; // destructuring: no single name
                    }
                    let is_fn = v.child_by_field_name("value").is_some_and(|val| {
                        matches!(
                            val.kind(),
                            "arrow_function" | "function_expression" | "function"
                        )
                    });
                    let id = format!("{}.{}", self.module, slug(self.text(name)));
                    let display = format!("{keyword} {}", self.text(name));
                    let kind = if is_fn { "function" } else { "config" };
                    let up = self.up();
                    self.symbol(
                        name,
                        v,
                        anchor,
                        display,
                        id,
                        Inferred::new(kind, "tree-sitter: variable_declarator"),
                        up,
                        None,
                        is_fn,
                    );
                }
            }
            "interface_declaration" | "type_alias_declaration" | "enum_declaration" => {
                let Some(name) = d.child_by_field_name("name") else {
                    return;
                };
                let word = kind.split('_').next().unwrap_or("type");
                let word = if word == "type" { "type" } else { word };
                let id = format!("{}.{}", self.module, slug(self.text(name)));
                let display = format!("{word} {}", self.text(name));
                let up = self.up();
                self.symbol(
                    name,
                    d,
                    anchor,
                    display,
                    id,
                    Inferred::new("component", why),
                    up,
                    None,
                    false,
                );
            }
            _ => {}
        }
    }

    /// Methods and arrow-function fields of a class, and the types of its fields.
    fn class_body(&mut self, body: Node, class_id: &str, class_line: usize) {
        let mut class = Class {
            key: format!("ts:{}:{class_line}", self.file.rel),
            id: class_id.to_string(),
            name: String::new(),
            line: class_line,
            start: body.start_position().row + 1,
            end: body.end_position().row + 1,
            fields: HashMap::new(),
            outputs: HashSet::new(),
            selector: None,
            template: None,
        };
        let mut cursor = body.walk();
        let members: Vec<Node> = body.named_children(&mut cursor).collect();
        for m in members {
            let Some(name) = m.child_by_field_name("name") else {
                continue;
            };
            let field_name = self.text(name).to_string();
            let is_method = match m.kind() {
                "method_definition" => true,
                "public_field_definition" => {
                    let value = m.child_by_field_name("value");
                    if let Some(t) = field_type(m, value, self.src) {
                        class.fields.insert(field_name.clone(), t);
                    }
                    if value.is_some_and(|v| is_output(v, self.src)) {
                        class.outputs.insert(field_name.clone());
                    }
                    value.is_some_and(|v| {
                        matches!(v.kind(), "arrow_function" | "function_expression")
                    })
                }
                _ => false,
            };
            if m.kind() == "method_definition" && field_name == "constructor" {
                for (n, t) in constructor_fields(m, self.src) {
                    class.fields.insert(n, t);
                }
            }
            if !is_method {
                continue;
            }
            let id = format!("{class_id}.{}", slug(&field_name));
            let parents = std::iter::once(class_id.to_string())
                .chain(self.up())
                .collect();
            self.symbol(
                name,
                m,
                m,
                field_name,
                id,
                Inferred::new("function", format!("tree-sitter: {}", m.kind())),
                parents,
                Some(class_line),
                true,
            );
        }
        self.classes.push(class);
    }

    /// A call at module level, like `bootstrapApplication(App, appConfig)`: drawn when a block documents it, named after what it calls.
    fn call(&mut self, stmt: Node) {
        let Some(mut call) = stmt
            .named_child(0)
            .filter(|c| c.kind() == "call_expression")
        else {
            return;
        };
        // A chain is named after its head: `bootstrapApplication(…).catch(…)` → bootstrapApplication.
        while let Some(head) = call
            .child_by_field_name("function")
            .filter(|f| f.kind() == "member_expression")
            .and_then(|f| f.child_by_field_name("object"))
            .filter(|o| o.kind() == "call_expression")
        {
            call = head;
        }
        let Some(callee) = call.child_by_field_name("function") else {
            return;
        };
        let name = self
            .text(callee)
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_string();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        {
            return;
        }
        let line = stmt.start_position().row + 1;
        let key = format!("ts:{}:{line}", self.file.rel);
        self.sites.push(Site {
            key: key.clone(),
            start: line,
            end: stmt.end_position().row + 1,
            name: String::new(),
            class: None,
            callable: false,
        });
        self.facts.symbols.push(Symbol {
            display: Some(name.clone()),
            name: Some(name),
            kind: Some(Inferred::new("function", "tree-sitter: call_expression")),
            parents: self.up(),
            end: Some(stmt.end_position().row + 1),
            ..Symbol::at(key, &self.file.rel, line)
        });
    }

    /// A Playwright test or group: `test('title', …)`, `test.describe('title', …)`, and the tests inside it.
    /// Whether the statement is one.
    fn test(&mut self, stmt: Node) -> bool {
        let line = stmt.start_position().row + 1;
        let Some(title) = test_title(self.line_text(line)) else {
            return false;
        };
        let key = format!("ts:{}:{line}", self.file.rel);
        self.sites.push(Site {
            key: key.clone(),
            start: line,
            end: stmt.end_position().row + 1,
            name: String::new(),
            class: None,
            callable: false,
        });
        // Tests keep the id the build derives from file and title (`levels-spec.every-line-has-words`).
        self.facts.symbols.push(Symbol {
            draw: Draw::Always,
            display: Some(title.clone()),
            name: Some(title),
            kind: Some(Inferred::new("test", "test call")),
            docs: docs_of(stmt, self.src),
            end: Some(stmt.end_position().row + 1),
            ..Symbol::at(key, &self.file.rel, line)
        });
        // The tests inside a group's callback.
        let call = stmt.named_child(0);
        let args = call.and_then(|c| c.child_by_field_name("arguments"));
        let mut cursor = stmt.walk();
        let callbacks: Vec<Node> = args
            .map(|a| a.named_children(&mut cursor).collect())
            .unwrap_or_default();
        for cb in callbacks {
            if let Some(body) = cb.child_by_field_name("body")
                && body.kind() == "statement_block"
            {
                let mut c2 = body.walk();
                let inner: Vec<Node> = body.named_children(&mut c2).collect();
                for s in inner {
                    if s.kind() == "expression_statement" {
                        self.test(s);
                    }
                }
            }
        }
        true
    }

    /***
     * The calls the syntax makes certain: `this.save(…)` in a class reaches its method, `load(…)` the function of that name in the file.
     ***/
    fn calls(&mut self, root: Node) {
        let mut seen = std::collections::HashSet::new();
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            let mut cursor = n.walk();
            let children: Vec<Node> = n.named_children(&mut cursor).collect();
            stack.extend(children.into_iter().rev());
            if n.kind() != "call_expression" {
                continue;
            }
            let Some(f) = n.child_by_field_name("function") else {
                continue;
            };
            if let Some((field, method)) = this_field_call(f, self.src) {
                self.member_call(
                    n.start_position().row + 1,
                    field,
                    method,
                    self.text(f).to_string(),
                );
                continue;
            }
            // `x.m(…)` on anything but `this`: which class `x` is, the syntax doesn't say.
            if f.kind() == "member_expression"
                && f.child_by_field_name("object")
                    .is_some_and(|o| o.kind() != "this")
                && let Some(p) = f.child_by_field_name("property")
            {
                self.could.push((
                    n.start_position().row + 1,
                    self.text(p).to_string(),
                    self.text(f).to_string(),
                ));
                continue;
            }
            let (name, on_this) = match f.kind() {
                "member_expression"
                    if f.child_by_field_name("object")
                        .is_some_and(|o| o.kind() == "this") =>
                {
                    let Some(p) = f.child_by_field_name("property") else {
                        continue;
                    };
                    (self.text(p).to_string(), true)
                }
                "identifier" => (self.text(f).to_string(), false),
                _ => continue,
            };
            let line = n.start_position().row + 1;
            let Some(caller) = self
                .sites
                .iter()
                .filter(|s| s.start <= line && line <= s.end)
                .min_by_key(|s| s.end - s.start)
            else {
                continue;
            };
            let class = if on_this { caller.class.clone() } else { None };
            if on_this && class.is_none() {
                continue;
            }
            let mut callees = self
                .sites
                .iter()
                .filter(|s| s.callable && s.name == name && s.class == class);
            let (Some(callee), None) = (callees.next(), callees.next()) else {
                continue;
            };
            if callee.key == caller.key || !seen.insert((caller.key.clone(), callee.key.clone())) {
                continue;
            }
            let text = self.text(f).to_string();
            self.facts.relations.push(Relation {
                from: caller.key.clone(),
                ..Relation::new(
                    Verb::Calls,
                    Target::Symbol(callee.key.clone()),
                    Evidence::new(line, format!("tree-sitter: {text}(…)")),
                )
            });
        }
    }

    /// `import { App } from './app/app'` and `import App from './app'`: the names a file takes from another.
    fn import(&mut self, n: Node) {
        let Some(source) = n.child_by_field_name("source") else {
            return;
        };
        let from = unquote(self.text(source)).to_string();
        let mut stack = vec![n];
        while let Some(x) = stack.pop() {
            let mut cursor = x.walk();
            stack.extend(x.named_children(&mut cursor));
            let name = match x.kind() {
                // `{ A as B }` names B here, A there; the class is found by A.
                "import_specifier" => x.child_by_field_name("name"),
                "import_clause" => x.named_child(0).filter(|c| c.kind() == "identifier"),
                _ => None,
            };
            if let Some(name) = name {
                let local = x
                    .child_by_field_name("alias")
                    .map_or_else(|| self.text(name), |a| self.text(a));
                self.imports.insert(local.to_string(), from.clone());
            }
        }
    }

    /// The innermost declaration around a line: the caller of a call on it.
    fn site_at(&self, line: usize) -> Option<&Site> {
        self.sites
            .iter()
            .filter(|s| s.start <= line && line <= s.end)
            .min_by_key(|s| s.end - s.start)
    }

    fn class_at(&self, line: usize) -> Option<&Class> {
        self.classes
            .iter()
            .filter(|c| c.start <= line && line <= c.end)
            .min_by_key(|c| c.end - c.start)
    }

    /// `this.field.method(…)`: an output it emits, or a call on the service the field holds.
    fn member_call(&mut self, line: usize, field: String, method: String, text: String) {
        let (Some(caller), Some(class)) = (self.site_at(line), self.class_at(line)) else {
            return;
        };
        let caller = caller.key.clone();
        if method == "emit" && class.outputs.contains(&field) {
            let class = class.key.clone();
            self.emits.push(Emit {
                by: caller,
                class,
                output: field,
                line,
            });
        } else if let Some(ty) = class.fields.get(&field).cloned() {
            self.member_calls.push(MemberCall {
                caller,
                ty,
                method,
                line,
                text,
            });
        }
    }

    /***
     * Messages between windows: what a function posts with `postMessage`, and what a `message` listener calls per type.
     *
     * A function that passes its parameter to `postMessage` posts what it gets
     * (`post(message)`, `tellParent(message)`), so a call to it with
     * `{ type: 'docair:wording' }` posts that type. A listener's `if (m.type
     * === 'docair:wording') previewWording(…)` calls `previewWording` for it.
     ***/
    fn messages(&mut self, root: Node) {
        let mut calls = Vec::new();
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            let mut cursor = n.walk();
            stack.extend(n.named_children(&mut cursor));
            if n.kind() == "call_expression" {
                calls.push(n);
            }
        }

        // Who posts what they get: (name, class).
        let mut posters: Vec<(String, Option<String>)> = Vec::new();
        for &c in &calls {
            if callee_name(c, self.src) != Some("postMessage") {
                continue;
            }
            let Some(a) = arg(c, 0).filter(|a| a.kind() == "identifier") else {
                continue;
            };
            let Some(func) = enclosing_function(c) else {
                continue;
            };
            if !parameters(func, self.src).contains(&self.text(a)) {
                continue;
            }
            let line = func.start_position().row + 1;
            if let Some(site) = self.sites.iter().find(|s| s.callable && s.start == line) {
                posters.push((site.name.clone(), site.class.clone()));
            }
        }
        for &c in &calls {
            let Some(ty) = arg(c, 0).and_then(|a| message_type(a, self.src)) else {
                continue;
            };
            let Some(f) = c.child_by_field_name("function") else {
                continue;
            };
            let line = c.start_position().row + 1;
            let Some(caller) = self.site_at(line) else {
                continue;
            };
            let posted = match f.kind() {
                "member_expression" => {
                    let name = f
                        .child_by_field_name("property")
                        .map_or("", |p| self.text(p));
                    let on_this = f
                        .child_by_field_name("object")
                        .is_some_and(|o| o.kind() == "this");
                    name == "postMessage"
                        || (on_this
                            && posters
                                .iter()
                                .any(|(n, class)| n == name && *class == caller.class))
                }
                "identifier" => posters
                    .iter()
                    .any(|(n, class)| n == self.text(f) && class.is_none()),
                _ => false,
            };
            if posted {
                let by = caller.key.clone();
                self.posts.push(PostedMessage { by, ty, line });
            }
        }
        let mut listens = Vec::new();
        for &c in &calls {
            if callee_name(c, self.src) != Some("addEventListener")
                || arg(c, 0).map(|a| unquote(self.text(a))) != Some("message")
            {
                continue;
            }
            let Some(handler) = arg(c, 1).and_then(|h| self.function_of(h, root)) else {
                continue;
            };
            let mut stack = vec![handler];
            while let Some(n) = stack.pop() {
                let mut cursor = n.walk();
                stack.extend(n.named_children(&mut cursor));
                if n.kind() != "if_statement" {
                    continue;
                }
                let (Some(cond), Some(then)) = (
                    n.child_by_field_name("condition"),
                    n.child_by_field_name("consequence"),
                ) else {
                    continue;
                };
                let types = compared_types(cond, self.src);
                if types.is_empty() {
                    continue;
                }
                let line = then.start_position().row + 1;
                let class = self.class_at(line).map(|c| c.id.clone());
                let mut inner = vec![then];
                while let Some(x) = inner.pop() {
                    let mut cursor = x.walk();
                    inner.extend(x.named_children(&mut cursor));
                    if x.kind() != "call_expression" {
                        continue;
                    }
                    let Some(f) = x.child_by_field_name("function") else {
                        continue;
                    };
                    let (name, on) = match f.kind() {
                        "identifier" => (self.text(f).to_string(), None),
                        "member_expression"
                            if f.child_by_field_name("object")
                                .is_some_and(|o| o.kind() == "this") =>
                        {
                            let Some(p) = f.child_by_field_name("property") else {
                                continue;
                            };
                            (self.text(p).to_string(), class.clone())
                        }
                        _ => continue,
                    };
                    let Some(site) = self
                        .sites
                        .iter()
                        .find(|s| s.callable && s.name == name && s.class == on)
                    else {
                        continue;
                    };
                    for ty in &types {
                        listens.push(MessageHandler {
                            handler: site.key.clone(),
                            name: name.clone(),
                            ty: ty.clone(),
                        });
                    }
                }
            }
        }
        self.listens.extend(listens);
    }

    /// The function a listener is: written in place, or a `const onMessage = (e) => …` named there.
    fn function_of<'t>(&self, h: Node<'t>, root: Node<'t>) -> Option<Node<'t>> {
        if matches!(
            h.kind(),
            "arrow_function" | "function_expression" | "function"
        ) {
            return Some(h);
        }
        if h.kind() != "identifier" {
            return None;
        }
        let name = self.text(h);
        let before = h.start_byte();
        // The nearest declaration of that name before the listener.
        let mut found: Option<(usize, Node)> = None;
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            let mut cursor = n.walk();
            stack.extend(n.named_children(&mut cursor));
            if n.kind() == "variable_declarator"
                && n.start_byte() < before
                && n.child_by_field_name("name")
                    .is_some_and(|x| self.text(x) == name)
                && let Some(v) = n.child_by_field_name("value")
                && matches!(v.kind(), "arrow_function" | "function_expression")
                && found.is_none_or(|(at, _)| at < n.start_byte())
            {
                found = Some((n.start_byte(), v));
            }
        }
        found.map(|(_, v)| v)
    }

    /// What the rules found, on the declaration that holds it.
    fn found(&mut self, rules: &RuleSet) {
        for found in rules.apply(&self.file.text) {
            let site = match found.about {
                About::Line(line) => self.sites.iter().find(|s| s.start == line),
                About::Within(line) => self
                    .sites
                    .iter()
                    .filter(|s| s.start <= line && line <= s.end)
                    .min_by_key(|s| s.end - s.start),
                About::Name(_) => None,
            };
            let Some(site) = site else {
                continue;
            };
            // `where:` asks for a field's type: `this.http` must be an HttpClient.
            let class = self
                .classes
                .iter()
                .filter(|c| c.start <= found.line && found.line <= c.end)
                .min_by_key(|c| c.end - c.start);
            let typed = found.requires.iter().all(|(field, ty)| {
                class.is_some_and(|c| c.fields.get(field).is_some_and(|t| t == ty))
            });
            if !typed {
                continue;
            }
            let by = site.key.clone();
            let evidence = Evidence::new(found.line, found.why.clone());
            match found.fact {
                Fact::Requests { at, reply } => self.facts.requests.push(Request {
                    by,
                    at,
                    payload: None,
                    reply,
                    evidence,
                }),
                Fact::Serves(at) => self.facts.serves.push(Serve { by, at, evidence }),
                Fact::Calls { to, .. } => self.named.push(NamedCall {
                    by,
                    name: to,
                    line: evidence.line,
                    why: evidence.why,
                }),
                // A file: the block names the store (docs/ADAPTERS.md §5.7).
                Fact::Access { write, path } => {
                    let verb = if write { Verb::Writes } else { Verb::Reads };
                    let at = path
                        .filter(|p| {
                            ["'", "\"", "`"]
                                .iter()
                                .any(|q| evidence.why.contains(&format!("{q}{p}{q}")))
                        })
                        .unwrap_or_default();
                    self.facts.relations.push(Relation {
                        from: by,
                        ..Relation::new(verb, Target::Ref(format!("file:{at}")), evidence)
                    });
                }
            }
        }
    }
}

/***
 * Find the calls across files: on injected services, from Angular outputs to the handlers templates bind, through `postMessage`, and to names rules found.
 *
 * A type or name resolves through the file's imports, then to the one class
 * or function of that name in the project. A line through messages reaches
 * one of several parts when listeners in several files take that type.
 ***/
/// More candidates than this for a call by name: no could lines.
const MAX_CANDIDATES: usize = 5;

/// Method names the platform's own objects have (arrays, strings, maps, promises, the DOM,
/// observables, the console): a call by such a name is far more often theirs.
const JS_STD_METHODS: &[&str] = &[
    "add",
    "addEventListener",
    "all",
    "append",
    "appendChild",
    "apply",
    "assign",
    "at",
    "bind",
    "call",
    "catch",
    "clear",
    "click",
    "close",
    "closest",
    "complete",
    "concat",
    "contains",
    "debug",
    "delete",
    "dispatchEvent",
    "emit",
    "endsWith",
    "entries",
    "error",
    "every",
    "exec",
    "fill",
    "filter",
    "finally",
    "find",
    "findIndex",
    "flat",
    "flatMap",
    "focus",
    "forEach",
    "freeze",
    "from",
    "get",
    "getAttribute",
    "getBoundingClientRect",
    "getElementById",
    "has",
    "includes",
    "indexOf",
    "info",
    "join",
    "json",
    "keys",
    "log",
    "map",
    "match",
    "matches",
    "max",
    "min",
    "next",
    "now",
    "open",
    "padStart",
    "parse",
    "pipe",
    "pop",
    "postMessage",
    "preventDefault",
    "push",
    "querySelector",
    "querySelectorAll",
    "race",
    "reduce",
    "reject",
    "remove",
    "removeEventListener",
    "replace",
    "resolve",
    "reverse",
    "round",
    "scrollIntoView",
    "send",
    "set",
    "setAttribute",
    "shift",
    "slice",
    "some",
    "sort",
    "splice",
    "split",
    "startsWith",
    "stopPropagation",
    "stringify",
    "subscribe",
    "test",
    "text",
    "then",
    "toFixed",
    "toggle",
    "toLowerCase",
    "toString",
    "toUpperCase",
    "trim",
    "unshift",
    "update",
    "values",
    "warn",
    "write",
];

pub fn link(files: Vec<FileRead>, html: &HashMap<String, String>) -> Facts {
    link_with_index(files, html, None)
}

/***
 * As `link`, and with a code index the calls it knows: from the function or method a reference is in to the one its symbol is defined as, certain.
 ***/
pub fn link_with_index(
    files: Vec<FileRead>,
    html: &HashMap<String, String>,
    index: Option<&docair_scip::Index>,
) -> Facts {
    let mut extra: Vec<Relation> = Vec::new();
    if let Some(index) = index {
        let site_at: HashMap<(&str, usize), &str> = files
            .iter()
            .flat_map(|f| {
                f.sites
                    .iter()
                    .filter(|s| s.callable)
                    .map(move |s| ((f.rel.as_str(), s.start), s.key.as_str()))
            })
            .collect();
        let mut seen = HashSet::new();
        for f in files.iter().filter(|f| index.covers(&f.rel)) {
            for r in index.calls(&f.rel) {
                let Some(caller) = f
                    .sites
                    .iter()
                    .filter(|s| s.start <= r.line && r.line <= s.end)
                    .min_by_key(|s| s.end - s.start)
                else {
                    continue;
                };
                let Some(to) = index
                    .definition(&r.symbol)
                    .and_then(|(path, line)| site_at.get(&(path, line)))
                else {
                    continue;
                };
                if *to == caller.key || !seen.insert((caller.key.clone(), to.to_string())) {
                    continue;
                }
                extra.push(Relation {
                    from: caller.key.clone(),
                    ..Relation::new(
                        Verb::Calls,
                        Target::Symbol(to.to_string()),
                        Evidence::new(
                            r.line,
                            format!("scip-typescript: {}", docair_scip::short(&r.symbol)),
                        ),
                    )
                });
            }
        }
    }
    let mut out = link_inner(files, html);
    // The index's calls first: a call the syntax also found is the same line.
    let known: HashSet<(String, String)> = out
        .relations
        .iter()
        .filter(|r| r.verb == Verb::Calls)
        .filter_map(|r| match &r.to {
            Target::Symbol(t) => Some((r.from.clone(), t.clone())),
            _ => None,
        })
        .collect();
    out.relations
        .extend(extra.into_iter().filter(|r| match &r.to {
            Target::Symbol(t) => !known.contains(&(r.from.clone(), t.clone())),
            _ => true,
        }));
    out
}

fn link_inner(files: Vec<FileRead>, html: &HashMap<String, String>) -> Facts {
    let mut out = Facts::default();
    let rels: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.rel.as_str(), i))
        .collect();
    let mut by_name: HashMap<&str, Vec<(usize, usize)>> = HashMap::new();
    let mut selectors: HashMap<&str, (usize, usize)> = HashMap::new();
    for (fi, f) in files.iter().enumerate() {
        for (ci, c) in f.classes.iter().enumerate() {
            by_name.entry(c.name.as_str()).or_default().push((fi, ci));
            if let Some(sel) = &c.selector {
                selectors.insert(sel.as_str(), (fi, ci));
            }
        }
    }
    // The file an import names: `./app/app` from `web/src/main.ts` → `web/src/app/app.ts`.
    let imported = |fi: usize, name: &str| -> Option<usize> {
        let spec = files[fi].imports.get(name)?;
        if !spec.starts_with('.') {
            return None;
        }
        let dir = files[fi].rel.rsplit_once('/').map_or("", |(d, _)| d);
        let base = normalize(&Path::new(dir).join(spec))?;
        ["", ".ts", ".tsx", ".js", ".mjs", "/index.ts", "/index.js"]
            .iter()
            .find_map(|ext| rels.get(format!("{base}{ext}").as_str()).copied())
    };
    let class_named = |fi: usize, name: &str| -> Option<(usize, usize)> {
        let at = |f: usize| {
            files[f]
                .classes
                .iter()
                .position(|c| c.name == name)
                .map(|ci| (f, ci))
        };
        if let Some(f) = imported(fi, name) {
            return at(f);
        }
        at(fi).or_else(|| match by_name.get(name).map(Vec::as_slice) {
            Some([one]) => Some(*one),
            _ => None,
        })
    };
    let method = |(fi, ci): (usize, usize), name: &str| -> Option<&Site> {
        let id = files[fi].classes[ci].id.as_str();
        files[fi]
            .sites
            .iter()
            .find(|s| s.callable && s.name == name && s.class.as_deref() == Some(id))
    };
    let template = |(fi, ci): (usize, usize)| -> Option<(&str, &str)> {
        match files[fi].classes[ci].template.as_ref()? {
            TemplateSource::Inline(t) => Some((t.as_str(), files[fi].rel.as_str())),
            TemplateSource::File(rel) => html.get(rel).map(|t| (t.as_str(), rel.as_str())),
        }
    };
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut lines: Vec<Relation> = Vec::new();
    let mut line = |from: &str, to: &str, evidence: Evidence, reach: Option<Reach>| {
        if from != to && seen.insert((from.to_string(), to.to_string())) {
            let r = Relation {
                from: from.to_string(),
                ..Relation::new(Verb::Calls, Target::Symbol(to.to_string()), evidence)
            };
            lines.push(match reach {
                Some(reach) => r.reach(reach),
                None => r,
            });
        }
    };
    for (fi, f) in files.iter().enumerate() {
        for m in &f.member_calls {
            let Some(callee) = class_named(fi, &m.ty).and_then(|c| method(c, &m.method)) else {
                continue;
            };
            let why = format!("tree-sitter: {}(…) on a {}", m.text, m.ty);
            line(&m.caller, &callee.key, Evidence::new(m.line, why), None);
        }
        for (ci, parent) in f.classes.iter().enumerate() {
            let Some((text, file)) = template((fi, ci)) else {
                continue;
            };
            for (tag, attrs) in elements(text) {
                let Some(&child) = selectors.get(tag.as_str()) else {
                    continue;
                };
                let c = &files[child.0].classes[child.1];
                for (attr, expr) in &attrs {
                    let Some(event) = attr.strip_prefix('(').and_then(|a| a.strip_suffix(')'))
                    else {
                        continue;
                    };
                    if !c.outputs.contains(event) {
                        continue;
                    }
                    let handlers: Vec<&Site> = called_names(expr)
                        .iter()
                        .filter_map(|n| method((fi, ci), n))
                        .collect();
                    let mut emitters: Vec<(&str, usize)> = files[child.0]
                        .emits
                        .iter()
                        .filter(|e| e.class == c.key && e.output == event)
                        .map(|e| (e.by.as_str(), e.line))
                        .collect();
                    let in_template = template(child).is_some_and(|(t, _)| emits_in(t, event));
                    if emitters.is_empty() || in_template {
                        emitters.push((c.key.as_str(), c.line));
                    }
                    for (from, at) in emitters {
                        for h in &handlers {
                            let why = format!(
                                "Angular: ({event})=\"{expr}\" on <{tag}> in {}",
                                file.rsplit('/').next().unwrap_or(file)
                            );
                            line(from, &h.key, Evidence::new(at, why), None);
                        }
                    }
                }
                let _ = parent;
            }
        }
        for n in &f.named {
            let target = class_named(fi, &n.name)
                .map(|(f2, ci)| files[f2].classes[ci].key.clone())
                .or_else(|| {
                    let callable = |f2: usize| {
                        files[f2]
                            .sites
                            .iter()
                            .find(|s| s.callable && s.class.is_none() && s.name == n.name)
                    };
                    imported(fi, &n.name)
                        .and_then(callable)
                        .or_else(|| callable(fi))
                        .map(|s| s.key.clone())
                });
            if let Some(to) = target {
                line(&n.by, &to, Evidence::new(n.line, n.why.clone()), None);
            }
        }
    }
    let mut listeners: HashMap<&str, Vec<(usize, &MessageHandler)>> = HashMap::new();
    for (fi, f) in files.iter().enumerate() {
        for l in &f.listens {
            listeners.entry(l.ty.as_str()).or_default().push((fi, l));
        }
    }
    for (fi, f) in files.iter().enumerate() {
        for p in &f.posts {
            // A page posts to another window: a listener in the same file is not where it goes.
            let to: Vec<&(usize, &MessageHandler)> = listeners
                .get(p.ty.as_str())
                .into_iter()
                .flatten()
                .filter(|(f2, _)| *f2 != fi)
                .collect();
            let several = to.iter().map(|(f2, _)| f2).collect::<HashSet<_>>().len() > 1;
            for (f2, l) in to {
                let why = format!(
                    "postMessage {{ type: '{}' }}: the message listener in {} calls {}",
                    p.ty,
                    files[*f2].rel.rsplit('/').next().unwrap_or(&files[*f2].rel),
                    l.name
                );
                let reach = several.then_some(Reach::OneOf);
                line(&p.by, &l.handler, Evidence::new(p.line, why), reach);
            }
        }
    }
    // Could lines: `x.m(…)` could reach each class method of that name, unless the platform has it too.
    let mut methods: HashMap<&str, Vec<&Site>> = HashMap::new();
    for f in &files {
        for site in f.sites.iter().filter(|s| s.callable && s.class.is_some()) {
            methods.entry(site.name.as_str()).or_default().push(site);
        }
    }
    for f in &files {
        for (at, m, text) in &f.could {
            if JS_STD_METHODS.contains(&m.as_str()) {
                continue;
            }
            let Some(caller) = f
                .sites
                .iter()
                .filter(|s| s.start <= *at && *at <= s.end)
                .min_by_key(|s| s.end - s.start)
            else {
                continue;
            };
            let to: Vec<&&Site> = methods
                .get(m.as_str())
                .into_iter()
                .flatten()
                .filter(|s| s.key != caller.key)
                .collect();
            if (1..=MAX_CANDIDATES).contains(&to.len()) {
                for site in to {
                    line(
                        &caller.key,
                        &site.key,
                        Evidence::new(*at, format!("tree-sitter: {text}(…), by its name")),
                        Some(Reach::Could),
                    );
                }
            }
        }
    }
    for f in files {
        out.extend(f.facts);
    }
    out.relations.extend(lines);
    out
}

/// The start tags of an HTML template with their attributes: `<app-chat-panel (closed)="reset()">`.
fn elements(html: &str) -> Vec<(String, Vec<(String, String)>)> {
    let b = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' || !b.get(i + 1).is_some_and(u8::is_ascii_alphabetic) {
            i += 1;
            continue;
        }
        i += 1;
        let start = i;
        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-') {
            i += 1;
        }
        let tag = html[start..i].to_ascii_lowercase();
        let mut attrs = Vec::new();
        loop {
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= b.len() || b[i] == b'>' || b[i] == b'/' {
                break;
            }
            let name_start = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>') {
                i += 1;
            }
            let name = html[name_start..i].to_string();
            let mut value = String::new();
            if b.get(i) == Some(&b'=') {
                i += 1;
                if let Some(&q) = b.get(i).filter(|q| matches!(q, b'"' | b'\'')) {
                    let v = i + 1;
                    i = v + html[v..].find(q as char).unwrap_or(html.len() - v);
                    value = html[v..i].to_string();
                    i += 1;
                } else {
                    let v = i;
                    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                        i += 1;
                    }
                    value = html[v..i].to_string();
                }
            }
            if name.is_empty() {
                i += 1;
            } else {
                attrs.push((name, value));
            }
        }
        out.push((tag, attrs));
    }
    out
}

/// The functions an event binding calls: `ask($event)` → ask; `chatOpen.set(false)` calls none of this component's.
fn called_names(expr: &str) -> Vec<String> {
    let b = expr.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$';
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !ident(b[i]) || (i > 0 && (ident(b[i - 1]) || b[i - 1] == b'.')) {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && ident(b[i]) {
            i += 1;
        }
        let mut j = i;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if b.get(j) == Some(&b'(') && !b[start].is_ascii_digit() {
            out.push(expr[start..i].to_string());
        }
    }
    out
}

/// Whether a template emits an output itself: `(click)="closed.emit()"`.
fn emits_in(template: &str, output: &str) -> bool {
    let needle = format!("{output}.emit(");
    template.match_indices(&needle).any(|(at, _)| {
        template[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '.' || c == '$'))
    })
}

/// The `i`th argument of a call.
fn arg<'t>(call: Node<'t>, i: u32) -> Option<Node<'t>> {
    call.child_by_field_name("arguments")?.named_child(i)
}

/// `'docair:wording'` → `docair:wording`.
fn unquote(s: &str) -> &str {
    s.trim_matches(|c| matches!(c, '\'' | '"' | '`'))
}

/// The name a call calls: `postMessage` of `w.postMessage(…)`, `f` of `f(…)`.
fn callee_name<'s>(call: Node, src: &'s [u8]) -> Option<&'s str> {
    let f = call.child_by_field_name("function")?;
    let n = match f.kind() {
        "member_expression" => f.child_by_field_name("property")?,
        "identifier" => f,
        _ => return None,
    };
    n.utf8_text(src).ok()
}

/// `this.field.method` of `this.field.method(…)`.
fn this_field_call(f: Node, src: &[u8]) -> Option<(String, String)> {
    if f.kind() != "member_expression" {
        return None;
    }
    let object = f
        .child_by_field_name("object")
        .filter(|o| o.kind() == "member_expression")?;
    if object.child_by_field_name("object")?.kind() != "this" {
        return None;
    }
    let field = object
        .child_by_field_name("property")?
        .utf8_text(src)
        .ok()?;
    let method = f.child_by_field_name("property")?.utf8_text(src).ok()?;
    Some((field.to_string(), method.to_string()))
}

/// The function, method or arrow a node is written in.
fn enclosing_function(n: Node) -> Option<Node> {
    let mut at = n.parent();
    while let Some(p) = at {
        if matches!(
            p.kind(),
            "method_definition"
                | "function_declaration"
                | "function_expression"
                | "arrow_function"
                | "generator_function_declaration"
        ) {
            return Some(p);
        }
        at = p.parent();
    }
    None
}

/// The names of a function's parameters: `(message: object)` → message.
fn parameters<'s>(func: Node, src: &'s [u8]) -> Vec<&'s str> {
    let Some(params) = func
        .child_by_field_name("parameters")
        .or_else(|| func.child_by_field_name("parameter"))
    else {
        return Vec::new();
    };
    if params.kind() == "identifier" {
        return params.utf8_text(src).ok().into_iter().collect();
    }
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .filter_map(|p| {
            let name = if p.kind() == "identifier" {
                p
            } else {
                p.child_by_field_name("pattern")?
            };
            name.utf8_text(src).ok()
        })
        .collect()
}

/// The `type` of a message written in place: `{ type: 'docair:wording', items }` → docair:wording.
fn message_type(obj: Node, src: &[u8]) -> Option<String> {
    if obj.kind() != "object" {
        return None;
    }
    let mut cursor = obj.walk();
    let pairs: Vec<Node> = obj.named_children(&mut cursor).collect();
    pairs.into_iter().find_map(|p| {
        let key = p.child_by_field_name("key")?;
        let value = p.child_by_field_name("value")?;
        let value_text = value.utf8_text(src).ok()?;
        (p.kind() == "pair"
            && unquote(key.utf8_text(src).ok()?) == "type"
            && value.kind() == "string")
            .then(|| unquote(value_text).to_string())
    })
}

/// The types a condition compares a message's type to: `m.type === 'docair:view'`, `m?.type == "x"`.
fn compared_types(cond: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![cond];
    while let Some(n) = stack.pop() {
        let mut cursor = n.walk();
        stack.extend(n.named_children(&mut cursor));
        if n.kind() != "binary_expression" {
            continue;
        }
        let op = n
            .child_by_field_name("operator")
            .and_then(|o| o.utf8_text(src).ok());
        if !matches!(op, Some("===" | "==")) {
            continue;
        }
        let (Some(l), Some(r)) = (
            n.child_by_field_name("left"),
            n.child_by_field_name("right"),
        ) else {
            continue;
        };
        let is_type = |x: Node| {
            x.kind() == "member_expression"
                && x.child_by_field_name("property")
                    .and_then(|p| p.utf8_text(src).ok())
                    == Some("type")
        };
        for (a, b) in [(l, r), (r, l)] {
            if is_type(a)
                && b.kind() == "string"
                && let Ok(t) = b.utf8_text(src)
            {
                out.push(unquote(t).to_string());
            }
        }
    }
    out
}

/// Whether a field is an Angular output: `output<string>()`, `output.required()`, `new EventEmitter()`.
fn is_output(value: Node, src: &[u8]) -> bool {
    let callee = match value.kind() {
        "call_expression" => value.child_by_field_name("function"),
        "new_expression" => value.child_by_field_name("constructor"),
        _ => None,
    };
    let Some(name) = callee.and_then(|c| c.utf8_text(src).ok()) else {
        return false;
    };
    matches!(name, "output" | "output.required" | "EventEmitter")
}

/// The string properties of a decorator's object: `@Component({ selector: 'x', templateUrl: './x.html' })`.
fn decorator_props(dec: Node, src: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut stack = vec![dec];
    while let Some(n) = stack.pop() {
        if n.kind() == "pair" {
            if let (Some(k), Some(v)) =
                (n.child_by_field_name("key"), n.child_by_field_name("value"))
                && matches!(v.kind(), "string" | "template_string")
                && let (Ok(k), Ok(v)) = (k.utf8_text(src), v.utf8_text(src))
            {
                let v = v
                    .strip_prefix(['\'', '"', '`'])
                    .and_then(|v| v.strip_suffix(['\'', '"', '`']))
                    .unwrap_or(v);
                out.insert(unquote(k).to_string(), v.to_string());
            }
            continue;
        }
        let mut cursor = n.walk();
        stack.extend(n.named_children(&mut cursor));
    }
    out
}

/// The Angular decorator a class is made with, if any: `Component`, `Injectable`, …
fn angular_decorator(text: &str) -> Option<&'static str> {
    const ANGULAR: &[&str] = &["Component", "Injectable", "Directive", "Pipe", "NgModule"];
    let name = text
        .trim_start_matches('@')
        .split(|c: char| !c.is_alphanumeric())
        .next()?;
    ANGULAR.iter().copied().find(|a| *a == name)
}

/// `Observable<X>` → `Observable`, `api.Client` → `Client`.
fn base_type(t: &str) -> String {
    let t = t.trim().trim_start_matches(':').trim();
    let t = t.split('<').next().unwrap_or(t);
    t.rsplit('.').next().unwrap_or(t).trim().to_string()
}

/// A field's type: its annotation, or the class it is injected as (`inject(HttpClient)`).
fn field_type(field: Node, value: Option<Node>, src: &[u8]) -> Option<String> {
    if let Some(t) = field.child_by_field_name("type") {
        return Some(base_type(t.utf8_text(src).ok()?));
    }
    let v = value?;
    if v.kind() != "call_expression" {
        return None;
    }
    let f = v.child_by_field_name("function")?;
    if f.utf8_text(src).ok()? != "inject" {
        return None;
    }
    let arg = v.child_by_field_name("arguments")?.named_child(0)?;
    Some(base_type(arg.utf8_text(src).ok()?))
}

/// Constructor parameters that are fields: `constructor(private http: HttpClient)`.
fn constructor_fields(ctor: Node, src: &[u8]) -> Vec<(String, String)> {
    let Some(params) = ctor.child_by_field_name("parameters") else {
        return Vec::new();
    };
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .filter_map(|p| {
            let mut c = p.walk();
            let is_field = p
                .children(&mut c)
                .any(|ch| matches!(ch.kind(), "accessibility_modifier" | "readonly"));
            if !is_field {
                return None;
            }
            let name = p.child_by_field_name("pattern")?.utf8_text(src).ok()?;
            let ty = p.child_by_field_name("type")?.utf8_text(src).ok()?;
            Some((name.to_string(), base_type(ty)))
        })
        .collect()
}

/// The JSDoc (`/** … */`) right above a declaration, without its comment marks; docair blocks (`/***`) are not docs.
fn docs_of(node: Node, src: &[u8]) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut sib = node.prev_named_sibling();
    while let Some(s) = sib {
        if s.kind() != "comment" {
            break;
        }
        let text = s.utf8_text(src).unwrap_or("");
        if text.starts_with("/***") {
            sib = s.prev_named_sibling();
            continue;
        }
        if !text.starts_with("/**") || text == "/**/" {
            break;
        }
        let body = text[3..].trim_end_matches("*/");
        for l in body.lines().rev() {
            let l = l.trim();
            let l = l.strip_prefix('*').unwrap_or(l);
            lines.push(l.strip_prefix(' ').unwrap_or(l).to_string());
        }
        sib = s.prev_named_sibling();
    }
    lines.reverse();
    let docs = lines.join("\n").trim().to_string();
    (!docs.is_empty()).then_some(docs)
}

/***
 * Find the package a file belongs to: the nearest `package.json` with a name, else the nearest Cargo package (a crate's web assets), never above the project's root.
 *
 * @reads project-files.package.json "reads package.json and Cargo.toml files to name the package"
 ***/
pub fn package_of(path: &Path, root: &Path) -> Option<(String, PathBuf)> {
    let mut dir = path.parent();
    while let Some(d) = dir.filter(|d| d.starts_with(root)) {
        if let Ok(text) = std::fs::read_to_string(d.join("package.json"))
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
            && let Some(name) = value.get("name").and_then(|n| n.as_str())
        {
            return Some((
                name.trim_start_matches('@').replace('/', "-"),
                d.to_path_buf(),
            ));
        }
        if let Ok(text) = std::fs::read_to_string(d.join("Cargo.toml"))
            && let Ok(value) = text.parse::<toml::Table>()
            && let Some(name) = value
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
        {
            return Some((name.to_string(), d.to_path_buf()));
        }
        dir = d.parent();
    }
    None
}

/// The module id of a file: the package, then its path inside the package without `src` and the extension.
/// `web/src/app/scan.service.ts` → `docair-web.app.scan-service`.
fn module_id(rel: &str, path: &Path, package: Option<&(String, PathBuf)>) -> String {
    let in_package: Vec<String> = match package.and_then(|(_, root)| path.strip_prefix(root).ok()) {
        Some(p) => p.iter().map(|c| c.to_string_lossy().into_owned()).collect(),
        None => rel.split('/').map(str::to_string).collect(),
    };
    let mut segs: Vec<String> = in_package
        .iter()
        .enumerate()
        .filter(|(i, s)| !(*i == 0 && s.as_str() == "src"))
        .map(|(_, s)| s.clone())
        .collect();
    if let Some(last) = segs.last_mut() {
        *last = last
            .rsplit_once('.')
            .map_or(last.clone(), |(stem, _)| stem.to_string());
    }
    let mut ids: Vec<String> = package.iter().map(|(n, _)| slug(n)).collect();
    ids.extend(segs.iter().map(|s| slug(s)).filter(|s| !s.is_empty()));
    ids.join(".")
}

/// The title of a `test(…)`, `it(…)` or `describe(…)` call, with modifiers like `.only` or `.describe`.
pub fn test_title(text: &str) -> Option<String> {
    let t = text.trim_start();
    let head = ["test", "it", "describe"]
        .into_iter()
        .find(|h| t.starts_with(h))?;
    let mut rest = &t[head.len()..];
    while let Some(r) = rest.strip_prefix('.') {
        let word: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        if ![
            "only", "skip", "describe", "serial", "parallel", "fixme", "fail", "slow",
        ]
        .contains(&word.as_str())
        {
            return None;
        }
        rest = &r[word.len()..];
    }
    let rest = rest.trim_start().strip_prefix('(')?.trim_start();
    let quote = rest
        .chars()
        .next()
        .filter(|c| matches!(c, '\'' | '"' | '`'))?;
    let body = &rest[1..];
    let end = body.find(quote)?;
    Some(body[..end].to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use docair_core::extract::extract_text;
    use docair_core::facts::{Meet, Target};
    use docair_core::lang::{Family, language_for};
    use docair_core::model::Origin;

    fn source(rel: &str, text: &str) -> SourceFile {
        SourceFile {
            path: PathBuf::from("/nonexistent").join(rel),
            rel: rel.to_string(),
            lang: language_for("ts").unwrap(),
            text: text.to_string(),
        }
    }

    fn rules_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("rules")
    }

    /// Every rule in `rules/` finds exactly what its fixture expects (docs/ADAPTERS.md §9.1).
    #[test]
    fn every_rule_passes_its_fixture() {
        let failures = docair_rules::check_fixtures(&rules_dir(), "ts", "//>", |yaml, src| {
            let set = RuleSet::load(grammar(false), &[("rule.yaml", yaml)])?;
            let facts = facts_with(&source("fixture.ts", src), &set);
            let shown = |key: &str| {
                facts
                    .symbols
                    .iter()
                    .find(|s| s.key == key)
                    .and_then(|s| s.display.clone())
                    .unwrap_or_default()
            };
            let requests = facts.requests.iter().map(|r| {
                let reply = r
                    .reply
                    .as_deref()
                    .map(|t| format!(" -> {t}"))
                    .unwrap_or_default();
                match &r.at {
                    Meet::Http(_) => format!("requests http {}{reply} by {}", r.at, shown(&r.by)),
                    Meet::Topic(_) | Meet::Env(_) => {
                        format!("requests {}{reply} by {}", r.at, shown(&r.by))
                    }
                }
            });
            let serves = facts
                .serves
                .iter()
                .map(|s| format!("serves {} by {}", s.at, shown(&s.by)));
            let relations = facts.relations.iter().map(|r| {
                let to = match &r.to {
                    Target::Symbol(k) => shown(k),
                    Target::Ref(id) => id.clone(),
                };
                format!("{} {to} from {}", r.verb.tag(), shown(&r.from))
            });
            Ok(requests.chain(serves).chain(relations).collect())
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
        assert_eq!(rules(false).ids().count(), RULES.len());
        assert_eq!(rules(true).ids().count(), RULES.len());
    }

    /// `x.m(…)` of unknown type could reach each class method of that name; platform names give nothing (docair-4wk.31).
    #[test]
    fn a_method_of_unknown_type_could_be_each_of_that_name() {
        let text = "export class Orders {\n  save(o: object) {}\n}\nexport class Carts {\n  save(c: object) {}\n}\nexport function run(x: any) {\n  x.save({});\n  x.map((y: any) => y);\n}\n";
        let facts = facts_with(&source("src/app/run.ts", text), rules(false));
        let names: HashMap<String, String> = facts
            .symbols
            .iter()
            .map(|s| (s.key.clone(), s.id.clone().unwrap_or_default()))
            .collect();
        let could: Vec<String> = facts
            .relations
            .iter()
            .filter(|r| r.reach == Reach::Could)
            .map(|r| match &r.to {
                Target::Symbol(k) => format!(
                    "{} -> {}",
                    names[&r.from],
                    names.get(k).cloned().unwrap_or_default()
                ),
                t => format!("{t:?}"),
            })
            .collect();
        assert_eq!(could.len(), 2, "{could:?}");
        assert!(
            could
                .iter()
                .all(|c| c.contains("run ->") && c.ends_with(".save")),
            "{could:?}"
        );
    }

    /// What a block documents, and what is drawn without one.
    #[test]
    fn declarations_tests_and_their_ids() {
        let text = "/** The cart. */\n@Component({ selector: 'cart' })\nexport class Cart {\n  private readonly http = inject(HttpClient);\n  items: string[] = [];\n  /** Total, in cents. */\n  total(): number { return 1; }\n  reset = () => this.items.splice(0);\n}\n\nexport interface Order { id: string }\nexport const LIMIT = 5;\nconst helper = (a: number) => a;\nfunction nested() { function inner() {} }\n\n/+++ A documented test. +++/\ntest.describe('cart', () => {\n  test('adds up', async () => {});\n});\n"
            .replace("+++", "***");
        let file = source("src/app/cart.ts", &text);
        let mut d = Vec::new();
        let blocks = extract_text(&file.rel, &file.text, Family::CStyle, &mut d);
        let facts = facts_with(&file, rules(false));
        let blocks = docair_core::merge::merge(&[file], vec![blocks], &[facts], &mut d);
        let drawn: Vec<_> = blocks[0]
            .iter()
            .map(|b| {
                (
                    b.hints.symbol.clone().unwrap_or_default(),
                    b.hints.id.clone().unwrap_or_default(),
                    b.origin,
                )
            })
            .collect();
        let has = |sym: &str, id: &str, origin: Origin| {
            assert!(
                drawn
                    .iter()
                    .any(|(s, i, o)| s == sym && i == id && *o == origin),
                "{sym} {id} {origin:?} not in {drawn:#?}"
            );
        };
        has("cart", "", Origin::Authored);
        has("adds up", "", Origin::Discovered);
        has("class Cart", "app.cart.cart", Origin::Discovered);
        has("total", "app.cart.cart.total", Origin::Discovered);
        has("reset", "app.cart.cart.reset", Origin::Discovered);
        has("interface Order", "app.cart.order", Origin::Discovered);
        has("const LIMIT", "app.cart.limit", Origin::Discovered);
        has("const helper", "app.cart.helper", Origin::Discovered);
        has("function nested", "app.cart.nested", Origin::Discovered);
        has("src/app/cart.ts", "app.cart", Origin::Discovered);
        assert!(
            !drawn
                .iter()
                .any(|(s, _, _)| s.contains("inner") || s == "items" || s == "http")
        );
        let cart = blocks[0]
            .iter()
            .find(|b| b.hints.symbol.as_deref() == Some("class Cart"))
            .unwrap();
        assert_eq!(cart.summary, "The cart.");
        assert!(
            cart.tags
                .iter()
                .any(|t| t.name == "kind" && t.reason.as_deref() == Some("Angular @Component"))
        );
    }

    /// Calls across files: an injected service, an Angular output bound in the
    /// parent's template, a message posted to a page, the bootstrapped component.
    #[test]
    fn calls_across_files() {
        let files = [
            (
                "web/src/app/scan.service.ts",
                "export class ScanService {\n  scan(files: File[]) {\n    return files;\n  }\n}\n",
            ),
            (
                "web/src/app/panel.ts",
                "@Component({ selector: 'app-panel', template: `<button (click)=\"closed.emit()\">x</button>` })\nexport class Panel {\n  readonly asked = output<string>();\n  readonly closed = output<void>();\n  send(q: string) {\n    this.asked.emit(q);\n  }\n}\n",
            ),
            (
                "web/src/app/app.ts",
                "import { ScanService } from './scan.service';\n@Component({ selector: 'app-root', templateUrl: './app.html' })\nexport class App {\n  private readonly scanner = inject(ScanService);\n  generate(files: File[]) {\n    this.scanner.scan(files);\n    this.post({ type: 'page:show', files });\n  }\n  ask(q: string) {}\n  reset() {}\n  private post(message: object) {\n    this.frame.contentWindow.postMessage(message, '*');\n  }\n}\n",
            ),
            (
                "web/src/main.ts",
                "import { App } from './app/app';\nbootstrapApplication(App, appConfig);\n",
            ),
            (
                "page/page.js",
                "function show(files) {}\nfunction wire() {\n  window.addEventListener('message', (e) => {\n    const m = e.data;\n    if (m && m.type === 'page:show') show(m.files);\n  });\n}\n",
            ),
        ];
        let html = HashMap::from([(
            "web/src/app/app.html".to_string(),
            "<main>\n  <app-panel (asked)=\"ask($event)\" (closed)=\"reset()\" />\n</main>\n"
                .to_string(),
        )]);
        let read = files
            .iter()
            .map(|(rel, text)| read_file(&source(rel, text), rules(false)))
            .collect();
        let facts = link(read, &html);
        let name = |key: &str| {
            facts
                .symbols
                .iter()
                .find(|s| s.key == key)
                .and_then(|s| s.id.clone().or(s.display.clone()))
                .unwrap_or_else(|| key.to_string())
        };
        let mut calls: Vec<String> = facts
            .relations
            .iter()
            .filter_map(|r| match &r.to {
                Target::Symbol(to) => Some(format!("{} -> {}", name(&r.from), name(to))),
                Target::Ref(_) => None,
            })
            .collect();
        calls.sort();
        assert_eq!(
            calls,
            [
                "bootstrapApplication -> web.src.app.app.app",
                // In the same file: the listener calls show.
                "page.page.wire -> page.page.show",
                // Through postMessage to the page's listener.
                "web.src.app.app.app.generate -> page.page.show",
                "web.src.app.app.app.generate -> web.src.app.app.app.post",
                // On the injected service.
                "web.src.app.app.app.generate -> web.src.app.scan-service.scan-service.scan",
                // Outputs: emitted in the template, and in a method.
                "web.src.app.panel.panel -> web.src.app.app.app.reset",
                "web.src.app.panel.panel.send -> web.src.app.app.app.ask",
            ]
        );
    }

    #[test]
    fn test_titles() {
        assert_eq!(
            test_title("test('a page', async () => {").as_deref(),
            Some("a page")
        );
        assert_eq!(
            test_title("  test.describe(\"web\", () => {").as_deref(),
            Some("web")
        );
        assert_eq!(test_title("it.only(`x y`, fn)").as_deref(), Some("x y"));
        assert_eq!(test_title("testing('x')"), None);
        assert_eq!(test_title("test.extend({})"), None);
    }
}

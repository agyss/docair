//! The code index of a save (docs/REGENERATE.md §4.2, §4.3): the code's tokens without
//! comments and the key they make, a kept index's positions moved to new texts, and in `docair
//! serve` the map of each function's calls, with what changed asked of `docair-index` in the
//! background.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use docair_core::adapter::Project;
use docair_core::config::CodeSettings;
use docair_core::model::Diagnostic;
use docair_scip::background::{FullRequest, ShortRequest};
use docair_scip::{Index, KeptIndex, RustIndexer, text_hash};
use tree_sitter::{Node, Parser, Tree};

/// When the names a change touched are in more lines than this, the full index is made instead of a
/// short run (docs/REGENERATE.md §4.2).
const MAX_USAGE_LINES: usize = 100;

/// A token of the code: where it starts (line 1-based, column in bytes) and its bytes in the text.
#[derive(Debug, Clone, Copy)]
struct Tok {
    line: usize,
    col: usize,
    start: usize,
    end: usize,
}

fn parse(text: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .expect("tree-sitter-rust matches the tree-sitter ABI");
    parser.parse(text, None)
}

fn is_comment(n: &Node) -> bool {
    matches!(n.kind(), "line_comment" | "block_comment")
}

/// The code's tokens, in order: every leaf of the syntax tree outside comments.
fn tokens(tree: &Tree) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        if is_comment(&n) {
            continue;
        }
        if n.child_count() == 0 {
            if n.end_byte() > n.start_byte() {
                let p = n.start_position();
                out.push(Tok {
                    line: p.row + 1,
                    col: p.column,
                    start: n.start_byte(),
                    end: n.end_byte(),
                });
            }
            continue;
        }
        let mut c = n.walk();
        let kids: Vec<Node> = n.children(&mut c).collect();
        stack.extend(kids.into_iter().rev());
    }
    out
}

/***
 * The code as the index's key sees it: its tokens without comments and whitespace, so an edit of comments or doc blocks keeps the index (docs/REGENERATE.md §4.3).
 ***/
pub fn code_text(text: &str) -> Arc<str> {
    static SEEN: OnceLock<Mutex<HashMap<u64, Arc<str>>>> = OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    let h = text_hash(text);
    if let Some(c) = seen.lock().unwrap_or_else(|e| e.into_inner()).get(&h) {
        return c.clone();
    }
    let code: Arc<str> = match parse(text) {
        Some(tree) => {
            let toks = tokens(&tree);
            let mut s = String::with_capacity(text.len());
            for t in &toks {
                s.push_str(&text[t.start..t.end]);
                s.push('\u{1}');
            }
            s.into()
        }
        None => text.into(),
    };
    let mut seen = seen.lock().unwrap_or_else(|e| e.into_inner());
    if seen.len() > 20_000 {
        seen.clear();
    }
    seen.insert(h, code.clone());
    code
}

/// A function of a file: who it is, its body's tokens, and the lines of the whole function.
struct Function {
    ident: String,
    body: (usize, usize),
    lines: (usize, usize),
}

/// The name of a node's field, as written.
fn field_text<'a>(n: &Node, field: &str, text: &'a str) -> &'a str {
    n.child_by_field_name(field)
        .map_or("", |c| &text[c.start_byte()..c.end_byte()])
}

/// Who a function is: the impls, traits, modules and functions around it, and its name.
fn ident_of(n: &Node, text: &str) -> String {
    let mut parts = vec![field_text(n, "name", text).to_string()];
    let mut up = n.parent();
    while let Some(p) = up {
        match p.kind() {
            "impl_item" => {
                let ty = field_text(&p, "type", text);
                parts.push(match p.child_by_field_name("trait") {
                    Some(_) => format!("<{ty} as {}>", field_text(&p, "trait", text)),
                    None => ty.to_string(),
                });
            }
            "trait_item" | "mod_item" | "function_item" => {
                parts.push(field_text(&p, "name", text).to_string())
            }
            _ => {}
        }
        up = p.parent();
    }
    parts.reverse();
    parts.join("::")
}

/// The index of the first token at or after a byte.
fn tok_at(toks: &[Tok], byte: usize) -> usize {
    toks.partition_point(|t| t.start < byte)
}

fn functions(tree: &Tree, text: &str, toks: &[Tok]) -> Vec<Function> {
    let mut out = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        if n.kind() == "function_item"
            && let Some(body) = n.child_by_field_name("body")
        {
            let id = ident_of(&n, text);
            let k = seen.entry(id.clone()).or_insert(0);
            *k += 1;
            let ident = if *k == 1 { id } else { format!("{id}#{k}") };
            out.push(Function {
                ident,
                body: (
                    tok_at(toks, body.start_byte()),
                    tok_at(toks, body.end_byte()),
                ),
                lines: (n.start_position().row + 1, n.end_position().row + 1),
            });
        }
        let mut c = n.walk();
        let kids: Vec<Node> = n.children(&mut c).collect();
        stack.extend(kids.into_iter().rev());
    }
    out
}

/// The `use` declarations of a file, as tokens.
fn uses_text(tree: &Tree, text: &str, toks: &[Tok]) -> String {
    let mut out = String::new();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        if n.kind() == "use_declaration" {
            for t in &toks[tok_at(toks, n.start_byte())..tok_at(toks, n.end_byte())] {
                out.push_str(&text[t.start..t.end]);
                out.push('\u{1}');
            }
            out.push('\n');
            continue;
        }
        if n.kind() == "function_item" {
            continue;
        }
        let mut c = n.walk();
        let kids: Vec<Node> = n.children(&mut c).collect();
        stack.extend(kids.into_iter().rev());
    }
    out
}

/// How a kept index's positions in a file go to the file's new text, and which lines it cannot
/// hold any more.
pub struct FileMap {
    to: HashMap<(usize, usize), (usize, usize)>,
    pub gaps: Vec<(usize, usize)>,
}

impl FileMap {
    pub fn get(&self, line: usize, col: usize) -> Option<(usize, usize)> {
        self.to.get(&(line, col)).copied()
    }
}

/***
 * Map a kept index's positions from a file's old text to its new one (docs/REGENERATE.md §4.3).
 *
 * The same code without comments: every token maps. Else a function keeps
 * its positions when its body's tokens are the same, and the code outside
 * functions when the file's `use` declarations are the same; the lines of
 * the other functions, and of code outside them that changed, are gaps,
 * read by the syntax until a short run says more.
 ***/
pub fn map_file(old: &str, new: &str) -> FileMap {
    let (Some(ot), Some(nt)) = (parse(old), parse(new)) else {
        return FileMap {
            to: HashMap::new(),
            gaps: vec![(1, usize::MAX)],
        };
    };
    let (ok, nk) = (tokens(&ot), tokens(&nt));
    let otext = |t: &Tok| &old[t.start..t.end];
    let ntext = |t: &Tok| &new[t.start..t.end];
    let mut to = HashMap::new();
    let same_code = ok.len() == nk.len() && ok.iter().zip(&nk).all(|(a, b)| otext(a) == ntext(b));
    if same_code {
        for (a, b) in ok.iter().zip(&nk) {
            to.insert((a.line, a.col), (b.line, b.col));
        }
        return FileMap {
            to,
            gaps: Vec::new(),
        };
    }
    let (of, nf) = (functions(&ot, old, &ok), functions(&nt, new, &nk));
    let by_ident: HashMap<&str, &Function> = of.iter().map(|f| (f.ident.as_str(), f)).collect();
    let in_body = |fs: &[Function], i: usize| fs.iter().any(|f| f.body.0 <= i && i < f.body.1);
    let mut gaps = Vec::new();
    let mut reused_new: Vec<(usize, usize)> = Vec::new();
    for f in &nf {
        let reused = by_ident.get(f.ident.as_str()).is_some_and(|o| {
            let (a, b) = (&ok[o.body.0..o.body.1], &nk[f.body.0..f.body.1]);
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| otext(x) == ntext(y))
        });
        match (reused, by_ident.get(f.ident.as_str())) {
            (true, Some(o)) => {
                for (x, y) in ok[o.body.0..o.body.1].iter().zip(&nk[f.body.0..f.body.1]) {
                    to.insert((x.line, x.col), (y.line, y.col));
                }
                reused_new.push(f.body);
            }
            _ => gaps.push(f.lines),
        }
    }
    // Code outside functions keeps its positions where it is the same, if the imports are.
    let same_uses = uses_text(&ot, old, &ok) == uses_text(&nt, new, &nk);
    let mut mapped_new: HashSet<usize> = HashSet::new();
    if same_uses {
        let a: Vec<&str> = ok.iter().map(otext).collect();
        let b: Vec<&str> = nk.iter().map(ntext).collect();
        for op in similar::capture_diff_slices(similar::Algorithm::Myers, &a, &b) {
            if let similar::DiffOp::Equal {
                old_index,
                new_index,
                len,
            } = op
            {
                for k in 0..len {
                    let (i, j) = (old_index + k, new_index + k);
                    if in_body(&of, i) || in_body(&nf, j) {
                        continue;
                    }
                    to.insert((ok[i].line, ok[i].col), (nk[j].line, nk[j].col));
                    mapped_new.insert(j);
                }
            }
        }
    }
    // Lines outside functions with a token the old text does not explain.
    for (j, t) in nk.iter().enumerate() {
        if !in_body(&nf, j) && !mapped_new.contains(&j) && gaps.last() != Some(&(t.line, t.line)) {
            gaps.push((t.line, t.line));
        }
    }
    FileMap { to, gaps }
}

/// The items of a file outside function bodies, by kind and who they are: their tokens.
fn items(text: &str) -> HashMap<String, (String, String)> {
    let mut out = HashMap::new();
    let Some(tree) = parse(text) else { return out };
    let toks = tokens(&tree);
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        let named = matches!(
            n.kind(),
            "function_item"
                | "function_signature_item"
                | "struct_item"
                | "enum_item"
                | "union_item"
                | "trait_item"
                | "type_item"
                | "const_item"
                | "static_item"
                | "macro_definition"
                | "mod_item"
                | "impl_item"
                | "use_declaration"
                | "associated_type"
        );
        if named {
            let name = match n.kind() {
                "impl_item" => field_text(&n, "type", text).to_string(),
                "use_declaration" => {
                    let arg = field_text(&n, "argument", text);
                    arg.rsplit([':', ' ', '{', ','])
                        .find(|w| !w.is_empty())
                        .unwrap_or("")
                        .to_string()
                }
                _ => field_text(&n, "name", text).to_string(),
            };
            // A function's signature, not its body; an impl's or trait's header, not its items.
            let end = match n.kind() {
                "function_item" => n
                    .child_by_field_name("body")
                    .map_or(n.end_byte(), |b| b.start_byte()),
                "impl_item" | "trait_item" | "mod_item" => n
                    .child_by_field_name("body")
                    .map_or(n.end_byte(), |b| b.start_byte()),
                _ => n.end_byte(),
            };
            let mut sig = String::new();
            for t in &toks[tok_at(&toks, n.start_byte())..tok_at(&toks, end)] {
                sig.push_str(&text[t.start..t.end]);
                sig.push('\u{1}');
            }
            let who = format!("{} {}", n.kind(), ident_of(&n, text));
            let k = out.len();
            out.insert(
                if out.contains_key(&who) {
                    format!("{who}#{k}")
                } else {
                    who
                },
                (name, sig),
            );
        }
        if n.kind() == "function_item" {
            continue;
        }
        let mut c = n.walk();
        let kids: Vec<Node> = n.children(&mut c).collect();
        stack.extend(kids.into_iter().rev());
    }
    out
}

/// The names of the items a change of a file added, removed or changed outside function bodies:
/// other files that name them are resolved again (docs/REGENERATE.md §4.2).
pub fn changed_names(old: &str, new: &str) -> HashSet<String> {
    let (a, b) = (items(old), items(new));
    let mut out = HashSet::new();
    for (k, (name, sig)) in &a {
        if b.get(k).is_none_or(|(_, s)| s != sig) {
            out.insert(name.clone());
        }
    }
    for (k, (name, _)) in &b {
        if !a.contains_key(k) {
            out.insert(name.clone());
        }
    }
    out.retain(|n| !n.is_empty() && n != "self" && n != "super" && n != "crate");
    out
}

/// Whether `text` has `name` as a whole word.
fn names(text: &str, name: &str) -> usize {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    text.lines()
        .filter(|l| {
            l.match_indices(name).any(|(i, _)| {
                !l[..i].chars().next_back().is_some_and(word)
                    && !l[i + name.len()..].chars().next().is_some_and(word)
            })
        })
        .count()
}

/// The last kept index read: its key, the index, and the texts it was made from.
type LastKept = (String, Arc<Index>, Arc<HashMap<String, String>>);

/// A kept index read once per process: its index and the texts it was made from.
fn load(kept: &KeptIndex) -> Option<(Index, Arc<HashMap<String, String>>)> {
    static LAST: OnceLock<Mutex<Option<LastKept>>> = OnceLock::new();
    let last = LAST.get_or_init(Default::default);
    if let Some((k, i, s)) = last.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        && *k == kept.key
    {
        return Some(((**i).clone(), s.clone()));
    }
    let index = Index::read(&kept.scip).ok()?;
    let sources = Arc::new(kept.sources());
    *last.lock().unwrap_or_else(|e| e.into_inner()) =
        Some((kept.key.clone(), Arc::new(index.clone()), sources.clone()));
    Some((index, sources))
}

/***
 * The code index of the project's Rust files with `[code] run_indexers` (docs/REGENERATE.md §4.2).
 *
 * The index kept for the same code, its positions moved to edited
 * comments. Else, outside `docair serve`, a new one made now. In serve, the
 * project's last index with what changed since: short runs' word on the
 * files they read, unchanged functions kept, the rest left to the syntax,
 * and a short run asked for the edited files and those naming a changed
 * item; the full index follows once serve has been idle.
 *
 * @calls docair-scip.rust-indexer.find "finds docair-index or rust-analyzer"
 * @calls docair-scip.make_index "makes and keeps an index"
 * @calls docair-lang-rust.kept.map_file "moves positions to the new text"
 * @calls docair-scip.background.request_short "asks docair-index about the edited files"
 * @calls docair-scip.background.request_full "asks for the full index once serve is idle"
 ***/
pub fn index_for(
    project: &Project,
    root: &Path,
    code: &CodeSettings,
    diags: &mut Vec<Diagnostic>,
) -> Option<Index> {
    let indexer = RustIndexer::find()?;
    let version = indexer.version()?;
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap_or_default();
    let raw: Vec<(&str, &str)> = project
        .files
        .iter()
        .filter(|f| f.rel.ends_with(".rs") || f.rel.ends_with("Cargo.toml"))
        .map(|f| (f.rel.as_str(), f.text.as_str()))
        .collect();
    let key_texts: Vec<(&str, Arc<str>)> = raw
        .iter()
        .map(|(rel, text)| {
            let k: Arc<str> = if rel.ends_with(".rs") {
                code_text(text)
            } else {
                (*text).into()
            };
            (*rel, k)
        })
        .collect();
    let key_src: Vec<(&str, &str)> = key_texts.iter().map(|(r, t)| (*r, &**t)).collect();
    let key = docair_scip::rust_key(&version, &key_src, &lock);
    let keep = docair_scip::cache_dir();
    let mut raw_all: Vec<(&str, &str)> = raw.clone();
    raw_all.push(("Cargo.lock", &lock));
    let made_now = |diags: &mut Vec<Diagnostic>| -> Option<Index> {
        match docair_scip::make_index(&indexer, root, &key, &raw_all, &keep) {
            Ok(k) => load(&k).map(|(i, _)| i),
            Err(e) => {
                diags.push(
                    Diagnostic::new("W119", format!("no code index for the calls: {e}"))
                        .at("Cargo.toml", 1),
                );
                None
            }
        }
    };
    // The index of this code: its positions moved where only comments changed.
    if let Some(kept) = docair_scip::kept_index(&keep, &key) {
        let (mut index, old) = load(&kept)?;
        for (rel, text) in raw.iter().filter(|(r, _)| r.ends_with(".rs")) {
            if let Some(o) = old.get(*rel).filter(|o| o.as_str() != *text) {
                let m = map_file(o, text);
                index.remap_file(rel, &|l, c| m.get(l, c));
            }
        }
        if code.background {
            compare_overlays(&index, &raw, &keep, root);
        }
        docair_scip::remember(&keep, root, &key);
        return Some(index);
    }
    if !code.background {
        return made_now(diags);
    }
    let Some(base) =
        docair_scip::last_key(&keep, root).and_then(|k| docair_scip::kept_index(&keep, &k))
    else {
        return made_now(diags);
    };
    let (mut index, old) = load(&base)?;
    let mut changed: Vec<String> = Vec::new();
    let mut names_changed: HashSet<String> = HashSet::new();
    for (rel, text) in raw.iter().filter(|(r, _)| r.ends_with(".rs")) {
        if let Some(part) = docair_scip::background::overlay(rel, text_hash(text)) {
            index.replace_file(rel, &part);
            continue;
        }
        match old.get(*rel) {
            Some(o) if o.as_str() == *text => {}
            Some(o) => {
                let m = map_file(o, text);
                index.remap_file(rel, &|l, c| m.get(l, c));
                index.set_gaps(rel, m.gaps.clone());
                if !m.gaps.is_empty() {
                    changed.push(rel.to_string());
                    names_changed.extend(changed_names(o, text));
                }
            }
            None => {
                changed.push(rel.to_string());
                names_changed.extend(changed_names("", text));
            }
        }
    }
    let here: HashSet<&str> = raw.iter().map(|(r, _)| *r).collect();
    let gone: Vec<String> = index
        .paths()
        .filter(|p| p.ends_with(".rs") && !here.contains(p))
        .map(String::from)
        .collect();
    for p in &gone {
        index.drop_file(p);
    }
    // Cargo's files changed: the crates may have, so the full index, at once.
    let cargo_changed = raw_all
        .iter()
        .filter(|(r, _)| !r.ends_with(".rs"))
        .any(|(r, t)| old.get(*r).is_none_or(|o| o.as_str() != *t));
    let full = |now: bool| {
        docair_scip::background::request_full(
            FullRequest {
                root: root.to_path_buf(),
                keep: keep.clone(),
                indexer: indexer.clone(),
                key: key.clone(),
                raw_sources: raw_all
                    .iter()
                    .map(|(p, t)| (p.to_string(), t.to_string()))
                    .collect(),
            },
            now,
        )
    };
    if cargo_changed || !gone.is_empty() {
        full(true);
    } else if !changed.is_empty() {
        match &indexer {
            RustIndexer::DocairIndex(bin) => {
                // The other files that name what the change touched.
                let mut lines = 0;
                let mut files: Vec<String> = changed.clone();
                for (rel, text) in raw
                    .iter()
                    .filter(|(r, _)| r.ends_with(".rs") && !changed.iter().any(|c| c == *r))
                {
                    let n: usize = names_changed.iter().map(|name| names(text, name)).sum();
                    if n > 0 {
                        lines += n;
                        files.push(rel.to_string());
                    }
                }
                if lines > MAX_USAGE_LINES {
                    full(true);
                } else {
                    docair_scip::background::request_short(ShortRequest {
                        root: root.to_path_buf(),
                        keep: keep.clone(),
                        bin: bin.clone(),
                        base: base.clone(),
                        files,
                    });
                    full(false);
                }
            }
            RustIndexer::RustAnalyzer => full(true),
        }
    }
    Some(index)
}

/// The full index of this code landed: compare it with what short runs said about the files they
/// read; where they differ, short runs load everything until `Cargo.lock` changes
/// (docs/REGENERATE.md §4.8). Then their word is forgotten.
fn compare_overlays(index: &Index, raw: &[(&str, &str)], keep: &Path, root: &Path) {
    let now: HashMap<&str, u64> = raw.iter().map(|(r, t)| (*r, text_hash(t))).collect();
    let mut differ = Vec::new();
    for (path, hash, part) in docair_scip::background::overlaid() {
        if now.get(path.as_str()) == Some(&hash) && part.file(&path) != index.file(&path) {
            differ.push(path);
        }
    }
    if !differ.is_empty() {
        eprintln!(
            "docair serve: the full code index differs from the short run in {}; short runs load everything from now on",
            differ.join(", ")
        );
        docair_scip::background::turn_rule_off(keep, root);
    }
    docair_scip::background::clear_overlays();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_do_not_change_the_code() {
        let a = "fn a() {\n    b(); // calls b\n}\n";
        let b = "/// A.\nfn a() {\n    /* b */ b();\n}\n";
        assert_eq!(code_text(a), code_text(b));
        assert_ne!(code_text(a), code_text("fn a() { c(); }"));
    }

    #[test]
    fn a_comment_moves_every_position() {
        let old = "fn a() {\n    b();\n}\n";
        let new = "// new\n\nfn a() {\n    b();\n}\n";
        let m = map_file(old, new);
        assert_eq!(m.get(2, 4), Some((4, 4)), "b moves down two lines");
        assert!(m.gaps.is_empty());
    }

    #[test]
    fn an_edited_body_is_a_gap_and_the_other_functions_keep_their_places() {
        let old = "fn a() {\n    x();\n}\n\nfn b() {\n    y();\n}\n";
        let new = "fn a() {\n    x();\n    z();\n}\n\nfn b() {\n    y();\n}\n";
        let m = map_file(old, new);
        assert_eq!(
            m.get(2, 4),
            None,
            "a's body changed: its calls wait for the short run"
        );
        assert_eq!(m.get(6, 4), Some((7, 4)), "b is the same, one line down");
        assert!(
            m.gaps.contains(&(1, 4)),
            "a's lines are a gap: {:?}",
            m.gaps
        );
    }

    #[test]
    fn a_changed_signature_names_the_item() {
        let old = "pub fn load(p: &str) -> u8 { 1 }\nfn other() {}\n";
        let new = "pub fn load(p: &str, q: u8) -> u8 { 1 }\nfn other() { let _ = 2; }\n";
        let n = changed_names(old, new);
        assert!(n.contains("load"));
        assert!(!n.contains("other"), "a body edit changes no name: {n:?}");
        assert_eq!(names("let x = load(1);\nfn loader() {}\n", "load"), 1);
    }
}

//! The SCIP index of files, as `rust-analyzer scip` writes it, and lines for code a derive
//! generates (docs/REGENERATE.md §4.7).
//!
//! The occurrence and symbol part follows rust-analyzer's `cli/scip.rs` (MIT or Apache-2.0):
//! every token of rust-analyzer's static index with a moniker is an occurrence, named by its
//! moniker. Symbol information is left out: docair does not read it.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use ra_ap_hir::{AsAssocItem, ModuleDef, PathResolution, Semantics};
use ra_ap_ide::{
    LineCol, Moniker, MonikerDescriptorKind, MonikerIdentifier, MonikerResult, StaticIndex,
    StaticIndexedFile, TextRange, TokenId, TokenStaticData, TryToNav, VendoredLibrariesConfig,
};
use ra_ap_ide_db::RootDatabase;
use ra_ap_ide_db::defs::{Definition, IdentClass};

use crate::moniker::{Sym, symbol_of};
use ra_ap_syntax::{AstNode, ast, ast::HasAttrs, ast::HasName};
use ra_ap_vfs::{FileId, VfsPath};
use scip::types as scip_types;

use crate::load::Loaded;

/// A document's occurrences in SCIP's form: ranges as lines and columns.
fn document(
    db: &RootDatabase,
    relative_path: String,
    file: FileId,
    occurrences: Vec<(TextRange, String, bool)>,
) -> Option<scip_types::Document> {
    if occurrences.is_empty() {
        return None;
    }
    let lines = ra_ap_ide_db::line_index(db, file).clone();
    let range = |r: TextRange| -> Vec<i32> {
        let LineCol { line: sl, col: sc } = lines.line_col(r.start());
        let LineCol { line: el, col: ec } = lines.line_col(r.end());
        if sl == el {
            vec![sl as i32, sc as i32, ec as i32]
        } else {
            vec![sl as i32, sc as i32, el as i32, ec as i32]
        }
    };
    Some(scip_types::Document {
        relative_path,
        language: "rust".to_owned(),
        occurrences: occurrences
            .into_iter()
            .map(|(r, symbol, is_definition)| scip_types::Occurrence {
                range: range(r),
                symbol,
                symbol_roles: if is_definition {
                    scip_types::SymbolRole::Definition as i32
                } else {
                    0
                },
                ..Default::default()
            })
            .collect(),
        position_encoding: scip_types::PositionEncoding::UTF8CodeUnitOffsetFromLineStart.into(),
        ..Default::default()
    })
}

/***
 * The SCIP index of the whole workspace: rust-analyzer's static index, as `rust-analyzer scip` writes it, and the lines of code derives generate.
 *
 * @calls docair-index.write.derive_lines "adds the lines of code derives generate"
 ***/
pub fn index(l: &Loaded, files: &[FileId]) -> scip_types::Index {
    let analysis = l.host.analysis();
    let db = l.host.raw_database();
    let root: VfsPath = l.root.clone().into();
    let t = std::time::Instant::now();
    let si = StaticIndex::compute(
        &analysis,
        VendoredLibrariesConfig::Included {
            workspace_root: &root,
        },
        false,
    );
    crate::say("  rust-analyzer's static index", t.elapsed());
    let mut derive_time = std::time::Duration::ZERO;
    let want: HashSet<FileId> = files.iter().copied().collect();
    let mut documents = Vec::new();
    let mut symbols = SymbolGenerator::default();
    for StaticIndexedFile {
        file_id, tokens, ..
    } in &si.files
    {
        if !want.contains(file_id) {
            continue;
        }
        symbols.local_count = 0;
        let Some(relative_path) = l.rel(*file_id) else {
            continue;
        };
        let mut occurrences = Vec::new();
        let mut defined: HashSet<String> = HashSet::new();
        for (text_range, id) in tokens {
            let token = si.tokens.get(*id).expect("a token of the index");
            let Some(symbol) = symbols.token_symbol(*id, token) else {
                continue;
            };
            let is_definition = token
                .definition
                .is_some_and(|d| d.file_id == *file_id && d.range == *text_range);
            if is_definition {
                defined.insert(symbol.clone());
            }
            occurrences.push((*text_range, symbol, is_definition));
        }
        let t = std::time::Instant::now();
        occurrences.extend(derive_lines(db, *file_id, &defined));
        derive_time += t.elapsed();
        documents.extend(document(db, relative_path, *file_id, occurrences));
    }
    crate::say("  derive lines", derive_time);
    scip_types::Index {
        documents,
        ..Default::default()
    }
}

/// Each definition met: where it is (file and range), and its symbol.
type DefTable<'db> = HashMap<Definition<'db>, (Option<(FileId, TextRange)>, Option<Sym>)>;

/// The definitions a token names, as rust-analyzer's static index finds them (its
/// `get_definitions`): through the macros it is in.
fn definitions<'db>(
    sema: &Semantics<'db, RootDatabase>,
    token: ra_ap_syntax::SyntaxToken,
) -> Vec<Definition<'db>> {
    for t in sema.descend_into_macros_exact(token) {
        if let Some(c) = IdentClass::classify_token(sema, &t) {
            let defs: Vec<Definition<'db>> = c.definitions().into_iter().map(|(d, _)| d).collect();
            if !defs.is_empty() {
                return defs;
            }
        }
    }
    Vec::new()
}

/***
 * The SCIP index of some files only, without the static index of the whole workspace: each file's tokens and what they name, as rust-analyzer's static index finds them, named by the same symbols (`moniker::symbol_of`), and the lines of code derives generate (docs/REGENERATE.md §4.4).
 *
 * @calls docair-index.moniker.symbol_of "names a definition"
 * @calls docair-index.write.derive_lines "adds the lines of code derives generate"
 ***/
pub fn index_files(l: &Loaded, files: &[FileId]) -> scip_types::Index {
    let db = l.host.raw_database();
    let mut documents = Vec::new();
    ra_ap_hir::attach_db(db, || {
        let sema = Semantics::new(db);
        let mut known: DefTable<'_> = HashMap::new();
        for file in files {
            let Some(relative_path) = l.rel(*file) else {
                continue;
            };
            let t = std::time::Instant::now();
            let root = sema.parse_guess_edition(*file).syntax().clone();
            let mut named: Vec<(TextRange, Definition<'_>)> = Vec::new();
            if let Some(module) = sema.file_to_module_def(*file) {
                named.push((root.text_range(), Definition::Module(module)));
            }
            for token in root
                .descendants_with_tokens()
                .filter_map(|e| e.into_token())
            {
                let range = token.text_range();
                for def in definitions(&sema, token) {
                    named.push((range, def));
                }
            }
            crate::say("  what the tokens name", t.elapsed());
            let t = std::time::Instant::now();
            let mut locals: HashMap<Definition<'_>, usize> = HashMap::new();
            let mut occurrences = Vec::new();
            let mut defined: HashSet<String> = HashSet::new();
            for (range, def) in named {
                let (at, sym) = known.entry(def).or_insert_with(|| {
                    let at = def
                        .try_to_nav(&sema)
                        .map(|n| n.call_site)
                        .map(|n| (n.file_id, n.focus_or_full_range()));
                    (at, symbol_of(db, def))
                });
                let symbol = match sym {
                    Some(Sym::Global(s)) => s.clone(),
                    Some(Sym::Local) => {
                        let n = locals.len();
                        let n = *locals.entry(def).or_insert(n);
                        scip::symbol::format_symbol(scip_types::Symbol::new_local(n))
                    }
                    None => continue,
                };
                let is_definition = at.is_some_and(|(f, r)| f == *file && r == range);
                if is_definition {
                    defined.insert(symbol.clone());
                }
                occurrences.push((range, symbol, is_definition));
            }
            crate::say("  their symbols", t.elapsed());
            let t = std::time::Instant::now();
            occurrences.extend(derive_lines(db, *file, &defined));
            crate::say("  derive lines", t.elapsed());
            documents.extend(document(db, relative_path, *file, occurrences));
        }
    });
    scip_types::Index {
        documents,
        ..Default::default()
    }
}

/***
 * The lines of code a derive generates, drawn on the type that carries the derive (docs/REGENERATE.md §4.7): a definition at the type's name for each method the derive's code defines, and a reference at the type's name for each of our functions that code calls. Built-in derives generate no code to read.
 ***/
fn derive_lines(
    db: &RootDatabase,
    file: FileId,
    defined: &HashSet<String>,
) -> Vec<(TextRange, String, bool)> {
    let mut out: Vec<(TextRange, String, bool)> = Vec::new();
    ra_ap_hir::attach_db(db, || {
        let sema = Semantics::new(db);
        let source = sema.parse_guess_edition(file);
        for adt in source.syntax().descendants().filter_map(ast::Adt::cast) {
            let Some(name) = adt.name().map(|n| n.syntax().text_range()) else {
                continue;
            };
            let Some(this) = sema.to_def(&adt) else {
                continue;
            };
            for attr in adt.attrs() {
                let Some(meta) = attr.meta() else { continue };
                if meta.path().is_none_or(|p| p.syntax().text() != "derive") {
                    continue;
                }
                let Some(expansions) = sema.expand_derive_macro(&meta) else {
                    continue;
                };
                for exp in expansions.into_iter().flatten() {
                    for node in exp.value.descendants() {
                        // A method the derive defines on the type itself (not on its own helpers).
                        if let Some(f) = ast::Fn::cast(node.clone()) {
                            let on_this = sema.to_def(&f).filter(|f| {
                                f.as_assoc_item(db).and_then(|a| match a.container(db) {
                                    ra_ap_hir::AssocItemContainer::Impl(i) => {
                                        i.self_ty(db).as_adt()
                                    }
                                    _ => None,
                                }) == Some(this)
                            });
                            if let Some(Sym::Global(s)) =
                                on_this.and_then(|f| symbol_of(db, Definition::from(f)))
                                && !defined.contains(&s)
                            {
                                out.push((name, s, true));
                            }
                            continue;
                        }
                        // One of our functions the generated code calls; `#[serde(default = "yes")]`
                        // makes a path to `yes`.
                        let target = if let Some(m) = ast::MethodCallExpr::cast(node.clone()) {
                            sema.resolve_method_call(&m)
                        } else if let Some(p) = ast::Path::cast(node.clone()) {
                            match sema.resolve_path(&p) {
                                Some(PathResolution::Def(ModuleDef::Function(f))) => Some(f),
                                _ => None,
                            }
                        } else {
                            None
                        };
                        let Some(f) =
                            target.filter(|f| f.module(db).krate(db).origin(db).is_local())
                        else {
                            continue;
                        };
                        if let Some(Sym::Global(s)) = symbol_of(db, Definition::from(f)) {
                            out.push((name, s, false));
                        }
                    }
                }
            }
        }
    });
    // The generated code calling the methods it generates is the type calling itself: no line.
    let own: HashSet<(u32, String)> = out
        .iter()
        .filter(|o| o.2)
        .map(|o| (u32::from(o.0.start()), o.1.clone()))
        .collect();
    out.retain(|o| o.2 || !own.contains(&(u32::from(o.0.start()), o.1.clone())));
    out.sort_by(|a, b| {
        (u32::from(a.0.start()), &a.1, a.2).cmp(&(u32::from(b.0.start()), &b.1, b.2))
    });
    out.dedup();
    out
}

pub fn save(index: scip_types::Index, out: &Path) -> Result<()> {
    scip::write_message_to_file(out, index)
        .map_err(|e| anyhow::format_err!("{}: {e}", out.display()))
}

#[derive(Default)]
struct SymbolGenerator {
    token_to_symbol: HashMap<TokenId, Option<String>>,
    local_count: usize,
}

impl SymbolGenerator {
    /// A token's symbol: from its moniker, or `local N`, counted in each document.
    fn token_symbol(&mut self, id: TokenId, token: &TokenStaticData) -> Option<String> {
        let mut local_count = self.local_count;
        let s = self
            .token_to_symbol
            .entry(id)
            .or_insert_with(|| {
                Some(match token.moniker.as_ref()? {
                    MonikerResult::Moniker(m) => scip::symbol::format_symbol(symbol(m)),
                    MonikerResult::Local { .. } => {
                        let local = scip_types::Symbol::new_local(local_count);
                        local_count += 1;
                        scip::symbol::format_symbol(local)
                    }
                })
            })
            .clone();
        self.local_count = local_count;
        s
    }
}

fn symbol(moniker: &Moniker) -> scip_types::Symbol {
    scip_types::Symbol {
        scheme: "rust-analyzer".into(),
        package: Some(scip_types::Package {
            manager: "cargo".to_owned(),
            name: moniker.package_information.name.clone(),
            version: moniker
                .package_information
                .version
                .clone()
                .unwrap_or_else(|| ".".to_owned()),
            special_fields: Default::default(),
        })
        .into(),
        descriptors: descriptors(&moniker.identifier),
        special_fields: Default::default(),
    }
}

fn descriptors(identifier: &MonikerIdentifier) -> Vec<scip_types::Descriptor> {
    use scip_types::descriptor::Suffix::*;
    identifier
        .description
        .iter()
        .map(|d| scip_types::Descriptor {
            name: d.name.to_string(),
            disambiguator: String::new(),
            suffix: match d.desc {
                MonikerDescriptorKind::Namespace => Namespace,
                MonikerDescriptorKind::Type => Type,
                MonikerDescriptorKind::Term => Term,
                MonikerDescriptorKind::Method => Method,
                MonikerDescriptorKind::TypeParameter => TypeParameter,
                MonikerDescriptorKind::Parameter => Parameter,
                MonikerDescriptorKind::Macro => Macro,
                MonikerDescriptorKind::Meta => Meta,
            }
            .into(),
            special_fields: Default::default(),
        })
        .collect()
}

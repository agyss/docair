//! The guard (docs/REGENERATE.md §4.6). A crate left out can change our code only through its
//! own types, traits, functions and macros, and where one is missing the function shows it: an
//! inferred type with `{unknown}`, or a path, method call, field or macro that does not resolve.
//! A function's signals are counted; a short run compares them with the full index's.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use ra_ap_hir::Semantics;
use ra_ap_syntax::{AstNode, SyntaxNode, ast, ast::HasName};
use ra_ap_vfs::FileId;

use crate::load::Loaded;

/// Each function's signals, by `file|who`.
pub type Signals = BTreeMap<String, usize>;

/// Who a function is in its file: the impls, traits, modules and functions around it, and its name.
fn ident(f: &ast::Fn) -> String {
    let name = |n: Option<ast::Name>| n.map_or_else(String::new, |n| n.text().to_string());
    let mut parts = vec![name(f.name())];
    for a in f.syntax().ancestors().skip(1) {
        if let Some(i) = ast::Impl::cast(a.clone()) {
            let ty = i
                .self_ty()
                .map_or_else(String::new, |t| t.syntax().text().to_string());
            parts.push(match i.trait_() {
                Some(t) => format!("<{ty} as {}>", t.syntax().text()),
                None => ty,
            });
        } else if let Some(t) = ast::Trait::cast(a.clone()) {
            parts.push(name(t.name()));
        } else if let Some(m) = ast::Module::cast(a.clone()) {
            parts.push(name(m.name()));
        } else if let Some(g) = ast::Fn::cast(a.clone()) {
            parts.push(name(g.name()));
        }
    }
    parts.reverse();
    parts.join("::")
}

/***
 * Count each function's signals in the given files.
 ***/
pub fn signals(l: &Loaded, files: &[FileId]) -> Signals {
    let db = l.host.raw_database();
    let mut out = Signals::new();
    for file in files {
        let Some(rel) = l.rel(*file) else { continue };
        ra_ap_hir::attach_db(db, || {
            let sema = Semantics::new(db);
            let source = sema.parse_guess_edition(*file);
            let mut whose: BTreeMap<u32, String> = BTreeMap::new();
            let mut seen: BTreeMap<String, usize> = BTreeMap::new();
            for f in source.syntax().descendants().filter_map(ast::Fn::cast) {
                let id = ident(&f);
                let n = seen.entry(id.clone()).or_insert(0);
                *n += 1;
                let key = if *n == 1 {
                    format!("{rel}|{id}")
                } else {
                    format!("{rel}|{id}#{n}")
                };
                whose.insert(u32::from(f.syntax().text_range().start()), key.clone());
                out.entry(key).or_insert(0);
            }
            let key_of = |node: &SyntaxNode| {
                node.ancestors()
                    .find_map(ast::Fn::cast)
                    .and_then(|f| {
                        whose
                            .get(&u32::from(f.syntax().text_range().start()))
                            .cloned()
                    })
                    .unwrap_or_else(|| format!("{rel}|-"))
            };
            for node in source.syntax().descendants() {
                let signal = if let Some(m) = ast::MethodCallExpr::cast(node.clone()) {
                    sema.resolve_method_call(&m).is_none()
                        || sema
                            .type_of_expr(&m.into())
                            .is_some_and(|t| t.original.contains_unknown())
                } else if let Some(p) = ast::Path::cast(node.clone()) {
                    // Paths inside attributes (`#[derive(…)]`, `#[serde(…)]`) are not code.
                    !p.syntax()
                        .ancestors()
                        .any(|a| ast::Attr::can_cast(a.kind()))
                        && sema.resolve_path(&p).is_none()
                } else if let Some(fe) = ast::FieldExpr::cast(node.clone()) {
                    sema.resolve_field(&fe).is_none()
                } else if let Some(mc) = ast::MacroCall::cast(node.clone()) {
                    sema.resolve_macro_call(&mc).is_none()
                } else if let Some(p) = ast::IdentPat::cast(node.clone()) {
                    sema.type_of_binding_in_pat(&p)
                        .is_some_and(|t| t.contains_unknown())
                } else if let Some(e) = ast::Expr::cast(node.clone()) {
                    matches!(
                        e,
                        ast::Expr::CallExpr(_)
                            | ast::Expr::FieldExpr(_)
                            | ast::Expr::TryExpr(_)
                            | ast::Expr::AwaitExpr(_)
                    ) && sema
                        .type_of_expr(&e)
                        .is_some_and(|t| t.original.contains_unknown())
                } else {
                    false
                };
                if signal {
                    *out.entry(key_of(&node)).or_insert(0) += 1;
                }
            }
        });
    }
    out
}

pub fn write(s: &Signals, path: &Path) -> Result<()> {
    std::fs::write(path, serde_json::to_string(s)?)?;
    Ok(())
}

pub fn read(path: &Path) -> Result<Signals> {
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

/// The functions with more signals than the full index recorded for them (none for a new one).
pub fn fires(now: &Signals, base: &Signals) -> Vec<String> {
    now.iter()
        .filter(|(k, n)| **n > base.get(*k).copied().unwrap_or(0))
        .map(|(k, _)| k.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn more_signals_than_the_base_fire() {
        let base: Signals = [("a.rs|f".to_string(), 1), ("a.rs|g".to_string(), 0)].into();
        let now: Signals = [
            ("a.rs|f".to_string(), 1),
            ("a.rs|g".to_string(), 2),
            ("a.rs|h".to_string(), 0),
        ]
        .into();
        assert_eq!(fires(&now, &base), vec!["a.rs|g".to_string()]);
    }
}

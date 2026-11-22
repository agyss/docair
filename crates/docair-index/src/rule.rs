//! What a short run leaves out (docs/REGENERATE.md §4.5). Kept: the workspace's crates, `core`,
//! `alloc`, `std` and the crates they depend on directly; then, until nothing new comes in, every
//! crate a kept crate passes on with `pub use` or `pub extern crate`, and every crate that brings a
//! kept crate macros. Everything else is left out, and core's SIMD intrinsics.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use anyhow::Result;
use ra_ap_hir::Crate;

use crate::load::{Loaded, package_dir};

/// What is left out: each crate's root file (or core's `core_arch` module) and the text it gets.
#[derive(Debug, Clone)]
pub struct Rule {
    pub left_out: Vec<(String, String)>,
}

impl Rule {
    fn to_json(&self) -> String {
        serde_json::Value::Array(
            self.left_out
                .iter()
                .map(|(p, t)| serde_json::json!([p, t]))
                .collect(),
        )
        .to_string()
    }

    fn from_json(text: &str) -> Option<Rule> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let left_out = v
            .as_array()?
            .iter()
            .filter_map(|e| {
                Some((
                    e.get(0)?.as_str()?.to_string(),
                    e.get(1)?.as_str()?.to_string(),
                ))
            })
            .collect();
        Some(Rule { left_out })
    }
}

/***
 * The rule for this workspace: kept in `cache` under the hash of `Cargo.lock`, the toolchain and docair-index's version, else worked out from the crate graph and `cargo metadata`.
 *
 * @calls docair-index.rule.work_out "works the rule out"
 ***/
pub fn rule(l: &Loaded, cache: Option<&Path>) -> Result<Rule> {
    let lock = std::fs::read_to_string(l.root.join("Cargo.lock")).unwrap_or_default();
    let rustc = Command::new("rustc")
        .arg("-V")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let key = docair_scip::text_hash(&format!("{lock}\u{1}{rustc}\u{1}{}", crate::VERSION));
    let file = cache.map(|c| c.join(format!("rule-{key:016x}.json")));
    if let Some(r) = file
        .as_ref()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| Rule::from_json(&t))
    {
        return Ok(r);
    }
    let r = work_out(l)?;
    if let Some(f) = file {
        let _ = std::fs::create_dir_all(f.parent().unwrap_or(Path::new(".")));
        let _ = std::fs::write(f, r.to_json());
    }
    Ok(r)
}

/// The root files of the workspace's proc-macro targets, from `cargo metadata`.
fn proc_macro_roots(root: &Path) -> HashSet<String> {
    let Ok(out) = Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .current_dir(root)
        .output()
    else {
        return HashSet::new();
    };
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    let mut roots = HashSet::new();
    for p in v["packages"].as_array().into_iter().flatten() {
        for t in p["targets"].as_array().into_iter().flatten() {
            let kinds = t["kind"].as_array().into_iter().flatten();
            if kinds.into_iter().any(|k| k == "proc-macro")
                && let Some(src) = t["src_path"].as_str()
            {
                let src = Path::new(src);
                let src = src.canonicalize().unwrap_or_else(|_| src.to_path_buf());
                roots.insert(src.to_string_lossy().to_string());
            }
        }
    }
    roots
}

/***
 * Work the rule out from the crate graph, `cargo metadata` and the kept crates' sources.
 ***/
fn work_out(l: &Loaded) -> Result<Rule> {
    let db = l.host.raw_database();
    let proc_macros = proc_macro_roots(l.root.as_ref());
    let left_out = ra_ap_hir::attach_db(db, || {
        let all = Crate::all(db);
        let name = |k: &Crate| {
            k.display_name(db)
                .map_or_else(String::new, |n| n.to_string())
        };
        let root_of = |k: &Crate| l.path(k.root_file(db)).unwrap_or_default();
        let files_of = |k: &Crate| -> Vec<String> {
            let dir = package_dir(&root_of(k));
            l.vfs
                .iter()
                .filter_map(|(_, p)| p.as_path().map(|p| p.as_str().to_string()))
                .filter(|p| p.starts_with(&dir) && p.ends_with(".rs"))
                .collect()
        };
        let is_proc_macro = |k: &Crate| proc_macros.contains(&root_of(k));
        let mut keep: HashSet<Crate> = all
            .iter()
            .copied()
            .filter(|k| {
                k.origin(db).is_local() || ["core", "alloc", "std"].contains(&name(k).as_str())
            })
            .collect();
        let first: Vec<Crate> = keep.iter().copied().collect();
        for k in first {
            keep.extend(k.dependencies(db).into_iter().map(|d| d.krate));
        }
        // What kept crates pass on, and the crates that bring them macros, until nothing new comes in.
        let mut todo: Vec<Crate> = keep
            .iter()
            .copied()
            .filter(|k| !k.origin(db).is_local())
            .collect();
        let mut scanned: HashSet<Crate> = HashSet::new();
        while let Some(k) = todo.pop() {
            if !scanned.insert(k) {
                continue;
            }
            let deps: HashMap<String, Crate> = k
                .dependencies(db)
                .into_iter()
                .map(|d| (d.name.as_str().to_string(), d.krate))
                .collect();
            let texts: Vec<String> = files_of(&k)
                .iter()
                .filter_map(|f| std::fs::read_to_string(f).ok())
                .collect();
            let mut named = passed_on(&texts);
            // What the crate's root file imports: where the types its API names come from
            // (`use tree_sitter_language::LanguageFn; pub const LANGUAGE: LanguageFn`).
            if let Ok(root_text) = std::fs::read_to_string(root_of(&k)) {
                named.extend(imported(&root_text));
            }
            for (n, d) in &deps {
                if keep.contains(d) {
                    continue;
                }
                let passed_on = named.contains(n);
                let macros = is_proc_macro(d) || {
                    let dt: Vec<String> = files_of(d)
                        .iter()
                        .filter_map(|f| std::fs::read_to_string(f).ok())
                        .collect();
                    dt.iter().any(|t| t.contains("#[macro_export]"))
                        || d.dependencies(db).iter().any(|dd| {
                            is_proc_macro(&dd.krate) && {
                                let m = dd.name.as_str().to_string();
                                dt.iter().any(|t| {
                                    t.contains(&format!("pub use {m}"))
                                        || t.contains(&format!("pub use ::{m}"))
                                })
                            }
                        })
                };
                if (passed_on || macros) && keep.insert(*d) {
                    todo.push(*d);
                }
            }
        }
        let mut out: Vec<(String, String)> = all
            .iter()
            .filter(|k| !keep.contains(k))
            .map(|k| (root_of(k), String::new()))
            .filter(|(p, _)| !p.is_empty())
            .collect();
        if let Some(p) = l
            .vfs
            .iter()
            .filter_map(|(_, p)| p.as_path().map(|p| p.as_str().to_string()))
            .find(|p| p.ends_with("stdarch/crates/core_arch/src/mod.rs"))
        {
            out.push((p, "pub mod arch {}\n".to_string()));
        }
        out.sort();
        out.dedup();
        out
    });
    Ok(Rule { left_out })
}

/// The crates a file's `use` declarations import from: `use name::…`, `use ::name::…`.
fn imported(text: &str) -> HashSet<String> {
    let ident = |s: &str| -> String {
        s.chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect()
    };
    let mut out = HashSet::new();
    for line in text.lines() {
        let l = line.trim_start();
        let l = l.strip_prefix("pub ").unwrap_or(l);
        if let Some(rest) = l.strip_prefix("use ") {
            let rest = rest.trim_start().trim_start_matches("::");
            let name = ident(rest);
            if !name.is_empty() && rest[name.len()..].starts_with("::") {
                out.insert(name);
            }
        }
    }
    out
}

/// The names a crate's sources pass on: `pub use name::…`, `pub use ::name::…`, `pub use { name::…, … }`,
/// `pub extern crate name`, not `pub(crate)`, `pub(super)` or `pub(in …)`.
fn passed_on(texts: &[String]) -> HashSet<String> {
    {
        let ident = |s: &str| -> String {
            s.chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect()
        };
        let mut out = HashSet::new();
        for t in texts {
            for (i, _) in t.match_indices("pub") {
                let before = t[..i].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let mut rest = t[i + 3..].trim_start();
                if let Some(r) = rest.strip_prefix('(') {
                    let inner = r.split(')').next().unwrap_or("");
                    if inner.contains("crate")
                        || inner.contains("super")
                        || inner.contains("in ")
                        || inner.contains("self")
                    {
                        continue;
                    }
                    rest = r.split_once(')').map_or("", |(_, b)| b).trim_start();
                }
                let after = if let Some(r) = rest.strip_prefix("use") {
                    r
                } else if let Some(r) = rest.strip_prefix("extern") {
                    r.trim_start().strip_prefix("crate").unwrap_or("")
                } else {
                    continue;
                };
                if !after.starts_with(char::is_whitespace) {
                    continue;
                }
                let after = after.trim_start().trim_start_matches("::");
                if let Some(inner) = after.strip_prefix('{') {
                    let inner = inner.split('}').next().unwrap_or("");
                    for part in inner.split(',') {
                        let p = part.trim().trim_start_matches("::");
                        if p.contains("::") {
                            out.insert(ident(p));
                        }
                    }
                } else {
                    out.insert(ident(after));
                }
            }
        }
        out
    }
}

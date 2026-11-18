//! SCIP code indexes (docs/ADAPTERS.md §5.4).
//!
//! Following a call to the function it reaches needs each language's own
//! rules. docair doesn't re-implement them: the languages' own tools write a
//! SCIP index (`rust-analyzer scip`, `scip-typescript`), and this crate reads
//! any of them into references: on this line, the code names that function.
//! The language adapters map those to their own symbols.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use protobuf::Message;

/// The SCIP role bit of a definition.
const DEFINITION: i32 = 1;

/// A reference to a function or method, and the line it is on (1-based).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub line: usize,
    /// Where the name starts on the line (0-based, in UTF-8 bytes as rust-analyzer counts them).
    pub col: usize,
    pub symbol: String,
}

/// What an index says about functions and methods.
#[derive(Debug, Default)]
pub struct Index {
    /// References to functions and methods, by file, in source order.
    refs: HashMap<String, Vec<Reference>>,
    /// Where each function or method is defined: file and line.
    defs: HashMap<String, (String, usize)>,
    /// (trait, method) → the methods that implement it.
    impls: HashMap<(String, String), Vec<String>>,
    /// The files the index read.
    docs: std::collections::HashSet<String>,
}

/// The descriptors of a symbol: `adapter/Adapter#file().` of
/// `rust-analyzer cargo docair-core 0.1.0 adapter/Adapter#file().`.
fn descriptors(symbol: &str) -> &str {
    symbol.splitn(5, ' ').nth(4).unwrap_or(symbol)
}

/// The last descriptor: `Adapter#file().`, `impl#[RustAdapter][Adapter]file().`.
fn last(symbol: &str) -> &str {
    let d = descriptors(symbol);
    // A `/` inside brackets (a type in an impl) does not split.
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in d.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            '/' if depth == 0 => start = i + 1,
            _ => {}
        }
    }
    &d[start..]
}

/// Whether a symbol is a function or method: its last descriptor is a method (`name().`).
pub fn callable(symbol: &str) -> bool {
    !symbol.starts_with("local ") && symbol.ends_with(").")
}

fn method_name(descriptor: &str) -> Option<&str> {
    let name = descriptor
        .rsplit(['#', ']', '.'])
        .find(|s| !s.is_empty() && *s != ")")?;
    let name = name.split('(').next()?;
    (!name.is_empty()).then_some(name)
}

/// A trait method, as calls through the trait name it: `Adapter#file().` → (`Adapter`, `file`).
pub fn trait_method(symbol: &str) -> Option<(String, String)> {
    let d = last(symbol);
    if d.starts_with("impl#") || !callable(symbol) {
        return None;
    }
    let (owner, method) = d.split_once('#')?;
    Some((owner.to_string(), method_name(method)?.to_string()))
}

/// A method that implements a trait method: `impl#[RustAdapter][Adapter]file().` → (`Adapter`, `file`).
pub fn implemented(symbol: &str) -> Option<(String, String)> {
    let d = last(symbol).strip_prefix("impl#")?;
    let groups: Vec<&str> = d
        .split('[')
        .skip(1)
        .map(|g| g.split(']').next().unwrap_or(""))
        .collect();
    let [_, trait_name] = groups.as_slice() else {
        return None;
    };
    let method = d.rsplit(']').next()?;
    Some((trait_name.to_string(), method_name(method)?.to_string()))
}

/// A symbol as people read it: `config::load`, `Adapter::file`, `RustAdapter::project`.
pub fn short(symbol: &str) -> String {
    let d = last(symbol);
    let d = d.trim_end_matches('.').trim_end_matches("()");
    let d = match d.strip_prefix("impl#") {
        Some(rest) => {
            let ty = rest.split(']').next().unwrap_or("").trim_start_matches('[');
            let method = rest.rsplit(']').next().unwrap_or("");
            format!("{ty}::{method}")
        }
        None => d.replace('#', "::"),
    };
    let module = descriptors(symbol)
        .rsplit_once('/')
        .map(|(m, _)| m.rsplit('/').next().unwrap_or(m))
        .filter(|m| !m.contains('['));
    match module {
        Some(m) if !d.contains("::") => format!("{m}::{d}"),
        _ => d,
    }
}

impl Index {
    /// Read an index file.
    pub fn read(path: &Path) -> Result<Index, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Index::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Read an index from its bytes.
    pub fn parse(bytes: &[u8]) -> Result<Index, String> {
        let index = scip::types::Index::parse_from_bytes(bytes).map_err(|e| e.to_string())?;
        let mut out = Index::default();
        for doc in index.documents {
            let path = doc.relative_path.clone();
            out.docs.insert(path.clone());
            for o in doc.occurrences {
                if !callable(&o.symbol) || o.range.is_empty() {
                    continue;
                }
                let line = o.range[0] as usize + 1;
                if o.symbol_roles & DEFINITION != 0 {
                    if let Some(key) = implemented(&o.symbol) {
                        out.impls.entry(key).or_default().push(o.symbol.clone());
                    }
                    out.defs.entry(o.symbol).or_insert((path.clone(), line));
                } else {
                    out.refs.entry(path.clone()).or_default().push(Reference {
                        line,
                        col: o.range.get(1).map_or(0, |c| *c as usize),
                        symbol: o.symbol,
                    });
                }
            }
        }
        Ok(out)
    }

    /// Add what another index says (one per language or tool).
    pub fn extend(&mut self, other: Index) {
        for (path, refs) in other.refs {
            self.refs.entry(path).or_default().extend(refs);
        }
        for (symbol, def) in other.defs {
            self.defs.entry(symbol).or_insert(def);
        }
        for (key, impls) in other.impls {
            self.impls.entry(key).or_default().extend(impls);
        }
        self.docs.extend(other.docs);
    }

    /// Whether the index read this file.
    pub fn covers(&self, path: &str) -> bool {
        self.docs.contains(path)
    }

    /// The references to functions and methods in a file (not their definitions), in source order.
    pub fn calls(&self, path: &str) -> &[Reference] {
        self.refs.get(path).map_or(&[], Vec::as_slice)
    }

    /// Where a function or method is defined: its file and line (1-based).
    pub fn definition(&self, symbol: &str) -> Option<(&str, usize)> {
        self.defs.get(symbol).map(|(p, l)| (p.as_str(), *l))
    }

    /// The methods that implement a trait method (`…/Adapter#file().` → `impl#[RustAdapter][Adapter]file().`, …).
    pub fn implementations(&self, trait_method_symbol: &str) -> &[String] {
        trait_method(trait_method_symbol)
            .and_then(|key| self.impls.get(&key))
            .map_or(&[], Vec::as_slice)
    }
}

/// Whether `rust-analyzer` is installed.
pub fn has_rust_analyzer() -> bool {
    Command::new("rust-analyzer")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/***
 * Write a SCIP index of a Cargo project with `rust-analyzer scip`.
 ***/
pub fn run_rust_analyzer(root: &Path, out: &Path) -> Result<(), String> {
    let run = Command::new("rust-analyzer")
        .arg("scip")
        .arg(root)
        .arg("--output")
        .arg(out)
        .output()
        .map_err(|e| format!("rust-analyzer scip: {e}"))?;
    if !run.status.success() || !out.exists() {
        let err = String::from_utf8_lossy(&run.stderr);
        return Err(format!(
            "rust-analyzer scip failed: {}",
            err.lines().last().unwrap_or("no output")
        ));
    }
    Ok(())
}

/// rust-analyzer's version, so an index another version made is not taken for a new one.
pub fn rust_analyzer_version() -> Option<String> {
    let out = Command::new("rust-analyzer")
        .arg("--version")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn fnv(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x0100_0000_01b3);
    }
}

/// The key an index is kept under: the indexer's version, the sources it reads (by path and
/// text), and the lock file that fixes the dependencies.
pub fn rust_index_key(version: &str, sources: &[(&str, &str)], lock: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    fnv(&mut h, version.as_bytes());
    let mut sorted: Vec<&(&str, &str)> = sources.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (rel, text) in sorted {
        fnv(&mut h, &[0]);
        fnv(&mut h, rel.as_bytes());
        fnv(&mut h, &[0]);
        fnv(&mut h, text.as_bytes());
    }
    fnv(&mut h, &[1]);
    fnv(&mut h, lock.as_bytes());
    format!("{h:016x}")
}

/// Where docair keeps what it can make again: `$DOCAIR_CACHE`, else the user's cache folder.
pub fn cache_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("DOCAIR_CACHE") {
        return d.into();
    }
    if let Some(d) = std::env::var_os("XDG_CACHE_HOME") {
        return std::path::PathBuf::from(d).join("docair");
    }
    if let Some(d) = std::env::var_os("HOME") {
        return std::path::PathBuf::from(d).join(".cache").join("docair");
    }
    std::env::temp_dir().join("docair-cache")
}

/// How many indexes are kept: the ones of a few recent states of the code.
const KEPT: usize = 6;

/***
 * A SCIP index of a Cargo project: the one kept for the same sources, or a new one by `rust-analyzer scip`, then kept.
 *
 * `sources` are the files rust-analyzer reads, by path and text; with the
 * lock file and the indexer's version they make the key. Only the newest
 * indexes are kept.
 *
 * @calls docair-scip.rust_index_key "works out the key"
 * @calls docair-scip.run_rust_analyzer "makes a new index"
 ***/
pub fn kept_rust_index(
    root: &Path,
    sources: &[(&str, &str)],
    keep: &Path,
) -> Result<std::path::PathBuf, String> {
    let version = rust_analyzer_version().ok_or("rust-analyzer --version failed")?;
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap_or_default();
    let path = keep.join(format!(
        "rust-{}.scip",
        rust_index_key(&version, sources, &lock)
    ));
    if path.is_file() {
        return Ok(path);
    }
    std::fs::create_dir_all(keep).map_err(|e| format!("{}: {e}", keep.display()))?;
    let fresh = keep.join(format!("rust-{}.scip.part", std::process::id()));
    run_rust_analyzer(root, &fresh)?;
    std::fs::rename(&fresh, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    // Only the newest are kept.
    let mut old: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(keep)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("rust-") && n.ends_with(".scip"))
        })
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .collect();
    old.sort_by_key(|o| std::cmp::Reverse(o.0));
    for (_, p) in old.into_iter().skip(KEPT) {
        let _ = std::fs::remove_file(p);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_index_is_kept_by_what_the_indexer_reads() {
        use super::rust_index_key;
        let a = rust_index_key(
            "ra 1",
            &[("src/lib.rs", "fn a() {}"), ("Cargo.toml", "[package]")],
            "lock",
        );
        assert_eq!(
            a,
            rust_index_key(
                "ra 1",
                &[("Cargo.toml", "[package]"), ("src/lib.rs", "fn a() {}")],
                "lock"
            ),
            "the order of the files does not matter"
        );
        assert_ne!(
            a,
            rust_index_key(
                "ra 1",
                &[("src/lib.rs", "fn b() {}"), ("Cargo.toml", "[package]")],
                "lock"
            ),
            "a changed file does"
        );
        assert_ne!(
            a,
            rust_index_key(
                "ra 2",
                &[("src/lib.rs", "fn a() {}"), ("Cargo.toml", "[package]")],
                "lock"
            ),
            "another indexer does"
        );
        assert_ne!(
            a,
            rust_index_key(
                "ra 1",
                &[("src/lib.rs", "fn a() {}"), ("Cargo.toml", "[package]")],
                "lock 2"
            ),
            "other dependencies do"
        );
    }

    use super::*;

    const TRAIT: &str = "rust-analyzer cargo docair-core 0.1.0 adapter/Adapter#file().";
    const IMPL: &str =
        "rust-analyzer cargo docair-lang-rust 0.1.0 impl#[RustAdapter][Adapter]file().";
    const FREE: &str = "rust-analyzer cargo docair-core 0.1.0 config/load().";
    const INHERENT: &str = "rust-analyzer cargo docair-core 0.1.0 facts/impl#[Facts]extend().";

    #[test]
    fn symbols_say_what_they_are() {
        assert!(callable(FREE) && callable(IMPL) && callable(TRAIT));
        assert!(!callable("local 3"));
        assert!(!callable(
            "rust-analyzer cargo docair-core 0.1.0 model/Node#"
        ));
        assert_eq!(trait_method(TRAIT), Some(("Adapter".into(), "file".into())));
        assert_eq!(trait_method(IMPL), None);
        assert_eq!(trait_method(INHERENT), None);
        assert_eq!(implemented(IMPL), Some(("Adapter".into(), "file".into())));
        assert_eq!(implemented(INHERENT), None);
        assert_eq!(short(FREE), "config::load");
        assert_eq!(short(TRAIT), "Adapter::file");
        assert_eq!(short(IMPL), "RustAdapter::file");
        assert_eq!(short(INHERENT), "Facts::extend");
    }

    #[test]
    fn an_index_gives_calls_definitions_and_implementations() {
        let occ = |line: i32, symbol: &str, roles: i32| {
            let mut o = scip::types::Occurrence::new();
            o.range = vec![line, 4, 10];
            o.symbol = symbol.into();
            o.symbol_roles = roles;
            o
        };
        let mut doc = scip::types::Document::new();
        doc.relative_path = "src/lib.rs".into();
        doc.occurrences = vec![
            occ(2, FREE, DEFINITION),
            occ(9, IMPL, DEFINITION),
            occ(20, FREE, 0),
            occ(21, TRAIT, 0),
            occ(22, "local 1", 0),
        ];
        let mut index = scip::types::Index::new();
        index.documents = vec![doc];
        let i = Index::parse(&index.write_to_bytes().unwrap()).unwrap();
        let calls: Vec<_> = i.calls("src/lib.rs").iter().map(|r| r.line).collect();
        assert_eq!(calls, [21, 22]);
        assert_eq!(i.definition(FREE), Some(("src/lib.rs", 3)));
        assert_eq!(i.implementations(TRAIT), [IMPL.to_string()]);
    }
}

//! SCIP code indexes (docs/ADAPTERS.md §5.4).
//!
//! Following a call to the function it reaches needs each language's own
//! rules. docair doesn't re-implement them: the languages' own tools write a
//! SCIP index (`rust-analyzer scip`, `scip-typescript`), and this crate reads
//! any of them into references: on this line, the code names that function.
//! The language adapters map those to their own symbols.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

/// What an index says about one file: the names it uses and the definitions in it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FileData {
    /// References to functions and methods, in source order.
    refs: Vec<Reference>,
    /// References to types and their fields and variants, in source order (docs/ADAPTERS.md §5.8).
    uses: Vec<Reference>,
    /// Definitions of functions, methods and types, in source order.
    defs: Vec<Reference>,
}

/// What an index says about functions and methods.
#[derive(Debug, Default, Clone)]
pub struct Index {
    /// The files the index read, in the order of its documents.
    order: Vec<String>,
    files: HashMap<String, FileData>,
    /// Where each function, method or type is defined: file and line (the first definition wins).
    defs: HashMap<String, (String, usize)>,
    /// (trait, method) → the methods that implement it.
    impls: HashMap<(String, String), Vec<String>>,
    /// Lines of a file the index does not hold, though it read the file: functions edited since
    /// (docs/REGENERATE.md §4.3). The syntax reads their calls.
    gaps: HashMap<String, Vec<(usize, usize)>>,
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

/// The type a symbol belongs to, and the member it names (docs/ADAPTERS.md §5.8):
/// `… Cli#` → (`… Cli#`, none), `… Cli#command.` → (`… Cli#`, `command`),
/// `… Command#Build#root.` → (`… Command#`, `Build#root`),
/// `… config/impl#[CandidateLimit]DEFAULT.` → (`… config/CandidateLimit#`, `DEFAULT`).
/// None for functions and methods, locals, modules, macros and what no type holds.
pub fn type_of(symbol: &str) -> Option<(String, Option<String>)> {
    if symbol.starts_with("local ") || callable(symbol) {
        return None;
    }
    let d = descriptors(symbol);
    let head = &symbol[..symbol.len() - d.len()];
    // The namespaces before the type: `model/` of `model/Graph#nodes.`.
    let mut depth = 0i32;
    let mut start = 0;
    let mut hash = None;
    for (i, c) in d.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            '/' if depth == 0 && hash.is_none() => start = i + 1,
            '#' if depth == 0 && hash.is_none() => hash = Some(i),
            _ => {}
        }
    }
    let rest = &d[start..];
    let member = |m: &str| {
        let m = m.trim_end_matches(['.', '#']);
        (!m.is_empty()).then(|| m.to_string())
    };
    if let Some(inner) = rest.strip_prefix("impl#[") {
        // A constant or type written in an impl: its type is the one in brackets.
        let (ty, m) = inner.split_once(']')?;
        if ty.contains(['`', '<', ' ']) || m.starts_with('[') {
            return None;
        }
        return Some((format!("{head}{}{ty}#", &d[..start]), member(m)));
    }
    let hash = hash?.checked_sub(start)?;
    let ty = &rest[..=hash];
    if ty.contains(['[', '(']) {
        return None;
    }
    Some((
        format!("{head}{}{ty}", &d[..start]),
        member(&rest[hash + 1..]),
    ))
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
            let mut data = FileData::default();
            for o in doc.occurrences {
                if o.range.is_empty() {
                    continue;
                }
                let at = Reference {
                    line: o.range[0] as usize + 1,
                    col: o.range.get(1).map_or(0, |c| *c as usize),
                    symbol: o.symbol,
                };
                let definition = o.symbol_roles & DEFINITION != 0;
                if !callable(&at.symbol) {
                    // A type, its fields and variants: where each type is, and every name of one.
                    if definition {
                        if at.symbol.ends_with('#') && type_of(&at.symbol).is_some() {
                            data.defs.push(at);
                        }
                    } else if type_of(&at.symbol).is_some() {
                        data.uses.push(at);
                    }
                    continue;
                }
                if definition {
                    data.defs.push(at);
                } else {
                    data.refs.push(at);
                }
            }
            out.put(path, data);
        }
        out.rebuild();
        Ok(out)
    }

    /// A file's data, in place of what the index held for it, keeping its place among the files.
    fn put(&mut self, path: String, data: FileData) {
        if !self.files.contains_key(&path) {
            self.order.push(path.clone());
        }
        self.files.insert(path, data);
    }

    /// The definitions and implementations again, from the files in their order.
    fn rebuild(&mut self) {
        self.defs.clear();
        self.impls.clear();
        for path in &self.order {
            let Some(data) = self.files.get(path) else {
                continue;
            };
            for d in &data.defs {
                if callable(&d.symbol)
                    && let Some(key) = implemented(&d.symbol)
                {
                    self.impls.entry(key).or_default().push(d.symbol.clone());
                }
                self.defs
                    .entry(d.symbol.clone())
                    .or_insert((path.clone(), d.line));
            }
        }
    }

    /// Add what another index says (one per language or tool).
    pub fn extend(&mut self, other: Index) {
        let Index {
            order,
            mut files,
            gaps,
            ..
        } = other;
        for path in order {
            if let Some(data) = files.remove(&path) {
                self.put(path, data);
            }
        }
        self.gaps.extend(gaps);
        self.rebuild();
    }

    /// The index with every document's path under `prefix`: an indexer run in `web/` writes
    /// `src/app/app.ts`, the project knows `web/src/app/app.ts`.
    pub fn under(self, prefix: &str) -> Index {
        if prefix.is_empty() {
            return self;
        }
        let at = |p: &String| format!("{}/{p}", prefix.trim_end_matches('/'));
        let mut out = Index {
            order: self.order.iter().map(at).collect(),
            files: self.files.iter().map(|(p, d)| (at(p), d.clone())).collect(),
            gaps: self.gaps.iter().map(|(p, g)| (at(p), g.clone())).collect(),
            ..Index::default()
        };
        out.rebuild();
        out
    }

    /// Whether the index read this file.
    pub fn covers(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }

    /// Whether the index holds this line of a file: it read the file, and the line is in no
    /// function edited since (docs/REGENERATE.md §4.3).
    pub fn covers_line(&self, path: &str, line: usize) -> bool {
        self.covers(path)
            && !self
                .gaps
                .get(path)
                .is_some_and(|g| g.iter().any(|(a, b)| *a <= line && line <= *b))
    }

    /// The files the index read, in its order.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(String::as_str)
    }

    /// What the index says about one file.
    pub fn file(&self, path: &str) -> Option<&FileData> {
        self.files.get(path)
    }

    /// The references to functions and methods in a file (not their definitions), in source order.
    pub fn calls(&self, path: &str) -> &[Reference] {
        self.files.get(path).map_or(&[], |d| d.refs.as_slice())
    }

    /// The names of types, their fields and variants in a file (not their definitions), in source order.
    pub fn uses(&self, path: &str) -> &[Reference] {
        self.files.get(path).map_or(&[], |d| d.uses.as_slice())
    }

    /// Where a function, method or type is defined: its file and line (1-based).
    pub fn definition(&self, symbol: &str) -> Option<(&str, usize)> {
        self.defs.get(symbol).map(|(p, l)| (p.as_str(), *l))
    }

    /// The methods that implement a trait method (`…/Adapter#file().` → `impl#[RustAdapter][Adapter]file().`, …).
    pub fn implementations(&self, trait_method_symbol: &str) -> &[String] {
        trait_method(trait_method_symbol)
            .and_then(|key| self.impls.get(&key))
            .map_or(&[], Vec::as_slice)
    }

    /// Move a file's positions to its new text: `to` maps a position (line 1-based, column) of
    /// the text the index read to the new text, or drops it with `None` (docs/REGENERATE.md §4.3).
    pub fn remap_file(&mut self, path: &str, to: &dyn Fn(usize, usize) -> Option<(usize, usize)>) {
        let Some(data) = self.files.get_mut(path) else {
            return;
        };
        for list in [&mut data.refs, &mut data.uses, &mut data.defs] {
            list.retain_mut(|r| match to(r.line, r.col) {
                Some((line, col)) => {
                    r.line = line;
                    r.col = col;
                    true
                }
                None => false,
            });
        }
        self.rebuild();
    }

    /// Mark lines of a file the index no longer holds: the syntax reads their calls.
    pub fn set_gaps(&mut self, path: &str, lines: Vec<(usize, usize)>) {
        if lines.is_empty() {
            self.gaps.remove(path);
        } else {
            self.gaps.insert(path.to_string(), lines);
        }
    }

    /// Forget a file: the syntax reads its calls.
    pub fn drop_file(&mut self, path: &str) {
        if self.files.remove(path).is_some() {
            self.order.retain(|p| p != path);
            self.gaps.remove(path);
            self.rebuild();
        }
    }

    /// Take a file's data from another index, in place of this one's: what a short run says
    /// about an edited file (docs/REGENERATE.md §4.4).
    pub fn replace_file(&mut self, path: &str, from: &Index) {
        match from.files.get(path) {
            Some(data) => {
                self.put(path.to_string(), data.clone());
                self.gaps.remove(path);
            }
            None => {
                self.files.remove(path);
                self.order.retain(|p| p != path);
            }
        }
        self.rebuild();
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

/// scip-typescript installed for a TypeScript project: `node_modules/.bin/scip-typescript` in its folder.
pub fn scip_typescript_in(dir: &Path) -> Option<std::path::PathBuf> {
    let bin = dir
        .join("node_modules")
        .join(".bin")
        .join("scip-typescript");
    bin.is_file().then_some(bin)
}

/***
 * The index scip-typescript writes for the TypeScript project in `dir`, kept like rust-analyzer's: under the tool's version, the sources and the lock file, so it runs again only when the code changed.
 *
 * `sources` are the files it reads, by path and text.
 ***/
pub fn kept_ts_index(
    dir: &Path,
    sources: &[(&str, &str)],
    keep: &Path,
) -> Result<std::path::PathBuf, String> {
    let bin = scip_typescript_in(dir).ok_or("scip-typescript is not installed in node_modules")?;
    let version =
        std::fs::read_to_string(dir.join("node_modules/@sourcegraph/scip-typescript/package.json"))
            .unwrap_or_default();
    let lock = std::fs::read_to_string(dir.join("package-lock.json")).unwrap_or_default();
    let path = keep.join(format!(
        "ts-{}.scip",
        rust_index_key(&version, sources, &lock)
    ));
    if path.is_file() {
        return Ok(path);
    }
    std::fs::create_dir_all(keep).map_err(|e| format!("{}: {e}", keep.display()))?;
    let fresh = keep.join(format!("ts-{}.scip.part", run_name()));
    let out = std::process::Command::new(&bin)
        .arg("index")
        .arg("--cwd")
        .arg(dir)
        .arg("--output")
        .arg(&fresh)
        .output()
        .map_err(|e| format!("{}: {e}", bin.display()))?;
    if !out.status.success() || !fresh.is_file() {
        return Err(format!(
            "scip-typescript failed: {}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .last()
                .unwrap_or("")
        ));
    }
    std::fs::rename(&fresh, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
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

/// A name of this run of an indexer, for the file it writes before it is kept: the process and a
/// count, so two runs at once in one process (both commits of a comparison, docs/REGENERATE.md) do
/// not write the same file.
fn run_name() -> String {
    static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}-{n}", std::process::id())
}

/// The hash docair keeps a file's text under: FNV-1a of its bytes.
pub fn text_hash(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    fnv(&mut h, text.as_bytes());
    h
}

/// What makes the code index of a Cargo project (docs/REGENERATE.md §4.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustIndexer {
    /// `docair-index`, beside docair or on the PATH: it also makes short runs and derive lines.
    DocairIndex(PathBuf),
    /// `rust-analyzer scip`.
    RustAnalyzer,
}

impl RustIndexer {
    /// `docair-index` beside the running program, else on the PATH, else `rust-analyzer`;
    /// `DOCAIR_INDEXER=rust-analyzer` or a path to a program chooses.
    pub fn find() -> Option<RustIndexer> {
        match std::env::var("DOCAIR_INDEXER").ok().as_deref() {
            Some("rust-analyzer") => {
                return has_rust_analyzer().then_some(RustIndexer::RustAnalyzer);
            }
            Some(p) if !p.is_empty() => return Some(RustIndexer::DocairIndex(PathBuf::from(p))),
            _ => {}
        }
        let name = format!("docair-index{}", std::env::consts::EXE_SUFFIX);
        let beside = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.join(&name)))
            .filter(|p| p.is_file());
        let on_path = || {
            std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|d| d.join(&name))
                    .find(|p| p.is_file())
            })
        };
        match beside.or_else(on_path) {
            Some(p) => Some(RustIndexer::DocairIndex(p)),
            None => has_rust_analyzer().then_some(RustIndexer::RustAnalyzer),
        }
    }

    /// The indexer's version: an index another version made is not taken for a new one.
    pub fn version(&self) -> Option<String> {
        let out = match self {
            RustIndexer::DocairIndex(p) => Command::new(p).arg("--version").output().ok()?,
            RustIndexer::RustAnalyzer => return rust_analyzer_version(),
        };
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Index the whole project into `out`; `docair-index` also writes each function's signals
    /// beside it (the guard's base, docs/REGENERATE.md §4.6).
    pub fn full(&self, root: &Path, out: &Path) -> Result<(), String> {
        match self {
            RustIndexer::RustAnalyzer => run_rust_analyzer(root, out),
            RustIndexer::DocairIndex(bin) => {
                let run = Command::new(bin)
                    .arg("full")
                    .arg("--root")
                    .arg(root)
                    .arg("--out")
                    .arg(out)
                    .arg("--signals")
                    .arg(out.with_extension("signals.json"))
                    .output()
                    .map_err(|e| format!("{}: {e}", bin.display()))?;
                if !run.status.success() || !out.exists() {
                    let err = String::from_utf8_lossy(&run.stderr);
                    let why = err.lines().last().unwrap_or("no output").to_string();
                    // A toolchain docair-index cannot work with: rust-analyzer's own index.
                    if has_rust_analyzer() {
                        eprintln!(
                            "docair: docair-index failed ({why}); rust-analyzer scip instead"
                        );
                        return run_rust_analyzer(root, out);
                    }
                    return Err(format!("docair-index failed: {why}"));
                }
                Ok(())
            }
        }
    }
}

/// A kept code index of a Cargo project, and where its files are.
#[derive(Debug, Clone)]
pub struct KeptIndex {
    pub key: String,
    /// The SCIP index.
    pub scip: PathBuf,
}

impl KeptIndex {
    fn at(keep: &Path, key: &str) -> KeptIndex {
        KeptIndex {
            key: key.to_string(),
            scip: keep.join(format!("rust-{key}.scip")),
        }
    }

    /// The texts the index was made from (`.rs` files, `Cargo.toml` files, `Cargo.lock`), by path.
    pub fn sources(&self) -> HashMap<String, String> {
        std::fs::read_to_string(self.scip.with_extension("sources.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Each function's signals in the index (docs/REGENERATE.md §4.6), when `docair-index` made it.
    pub fn signals(&self) -> Option<PathBuf> {
        Some(self.scip.with_extension("signals.json")).filter(|p| p.is_file())
    }
}

/// The key a Rust index is kept under: the indexer's version, the sources as the key sees them
/// (the code's tokens without comments, docs/REGENERATE.md §4.3), and `Cargo.lock`.
pub fn rust_key(version: &str, key_sources: &[(&str, &str)], lock: &str) -> String {
    rust_index_key(version, key_sources, lock)
}

/// The kept index under `key`, if there is one.
pub fn kept_index(keep: &Path, key: &str) -> Option<KeptIndex> {
    let kept = KeptIndex::at(keep, key);
    kept.scip.is_file().then_some(kept)
}

/// The key of the index last kept for the project at `root`: the base of a save's map.
pub fn last_key(keep: &Path, root: &Path) -> Option<String> {
    let k = std::fs::read_to_string(pointer(keep, root)).ok()?;
    let k = k.trim().to_string();
    kept_index(keep, &k).map(|_| k)
}

fn pointer(keep: &Path, root: &Path) -> PathBuf {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    keep.join(format!(
        "rust-last-{:016x}.txt",
        text_hash(&root.to_string_lossy())
    ))
}

/***
 * Make the code index of a Cargo project and keep it under `key`, with the texts it was made from and, from `docair-index`, each function's signals; only the newest indexes are kept.
 *
 * `raw_sources` are the files the indexer reads, by path and text, and
 * `Cargo.lock`; the index is the project's last (`last_key`).
 *
 * @calls docair-scip.rust-indexer.full "indexes the project"
 ***/
pub fn make_index(
    indexer: &RustIndexer,
    root: &Path,
    key: &str,
    raw_sources: &[(&str, &str)],
    keep: &Path,
) -> Result<KeptIndex, String> {
    std::fs::create_dir_all(keep).map_err(|e| format!("{}: {e}", keep.display()))?;
    let kept = KeptIndex::at(keep, key);
    let fresh = keep.join(format!("rust-{}.part.scip", run_name()));
    indexer.full(root, &fresh)?;
    let sources: HashMap<&str, &str> = raw_sources.iter().copied().collect();
    let json = serde_json::to_string(&sources).map_err(|e| e.to_string())?;
    std::fs::write(kept.scip.with_extension("sources.json"), json).map_err(|e| e.to_string())?;
    let signals = fresh.with_extension("signals.json");
    if signals.is_file() {
        let _ = std::fs::rename(&signals, kept.scip.with_extension("signals.json"));
    }
    std::fs::rename(&fresh, &kept.scip).map_err(|e| format!("{}: {e}", kept.scip.display()))?;
    remember(keep, root, key);
    // Only the newest are kept, each with its texts and signals.
    let mut old: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(keep)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.starts_with("rust-") && n.ends_with(".scip") && !n.contains(".part")
            })
        })
        .filter_map(|p| Some((p.metadata().ok()?.modified().ok()?, p)))
        .collect();
    old.sort_by_key(|o| std::cmp::Reverse(o.0));
    for (_, p) in old.into_iter().skip(KEPT) {
        let _ = std::fs::remove_file(p.with_extension("sources.json"));
        let _ = std::fs::remove_file(p.with_extension("signals.json"));
        let _ = std::fs::remove_file(p);
    }
    Ok(kept)
}

/// Note `key` as the project's last index.
pub fn remember(keep: &Path, root: &Path, key: &str) {
    let _ = std::fs::write(pointer(keep, root), key);
}

/***
 * The kept code index of a Cargo project with exactly these sources, or a new one, then kept.
 *
 * `sources` are the files rust-analyzer reads, by path and text; with the
 * lock file and the indexer's version they make the key. Only the newest
 * indexes are kept.
 *
 * @calls docair-scip.rust_index_key "works out the key"
 * @calls docair-scip.make_index "makes a new index"
 ***/
pub fn kept_rust_index(
    root: &Path,
    sources: &[(&str, &str)],
    keep: &Path,
) -> Result<PathBuf, String> {
    let indexer =
        RustIndexer::find().ok_or("no code indexer: neither docair-index nor rust-analyzer")?;
    let version = indexer.version().ok_or("the indexer's --version failed")?;
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap_or_default();
    let key = rust_index_key(&version, sources, &lock);
    if let Some(k) = kept_index(keep, &key) {
        return Ok(k.scip);
    }
    let mut raw: Vec<(&str, &str)> = sources.to_vec();
    raw.push(("Cargo.lock", &lock));
    Ok(make_index(&indexer, root, &key, &raw, keep)?.scip)
}

pub mod background;

#[cfg(test)]
mod tests {
    use protobuf::Message;

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

    /// A name of a type, a field or a variant gives the type it belongs to (docs/ADAPTERS.md §5.8).
    #[test]
    fn a_name_gives_its_type() {
        let cli = "rust-analyzer cargo docair-cli 0.1.0 Cli#";
        let of = |s: &str| type_of(s).map(|(t, m)| (t, m.unwrap_or_default()));
        assert_eq!(of(cli), Some((cli.to_string(), String::new())));
        assert_eq!(
            of("rust-analyzer cargo docair-cli 0.1.0 Cli#command."),
            Some((cli.to_string(), "command".into()))
        );
        assert_eq!(
            of("rust-analyzer cargo docair-cli 0.1.0 Command#Build#root."),
            Some((
                "rust-analyzer cargo docair-cli 0.1.0 Command#".into(),
                "Build#root".into()
            ))
        );
        assert_eq!(
            of("rust-analyzer cargo docair-core 0.1.0 model/Graph#nodes."),
            Some((
                "rust-analyzer cargo docair-core 0.1.0 model/Graph#".into(),
                "nodes".into()
            ))
        );
        assert_eq!(
            of("rust-analyzer cargo docair-core 0.1.0 config/impl#[CandidateLimit]DEFAULT."),
            Some((
                "rust-analyzer cargo docair-core 0.1.0 config/CandidateLimit#".into(),
                "DEFAULT".into()
            ))
        );
        assert_eq!(type_of(FREE), None, "a function");
        assert_eq!(type_of(INHERENT), None, "a method");
        assert_eq!(type_of("local 3"), None);
        assert_eq!(
            type_of("rust-analyzer cargo docair-cli 0.1.0 tidy/"),
            None,
            "a module"
        );
        assert_eq!(
            type_of("rust-analyzer cargo docair-core 0.1.0 config/DEFAULT_STORE."),
            None,
            "a constant of no type"
        );
        assert_eq!(
            of("scip-typescript npm docair-web 0.0.0 src/app/`progress-line.ts`/Progress#phase."),
            Some((
                "scip-typescript npm docair-web 0.0.0 src/app/`progress-line.ts`/Progress#".into(),
                "phase".into()
            ))
        );
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

    /// An index of documents, each with (line, symbol, definition) occurrences.
    /// Documents, each with its (line, symbol, definition) occurrences.
    type Docs<'a> = &'a [(&'a str, &'a [(i32, &'a str, bool)])];

    fn index_of(docs: Docs<'_>) -> Index {
        let mut index = scip::types::Index::new();
        for (path, occs) in docs {
            let mut doc = scip::types::Document::new();
            doc.relative_path = path.to_string();
            for (line, symbol, def) in occs.iter() {
                let mut o = scip::types::Occurrence::new();
                o.range = vec![*line, 4, 10];
                o.symbol = symbol.to_string();
                o.symbol_roles = if *def { DEFINITION } else { 0 };
                doc.occurrences.push(o);
            }
            index.documents.push(doc);
        }
        Index::parse(&index.write_to_bytes().unwrap()).unwrap()
    }

    #[test]
    fn a_file_moves_to_its_new_text_and_takes_a_short_runs_word() {
        let mut i = index_of(&[
            (
                "a.rs",
                &[(2, FREE, true), (20, FREE, false), (21, TRAIT, false)],
            ),
            ("b.rs", &[(9, IMPL, true)]),
        ]);
        // A comment above: every position one line down; the call on line 22 is dropped.
        i.remap_file("a.rs", &|l, c| (l != 22).then_some((l + 1, c)));
        assert_eq!(i.definition(FREE), Some(("a.rs", 4)));
        let calls: Vec<_> = i.calls("a.rs").iter().map(|r| r.line).collect();
        assert_eq!(
            calls,
            [22],
            "the call on old line 21 moved; the one on 22 was dropped"
        );
        // A function edited since: its lines are a gap the syntax reads.
        i.set_gaps("a.rs", vec![(20, 25)]);
        assert!(i.covers_line("a.rs", 4) && !i.covers_line("a.rs", 22));
        // A short run's word on b.rs takes the place of what the index held.
        let short = index_of(&[("b.rs", &[(12, IMPL, true), (14, FREE, false)])]);
        i.replace_file("b.rs", &short);
        assert_eq!(i.definition(IMPL), Some(("b.rs", 13)));
        assert_eq!(i.calls("b.rs").len(), 1);
        assert_eq!(
            i.paths().collect::<Vec<_>>(),
            ["a.rs", "b.rs"],
            "b.rs keeps its place"
        );
        i.drop_file("a.rs");
        assert!(!i.covers("a.rs") && i.definition(FREE).is_none());
    }

    #[test]
    fn a_kept_index_comes_with_its_texts_and_is_the_projects_last() {
        let keep = std::env::temp_dir().join(format!("docair-scip-test-{}", std::process::id()));
        let root = keep.join("project");
        std::fs::create_dir_all(&root).unwrap();
        let fake = keep.join("indexer.sh");
        std::fs::write(
            &fake,
            "#!/bin/sh\n# full --root R --out O --signals S\nprintf 'x' > \"$5\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::PermissionsExt::set_mode(
            &mut std::fs::metadata(&fake).unwrap().permissions(),
            0o755,
        );
        #[cfg(unix)]
        std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let indexer = RustIndexer::DocairIndex(fake);
        let kept =
            make_index(&indexer, &root, "k1", &[("src/lib.rs", "fn a() {}")], &keep).unwrap();
        assert!(kept.scip.is_file());
        assert_eq!(
            kept.sources().get("src/lib.rs").map(String::as_str),
            Some("fn a() {}")
        );
        assert_eq!(last_key(&keep, &root).as_deref(), Some("k1"));
        assert!(kept_index(&keep, "k2").is_none());
        let _ = std::fs::remove_dir_all(&keep);
    }
}

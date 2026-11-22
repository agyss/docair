//! Loading a Cargo workspace into rust-analyzer, and shaping what it sees (docs/REGENERATE.md
//! §4.5): crates left out get empty files, and the workspace's files a short run does not index
//! get their function bodies left out. Only the input changes; rust-analyzer is not forked.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use ra_ap_hir::{ChangeWithProcMacros, Crate};
use ra_ap_ide::AnalysisHost;
use ra_ap_load_cargo::{LoadCargoConfig, ProcMacroServerChoice, load_workspace_at};
use ra_ap_project_model::{CargoConfig, RustLibSource};
use ra_ap_syntax::{AstNode, Edition, SourceFile, ast};
use ra_ap_vfs::{AbsPathBuf, FileId, Vfs};

/// A workspace loaded into rust-analyzer.
pub struct Loaded {
    pub host: AnalysisHost,
    pub vfs: Vfs,
    pub root: AbsPathBuf,
}

/***
 * Load the Cargo workspace at `root` as `rust-analyzer scip` does: std's sources, every target, `cfg(test)`, build scripts' outputs, and proc macros through the toolchain's server.
 ***/
pub fn load(root: &Path, whole: bool) -> Result<Loaded> {
    let root = AbsPathBuf::assert_utf8(root.canonicalize()?);
    let cargo = CargoConfig {
        sysroot: Some(RustLibSource::Discover),
        all_targets: true,
        set_test: true,
        ..Default::default()
    };
    let config = LoadCargoConfig {
        load_out_dirs_from_check: true,
        with_proc_macro_server: ProcMacroServerChoice::Sysroot,
        // The whole workspace: name resolution of every crate first, on every core, as
        // `rust-analyzer scip` does; a short run resolves what its files need, on one.
        prefill_caches: whole,
        num_worker_threads: if whole {
            std::thread::available_parallelism().map_or(1, |n| n.get())
        } else {
            1
        },
        proc_macro_processes: 1,
    };
    let (db, vfs, _) = load_workspace_at(root.as_ref(), &cargo, &config, &|_| {})?;
    Ok(Loaded {
        host: AnalysisHost::with_database(db),
        vfs,
        root,
    })
}

impl Loaded {
    /// A file's path relative to the root, for a file under it.
    pub fn rel(&self, file: FileId) -> Option<String> {
        let p = self
            .vfs
            .file_path(file)
            .as_path()?
            .strip_prefix(&self.root)?
            .as_str()
            .to_owned();
        Some(p)
    }

    /// The file at a path relative to the root.
    pub fn file_id(&self, rel: &str) -> Option<FileId> {
        let want = self.root.join(rel);
        self.vfs
            .iter()
            .find(|(_, p)| p.as_path().is_some_and(|p| want == p))
            .map(|(id, _)| id)
    }

    /// The path of a file, as rust-analyzer read it.
    pub fn path(&self, file: FileId) -> Option<String> {
        self.vfs
            .file_path(file)
            .as_path()
            .map(|p| p.as_str().to_string())
    }

    /// The Rust files of the workspace's own crates: under the root, outside `target`.
    pub fn workspace_files(&self) -> Vec<FileId> {
        let mut out: Vec<(String, FileId)> = self
            .vfs
            .iter()
            .filter_map(|(id, _)| {
                let rel = self.rel(id)?;
                (rel.ends_with(".rs") && !rel.starts_with("target/") && !rel.contains("/target/"))
                    .then_some((rel, id))
            })
            .collect();
        out.sort();
        out.into_iter().map(|(_, id)| id).collect()
    }

    /// Give files new texts, before anything is analysed.
    pub fn set_texts(&mut self, texts: Vec<(FileId, String)>) {
        if texts.is_empty() {
            return;
        }
        let mut change = ChangeWithProcMacros::default();
        for (f, t) in texts {
            change.change_file(f, Some(t));
        }
        self.host.apply_change(change);
    }
}

/// A crate's folder: the nearest folder above its root file that holds a `Cargo.toml`.
pub fn package_dir(root_file: &str) -> String {
    let mut d = Path::new(root_file).parent();
    while let Some(dir) = d {
        if dir.join("Cargo.toml").is_file() {
            return format!("{}/", dir.display());
        }
        d = dir.parent();
    }
    root_file
        .rsplit_once('/')
        .map_or(root_file.to_string(), |(d, _)| format!("{d}/"))
}

/***
 * Leave out what the rule does not keep (docs/REGENERATE.md §4.5): each such crate's root file empty and its other files blanked, core's SIMD intrinsics an empty module, and the files of crates outside the closure of the targets' crates blanked. The number of crates left out.
 ***/
pub fn shape(l: &mut Loaded, rule: &crate::rule::Rule, targets: &[FileId]) -> usize {
    let db = l.host.raw_database();
    let left: HashSet<&str> = rule.left_out.iter().map(|(p, _)| p.as_str()).collect();
    let dirs: Vec<String> = rule.left_out.iter().map(|(p, _)| package_dir(p)).collect();
    // The closure of the crates the targets belong to.
    let target_paths: Vec<String> = targets.iter().filter_map(|f| l.path(*f)).collect();
    let (inside, outside): (Vec<String>, Vec<String>) = ra_ap_hir::attach_db(db, || {
        let all = Crate::all(db);
        let dir_of = |k: &Crate| package_dir(&l.path(k.root_file(db)).unwrap_or_default());
        let mine: Vec<Crate> = all
            .iter()
            .copied()
            .filter(|k| {
                k.origin(db).is_local() && target_paths.iter().any(|t| t.starts_with(&dir_of(k)))
            })
            .collect();
        let mut seen = HashSet::new();
        let mut todo = mine;
        while let Some(k) = todo.pop() {
            if seen.insert(k) {
                todo.extend(k.dependencies(db).into_iter().map(|d| d.krate));
            }
        }
        (
            seen.iter().map(dir_of).collect(),
            all.iter()
                .filter(|k| !seen.contains(k))
                .map(dir_of)
                .collect(),
        )
    });
    let outside: Vec<String> = outside
        .into_iter()
        .filter(|d| {
            !inside
                .iter()
                .any(|i| i.starts_with(d.as_str()) || d.starts_with(i.as_str()))
        })
        .collect();
    let mut texts = Vec::new();
    for (id, p) in l.vfs.iter() {
        let Some(p) = p.as_path().map(|p| p.as_str().to_string()) else {
            continue;
        };
        if let Some((_, stub)) = rule.left_out.iter().find(|(r, _)| *r == p) {
            texts.push((id, stub.clone()));
        } else if p.ends_with(".rs")
            && (dirs.iter().any(|d| p.starts_with(d.as_str()))
                || outside.iter().any(|d| p.starts_with(d.as_str())))
        {
            texts.push((id, String::new()));
        }
    }
    let n = left.len();
    l.set_texts(texts);
    n
}

/***
 * Leave out the function bodies of the workspace's files that are not indexed now: what other files mean rests on items and signatures, which stay (docs/REGENERATE.md §4.4). A body with an `impl` in it stays: an impl counts everywhere.
 ***/
pub fn strip_bodies(l: &mut Loaded, keep: &[FileId]) {
    let keep: HashSet<FileId> = keep.iter().copied().collect();
    let mut texts = Vec::new();
    for f in l.workspace_files() {
        if keep.contains(&f) {
            continue;
        }
        let Some(rel) = l.rel(f) else { continue };
        let Ok(text) = std::fs::read_to_string(l.root.join(&rel)) else {
            continue;
        };
        let file = SourceFile::parse(&text, Edition::CURRENT).tree();
        let mut bodies: Vec<(usize, usize)> = file
            .syntax()
            .descendants()
            .filter_map(ast::Fn::cast)
            .filter_map(|f| f.body())
            .map(|b| b.syntax().text_range())
            .map(|r| (u32::from(r.start()) as usize, u32::from(r.end()) as usize))
            .filter(|(a, b)| !text[*a..*b].contains("impl"))
            .collect();
        // Outer bodies hold inner ones: keep the outermost of each.
        bodies.sort();
        let mut outer: Vec<(usize, usize)> = Vec::new();
        for b in bodies {
            if outer.last().is_none_or(|o| b.0 >= o.1) {
                outer.push(b);
            }
        }
        if outer.is_empty() {
            continue;
        }
        let mut new = String::with_capacity(text.len());
        let mut at = 0;
        for (a, b) in outer {
            new.push_str(&text[at..a]);
            new.push_str("{}");
            at = b;
        }
        new.push_str(&text[at..]);
        texts.push((f, new));
    }
    l.set_texts(texts);
}

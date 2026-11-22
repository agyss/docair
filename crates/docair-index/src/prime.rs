//! Name resolution before a short run indexes its files (docair-live.64): rust-analyzer's caches
//! of the crates the files need, filled ahead on several threads. Resolving what the files' tokens name then
//! finds the crates' names and items ready, instead of working them out on one thread as it goes.

use std::collections::HashSet;

use ra_ap_hir::Crate;

use crate::load::Loaded;

/// How many threads prime the caches: more than 8 gained nothing on docair (docs/REGENERATE.md §4.11).
const THREADS: usize = 8;

/// The crates the files belong to and everything they depend on, dependencies first.
fn closure(l: &Loaded, files: &[ra_ap_vfs::FileId]) -> Vec<Crate> {
    let db = l.host.raw_database();
    let paths: Vec<String> = files.iter().filter_map(|f| l.path(*f)).collect();
    ra_ap_hir::attach_db(db, || {
        fn visit(
            db: &ra_ap_ide_db::RootDatabase,
            k: Crate,
            seen: &mut HashSet<Crate>,
            out: &mut Vec<Crate>,
        ) {
            if !seen.insert(k) {
                return;
            }
            for d in k.dependencies(db) {
                visit(db, d.krate, seen, out);
            }
            out.push(k);
        }
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for k in Crate::all(db) {
            let dir = crate::load::package_dir(&l.path(k.root_file(db)).unwrap_or_default());
            if k.origin(db).is_local() && paths.iter().any(|p| p.starts_with(&dir)) {
                visit(db, k, &mut seen, &mut out);
            }
        }
        out
    })
}

/***
 * Fill rust-analyzer's caches (def maps, import maps, symbols) of the files' crates and what they depend on, on up to 8 threads, as the whole index's load does for every crate on every core.
 ***/
pub fn caches(l: &Loaded, files: &[ra_ap_vfs::FileId]) {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(THREADS));
    let scope: Vec<_> = closure(l, files).into_iter().map(|k| k.base()).collect();
    let _ = l
        .host
        .analysis()
        .parallel_prime_caches(&scope, threads, |_| {});
}

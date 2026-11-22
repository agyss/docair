//! Code indexes made in the background in `docair serve` (docs/REGENERATE.md §4.2): the short
//! run after a save, and the full index once serve has been idle. One worker makes them, one at
//! a time; when one lands, serve is told and writes the documentation again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::{Index, KeptIndex, RustIndexer, text_hash};

/// How long serve is quiet before it makes the full index (docs/REGENERATE.md §4.8).
pub const IDLE: Duration = Duration::from_secs(60);

/// What serve does when an index lands: write the documentation again.
pub type Landed = Arc<dyn Fn() + Send + Sync>;

/// A short run asked for (docs/REGENERATE.md §4.4).
#[derive(Debug, Clone)]
pub struct ShortRequest {
    pub root: PathBuf,
    pub keep: PathBuf,
    /// `docair-index`.
    pub bin: PathBuf,
    /// The index the map is based on: its signals are the guard's base.
    pub base: KeptIndex,
    /// The files to resolve, relative to the root.
    pub files: Vec<String>,
}

/// The full index asked for, made once serve has been idle (docs/REGENERATE.md §4.8).
#[derive(Debug, Clone)]
pub struct FullRequest {
    pub root: PathBuf,
    pub keep: PathBuf,
    pub indexer: RustIndexer,
    pub key: String,
    /// The files the indexer reads and `Cargo.lock`, by path and text, kept beside the index.
    pub raw_sources: Vec<(String, String)>,
}

#[derive(Default)]
struct State {
    landed: Option<Landed>,
    short: Option<ShortRequest>,
    full: Option<FullRequest>,
    /// The full index is wanted at once, not once serve is idle: Cargo's files changed.
    full_now: bool,
    last_request: Option<Instant>,
    /// What short runs said about files, by path: the hash of the text they read, and the index.
    overlays: HashMap<String, (u64, Arc<Index>)>,
    worker: bool,
}

fn state() -> &'static (Mutex<State>, Condvar) {
    static STATE: OnceLock<(Mutex<State>, Condvar)> = OnceLock::new();
    STATE.get_or_init(|| (Mutex::new(State::default()), Condvar::new()))
}

fn lock() -> MutexGuard<'static, State> {
    state().0.lock().unwrap_or_else(|e| e.into_inner())
}

/// What to do when an index made in the background lands: serve writes the documentation again.
pub fn set_on_landed(f: Landed) {
    lock().landed = Some(f);
}

/// What a short run said about a file, if it read this text of it: an index holding the file.
pub fn overlay(path: &str, hash: u64) -> Option<Arc<Index>> {
    lock()
        .overlays
        .get(path)
        .filter(|(h, _)| *h == hash)
        .map(|(_, i)| i.clone())
}

/// The files short runs said something about, with the hash of the text they read.
pub fn overlaid() -> Vec<(String, u64, Arc<Index>)> {
    lock()
        .overlays
        .iter()
        .map(|(p, (h, i))| (p.clone(), *h, i.clone()))
        .collect()
}

/// Forget what short runs said: a full index holds everything now.
pub fn clear_overlays() {
    lock().overlays.clear();
}

/// Ask for a short run; a newer request takes the place of one not started yet.
pub fn request_short(req: ShortRequest) {
    let mut s = lock();
    s.short = Some(req);
    s.last_request = Some(Instant::now());
    start(&mut s);
    state().1.notify_all();
}

/// Ask for the full index, made once serve has been quiet for `IDLE`, or `now`; a newer request
/// takes the place of the last.
pub fn request_full(req: FullRequest, now: bool) {
    let mut s = lock();
    s.full = Some(req);
    s.full_now |= now;
    s.last_request.get_or_insert_with(Instant::now);
    start(&mut s);
    state().1.notify_all();
}

/// Whether a full index is asked for and not made yet.
pub fn full_pending() -> bool {
    lock().full.is_some()
}

fn start(s: &mut State) {
    if !s.worker {
        s.worker = true;
        std::thread::spawn(work);
    }
}

enum Job {
    Short(ShortRequest),
    Full(FullRequest),
}

fn work() {
    loop {
        let job = {
            let (m, cv) = state();
            let mut s = m.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if let Some(r) = s.short.take() {
                    break Job::Short(r);
                }
                if s.full.is_some() {
                    let quiet = s.last_request.map_or(IDLE, |t| t.elapsed());
                    if quiet >= IDLE || s.full_now {
                        s.full_now = false;
                        break Job::Full(s.full.take().expect("asked for"));
                    }
                    s = cv
                        .wait_timeout(s, IDLE - quiet)
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                } else {
                    s = cv.wait(s).unwrap_or_else(|e| e.into_inner());
                }
            }
        };
        let landed = match job {
            Job::Short(r) => short(&r),
            Job::Full(r) => full(&r),
        };
        if landed && let Some(f) = lock().landed.clone() {
            f();
        }
    }
}

/// Whether short runs leave crates out for this project: not after the full index found them
/// different (docs/REGENERATE.md §4.8), until `Cargo.lock` changes.
fn rule_off(keep: &Path, root: &Path) -> bool {
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap_or_default();
    std::fs::read_to_string(rule_flag(keep, root))
        .is_ok_and(|t| t.trim() == format!("{:016x}", text_hash(&lock)))
}

fn rule_flag(keep: &Path, root: &Path) -> PathBuf {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    keep.join(format!(
        "rule-off-{:016x}.txt",
        text_hash(&root.to_string_lossy())
    ))
}

/// Short runs load everything for this project from now on, until `Cargo.lock` changes: the full
/// index found something they said different.
pub fn turn_rule_off(keep: &Path, root: &Path) {
    let lock = std::fs::read_to_string(root.join("Cargo.lock")).unwrap_or_default();
    let _ = std::fs::write(rule_flag(keep, root), format!("{:016x}", text_hash(&lock)));
}

fn short(r: &ShortRequest) -> bool {
    let t = Instant::now();
    let out = r
        .keep
        .join(format!("rust-short-{}.part.scip", std::process::id()));
    let mut cmd = Command::new(&r.bin);
    cmd.arg("files")
        .arg("--root")
        .arg(&r.root)
        .arg("--out")
        .arg(&out)
        .arg("--cache")
        .arg(&r.keep)
        .arg("--files")
        .arg(r.files.join(","));
    match r.base.signals() {
        Some(base) if !rule_off(&r.keep, &r.root) => {
            cmd.arg("--signals-base").arg(base);
        }
        _ => {
            cmd.arg("--no-stubs");
        }
    }
    let run = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("docair serve: docair-index: {e}");
            return false;
        }
    };
    let report: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap_or_default();
    if !run.status.success() {
        let err = String::from_utf8_lossy(&run.stderr);
        eprintln!(
            "docair serve: docair-index failed: {}",
            err.lines().last().unwrap_or("no output")
        );
        return false;
    }
    let index = match Index::read(&out) {
        Ok(i) => Arc::new(i),
        Err(e) => {
            eprintln!("docair serve: docair-index: {e}");
            return false;
        }
    };
    let _ = std::fs::remove_file(&out);
    let texts = report["texts"].as_object().cloned().unwrap_or_default();
    {
        let mut s = lock();
        for (path, hash) in &texts {
            if let Some(h) = hash.as_str().and_then(|h| u64::from_str_radix(h, 16).ok()) {
                s.overlays.insert(path.clone(), (h, index.clone()));
            }
        }
    }
    eprintln!(
        "docair serve: code index of {} file(s) in {:.1} s ({} crates left out; the guard: {})",
        texts.len(),
        t.elapsed().as_secs_f64(),
        report["left_out"].as_u64().unwrap_or(0),
        report["guard"].as_str().unwrap_or("?")
    );
    true
}

/// Whether the files on disk still have the texts a request was made with.
fn still(r: &FullRequest) -> bool {
    r.raw_sources
        .iter()
        .all(|(p, t)| std::fs::read_to_string(r.root.join(p)).is_ok_and(|now| now == *t))
}

fn full(r: &FullRequest) -> bool {
    let t = Instant::now();
    // The index is kept under the key of the texts the request was made with: when the files
    // changed since, or change while it is made, it would hold other texts than its key says.
    if !still(r) {
        return false;
    }
    let raw: Vec<(&str, &str)> = r
        .raw_sources
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    match crate::make_index(&r.indexer, &r.root, &r.key, &raw, &r.keep) {
        Ok(kept) if !still(r) => {
            let _ = std::fs::remove_file(kept.scip.with_extension("sources.json"));
            let _ = std::fs::remove_file(kept.scip.with_extension("signals.json"));
            let _ = std::fs::remove_file(&kept.scip);
            false
        }
        Ok(_) => {
            eprintln!(
                "docair serve: the full code index in {:.1} s",
                t.elapsed().as_secs_f64()
            );
            true
        }
        Err(e) => {
            eprintln!("docair serve: the full code index: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_index_is_made_only_of_the_texts_it_was_asked_for() {
        let root = std::env::temp_dir().join(format!("docair-bg-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "fn a() {}").unwrap();
        let r = FullRequest {
            root: root.clone(),
            keep: root.join("keep"),
            indexer: RustIndexer::RustAnalyzer,
            key: "k".into(),
            raw_sources: vec![("src/lib.rs".into(), "fn a() {}".into())],
        };
        assert!(still(&r), "the file is as asked");
        std::fs::write(root.join("src/lib.rs"), "fn b() {}").unwrap();
        assert!(
            !still(&r),
            "the file changed since: no index under the old key"
        );
        assert!(!full(&r), "nothing is made");
        assert!(!r.keep.join("rust-k.scip").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}

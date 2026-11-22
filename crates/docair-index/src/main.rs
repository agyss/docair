//! `docair-index`: docair's code indexer (docs/REGENERATE.md §4.4, §4.8).
//!
//! rust-analyzer as a library: it writes a Cargo project's SCIP index, whole (`full`, in place of
//! `rust-analyzer scip`, with the same occurrences), or for the files a save changed (`files`),
//! with only what those files need loaded and a guard that loads everything when that was not
//! enough. Both add lines for code a derive generates (§4.7). The process exits when done:
//! nothing of rust-analyzer stays in memory.

mod guard;
mod load;
mod moniker;
mod prime;
mod rule;
mod write;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::{Parser, Subcommand};

/// With `DOCAIR_TIMING=1`, say on stderr how long a part of a step took.
pub fn say(what: &str, d: std::time::Duration) {
    if std::env::var("DOCAIR_TIMING").is_ok_and(|v| !v.is_empty() && v != "0") {
        eprintln!(
            "docair-index timing: {what}: {:.1} ms",
            d.as_secs_f64() * 1000.0
        );
    }
}

/// With `DOCAIR_TIMING=1`, say on stderr how long a step took since `t`, as docair does.
fn timing(step: &str, t: &mut Instant) {
    if std::env::var("DOCAIR_TIMING").is_ok_and(|v| !v.is_empty() && v != "0") {
        eprintln!(
            "docair-index timing: {step}: {:.1} ms",
            t.elapsed().as_secs_f64() * 1000.0
        );
    }
    *t = Instant::now();
}

/// The version: docair's, and the rust-analyzer crates it links.
const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (rust-analyzer ra_ap 0.0.358)");

#[derive(Parser)]
#[command(name = "docair-index", version = VERSION, about = "docair's code indexer: rust-analyzer as a library")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Index the whole project, everything loaded; write each function's signals for the guard.
    Full {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        signals: Option<PathBuf>,
    },
    /// Index the given files (relative to the root) with only what they need loaded.
    Files {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, value_delimiter = ',')]
        files: Vec<String>,
        /// Where the rule's list of crates left out is kept.
        #[arg(long)]
        cache: Option<PathBuf>,
        /// The full index's signals: the guard compares with them.
        #[arg(long)]
        signals_base: Option<PathBuf>,
        /// Load everything: leave no crate out.
        #[arg(long)]
        no_stubs: bool,
    },
}

/***
 * Index a Cargo project, whole or for some files, and write the SCIP index.
 *
 * @calls docair-index.full "indexes the whole project"
 * @calls docair-index.files "indexes the files a save changed"
 ***/
fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Full { root, out, signals } => full(&root, &out, signals.as_deref()),
        Cmd::Files {
            root,
            out,
            files,
            cache,
            signals_base,
            no_stubs,
        } => self::files(
            &root,
            &out,
            &files,
            cache.as_deref(),
            signals_base.as_deref(),
            no_stubs,
        ),
    }
}

/***
 * The whole project, everything loaded: the SCIP index and each function's signals.
 *
 * @calls docair-index.load.load "loads the workspace"
 * @calls docair-index.write.index "makes the index"
 * @calls docair-index.guard.signals "counts each function's signals"
 ***/
fn full(
    root: &std::path::Path,
    out: &std::path::Path,
    signals: Option<&std::path::Path>,
) -> Result<()> {
    let t = Instant::now();
    let mut step = Instant::now();
    let l = load::load(root, true)?;
    timing("load", &mut step);
    let files = l.workspace_files();
    let index = write::index(&l, &files);
    timing("the index", &mut step);
    write::save(index, out)?;
    if let Some(p) = signals {
        guard::write(&guard::signals(&l, &files), p)?;
        timing("the signals", &mut step);
    }
    eprintln!(
        "docair-index: {} files in {:.1} s",
        files.len(),
        t.elapsed().as_secs_f64()
    );
    Ok(())
}

/***
 * The files a save changed: loaded with only what they need, the rest of the workspace's bodies left out, the guard checked; a JSON report on stdout.
 *
 * @calls docair-index.load.load "loads the workspace"
 * @calls docair-index.rule.rule "works out which crates are left out"
 * @calls docair-index.load.shape "leaves them out"
 * @calls docair-index.load.strip_bodies "leaves out the other files' function bodies"
 * @calls docair-index.prime.caches "resolves names on several threads first"
 * @calls docair-index.write.index "makes the index of the files"
 * @calls docair-index.guard.signals "counts the files' functions' signals"
 ***/
fn files(
    root: &std::path::Path,
    out: &std::path::Path,
    rels: &[String],
    cache: Option<&std::path::Path>,
    signals_base: Option<&std::path::Path>,
    no_stubs: bool,
) -> Result<()> {
    let t = Instant::now();
    let mut step = Instant::now();
    let mut l = load::load(root, false)?;
    timing("load", &mut step);
    let targets: Vec<ra_ap_vfs::FileId> = rels.iter().filter_map(|r| l.file_id(r)).collect();
    let texts: BTreeMap<String, String> = targets
        .iter()
        .filter_map(|f| {
            let rel = l.rel(*f)?;
            let text = std::fs::read_to_string(l.root.join(&rel)).ok()?;
            Some((rel, format!("{:016x}", docair_scip::text_hash(&text))))
        })
        .collect();
    let base = signals_base.map(guard::read).transpose()?;
    let stubbing = !no_stubs && base.is_some();
    let mut left_out = 0;
    if stubbing {
        let rule = rule::rule(&l, cache)?;
        timing("the rule", &mut step);
        left_out = load::shape(&mut l, &rule, &targets);
        timing("left out", &mut step);
    }
    load::strip_bodies(&mut l, &targets);
    timing("bodies left out", &mut step);
    prime::caches(&l, &targets);
    timing("names resolved first", &mut step);
    let mut index = write::index_files(&l, &targets);
    timing("the index", &mut step);
    let mut guard = if stubbing {
        "kept"
    } else {
        "everything loaded"
    };
    if let Some(base) = base.as_ref().filter(|_| stubbing) {
        let fired = guard::fires(&guard::signals(&l, &targets), base);
        timing("the guard", &mut step);
        if !fired.is_empty() {
            eprintln!(
                "docair-index: the guard: more signals than the full index in {}; loading everything",
                fired.join(", ")
            );
            l = load::load(root, false)?;
            // A new load has new file ids.
            let targets: Vec<ra_ap_vfs::FileId> =
                rels.iter().filter_map(|r| l.file_id(r)).collect();
            load::strip_bodies(&mut l, &targets);
            prime::caches(&l, &targets);
            index = write::index_files(&l, &targets);
            guard = "redone with everything loaded";
            left_out = 0;
        }
    }
    write::save(index, out)?;
    println!(
        "{}",
        serde_json::json!({
            "texts": texts,
            "left_out": left_out,
            "guard": guard,
            "seconds": t.elapsed().as_secs_f64(),
        })
    );
    Ok(())
}

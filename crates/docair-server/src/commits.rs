//! Two commits of a project compared: each read from a detached git worktree in a scratch
//! folder, scanned, and its graph kept in the cache folder; the graph diff, the changed lines
//! and the flows through the changed parts (docs/CHANGES.md). The CLI's `docair diff` and
//! `docair explain` and the server's `POST /api/explain` share it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use docair_core::model::Graph;
use docair_core::{Options, scan};
use serde::{Deserialize, Serialize};

/// A commit's graph and its parts' fingerprints, as kept in the cache.
#[derive(Serialize, Deserialize)]
pub struct KeptGraph {
    pub graph: Graph,
    pub fingerprints: HashMap<String, String>,
}

pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .context("running git")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// The key a commit's graph is kept under: the commit, docair's version and the commit's docair.toml.
fn graph_key(top: &Path, sha: &str, prefix: &str) -> String {
    let toml = git(top, &["show", &format!("{sha}:{prefix}docair.toml")]).unwrap_or_default();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in env!("CARGO_PKG_VERSION")
        .bytes()
        .chain([0])
        .chain(toml.bytes())
    {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{sha}-{h:016x}")
}

/***
 * A commit's graph: kept in the cache folder, else scanned from a detached git worktree in a scratch folder that is removed afterwards.
 *
 * @calls docair-core.scan "scans the commit"
 * @calls docair-core.diff.fingerprints "fingerprints its parts"
 ***/
pub fn graph_at(top: &Path, prefix: &str, sha: &str, lenient: bool) -> Result<KeptGraph> {
    let keep = docair_scip::cache_dir().join("graphs");
    let path = keep.join(format!("{}.json", graph_key(top, sha, prefix)));
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(kept) = serde_json::from_str::<KeptGraph>(&text)
    {
        return Ok(kept);
    }
    let scratch = tempfile::Builder::new().prefix("docair-diff-").tempdir()?;
    let tree = scratch.path().join("tree");
    let tree_s = tree.to_string_lossy().to_string();
    git(
        top,
        &["worktree", "add", "--detach", "--quiet", &tree_s, sha],
    )?;
    let root = tree.join(prefix);
    let scanned = scan(
        &root,
        docair_adapters::all(),
        &Options {
            lenient,
            ..Options::default()
        },
    );
    let fingerprints = scanned
        .as_ref()
        .map(|g| {
            docair_core::diff::fingerprints(g, &|p| std::fs::read_to_string(root.join(p)).ok())
        })
        .unwrap_or_default();
    // The worktree is ours: removed whatever the scan found.
    let _ = git(top, &["worktree", "remove", "--force", &tree_s]);
    let kept = KeptGraph {
        graph: scanned?,
        fingerprints,
    };
    std::fs::create_dir_all(&keep)?;
    std::fs::write(&path, serde_json::to_string(&kept)?)?;
    Ok(kept)
}

/// Two commits compared: both graphs, the changes, and the names the commits go by in a store.
pub struct CommitPair {
    pub root: PathBuf,
    pub a: Graph,
    pub b: Graph,
    pub changes: docair_core::changes::Changes,
    /// `<sha of A>..<sha of B or "work">`, short, for the store's file name.
    pub key: String,
}

/***
 * Compare two commits, or a commit and the working tree: both graphs (kept in the cache), the graph diff, the changed lines and the flows through the changed parts.
 *
 * @calls docair-server.commits.graph_at "reads each commit's graph"
 * @calls docair-core.diff.diff "compares the graphs"
 * @calls docair-core.changes.changes "compares the flows through the changed parts"
 ***/
pub fn compare(root: &Path, a: &str, b: Option<&str>, lenient: bool) -> Result<CommitPair> {
    compare_inner(root, a, b, None, lenient)
}

/// As `compare` with the working tree as B, whose graph is already built (`docair serve` keeps it).
pub fn compare_work(root: &Path, a: &str, work: &Graph, lenient: bool) -> Result<CommitPair> {
    compare_inner(root, a, None, Some(work), lenient)
}

fn compare_inner(
    root: &Path,
    a: &str,
    b: Option<&str>,
    work: Option<&Graph>,
    lenient: bool,
) -> Result<CommitPair> {
    let root = root
        .canonicalize()
        .with_context(|| format!("{}", root.display()))?;
    let top = PathBuf::from(git(&root, &["rev-parse", "--show-toplevel"])?.trim());
    let prefix = root
        .strip_prefix(&top)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
        .filter(|p| !p.is_empty())
        .map_or(String::new(), |p| format!("{p}/"));
    let sha = |rev: &str| -> Result<String> {
        Ok(git(
            &top,
            &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
        )?
        .trim()
        .to_string())
    };
    let sha_a = sha(a)?;
    let at_a = graph_at(&top, &prefix, &sha_a, lenient)?;
    let (at_b, name_b, range) = match b {
        Some(b) => {
            let sha_b = sha(b)?;
            (
                graph_at(&top, &prefix, &sha_b, lenient)?,
                b.to_string(),
                vec![sha_a.clone(), sha_b],
            )
        }
        None => {
            let g = match work {
                Some(g) => g.clone(),
                None => scan(
                    &root,
                    docair_adapters::all(),
                    &Options {
                        lenient,
                        ..Options::default()
                    },
                )?,
            };
            let fingerprints = docair_core::diff::fingerprints(&g, &|p| {
                std::fs::read_to_string(root.join(p)).ok()
            });
            (
                KeptGraph {
                    graph: g,
                    fingerprints,
                },
                "the working tree".to_string(),
                vec![sha_a.clone()],
            )
        }
    };
    let mut args: Vec<&str> = vec!["diff", "-U0", "--no-color", "--no-ext-diff"];
    args.extend(range.iter().map(String::as_str));
    let scope = if prefix.is_empty() {
        ".".to_string()
    } else {
        prefix.clone()
    };
    args.extend(["--", scope.as_str()]);
    let unified = git(&top, &args)?;
    // Paths as the project knows them: under its folder in the repository.
    let lines: Vec<docair_core::changes::LinesTouched> =
        docair_core::changes::changed_lines(&unified)
            .into_iter()
            .map(|mut c| {
                let strip = |f: Option<String>| {
                    f.map(|f| f.strip_prefix(&prefix).map_or(f.clone(), str::to_string))
                };
                c.old_file = strip(c.old_file);
                c.new_file = strip(c.new_file);
                c
            })
            .collect();
    let graph = docair_core::diff::diff(
        &at_a.graph,
        &at_b.graph,
        &at_a.fingerprints,
        &at_b.fingerprints,
    );
    let mut changes =
        docair_core::changes::changes(&at_a.graph, &at_b.graph, graph, &lines, (a, &name_b));
    changes.working_tree = b.is_none();
    let short = |s: &str| s.chars().take(12).collect::<String>();
    let key = format!(
        "{}..{}",
        short(&sha_a),
        range.get(1).map_or("work".to_string(), |s| short(s))
    );
    Ok(CommitPair {
        root,
        a: at_a.graph,
        b: at_b.graph,
        changes,
        key,
    })
}

/// Where `docair explain` keeps the findings for this pair: in `[explain] store` (default `docs/changes`), `<key>.json`.
pub fn findings_file(pair: &CommitPair) -> PathBuf {
    let store = docair_core::config::load(&pair.root)
        .ok()
        .and_then(|c| c.explain)
        .and_then(|e| e.store)
        .unwrap_or_else(|| "docs/changes".to_string());
    pair.root.join(store).join(format!("{}.json", pair.key))
}

/***
 * B's page with its changes section: what changed since A, with the findings `docair explain` stored for the pair, when there are any (docs/CHANGES.md §5).
 *
 * @calls docair-viewer.doc_page.render_page_with "writes the page with its changes section"
 ***/
pub fn changes_page(pair: &CommitPair) -> String {
    let findings: Option<serde_json::Value> = std::fs::read_to_string(findings_file(pair))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let changed = docair_viewer::doc_page::Changed {
        before: &pair.a,
        changes: &pair.changes,
        findings: findings.as_ref(),
    };
    docair_viewer::doc_page::render_page_with(
        &pair.b,
        Some(&docair_scip::cache_dir().join("layouts")),
        Some(&changed),
    )
}

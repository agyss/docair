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

/// A commit's graph and its parts' fingerprints, as built and written to `.docair/changes/graphs`.
#[derive(Clone, Serialize, Deserialize)]
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

/// The graphs this process built, by `graph_key` (docair-zrr.30): a commit's files never change, and
/// neither does the docair that runs, so one built here is never stale. Nothing is read from an
/// earlier run.
static BUILT: std::sync::OnceLock<std::sync::Mutex<HashMap<String, KeptGraph>>> =
    std::sync::OnceLock::new();

/// The key a commit's graph is known by in this process: the commit, docair's version and the commit's docair.toml.
fn graph_key(top: &Path, sha: &str, prefix: &str) -> String {
    let toml = git(top, &["show", &format!("{sha}:{prefix}docair.toml")]).unwrap_or_default();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    // The fingerprints' format (`loose/exact`, docair-zrr.29) is part of what a kept graph holds.
    for b in env!("CARGO_PKG_VERSION")
        .bytes()
        .chain(*b"\0fp2\0")
        .chain(toml.bytes())
    {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{sha}-{h:016x}")
}

/***
 * A commit's graph, built fresh (docair-zrr.30): scanned from a detached git worktree in a scratch folder that is removed afterwards, never taken from a cache of an earlier run, since one may no longer be right. It is written to the project's `.docair/changes/graphs/<commit>.graph.json` for a reader, never read back; within one process a graph already built for the same commit and docair.toml is used again.
 *
 * @calls docair-core.scan "scans the commit"
 * @calls docair-core.diff.fingerprints "fingerprints its parts"
 ***/
pub fn graph_at(top: &Path, prefix: &str, sha: &str, lenient: bool) -> Result<KeptGraph> {
    let key = format!("{}-{lenient}", graph_key(top, sha, prefix));
    let built = BUILT.get_or_init(Default::default);
    if let Some(k) = built.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(k.clone());
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
    // Where a reader finds it, beside the reviews; never read back. A project docair cannot write
    // into still compares.
    let dir = top
        .join(prefix)
        .join(docair_core::config::DEFAULT_STORE)
        .join("graphs");
    let short = &sha[..sha.len().min(12)];
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(
            dir.join(format!("{short}.graph.json")),
            serde_json::to_string_pretty(&kept)? + "\n",
        );
    }
    built
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, kept.clone());
    Ok(kept)
}

/// Two commits compared: both graphs, the changes, and the names the commits go by in a store.
pub struct CommitPair {
    pub root: PathBuf,
    pub a: Graph,
    pub b: Graph,
    pub changes: docair_core::changes::Changes,
    /// `<sha of A>..<sha of B or "work">`, short, for the store's file name; A is the commit compared
    /// from, the merge base when the comparison starts there.
    pub key: String,
    /// What was meant (docs/EXPLAIN.md §9.2): the commit messages, the changed docs, the doc blocks whose words changed.
    pub intent: docair_core::review::Intent,
    /// Where a part's file opens at A and at B, for the page's links to the code (docair-8l6.82).
    pub links: docair_viewer::doc_page::CodeLinks,
}

/***
 * Where a part's file opens, at A and at B (docair-8l6.82): a commit on its repository host, from the
 * `origin` remote (GitHub and GitLab, https or ssh), at that commit's sha, the project's folder before
 * the path; the working tree in an editor (`vscode://file/<root>/<path>:<line>`), as its uncommitted text
 * is on no host. None where neither is there.
 ***/
fn code_links(
    top: &Path,
    root: &Path,
    prefix: &str,
    sha_a: &str,
    sha_b: Option<&str>,
) -> docair_viewer::doc_page::CodeLinks {
    let web = git(top, &["remote", "get-url", "origin"])
        .ok()
        .and_then(|u| web_of(u.trim()));
    let at = |sha: &str| {
        web.as_ref()
            .map(|(base, blob)| format!("{base}/{blob}/{sha}/{prefix}{{path}}#L{{line}}"))
    };
    docair_viewer::doc_page::CodeLinks {
        at_a: at(sha_a),
        at_b: match sha_b {
            Some(sha) => at(sha),
            None => Some(format!("vscode://file{}/{{path}}:{{line}}", root.display())),
        },
        rev_a: Some(sha_a.to_string()),
        rev_b: sha_b.map(str::to_string),
    }
}

/// The lines of a docair.toml that changed, `- ` for one only A has and `+ ` for one only B has, in
/// their order; blank lines and comments left out (docair-zrr.31).
fn toml_changed(a: &str, b: &str) -> Vec<String> {
    let lines = |t: &str| -> Vec<String> {
        t.lines()
            .map(str::trim_end)
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .map(str::to_string)
            .collect()
    };
    let (la, lb) = (lines(a), lines(b));
    la.iter()
        .filter(|l| !lb.contains(l))
        .map(|l| format!("- {l}"))
        .chain(
            lb.iter()
                .filter(|l| !la.contains(l))
                .map(|l| format!("+ {l}")),
        )
        .collect()
}

/// A git remote as its web address and the path part before a file: `git@github.com:o/r.git` →
/// (`https://github.com/o/r`, `blob`); GitLab's is `-/blob`. None for another host.
fn web_of(remote: &str) -> Option<(String, &'static str)> {
    let rest = remote
        .strip_prefix("https://")
        .or_else(|| remote.strip_prefix("http://"))
        .or_else(|| remote.strip_prefix("ssh://"))
        .unwrap_or(remote);
    let rest = rest.split_once('@').map_or(rest, |(_, r)| r);
    let (host, path) = rest.split_once([':', '/'])?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let blob = match host {
        "github.com" => "blob",
        "gitlab.com" => "-/blob",
        _ => return None,
    };
    (!path.is_empty()).then(|| (format!("https://{host}/{path}"), blob))
}

/***
 * Where `new` split from `old`, when a comparison should start there and not at `old` (docs/EDITOR.md
 * §2, docair-p2c.1): their merge base, unless that is `old` itself (old is an ancestor of new already)
 * or `new` (new is an ancestor of old: comparing backwards keeps the tips); none either when the two
 * share no commit.
 ***/
pub fn split_point(top: &Path, old: &str, new: &str) -> Option<String> {
    let base = git(top, &["merge-base", old, new]).ok()?.trim().to_string();
    (!base.is_empty() && base != old && base != new).then_some(base)
}

/***
 * Compare two commits, or a commit and the working tree, from where the new side left the old one (`split_point`): both graphs (kept in the cache), the graph diff, the changed lines and the flows through the changed parts.
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
    // The comparison starts where B left A (docs/EDITOR.md §2, docair-p2c.1): a branch shows what
    // it adds, not A's newer work as if B had removed it.
    let new_side = match b {
        Some(b) => sha(b)?,
        None => sha("HEAD")?,
    };
    let base = split_point(&top, &sha(a)?, &new_side);
    let sha_a = match &base {
        Some(base) => base.clone(),
        None => sha(a)?,
    };
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
    let mut graph = docair_core::diff::diff(
        &at_a.graph,
        &at_b.graph,
        &at_a.fingerprints,
        &at_b.fingerprints,
    );
    // Each commit is read with its own docair.toml: what changed in it is shown first (docair-zrr.31).
    let toml_of = |rev: Option<&str>| match rev {
        Some(sha) => {
            git(&top, &["show", &format!("{sha}:{prefix}docair.toml")]).unwrap_or_default()
        }
        None => std::fs::read_to_string(root.join("docair.toml")).unwrap_or_default(),
    };
    graph.toml = toml_changed(
        &toml_of(Some(&sha_a)),
        &toml_of(range.get(1).map(String::as_str)),
    );
    let mut changes =
        docair_core::changes::changes(&at_a.graph, &at_b.graph, graph, &lines, (a, &name_b));
    changes.working_tree = b.is_none();
    changes.base = base;
    let short = |s: &str| s.chars().take(12).collect::<String>();
    let key = format!(
        "{}..{}",
        short(&sha_a),
        range.get(1).map_or("work".to_string(), |s| short(s))
    );
    let mut intent = intent(&top, &range, &prefix)?;
    intent.words = docair_core::review::words_changed(&at_a.graph, &at_b.graph, &changes);
    // All of what changed, to read (docair-8l6.47): each changed part's whole text at A and at B,
    // and the files that changed but hold no part.
    let at_commit =
        |sha: &str, path: &str| git(&top, &["show", &format!("{sha}:{prefix}{path}")]).ok();
    let read_b = |path: &str| match range.get(1) {
        Some(sha_b) => at_commit(sha_b, path),
        None => std::fs::read_to_string(root.join(path)).ok(),
    };
    intent.code = docair_core::review::part_texts(
        &at_a.graph,
        &at_b.graph,
        &changes,
        &|path| at_commit(&sha_a, path),
        &read_b,
    );
    intent.files = other_files(&top, &range, &prefix, (&at_a.graph, &at_b.graph))?;
    let links = code_links(
        &top,
        &root,
        &prefix,
        &sha_a,
        range.get(1).map(String::as_str),
    );
    Ok(CommitPair {
        root,
        a: at_a.graph,
        b: at_b.graph,
        changes,
        key,
        intent,
        links,
    })
}

/***
 * What was meant between two commits, or a commit and the working tree (docs/EXPLAIN.md §9.2): the messages of the commits in the range that touch the project, and the project's changed Markdown files as their added and removed lines, cut so they fit a request (`docair_core::review::MAX_BODY`, `MAX_DOC`, `MAX_DOCS`). With the working tree as B, the commits up to `HEAD`, and `uncommitted` set.
 ***/
pub fn intent(top: &Path, range: &[String], prefix: &str) -> Result<docair_core::review::Intent> {
    use docair_core::review::{CommitNote, DocChange, Intent};
    let scope = if prefix.is_empty() {
        ".".to_string()
    } else {
        prefix.to_string()
    };
    let span = match range {
        [a, b] => format!("{a}..{b}"),
        [a] => format!("{a}..HEAD"),
        _ => return Ok(Intent::default()),
    };
    let log = git(
        top,
        &["log", "--format=%h%x1f%s%x1f%b%x1e", &span, "--", &scope],
    )?;
    let commits = log
        .split('\x1e')
        .filter_map(|r| {
            let mut f = r.trim_start_matches('\n').splitn(3, '\x1f');
            let (id, subject) = (f.next()?.trim(), f.next()?.trim());
            (!id.is_empty()).then(|| CommitNote {
                id: format!("commit:{id}"),
                subject: subject.to_string(),
                body: f.next().unwrap_or("").trim().to_string(),
            })
        })
        .collect();
    // The project's Markdown that changed, its lines with + or - in front.
    let docs_spec = format!(":(glob){prefix}**/*.md");
    let mut args: Vec<&str> = vec!["diff", "-U0", "--no-color", "--no-ext-diff"];
    args.extend(range.iter().map(String::as_str));
    args.extend(["--", docs_spec.as_str()]);
    let unified = git(top, &args)?;
    let mut docs: Vec<DocChange> = Vec::new();
    for file in unified
        .split("\ndiff --git ")
        .filter(|f| !f.trim().is_empty())
    {
        let Some(path) = file.lines().find_map(|l| {
            l.strip_prefix("+++ b/")
                .or_else(|| l.strip_prefix("--- a/"))
        }) else {
            continue;
        };
        let path = path.strip_prefix(prefix).unwrap_or(path).to_string();
        let lines: Vec<&str> = file
            .lines()
            .filter(|l| {
                (l.starts_with('+') || l.starts_with('-'))
                    && !l.starts_with("+++")
                    && !l.starts_with("---")
                    && l.trim().len() > 1
            })
            .collect();
        if lines.is_empty() {
            continue;
        }
        docs.push(DocChange {
            id: format!("doc:{path}"),
            path,
            lines: lines.join("\n"),
        });
    }
    Ok(Intent {
        commits,
        uncommitted: range.len() == 1,
        docs,
        words: Vec::new(),
        code: Vec::new(),
        files: Vec::new(),
    })
}

/// Files of the change the AI reads as lines, never: the docs go as docs, the rest is not the system's own text.
fn not_for_the_ai(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".md")
        || lower.ends_with(".lock")
        || lower.ends_with("package-lock.json")
        || lower.ends_with(".jsonl")
        || lower.starts_with("docs/changes/")
        || lower.starts_with(".docair/")
        || [
            ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".wasm", ".pdf", ".woff", ".woff2",
        ]
        .iter()
        .any(|e| lower.ends_with(e))
}

/***
 * The files that changed between two commits (or a commit and the working tree) and hold no part in either graph (docair-8l6.47): a stylesheet, a Dockerfile, a config; as their added and removed lines, `file:<path>`, cut at `MAX_FILE` a file and `MAX_FILES` together. Markdown goes as docs; lock files, the beads export, the review store and binary files not at all.
 ***/
pub fn other_files(
    top: &Path,
    range: &[String],
    prefix: &str,
    (a, b): (&Graph, &Graph),
) -> Result<Vec<docair_core::review::DocChange>> {
    use docair_core::review::DocChange;
    let held: std::collections::HashSet<&str> = a
        .nodes
        .iter()
        .chain(&b.nodes)
        .filter_map(|n| n.source.as_ref().map(|s| s.file.as_str()))
        .collect();
    let scope = if prefix.is_empty() {
        ".".to_string()
    } else {
        prefix.to_string()
    };
    let mut args: Vec<&str> = vec!["diff", "--name-only", "--no-color"];
    args.extend(range.iter().map(String::as_str));
    args.extend(["--", scope.as_str()]);
    let names = git(top, &args)?;
    let mut out = Vec::new();
    for path in names.lines().map(str::trim).filter(|p| !p.is_empty()) {
        let rel = path.strip_prefix(prefix).unwrap_or(path);
        if held.contains(rel) || not_for_the_ai(rel) {
            continue;
        }
        let mut args: Vec<&str> = vec!["diff", "-U0", "--no-color", "--no-ext-diff"];
        args.extend(range.iter().map(String::as_str));
        args.extend(["--", path]);
        let unified = git(top, &args)?;
        let lines: Vec<&str> = unified
            .lines()
            .filter(|l| {
                (l.starts_with('+') || l.starts_with('-'))
                    && !l.starts_with("+++")
                    && !l.starts_with("---")
            })
            .collect();
        if lines.is_empty() {
            continue;
        }
        out.push(DocChange {
            id: format!("file:{rel}"),
            path: rel.to_string(),
            lines: lines.join("\n"),
        });
    }
    Ok(out)
}

/// Where `docair explain` keeps the findings for this pair: in `[explain] store` (default `.docair/changes`, docair-8l6.74), `<key>.json`.
pub fn findings_file(pair: &CommitPair) -> PathBuf {
    let store = docair_core::config::load(&pair.root)
        .ok()
        .and_then(|c| c.explain)
        .and_then(|e| e.store)
        .unwrap_or_else(|| docair_core::config::DEFAULT_STORE.to_string());
    pair.root.join(store).join(format!("{}.json", pair.key))
}

/***
 * B's page with its changes section: what changed since A, with the findings `docair explain` stored for the pair, when there are any (docs/CHANGES.md §5).
 *
 * @calls docair-viewer.doc_page.render_page_with "writes the page with its changes section"
 ***/
pub fn changes_page(pair: &CommitPair) -> String {
    changes_page_with(pair, None)
}

/// As `changes_page`, keeping the pictures in `state` for the next page, as `docair serve` does
/// after each save (docs/REGENERATE.md §2).
pub fn changes_page_with(
    pair: &CommitPair,
    state: Option<&mut docair_viewer::page_state::PageState>,
) -> String {
    let findings: Option<serde_json::Value> = std::fs::read_to_string(findings_file(pair))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let changed = docair_viewer::doc_page::Changed {
        before: &pair.a,
        changes: &pair.changes,
        findings: findings.as_ref(),
        links: &pair.links,
    };
    let keep = docair_scip::cache_dir().join("layouts");
    match state {
        Some(s) => {
            docair_viewer::doc_page::render_page_kept(&pair.b, Some(&keep), Some(&changed), s)
        }
        None => docair_viewer::doc_page::render_page_with(&pair.b, Some(&keep), Some(&changed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// docair.toml's changed lines, as - and +; comments and blank lines do not count (docair-zrr.31).
    #[test]
    fn a_changed_docair_toml_is_its_lines() {
        let a = "system = \"shop\"\n# a note\n\n[view]\nhide = [\"web\"]\n";
        let b = "system = \"shop\"\n\n[view]\nhide = [\"web\", \"infra\"]\n[code]\nrun_indexers = true\n";
        assert_eq!(
            toml_changed(a, b),
            [
                "- hide = [\"web\"]",
                "+ hide = [\"web\", \"infra\"]",
                "+ [code]",
                "+ run_indexers = true"
            ]
        );
        assert!(toml_changed(a, a).is_empty());
    }

    /// A remote becomes the web address the page links into (docair-8l6.82); a token in it never does.
    #[test]
    fn a_remote_becomes_its_web_address() {
        let gh = Some(("https://github.com/o/r".to_string(), "blob"));
        assert_eq!(web_of("git@github.com:o/r.git"), gh);
        assert_eq!(web_of("https://github.com/o/r.git"), gh);
        assert_eq!(web_of("https://user:tok@github.com/o/r"), gh);
        assert_eq!(web_of("ssh://git@github.com/o/r.git"), gh);
        assert_eq!(
            web_of("git@gitlab.com:g/sub/r.git"),
            Some(("https://gitlab.com/g/sub/r".to_string(), "-/blob"))
        );
        assert_eq!(web_of("https://example.org/o/r.git"), None);
    }

    fn run(dir: &Path, args: &[&str]) {
        let mut all = vec![
            "-c",
            "user.name=docair test",
            "-c",
            "user.email=test@example.invalid",
        ];
        all.extend(args);
        git(dir, &all).unwrap();
    }

    /// docair-8l6.19: what was meant between two commits: their messages, the README's changed lines,
    /// and a doc block whose words alone changed.
    #[test]
    fn the_intent_is_read_from_the_commits_and_the_docs() {
        let dir = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        // SAFETY: this test's own cache; no other test of this binary reads DOCAIR_CACHE.
        unsafe { std::env::set_var("DOCAIR_CACHE", cache.path()) };
        let w = |rel: &str, text: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        w("docair.toml", "system = \"shop\"\n");
        w("README.md", "# shop\n\nThe shop sells things.\n");
        w("ui/style.css", "body { color: black; }\n");
        let lib = |summary: &str| {
            format!("/***\n * {summary}\n *\n * @id shop.buy\n ***/\npub fn buy() {{}}\n")
        };
        w("src/lib.rs", &lib("Buy the cart."));
        run(dir.path(), &["init", "-q", "-b", "main"]);
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-q", "-m", "a"]);
        w(
            "README.md",
            "# shop\n\nThe shop sells things.\n\nEvery order is audited.\n",
        );
        w("src/lib.rs", &lib("Buy the cart, and audit the order."));
        w("ui/style.css", "body { color: red; }\n");
        run(
            dir.path(),
            &[
                "commit",
                "-q",
                "-am",
                "Audit every order\n\nThe auditors asked for a trail of each order.",
            ],
        );
        let pair = compare(dir.path(), "HEAD~1", Some("HEAD"), false).unwrap();
        let i = &pair.intent;
        assert_eq!(i.commits.len(), 1, "{i:?}");
        assert!(
            i.commits[0].id.starts_with("commit:") && i.commits[0].subject == "Audit every order"
        );
        assert!(i.commits[0].body.contains("trail of each order"));
        assert!(!i.uncommitted);
        let readme = i
            .docs
            .iter()
            .find(|d| d.path == "README.md")
            .expect("the README changed");
        assert_eq!(readme.id, "doc:README.md");
        assert!(
            readme.lines.contains("+Every order is audited."),
            "{}",
            readme.lines
        );
        assert!(
            i.words
                .iter()
                .any(|w| w.part == "shop.buy" && w.after.contains("audit")),
            "{:?}",
            i.words
        );
        assert!(i.docs.iter().any(|d| d.id == "doc:README.md"));
        // docair-8l6.47: the changed part's whole text before and after, its doc block with it;
        // the changed file that holds no part, as its lines.
        let buy = i
            .code
            .iter()
            .find(|t| t.part == "shop.buy")
            .expect("shop.buy changed");
        let (before, after) = (
            buy.before.as_deref().unwrap(),
            buy.after.as_deref().unwrap(),
        );
        assert!(
            before.contains("Buy the cart.") && before.contains("pub fn buy()"),
            "{before}"
        );
        assert!(
            after.contains("audit the order") && after.contains("pub fn buy()"),
            "{after}"
        );
        let css = i
            .files
            .iter()
            .find(|f| f.path == "ui/style.css")
            .expect("the stylesheet changed");
        assert_eq!(css.id, "file:ui/style.css");
        assert!(
            css.lines.contains("+body { color: red; }")
                && css.lines.contains("-body { color: black; }"),
            "{}",
            css.lines
        );
        assert!(
            i.files.iter().all(|f| !f.path.ends_with(".md")),
            "Markdown goes as docs"
        );

        // The working tree as B: the commits up to HEAD, and what is not committed said.
        let work = compare(dir.path(), "HEAD~1", None, false).unwrap();
        assert!(work.intent.uncommitted && work.intent.commits.len() == 1);
    }
    /// docs/EDITOR.md §2, docair-p2c.1: a branch is compared from where it left the other side, so
    /// main's newer work does not show as removed; an ancestor, a backward comparison and unrelated
    /// histories keep the tips.
    #[test]
    fn a_branch_is_compared_from_where_it_split() {
        let dir = tempfile::tempdir().unwrap();
        let w = |rel: &str, text: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        let part =
            |id: &str| format!("/***\n * {id}.\n *\n * @id shop.{id}\n ***/\npub fn {id}() {{}}\n");
        w("docair.toml", "system = \"shop\"\n");
        w("src/lib.rs", &part("buy"));
        run(dir.path(), &["init", "-q", "-b", "main"]);
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-q", "-m", "buy"]);
        let split = git(dir.path(), &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        run(dir.path(), &["checkout", "-q", "-b", "feature"]);
        w("src/audit.rs", &part("audit"));
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-q", "-m", "audit"]);
        run(dir.path(), &["checkout", "-q", "main"]);
        w("src/refund.rs", &part("refund"));
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-q", "-m", "refund"]);

        let pair = compare(dir.path(), "main", Some("feature"), false).unwrap();
        assert_eq!(pair.changes.base.as_deref(), Some(split.as_str()));
        assert!(
            pair.changes.parts.iter().any(|p| p == "shop.audit"),
            "{:?}",
            pair.changes.parts
        );
        assert!(
            !pair.changes.parts.iter().any(|p| p == "shop.refund"),
            "main's newer work is no change of the branch: {:?}",
            pair.changes.parts
        );
        assert!(pair.key.starts_with(&split[..12]), "{}", pair.key);
        let summary = docair_core::changes::markdown(&pair.changes);
        assert!(
            summary.contains(&format!(
                "Compared from where `feature` left `main`: `{}`.",
                &split[..12]
            )),
            "{summary}"
        );
        // The working tree on the branch: from where HEAD left main.
        run(dir.path(), &["checkout", "-q", "feature"]);
        let work = compare(dir.path(), "main", None, false).unwrap();
        assert_eq!(work.changes.base.as_deref(), Some(split.as_str()));
        assert!(
            docair_core::changes::markdown(&work.changes)
                .contains("where the working tree left `main`")
        );
        // An ancestor, and backwards: the tips.
        let ahead = compare(dir.path(), "feature~1", Some("feature"), false).unwrap();
        assert_eq!(ahead.changes.base, None);
        let back = compare(dir.path(), "feature", Some("feature~1"), false).unwrap();
        assert_eq!(back.changes.base, None);
        assert!(
            back.changes.parts.iter().any(|p| p == "shop.audit"),
            "gone going back"
        );
        // Two histories that share no commit: the tips.
        run(dir.path(), &["checkout", "-q", "--orphan", "other"]);
        run(dir.path(), &["commit", "-q", "-m", "other"]);
        let apart = compare(dir.path(), "main", Some("other"), false).unwrap();
        assert_eq!(apart.changes.base, None);
    }
}

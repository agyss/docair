//! `docair pr`: docair's review on a pull request (docs/PULL_REQUESTS.md). `build` writes what a
//! run gives the pull request: the page and the comments; `plan` says what to do with the pull
//! request's comments. The workflow only lists the comments, uploads the page and carries the plan
//! out with `gh api`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use docair_server::commits::{changes_page, compare, findings_file, git};
use docair_viewer::pr::{self, PrComment};
use serde_json::{Value, json};

/// The page's file name: an artifact uploaded unzipped is named after its file.
const PAGE_FILE: &str = "docair-review.html";
/// What `docair pr build` writes for `docair pr plan`.
const COMMENTS_FILE: &str = "comments.json";
const SCHEMA: &str = "docair.pr/0";

/***
 * `docair pr build <BASE> [<HEAD>]` (docs/PULL_REQUESTS.md §2): A is the merge base of BASE and HEAD, B is HEAD's commit, as GitHub's Files changed; the AI reviews the change when a key is there (kept in `[explain] store`), and when not, or when the review fails, every piece is a change in docair's words and the summary says why; B's page with the review (`docair-review.html`), the comments for `docair pr plan` (`comments.json`), all of them as Markdown to read (`comments.md`) and each change's cut as an SVG (`pictures/<n>.svg`), in `<out>` (default `.docair/pr`).
 *
 * @calls docair-server.commits.compare "compares the merge base with the head"
 * @calls docair-cli.diff.review "the AI reviews the change"
 * @calls docair-server.commits.changes_page "the page with the review"
 * @calls docair-viewer.pr.review "a comment per change"
 ***/
pub fn cmd_build(
    root: &Path,
    base: &str,
    head: &str,
    out: Option<PathBuf>,
    lenient: bool,
) -> Result<ExitCode> {
    let root_abs = root
        .canonicalize()
        .with_context(|| format!("{}", root.display()))?;
    let commit = |rev: &str| -> Result<String> {
        Ok(git(
            &root_abs,
            &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
        )
        .with_context(|| format!("{rev} is no commit here"))?
        .trim()
        .to_string())
    };
    let (base_sha, head_sha) = (commit(base)?, commit(head)?);
    let Ok(a) = git(&root_abs, &["merge-base", &base_sha, &head_sha]) else {
        bail!(
            "{base} and {head} have no commit in common here: check out the whole history (in GitHub Actions, fetch-depth: 0)"
        );
    };
    let a = a.trim().to_string();
    eprintln!(
        "docair pr: from the merge base {} of {base} to {head} ({})",
        &a[..12.min(a.len())],
        &head_sha[..12.min(head_sha.len())]
    );
    let pair = compare(&root_abs, &a, Some(&head_sha), lenient)?;
    let no_ai: Option<String> = match crate::ask::ai_config(&pair.root)? {
        None => Some("no model is set up ([ai] in docair.toml)".into()),
        Some(ai) if ai.api_key.as_deref().is_none_or(str::is_empty) => Some(format!(
            "no key for the model in this run ({}); a pull request from a fork gets no secrets",
            ai.key_var
        )),
        Some(ai) => match crate::diff::review(&pair, &ai) {
            Ok(_) => None,
            Err(e) => {
                eprintln!("docair pr: the review failed: {e:#}");
                Some(format!(
                    "the review failed ({})",
                    one_line(&format!("{e:#}"))
                ))
            }
        },
    };
    let out = out.unwrap_or_else(|| pair.root.join(".docair/pr"));
    std::fs::create_dir_all(out.join("pictures"))?;
    std::fs::write(out.join(PAGE_FILE), changes_page(&pair))?;
    let findings: Option<Value> = if no_ai.is_none() {
        std::fs::read_to_string(findings_file(&pair))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
    } else {
        None
    };
    let changed = docair_viewer::doc_page::Changed {
        before: &pair.a,
        changes: &pair.changes,
        findings: findings.as_ref(),
        links: &pair.links,
    };
    let review = pr::review(&pair.b, &changed, no_ai.as_deref());
    let comments: Vec<&PrComment> = std::iter::once(&review.summary)
        .chain(&review.changes)
        .collect();
    let json = json!({
        "schema": SCHEMA,
        "a": a,
        "b": head_sha,
        "comments": comments.iter().map(|c| c.to_json()).collect::<Vec<_>>(),
    });
    std::fs::write(
        out.join(COMMENTS_FILE),
        serde_json::to_string_pretty(&json)? + "\n",
    )?;
    let md: Vec<String> = comments
        .iter()
        .map(|c| pr::with_page(&c.body, None))
        .collect();
    std::fs::write(out.join("comments.md"), md.join("\n---\n\n"))?;
    for (k, svg) in review.svgs.iter().enumerate() {
        if !svg.is_empty() {
            std::fs::write(out.join("pictures").join(format!("{}.svg", k + 1)), svg)?;
        }
    }
    println!(
        "docair pr: {} change{}{} → {}",
        review.changes.len(),
        if review.changes.len() == 1 { "" } else { "s" },
        no_ai.map_or(String::new(), |w| format!(" in docair's words ({w})")),
        out.display()
    );
    Ok(ExitCode::SUCCESS)
}

fn one_line(text: &str) -> String {
    let t = text.lines().next().unwrap_or("").trim();
    docair_core::review::cut(t, 200)
}

/***
 * `docair pr plan` (docs/PULL_REQUESTS.md §4): the comments `docair pr build` wrote, matched to the pull request's comments as GitHub lists them; each action as JSON on stdout, one array: `create` (body), `update` and `gone` (id, body), `keep` (id).
 *
 * @calls docair-viewer.pr.plan "matches the comments by their pieces"
 ***/
pub fn cmd_plan(
    dir: &Path,
    existing: &Path,
    page: Option<&str>,
    author: Option<&str>,
) -> Result<ExitCode> {
    let file = dir.join(COMMENTS_FILE);
    let text = std::fs::read_to_string(&file)
        .with_context(|| format!("{}: run docair pr build first", file.display()))?;
    let v: Value = serde_json::from_str(&text)?;
    if v["schema"].as_str() != Some(SCHEMA) {
        bail!("{}: not {SCHEMA}", file.display());
    }
    let new: Vec<PrComment> = v["comments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(PrComment::from_json)
        .collect();
    let listed = if existing == Path::new("-") {
        std::io::read_to_string(std::io::stdin())?
    } else {
        std::fs::read_to_string(existing).with_context(|| format!("{}", existing.display()))?
    };
    let old = pr::existing_from(&listed);
    let actions = pr::plan(&new, &old, page.filter(|p| !p.is_empty()), author);
    let out: Vec<Value> = actions.iter().map(|a| a.to_json()).collect();
    println!("{}", serde_json::to_string_pretty(&out)?);
    let count = |k: &str| actions.iter().filter(|a| a.action == k).count();
    eprintln!(
        "docair pr plan: {} to write, {} to update, {} kept, {} gone",
        count("create"),
        count("update"),
        count("keep"),
        count("gone")
    );
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn run(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=docair test",
                "-c",
                "user.email=test@example.invalid",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            ok.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&ok.stderr)
        );
    }

    /// A pull request whose base branch moved on since it branched off: the review starts at the
    /// merge base, so what landed on the base since shows as nothing undone; without a model, each
    /// piece is a change in docair's words; the plan writes every comment the first time.
    #[test]
    fn a_pull_request_is_reviewed_from_its_merge_base() {
        let dir = tempfile::tempdir().unwrap();
        let w = |rel: &str, text: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        w("docair.toml", "system = \"shop\"\n");
        w(
            "Cargo.toml",
            "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        w(
            "README.md",
            "# shop\n\n```docair\nThe shopper.\n\n@id      shopper\n@kind    external\n@part-of shop\n@calls   shop.buy\n@flow    buy 1 \"clicks Buy\"\n```\n",
        );
        let buy = "/***\n * Buy: total the cart.\n *\n * @id      shop.buy\n * @flow    buy 2 \"the cart is totalled\"\n ***/\npub fn buy() -> u8 {\n    1\n}\n";
        w("src/lib.rs", buy);
        run(dir.path(), &["init", "-q", "-b", "main"]);
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-q", "-m", "a shop"]);
        // The pull request: buying keeps the order.
        run(dir.path(), &["checkout", "-q", "-b", "keep"]);
        let keep = format!(
            "{}\n/***\n * Keep the order.\n *\n * @id      shop.save\n * @flow    buy 3 \"the order is kept\"\n ***/\npub fn save() {{}}\n",
            buy.replace(" * @flow", " * @calls   shop.save\n * @flow")
        );
        w("src/lib.rs", &keep);
        run(dir.path(), &["commit", "-q", "-am", "Keep each order"]);
        // Meanwhile the base: a refund, in a file of its own.
        run(dir.path(), &["checkout", "-q", "main"]);
        w(
            "src/refund.rs",
            "/***\n * Refund an order.\n *\n * @id shop.refund\n ***/\npub fn refund() {}\n",
        );
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-q", "-m", "Refunds"]);
        run(dir.path(), &["checkout", "-q", "keep"]);

        let out = dir.path().join("pr");
        let _ = cmd_build(dir.path(), "main", "HEAD", Some(out.clone()), true).unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(out.join(COMMENTS_FILE)).unwrap())
                .unwrap();
        assert_eq!(v["schema"], SCHEMA);
        let comments: Vec<PrComment> = v["comments"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(PrComment::from_json)
            .collect();
        assert_eq!(comments[0].kind, "summary");
        assert!(
            comments[0]
                .body
                .contains("No review by the AI:** no model is set up"),
            "{}",
            comments[0].body
        );
        let all: String = comments.iter().map(|c| c.body.as_str()).collect();
        assert!(
            all.contains("shop.save"),
            "the pull request's change: {all}"
        );
        assert!(
            !all.contains("refund"),
            "what landed on the base since is no change of the pull request: {all}"
        );
        assert!(comments.len() >= 2, "a comment per change: {all}");
        assert!(
            comments[1..]
                .iter()
                .all(|c| c.kind == "change" && !c.pieces.is_empty())
        );
        assert!(out.join(PAGE_FILE).exists());
        let md = std::fs::read_to_string(out.join("comments.md")).unwrap();
        assert!(!md.contains(pr::PAGE), "the preview has no page to link");
        if std::env::var_os("DOCAIR_SHOW").is_some() {
            println!("{md}");
        }

        // The first run writes every comment.
        let none = dir.path().join("none.json");
        std::fs::write(&none, "[]").unwrap();
        let new: Vec<PrComment> = comments.clone();
        let p = pr::plan(&new, &pr::existing_from("[]"), Some("https://x"), None);
        assert!(p.iter().all(|a| a.action == "create"));
        assert_eq!(p.len(), comments.len());
        assert!(cmd_plan(&out, &none, Some("https://x"), None).is_ok());
    }
}

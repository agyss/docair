//! `docair diff <A> [<B>]`: what two commits change in the system's flows (docs/CHANGES.md).
//!
//! Each commit is read from a git worktree in a scratch folder, scanned, and its graph kept in the
//! cache folder, so the same commit is scanned once. Without B, the working tree is B.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use docair_server::commits::{CommitPair, compare};

/***
 * `docair diff <A> [<B>]`: the flows two commits change, or a commit and the working tree; the changes as JSON to `--out` (`changes.json`) and as Markdown on stdout (`--json` prints the JSON).
 *
 * @calls docair-server.commits.compare "compares the commits"
 * @calls docair-core.changes.markdown "writes the summary"
 ***/
pub fn cmd_diff(
    root: &Path,
    a: &str,
    b: Option<&str>,
    out: Option<PathBuf>,
    lenient: bool,
    json: bool,
) -> Result<ExitCode> {
    let CommitPair { root, changes, .. } = compare(root, a, b, lenient)?;
    let out = out.unwrap_or_else(|| root.join(".docair"));
    std::fs::create_dir_all(&out)?;
    let file = out.join("changes.json");
    std::fs::write(&file, serde_json::to_string_pretty(&changes)? + "\n")?;
    if json {
        println!("{}", serde_json::to_string_pretty(&changes)?);
    } else {
        print!("{}", docair_core::changes::markdown(&changes));
        println!("\n→ {}", file.display());
    }
    Ok(ExitCode::SUCCESS)
}

/***
 * `docair explain <A> [<B>]`: the changes of `docair diff`, explained by the AI: links, what each changed flow now does, and workflows before and after; kept in `[explain] store` (`docs/changes/<A>..<B>.json`), so the next run asks only about what is new and never about what a human wrote; a pull request comment on stdout (`--json`: the findings).
 *
 * @calls docair-server.commits.compare "compares the commits"
 * @calls docair-ai.explain.explain "explains the changed flows"
 * @calls docair-ai.explain.comment "writes the comment"
 ***/
pub fn cmd_explain(
    root: &Path,
    a: &str,
    b: Option<&str>,
    lenient: bool,
    json: bool,
) -> Result<ExitCode> {
    let compared = compare(root, a, b, lenient)?;
    let Some(ai) = crate::ask::ai_config(&compared.root)? else {
        return Ok(ExitCode::from(2));
    };
    let config = docair_core::config::load(&compared.root)?;
    let settings = config.explain.unwrap_or_default();
    let store = compared
        .root
        .join(settings.store.as_deref().unwrap_or("docs/changes"));
    let file = store.join(format!("{}.json", compared.key));
    let kept: Option<docair_ai::explain::ChangeFindings> = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let max = settings
        .max_requests
        .unwrap_or(docair_ai::explain::MAX_REQUESTS);
    let explained = docair_ai::explain::explain(
        &ai,
        &compared.a,
        &compared.b,
        &compared.changes,
        kept.as_ref(),
        max,
    )?;
    std::fs::create_dir_all(&store)?;
    std::fs::write(&file, serde_json::to_string_pretty(&explained)? + "\n")?;
    if json {
        println!("{}", serde_json::to_string_pretty(&explained)?);
    } else {
        print!(
            "{}",
            docair_ai::explain::comment(&compared.changes, &explained, &compared.b)
        );
        for p in &explained.problems {
            eprintln!("left out: {p}");
        }
        eprintln!("→ {} ({} requests)", file.display(), explained.requests);
    }
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

    /// Two commits of a small project: a rename with the same body, a new step in a scenario, a changed body (docair-zrr.8).
    #[test]
    fn two_commits_give_their_flow_changes() {
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
        w(
            "Cargo.toml",
            "[package]\nname = \"shop\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        w(
            "README.md",
            "# shop\n\n```docair\nThe shopper.\n\n@id      shopper\n@kind    external\n@part-of shop\n@calls   shop.buy\n@flow    buy 1 \"clicks Buy\"\n```\n",
        );
        let lib_a = "/***\n * Buy: total the cart.\n *\n * @id      shop.buy\n * @flow    buy 2 \"the cart is totalled\"\n ***/\npub fn buy() -> u8 {\n    total() + tax()\n}\n\npub fn total() -> u8 {\n    1\n}\n\npub fn tax() -> u8 {\n    2\n}\n";
        w("src/lib.rs", lib_a);
        run(dir.path(), &["init", "-q", "-b", "main"]);
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-q", "-m", "a"]);
        let lib_b = "/***\n * Buy: total the cart.\n *\n * @id      shop.buy\n * @calls   shop.save\n * @flow    buy 2 \"the cart is totalled\"\n ***/\npub fn buy() -> u8 {\n    sum() + tax()\n}\n\npub fn sum() -> u8 {\n    1\n}\n\npub fn tax() -> u8 {\n    3\n}\n\n/***\n * Keep the order.\n *\n * @id      shop.save\n * @flow    buy 3 \"the order is kept\"\n ***/\npub fn save() {}\n";
        w("src/lib.rs", lib_b);
        run(dir.path(), &["commit", "-q", "-am", "b"]);
        let out = dir.path().join("out");
        let _ = cmd_diff(
            dir.path(),
            "HEAD~1",
            Some("HEAD"),
            Some(out.clone()),
            true,
            false,
        )
        .unwrap();
        let c: docair_core::changes::Changes =
            serde_json::from_str(&std::fs::read_to_string(out.join("changes.json")).unwrap())
                .unwrap();
        let moved: Vec<(&str, &str)> = c
            .graph
            .moved
            .iter()
            .map(|m| (m.from.as_str(), m.to.as_str()))
            .collect();
        assert_eq!(moved, [("shop.total", "shop.sum")], "{:#?}", c.graph);
        assert!(
            c.graph
                .changed
                .iter()
                .any(|x| x.id == "shop.tax" && x.what == ["body"]),
            "{:#?}",
            c.graph
        );
        assert_eq!(c.graph.added, ["shop.save"]);
        let buy = c
            .flows
            .iter()
            .find(|f| f.seed == "flow:buy")
            .unwrap_or_else(|| panic!("{:#?}", c.flows));
        assert_eq!(buy.how, "changed");
        assert_eq!(buy.parts_added, ["shop.save"], "{buy:#?}");
        assert!(
            buy.parts_removed.is_empty(),
            "the rename is no change to the flow: {buy:#?}"
        );
        // The commit's graph is kept: the next diff reads it, no worktree.
        assert!(
            std::fs::read_dir(cache.path().join("graphs"))
                .unwrap()
                .count()
                >= 2
        );
        let summary = docair_core::changes::markdown(&c);
        assert!(
            summary.contains("**flow:buy**: changed; now through `shop.save`"),
            "{summary}"
        );
        assert!(summary.contains("`shop.total` → `shop.sum`"), "{summary}");
    }
}

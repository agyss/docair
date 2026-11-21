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
 * `docair explain <A> [<B>]`: the review of the change (docs/EXPLAIN.md §9): the AI groups docair's pieces into changes, one difference in what the app does each, from the pieces, the commit messages and the changed docs, and says what, how, why and what to check; kept in `[explain] store` (`.docair/changes/<A>..<B>.json` by default), so the next run asks only when the pieces changed and never about what a human wrote; a pull request comment on stdout (`--json`: the store).
 *
 * @calls docair-server.commits.compare "compares the commits"
 * @calls docair-ai.explain.explain_with "reviews the change, at most 3 requests at once"
 * @calls docair-ai.explain.comment "writes the review as a comment"
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
    let store = compared.root.join(
        settings
            .store
            .as_deref()
            .unwrap_or(docair_core::config::DEFAULT_STORE),
    );
    let file = store.join(format!("{}.json", compared.key));
    let kept: Option<docair_ai::explain::ChangeFindings> = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let review = docair_ai::explain::ReviewSettings::of(Some(&settings));
    // What is sent and each answered request are said on stderr, so a long run shows it is working,
    // and each change as soon as it is whole (docair-8l6.65, 8l6.79).
    let said = std::sync::atomic::AtomicUsize::new(0);
    let asked = std::sync::atomic::AtomicUsize::new(0);
    let made = std::sync::atomic::AtomicUsize::new(0);
    let explained = docair_ai::explain::explain_with(
        &ai,
        (&compared.a, &compared.b),
        &compared.changes,
        &compared.intent,
        kept.as_ref(),
        &review,
        &|p| {
            if p.started > asked.swap(p.started, std::sync::atomic::Ordering::SeqCst) {
                eprintln!(
                    "docair explain: asking {} about {} pieces ({} characters) …",
                    ai.model, p.pieces, p.chars
                );
            }
            if p.answered > said.swap(p.answered, std::sync::atomic::Ordering::SeqCst) {
                eprintln!(
                    "docair explain: {} of {} requests answered",
                    p.answered, p.requests
                );
            }
            let from = made.fetch_max(p.found.len(), std::sync::atomic::Ordering::SeqCst);
            for (k, c) in p.found.iter().enumerate().skip(from) {
                eprintln!("docair explain: {}. {}", k + 1, c.title);
            }
        },
    )?;
    std::fs::create_dir_all(&store)?;
    std::fs::write(&file, serde_json::to_string_pretty(&explained)? + "\n")?;
    if json {
        println!("{}", serde_json::to_string_pretty(&explained)?);
    } else {
        print!(
            "{}",
            docair_ai::explain::comment(&compared.changes, &explained, (&compared.a, &compared.b))
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
        // Both commits' graphs are written beside the reviews, for a reader (docair-zrr.30); none in the cache.
        assert!(
            std::fs::read_dir(dir.path().join(".docair/changes/graphs"))
                .unwrap()
                .count()
                >= 2
        );
        assert!(!cache.path().join("graphs").exists(), "no graph is cached");
        let summary = docair_core::changes::markdown(&c);
        assert!(
            summary.contains("**flow:buy**: changed; now through `shop.save`"),
            "{summary}"
        );
        assert!(summary.contains("`shop.total` → `shop.sum`"), "{summary}");
    }

    /// More kinds of change between two commits (docair-zrr.34): a function removed and a flow step
    /// with it, several functions and steps added at once, a function moved to another module, a
    /// stretch replaced by another (a fork), a doc block's words alone, and a request rerouted to
    /// another Service that selects another Deployment.
    #[test]
    fn more_kinds_of_change_between_two_commits() {
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
        let block = |id: &str, summary: &str, tags: &str| {
            format!("/***\n * {summary}\n *\n * @id      {id}\n{tags} ***/\n")
        };
        let lib_a = format!(
            "{}pub fn buy() -> u8 {{\n    total() + tax()\n}}\n\npub fn total() -> u8 {{\n    let n = 1;\n    n * 100\n}}\n\n{}pub fn tax() -> u8 {{\n    2\n}}\n\n{}pub fn refund() {{}}\n\n{}pub fn help() {{}}\n",
            block(
                "shop.buy",
                "Buy: total the cart.",
                " * @calls   shop.tax\n * @flow    buy 2 \"the cart is totalled\"\n"
            ),
            block(
                "shop.tax",
                "The tax.",
                " * @flow    buy 3 \"the tax is added\"\n"
            ),
            block(
                "shop.refund",
                "Give the money back.",
                " * @flow    buy 4 \"a refund, if asked\"\n"
            ),
            block("shop.help", "Help.", ""),
        );
        w("src/lib.rs", &lib_a);
        let k8s_a = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: api\nspec:\n  template:\n    metadata:\n      labels:\n        app: api\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: api-next\nspec:\n  template:\n    metadata:\n      labels:\n        app: api-next\n---\napiVersion: v1\nkind: Service\nmetadata:\n  name: api\nspec:\n  selector:\n    app: api\n  ports:\n    - port: 80\n---\napiVersion: networking.k8s.io/v1\nkind: Ingress\nmetadata:\n  name: shop\nspec:\n  rules:\n    - http:\n        paths:\n          - path: /api\n            backend:\n              service:\n                name: api\n                port:\n                  number: 80\n";
        w("k8s/shop.yaml", k8s_a);
        run(dir.path(), &["init", "-q", "-b", "main"]);
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-q", "-m", "a"]);
        // B: refund gone with its step; save and audit new, with steps; total moved to math; tax
        // replaced by vat (a fork); buy's words alone changed; /api rerouted to a Service that
        // selects api-next.
        let lib_b = format!(
            "pub mod math;\n\n{}pub fn buy() -> u8 {{\n    math::total() + vat()\n}}\n\n{}pub fn vat() -> u8 {{\n    3\n}}\n\n{}pub fn save() {{}}\n\n{}pub fn audit() {{}}\n\n{}pub fn help() {{}}\n",
            block(
                "shop.buy",
                "Buy: total the cart, and keep the order.",
                " * @calls   shop.vat\n * @calls   shop.save\n * @flow    buy 2 \"the cart is totalled\"\n"
            ),
            block(
                "shop.vat",
                "The value-added tax.",
                " * @flow    buy 3 \"the vat is added\"\n"
            ),
            block(
                "shop.save",
                "Keep the order.",
                " * @calls   shop.audit\n * @flow    buy 4 \"the order is kept\"\n"
            ),
            block(
                "shop.audit",
                "Record the order.",
                " * @flow    buy 5 \"the order is recorded\"\n"
            ),
            block("shop.help", "Help the shopper find the right thing.", ""),
        );
        w("src/lib.rs", &lib_b);
        w(
            "src/math.rs",
            "pub fn total() -> u8 {\n    let n = 1;\n    n * 100\n}\n",
        );
        let k8s_b = k8s_a
            .replace("                name: api\n", "                name: api-v2\n")
            .replace(
                "---\napiVersion: networking.k8s.io/v1",
                "---\napiVersion: v1\nkind: Service\nmetadata:\n  name: api-v2\nspec:\n  selector:\n    app: api-next\n  ports:\n    - port: 80\n---\napiVersion: networking.k8s.io/v1",
            );
        w("k8s/shop.yaml", &k8s_b);
        run(dir.path(), &["add", "-A"]);
        run(dir.path(), &["commit", "-q", "-m", "b"]);
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
        let g = &c.graph;
        // Removed, with its step; several added at once, with theirs.
        assert!(g.removed.contains(&"shop.refund".to_string()), "{g:#?}");
        for p in ["shop.save", "shop.audit", "shop.vat"] {
            assert!(g.added.contains(&p.to_string()), "{p}: {g:#?}");
        }
        // Moved to another module, the same body: a move, no change.
        assert!(
            g.moved
                .iter()
                .any(|m| m.from == "shop.total" && m.to == "shop.math.total"),
            "{g:#?}"
        );
        // A doc block's words alone: words, not code.
        assert!(
            g.changed
                .iter()
                .any(|x| x.id == "shop.help" && x.what == ["summary"]),
            "{g:#?}"
        );
        // Rerouted: the Ingress's route goes to another Service, which selects another Deployment.
        assert!(
            g.lines_removed
                .contains(&"k8s.ingress.shop -route-> k8s.svc.api".to_string())
        );
        assert!(
            g.lines_added
                .contains(&"k8s.ingress.shop -route-> k8s.svc.api-v2".to_string())
        );
        assert!(
            g.lines_added
                .contains(&"k8s.svc.api-v2 -select-> k8s.deploy.api-next".to_string())
        );
        // The scenario: what it gained and lost.
        let buy = c.flows.iter().find(|f| f.seed == "flow:buy").unwrap();
        assert_eq!(buy.how, "changed");
        assert_eq!(buy.parts_added, ["shop.audit", "shop.save", "shop.vat"]);
        assert_eq!(buy.parts_removed, ["shop.refund", "shop.tax"]);
        // The fork: tax gone and vat in its place, at buy, are one piece of the review.
        let graphs: Vec<docair_core::model::Graph> =
            std::fs::read_dir(dir.path().join(".docair/changes/graphs"))
                .unwrap()
                .flatten()
                .map(|e| {
                    let v: serde_json::Value =
                        serde_json::from_str(&std::fs::read_to_string(e.path()).unwrap()).unwrap();
                    serde_json::from_value(v["graph"].clone()).unwrap()
                })
                .collect();
        let at_a = graphs
            .iter()
            .find(|g| g.nodes.iter().any(|n| n.id == "shop.refund"))
            .unwrap();
        let at_b = graphs
            .iter()
            .find(|g| g.nodes.iter().any(|n| n.id == "shop.vat"))
            .unwrap();
        let pieces = docair_core::review::pieces(at_a, at_b, &c);
        assert!(
            pieces
                .iter()
                .any(|p| p.step.removed.contains(&"shop.tax".to_string())
                    && p.step.added.contains(&"shop.vat".to_string())),
            "{:#?}",
            pieces.iter().map(|p| p.id()).collect::<Vec<_>>()
        );
    }
}

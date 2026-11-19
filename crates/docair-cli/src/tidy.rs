//! `docair tidy`: the hand-written tags that only repeat what the code says, listed by file, as a
//! patch, and with `--apply` written into the files (docs/ADAPTERS.md §6.1).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use docair_core::model::Graph;
use docair_core::tidy::{RepeatedTag, changes, repeated};
use docair_core::{Options, edit, scan};

/// The one line `docair build` and `docair check` end with while tags repeat the code.
pub fn hint(graph: &Graph) -> Option<String> {
    let n = repeated(graph).len();
    (n > 0).then(|| {
        format!(
            "{n} {} the code: run `docair tidy`",
            if n == 1 { "tag repeats" } else { "tags repeat" }
        )
    })
}

/***
 * `docair tidy`: list the tags the code repeats by file, write them as `docair.patch` to `--out`, and with `--apply` write the edits into the files and scan again.
 *
 * @calls docair-core.scan "builds the graph"
 * @calls docair-core.tidy.repeated "finds the tags the code repeats"
 * @calls docair-core.tidy.changes "turns them into edits"
 * @calls docair-core.edit.patch "writes the edits as a diff"
 ***/
pub fn cmd_tidy(root: &Path, out: Option<PathBuf>, lenient: bool, apply: bool) -> Result<ExitCode> {
    let options = Options {
        lenient,
        ..Options::default()
    };
    let graph = scan(root, docair_adapters::all(), &options)?;
    let found = repeated(&graph);
    if found.is_empty() {
        println!("docair tidy: no tag repeats the code ✓");
        return Ok(ExitCode::SUCCESS);
    }
    let mut resolved = Vec::new();
    for c in changes(&found) {
        match edit::resolve(&c, &graph.nodes) {
            Ok(c) => resolved.push(c),
            Err(e) => eprintln!("warning: {e}"),
        }
    }
    let file_of: BTreeMap<&str, &str> = resolved
        .iter()
        .map(|c| (c.node.as_str(), c.file.as_str()))
        .collect();
    let mut by_file: BTreeMap<&str, Vec<&RepeatedTag>> = BTreeMap::new();
    for r in &found {
        by_file
            .entry(file_of.get(r.node.as_str()).copied().unwrap_or("?"))
            .or_default()
            .push(r);
    }
    for (file, tags) in &by_file {
        println!("{file}");
        for r in tags {
            let at = match (&r.code_file, r.code_line) {
                (Some(f), l) if l > 0 => format!("{f}:{l}"),
                (Some(f), _) => f.clone(),
                _ => "the code".to_string(),
            };
            println!("  {:>4}  {}  ✂ the code says this ({at})", r.line, r.tag);
        }
    }
    let known: HashSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    let read = |p: &str| std::fs::read_to_string(root.join(p)).ok();
    let result = edit::patch(&resolved, &read, &known);
    for p in &result.problems {
        eprintln!("warning: {p}");
    }
    let out = out.unwrap_or_else(|| root.join(".docair"));
    std::fs::create_dir_all(&out)?;
    let patch = out.join("docair.patch");
    std::fs::write(&patch, &result.diff)?;
    let n = found.len();
    println!(
        "docair tidy: {n} {} the code, in {} {} → {}",
        if n == 1 { "tag repeats" } else { "tags repeat" },
        by_file.len(),
        if by_file.len() == 1 { "file" } else { "files" },
        patch.display()
    );
    if !apply {
        println!(
            "apply     docair tidy --apply (or `git apply {}`)",
            patch.display()
        );
        return Ok(ExitCode::SUCCESS);
    }
    for f in &result.files {
        let path = root.join(&f.path);
        std::fs::write(&path, &f.after).with_context(|| format!("writing {}", path.display()))?;
        println!("edited    {}", f.path);
    }
    let after = scan(root, docair_adapters::all(), &options)?;
    let left = repeated(&after).len();
    println!("docair tidy: {} removed, {left} left", n - left.min(n));
    Ok(ExitCode::SUCCESS)
}

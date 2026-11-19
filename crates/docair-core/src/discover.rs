//! Stage 1: pick the files docair reads.

use anyhow::Result;
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use std::path::Path;

use crate::config::Config;
use crate::lang::{Family, language_for_file};
use crate::model::SourceFile;

/***
 * Walk the project and pick a language row for every file docair can read.
 *
 * Honours `.gitignore`, `.docairignore` and the include/exclude globs of
 * `docair.toml`. Hidden directories and files with unknown extensions are
 * skipped. Data files such as JSON are read only when they are named like an
 * API description or listed in `[import]`, so lock files stay unread. The
 * result is sorted by path so that every scan is deterministic.
 *
 * @flow  scan 4 "walk the project"
 * @reads project-files "provides the files to scan"
 * @reads docair-core.lang.languages "maps file extensions to languages"
 * @out   files: SourceFile[]
 ***/
pub fn discover(root: &Path, config: &Config) -> Result<Vec<SourceFile>> {
    let include = globs(&config.include)?;
    let exclude = globs(&config.exclude)?;
    let data_files = data_globs(&config.api_files())?;
    let mut walker = WalkBuilder::new(root);
    walker
        .require_git(false)
        .add_custom_ignore_filename(".docairignore");

    let mut files = Vec::new();
    for entry in walker.build() {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if (!config.include.is_empty() && !include.is_match(&rel)) || exclude.is_match(&rel) {
            continue;
        }
        let Some(lang) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(language_for_file)
        else {
            continue;
        };
        if lang.family == Family::Data && !data_files.is_match(&rel) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        files.push(SourceFile {
            path: path.to_path_buf(),
            rel,
            lang,
            text,
        });
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(files)
}

fn globs(patterns: &[String]) -> Result<GlobSet> {
    let mut set = GlobSetBuilder::new();
    for p in patterns {
        set.add(GlobBuilder::new(p).literal_separator(true).build()?);
    }
    Ok(set.build()?)
}

/// Data file names ignore case: `OpenAPI.json` is read like `openapi.json`.
fn data_globs(patterns: &[String]) -> Result<GlobSet> {
    let mut set = GlobSetBuilder::new();
    for p in patterns {
        set.add(
            GlobBuilder::new(p)
                .literal_separator(true)
                .case_insensitive(true)
                .build()?,
        );
    }
    Ok(set.build()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_read_only_when_named_like_an_api_description_or_listed() {
        let dir = tempfile_dir();
        for f in [
            "package-lock.json",
            "api/openapi.json",
            "vendor/Payments-OpenAPI.json",
            "contracts/orders.json",
            "events/asyncapi.json",
        ] {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, "{}").unwrap();
        }
        let rels = |config: &Config| -> Vec<String> {
            discover(&dir, config)
                .unwrap()
                .into_iter()
                .map(|f| f.rel)
                .collect()
        };
        assert_eq!(
            rels(&Config::default()),
            [
                "api/openapi.json",
                "events/asyncapi.json",
                "vendor/Payments-OpenAPI.json"
            ]
        );
        let listed: Config = toml::from_str("[import]\napis = [\"contracts/*.json\"]").unwrap();
        assert!(rels(&listed).contains(&"contracts/orders.json".to_string()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "docair-discover-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}

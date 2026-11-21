//! `docair.toml` at the project root (SPEC §1.1).

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::ids::slug;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Id of the top-level system node. Default: the root directory's name.
    pub system: Option<String>,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    /// `false` turns unresolved refs into warnings, like `--lenient`.
    pub strict: Option<bool>,
    /// Folder of recorded walkthroughs, relative to the root (SPEC §10.3).
    pub walkthroughs: Option<String>,
    /// The model that answers `docair ask` (SPEC §10.1).
    pub ai: Option<AiSettings>,
    /// What the page shows when it opens (SPEC §11).
    pub view: Option<ViewSettings>,
    /// Test reports to read (SPEC §12.3).
    pub tests: Option<TestSettings>,
    /// Glossary files besides those named `GLOSSARY.md` (SPEC §13).
    #[serde(default)]
    pub glossary: Vec<String>,
    /// The decisions about words; default `docs/meanings.toml` (docs/MEANINGS.md).
    pub meanings: Option<String>,
    /// API descriptions to read: OpenAPI and AsyncAPI (docs/IMPORTS.md).
    pub import: Option<ImportSettings>,
    /// Checks that are off by default (SPEC §8).
    pub checks: Option<CheckSettings>,
    /// Calls between functions from code indexes (docs/ADAPTERS.md §5.4).
    pub code: Option<CodeSettings>,
    /// Explaining changes between commits (docs/EXPLAIN.md).
    pub explain: Option<ExplainSettings>,
}

/// Where `docair explain` keeps its reviews, unless `[explain] store` says otherwise: docair's own
/// output folder, so reviews are not part of the project (docair-8l6.74).
pub const DEFAULT_STORE: &str = ".docair/changes";

/// `[explain]` in `docair.toml`: where the findings about changes go, and what a run may cost.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplainSettings {
    /// The folder for the findings, relative to the root; default `.docair/changes` (`DEFAULT_STORE`).
    pub store: Option<String>,
    /// The most characters a review sends the model in its one request; default 1,000,000. A review
    /// that needs more is an error, never split (docair-8l6.70, 8l6.79).
    pub max_chars: Option<usize>,
    /// How much the model reasons before it answers: `low`, `medium` or `high`; default `low`
    /// (docair-8l6.78).
    pub reasoning: Option<String>,
}

/// Keys docair.toml no longer has, and why: a file that sets one fails, saying so (docair-8l6.87).
const GONE_KEYS: &[(&str, &str, &str)] = &[(
    "explain",
    "max_requests",
    "a review is one request now (docair-8l6.79); remove it, and use max_chars for its size",
)];

/// `[code]` in `docair.toml`: where the calls between functions come from.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeSettings {
    /// SCIP index files to read, relative to the root (built in CI, for example).
    #[serde(default)]
    pub scip: Vec<String>,
    /// Run the indexers docair finds installed (`rust-analyzer scip`) and read what they write.
    /// Never for an untrusted project (`Options::untrusted`): an indexer builds the code, and build scripts run.
    #[serde(default)]
    pub run_indexers: bool,
    /// How many functions a call by a name several have may get could lines to (docair-4wk.36):
    /// -1 none (no guesses), 0 every one, N at most N; default 5. A call with more is an error.
    pub max_candidates: Option<i64>,
}

/// How many could lines a call by a name several functions have may draw (`[code] max_candidates`,
/// docair-4wk.36).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateLimit {
    /// No could lines at all (-1).
    Off,
    /// One to every candidate, however many (0).
    All,
    /// To at most this many; a call with more is reported as an error.
    AtMost(usize),
}

impl CandidateLimit {
    /// The default: at most 5.
    pub const DEFAULT: CandidateLimit = CandidateLimit::AtMost(5);

    /// A call by a name more functions have than the limit allows: an error, so the limit is raised
    /// or guesses turned off rather than the call silently left undrawn (docair-4wk.36).
    pub fn too_many(
        file: &str,
        line: usize,
        text: &str,
        name: &str,
        found: usize,
        limit: usize,
    ) -> crate::model::Diagnostic {
        crate::model::Diagnostic::new(
            "E007",
            format!(
                "the call `{text}(…)` could go to {found} functions named `{name}`, more than [code] max_candidates ({limit}) allows, so none is drawn: raise it, set 0 to draw them all, or -1 for no guesses"
            ),
        )
        .at(file, line)
    }

    /// `[code] max_candidates` as a rule; the default when it says nothing.
    pub fn of(code: Option<&CodeSettings>) -> CandidateLimit {
        match code.and_then(|c| c.max_candidates) {
            None => CandidateLimit::DEFAULT,
            Some(0) => CandidateLimit::All,
            Some(n) if n < 0 => CandidateLimit::Off,
            Some(n) => CandidateLimit::AtMost(n as usize),
        }
    }
}

/***
 * The file or folder a path in `docair.toml` names, if it stays inside the root.
 *
 * The path must be relative and must not leave the root, also not through a
 * symbolic link, and it must be a regular file or a folder: an uploaded
 * `docair.toml` can't make docair read `/etc`, `../..` or `/dev/zero`.
 ***/
pub fn inside(root: &Path, rel: &str) -> Option<PathBuf> {
    if Path::new(rel).is_absolute() {
        return None;
    }
    let root = root.canonicalize().ok()?;
    let path = root.join(rel).canonicalize().ok()?;
    let meta = std::fs::metadata(&path).ok()?;
    (path.starts_with(&root) && (meta.is_file() || meta.is_dir())).then_some(path)
}

/// `[checks]` in `docair.toml`: checks that are off by default.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckSettings {
    /// W118: a route that nothing in the project requests.
    #[serde(default)]
    pub unrequested_routes: bool,
}

/// `[import]` in `docair.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportSettings {
    /// More JSON files to read as API descriptions, besides [`DEFAULT_API_FILES`].
    #[serde(default)]
    pub apis: Vec<String>,
    /// `[[import.api]]`: settings for single descriptions.
    #[serde(default)]
    pub api: Vec<ApiSettings>,
}

/// `[[import.api]]` in `docair.toml`: one API description.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiSettings {
    /// The description file, relative to the root.
    pub file: String,
    /// The node the description belongs to: the service whose routes it lists.
    pub part_of: Option<String>,
    /// Put in front of every route instead of the path of the description's servers.
    pub base: Option<String>,
    /// Routes the description leaves out on purpose, e.g. `GET /healthz`.
    #[serde(default)]
    pub ignore: Vec<String>,
}

/// JSON files read as API descriptions without being listed (names ignore case).
pub const DEFAULT_API_FILES: &[&str] = &[
    "**/*openapi*.json",
    "**/*swagger*.json",
    "**/*asyncapi*.json",
];

/// `[view]` in `docair.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewSettings {
    #[serde(default)]
    pub hide: Vec<String>,
    /// Parts to show inside hidden sections: their siblings are hidden instead.
    #[serde(default)]
    pub show: Vec<String>,
    pub tests: Option<bool>,
}

/// `[tests]` in `docair.toml`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestSettings {
    /// Playwright JSON reports, relative to the root.
    #[serde(default)]
    pub results: Vec<String>,
}

/// `[ai]` in `docair.toml`. The key is never in this file, only the name of
/// the environment variable that holds it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiSettings {
    /// Base URL of an OpenAI-compatible API, e.g. `https://api.together.xyz/v1`.
    pub endpoint: Option<String>,
    /// Model id as the endpoint names it, e.g. `zai-org/GLM-5.3-Flash`.
    pub model: Option<String>,
    /// Name of the environment variable with the key. Default: `DOCAIR_AI_API_KEY`.
    pub api_key_env: Option<String>,
    /// Upper bound for the answer, reasoning included.
    pub max_tokens: Option<u32>,
}

/// Where recorded walkthroughs live when `docair.toml` does not say.
pub const DEFAULT_WALKTHROUGHS: &str = "docs/walkthroughs";

impl Config {
    pub fn walkthrough_dir(&self) -> &str {
        self.walkthroughs.as_deref().unwrap_or(DEFAULT_WALKTHROUGHS)
    }

    /// Globs of the JSON files read as API descriptions: the defaults, `[import] apis` and every `[[import.api]] file`.
    pub fn api_files(&self) -> Vec<String> {
        let mut out: Vec<String> = DEFAULT_API_FILES.iter().map(|s| s.to_string()).collect();
        if let Some(import) = &self.import {
            out.extend(import.apis.iter().cloned());
            out.extend(import.api.iter().map(|a| a.file.clone()));
        }
        out
    }

    /// The `[[import.api]]` settings of one description, by its path relative to the root.
    pub fn api(&self, file: &str) -> Option<&ApiSettings> {
        self.import.as_ref()?.api.iter().find(|a| a.file == file)
    }

    pub fn system_id(&self, root: &Path) -> String {
        self.system.clone().unwrap_or_else(|| {
            let name = root
                .canonicalize()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "system".to_string());
            slug(&name)
        })
    }
}

/***
 * Load `docair.toml` from the project root, or use the defaults when there is none.
 *
 * @flow  scan 3 "read docair.toml"
 * @reads project-files.docair.toml "reads docair.toml from the project root"
 * @out   config: Config
 ***/
pub fn load(root: &Path) -> Result<Config> {
    let path = root.join("docair.toml");
    if !path.exists() {
        return Ok(Config::default());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    if let Ok(table) = text.parse::<toml::Table>() {
        for (section, key, why) in GONE_KEYS {
            if table.get(*section).and_then(|s| s.get(*key)).is_some() {
                anyhow::bail!(
                    "{}: `[{section}] {key}` is no longer used: {why}",
                    path.display()
                );
            }
        }
    }
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

#[cfg(test)]
mod inside_tests {
    use super::inside;

    /// A key docair.toml no longer has fails, naming it and why (docair-8l6.87).
    #[test]
    fn a_key_no_longer_used_fails_naming_it() {
        let dir = std::env::temp_dir().join(format!("docair-gone-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("docair.toml"), "[explain]\nmax_requests = 4\n").unwrap();
        let err = super::load(&dir).unwrap_err().to_string();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            err.contains("`[explain] max_requests` is no longer used")
                && err.contains("one request"),
            "{err}"
        );
    }

    #[test]
    fn a_path_in_docair_toml_stays_inside_the_root() {
        let dir = std::env::temp_dir().join(format!("docair-inside-{}", std::process::id()));
        let root = dir.join("root");
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("target/index.scip"), "").unwrap();
        std::fs::write(dir.join("secret"), "").unwrap();
        assert!(inside(&root, "target/index.scip").is_some());
        assert!(inside(&root, "target").is_some());
        assert!(inside(&root, "./target/../target/index.scip").is_some());
        assert!(inside(&root, "../secret").is_none(), "leaves the root");
        assert!(inside(&root, "/dev/zero").is_none(), "absolute");
        assert!(inside(&root, "missing.scip").is_none());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("secret"), root.join("link")).unwrap();
            assert!(inside(&root, "link").is_none(), "a link out of the root");
            std::os::unix::fs::symlink("/dev/zero", root.join("zero")).unwrap();
            assert!(inside(&root, "zero").is_none(), "no device");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

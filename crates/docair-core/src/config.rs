//! `docair.toml` at the project root (SPEC §1.1).

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

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
    /// API descriptions to read: OpenAPI and AsyncAPI (docs/IMPORTS.md).
    pub import: Option<ImportSettings>,
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
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

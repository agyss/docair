//! The AI settings: `[ai]` in docair.toml, the environment, and `.env` (SPEC §10.1).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use docair_core::config::AiSettings;

/// The variable that holds the key when `api_key_env` does not name another.
pub const DEFAULT_KEY_VAR: &str = "DOCAIR_AI_API_KEY";
/// Room for the answer, reasoning included.
pub const DEFAULT_MAX_TOKENS: u32 = 32_000;

/// The process environment, then a `.env` file at the project root.
///
/// `.env` is read into a map and never into the process environment.
#[derive(Debug, Default, Clone)]
pub struct Env {
    file: HashMap<String, String>,
    path: Option<PathBuf>,
}

impl Env {
    /***
     * Read the environment, plus `<root>/.env` when there is one.
     *
     * @id    docair-ai.config.env
     * @reads dotenv -> DOCAIR_AI_API_KEY
     ***/
    pub fn load(root: Option<&Path>) -> Env {
        let Some(path) = root.map(|r| r.join(".env")).filter(|p| p.is_file()) else {
            return Env::default();
        };
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        Env {
            file: parse_dotenv(&text),
            path: Some(path),
        }
    }

    /// The value and where it came from.
    pub fn get(&self, key: &str) -> Option<(String, String)> {
        if let Some(v) = std::env::var(key).ok().filter(|v| !v.trim().is_empty()) {
            return Some((v, format!("environment variable {key}")));
        }
        let v = self.file.get(key).filter(|v| !v.trim().is_empty())?;
        let at = self
            .path
            .as_ref()
            .map_or(".env".to_string(), |p| p.display().to_string());
        Some((v.clone(), format!("{key} in {at}")))
    }

    #[cfg(test)]
    fn from_pairs(pairs: &[(&str, &str)]) -> Env {
        Env {
            file: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            path: Some(PathBuf::from(".env")),
        }
    }
}

/// `KEY=value` lines; `#` comments, `export ` and quotes are allowed.
fn parse_dotenv(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let value = match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or("").to_string(),
            _ => value.split(" #").next().unwrap_or("").trim().to_string(),
        };
        out.insert(key.trim().to_string(), value);
    }
    out
}

/// Everything needed to reach the model. `Debug` never shows the key.
#[derive(Clone)]
pub struct AiConfig {
    /// Base URL, e.g. `https://api.together.xyz/v1`; requests go to `{endpoint}/chat/completions`.
    pub endpoint: String,
    pub model: String,
    pub api_key: Option<String>,
    /// The variable the key is (or would be) read from.
    pub key_var: String,
    /// Where the key was found, e.g. `DOCAIR_AI_API_KEY in /repo/.env`. Never the key.
    pub key_source: Option<String>,
    pub max_tokens: u32,
}

impl std::fmt::Debug for AiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiConfig")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "<set>"))
            .field("key_source", &self.key_source)
            .field("max_tokens", &self.max_tokens)
            .finish()
    }
}

impl AiConfig {
    /***
     * Combine `[ai]` from docair.toml with the environment, which wins. Without an endpoint and a model, AI is off.
     *
     * @id      docair-ai.config.resolve
     * @calls   docair-ai.config.env "reads the environment and .env"
     * @out     config: AiConfig
     ***/
    pub fn resolve(settings: Option<&AiSettings>, env: &Env) -> Option<AiConfig> {
        let pick = |var: &str, file: Option<&String>| {
            env.get(var)
                .map(|(v, _)| v)
                .or_else(|| file.cloned())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let endpoint = pick(
            "DOCAIR_AI_ENDPOINT",
            settings.and_then(|s| s.endpoint.as_ref()),
        )?;
        let model = pick("DOCAIR_AI_MODEL", settings.and_then(|s| s.model.as_ref()))?;
        let key_var = settings
            .and_then(|s| s.api_key_env.clone())
            .unwrap_or_else(|| DEFAULT_KEY_VAR.to_string());
        let key = env.get(&key_var);
        let max_tokens = env
            .get("DOCAIR_AI_MAX_TOKENS")
            .and_then(|(v, _)| v.parse().ok())
            .or(settings.and_then(|s| s.max_tokens))
            .unwrap_or(DEFAULT_MAX_TOKENS);
        Some(AiConfig {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            model,
            api_key: key.as_ref().map(|(k, _)| k.trim().to_string()),
            key_var,
            key_source: key.map(|(_, at)| at),
            max_tokens,
        })
    }

    /// The endpoint without any `user:password@` part, safe to record and show.
    pub fn public_endpoint(&self) -> String {
        match self.endpoint.split_once("://") {
            Some((scheme, rest)) => {
                let host_end = rest.find('/').unwrap_or(rest.len());
                let rest = match rest[..host_end].rfind('@') {
                    Some(at) => &rest[at + 1..],
                    None => rest,
                };
                format!("{scheme}://{rest}")
            }
            None => self.endpoint.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_lines() {
        let m = parse_dotenv("# c\nA=1\nexport B = two \nC=\"x # y\"\nD='q'\nE=v # note\nbroken\n");
        assert_eq!(m["A"], "1");
        assert_eq!(m["B"], "two");
        assert_eq!(m["C"], "x # y");
        assert_eq!(m["D"], "q");
        assert_eq!(m["E"], "v");
        assert!(!m.contains_key("broken"));
    }

    #[test]
    fn settings_need_endpoint_and_model_and_key_comes_from_env() {
        let settings = AiSettings {
            endpoint: Some("https://api.example.test/v1/".into()),
            model: Some("m".into()),
            api_key_env: Some("DOCAIR_TEST_KEY_VAR".into()),
            max_tokens: None,
            glossary_reasoning: None,
        };
        let env = Env::from_pairs(&[("DOCAIR_TEST_KEY_VAR", "secret")]);
        let cfg = AiConfig::resolve(Some(&settings), &env).unwrap();
        assert_eq!(cfg.endpoint, "https://api.example.test/v1");
        assert_eq!(cfg.api_key.as_deref(), Some("secret"));
        assert_eq!(
            cfg.key_source.as_deref(),
            Some("DOCAIR_TEST_KEY_VAR in .env")
        );
        assert_eq!(cfg.max_tokens, DEFAULT_MAX_TOKENS);
        assert!(
            !format!("{cfg:?}").contains("secret"),
            "Debug hides the key"
        );

        let no_model = AiSettings {
            model: None,
            ..settings.clone()
        };
        assert!(
            AiConfig::resolve(Some(&no_model), &env).is_none(),
            "AI is off"
        );
    }

    #[test]
    fn public_endpoint_drops_credentials() {
        let mut cfg = AiConfig::resolve(
            Some(&AiSettings {
                endpoint: Some("https://u:p@host.test/v1".into()),
                model: Some("m".into()),
                ..Default::default()
            }),
            &Env::default(),
        )
        .unwrap();
        assert_eq!(cfg.public_endpoint(), "https://host.test/v1");
        cfg.endpoint = "http://localhost:11434/v1".into();
        assert_eq!(cfg.public_endpoint(), "http://localhost:11434/v1");
    }
}

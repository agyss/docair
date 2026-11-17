//! Generic adapter (docs/ADAPTERS.md §10): for code that no other adapter
//! knows, it guesses what each doc block documents from the code line under it.

use docair_core::adapter::Adapter;
use docair_core::facts::{Facts, Inferred, Symbol};
use docair_core::lang::Family;
use docair_core::model::{Following, RawBlock, SourceFile};

/***
 * The generic adapter: what a doc block documents, guessed from the code line under it.
 *
 * Runs on every file, last: a more specific adapter that knows the same line
 * says what it is first.
 ***/
pub struct GenericAdapter;

impl Adapter for GenericAdapter {
    fn name(&self) -> &'static str {
        "generic"
    }

    fn handles(&self, _file: &SourceFile) -> bool {
        true
    }

    /***
     * One symbol for each block that stands above a line of code.
     *
     * @calls docair-lang-generic.guess "guesses what the line declares"
     ***/
    fn file(&self, file: &SourceFile, blocks: &[RawBlock]) -> Facts {
        let symbols = blocks
            .iter()
            .filter_map(|b| b.following.as_ref())
            .map(|f| guess(file, f))
            .filter(Symbol::has_identity)
            .collect();
        Facts {
            symbols,
            ..Facts::default()
        }
    }
}

/***
 * Guess symbol, name and kind from a code line by keyword, for languages without an adapter.
 ***/
pub fn guess(file: &SourceFile, following: &Following) -> Symbol {
    let text = following.text.as_str();
    let line = following.line;
    let mut s = Symbol::at(format!("generic:{}:{line}", file.rel), &file.rel, line);

    if file.lang.family == Family::Markup && text.starts_with('<') {
        let tag_name: String = text[1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let id_attr = text.split("id=\"").nth(1).and_then(|s| s.split('"').next());
        s.display = Some(format!("<{tag_name}>"));
        s.name = Some(id_attr.unwrap_or(&tag_name).to_string());
        s.kind = Some(Inferred::new("component", "markup element"));
        return s;
    }

    let infer = |s: &mut Symbol, keyword: &str, name: &str, kind: Option<&str>, why: &str| {
        s.display = Some(if keyword.is_empty() {
            name.to_string()
        } else {
            format!("{keyword} {name}")
        });
        s.name = Some(name.to_string());
        if let Some(kind) = kind {
            s.kind = Some(Inferred::new(kind, why));
        }
    };

    // Terraform / HCL: `resource "kind_cluster" "docair" {`
    if file.lang.name == "terraform" {
        let mut words = text.split_whitespace();
        if let Some(block_type) = words.next() {
            let labels: Vec<&str> = text.split('"').skip(1).step_by(2).collect();
            if !labels.is_empty() {
                // An attribute (`namespace = "docair"`) names itself with its key;
                // `name = "traefik"` names the thing the block describes.
                if words.next() == Some("=") {
                    let name = if block_type == "name" {
                        labels.join(".")
                    } else {
                        format!("{block_type} {}", labels.join("."))
                    };
                    infer(&mut s, "", &name, None, "");
                } else {
                    infer(&mut s, block_type, &labels.join("."), None, "");
                }
                return s;
            }
        }
    }

    const SKIP: &[&str] = &[
        "pub",
        "crate",
        "export",
        "default",
        "async",
        "static",
        "public",
        "private",
        "protected",
        "abstract",
        "unsafe",
        "extern",
        "declare",
        "readonly",
        "override",
    ];
    let mut tokens = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .filter(|t| !t.is_empty())
        .skip_while(|t| SKIP.contains(t));
    if let (Some(keyword), Some(name)) = (tokens.next(), tokens.next()) {
        let kind = match keyword {
            "fn" | "function" | "def" => Some("function"),
            "class" | "struct" | "interface" | "trait" | "enum" | "impl" | "type" => {
                Some("component")
            }
            "mod" | "module" | "namespace" => Some("module"),
            "const" | "let" | "var" if text.contains("=>") || text.contains("function") => {
                Some("function")
            }
            "const" | "let" | "var" => Some("config"),
            _ => None,
        };
        if let Some(kind) = kind {
            infer(
                &mut s,
                keyword,
                name,
                Some(kind),
                &format!("keyword `{keyword}`"),
            );
            if kind == "function" && file.lang.name == "typescript" {
                s.signature = docair_core::signature::ts_signature(text);
            }
            return s;
        }
    }

    // Tests: `test('title', …)`, `it("…")`, `test.describe('…')` are named after their title.
    if file.lang.name == "typescript"
        && let Some(title) = test_title(text)
    {
        infer(&mut s, "", &title, Some("test"), "test call");
        return s;
    }

    // TypeScript class members: `async upload(files: File[]) {`, `scan = (f) => …`
    if file.lang.name == "typescript"
        && let Some(name) = method_name(text)
    {
        infer(&mut s, "", name, Some("function"), "method");
        s.signature = docair_core::signature::ts_signature(text);
        return s;
    }

    // Kubernetes manifests (plain or Helm templates): `kind:` and `metadata.name`.
    if file.lang.name == "yaml"
        && (text.starts_with("apiVersion:") || text.starts_with("{{"))
        && let Some((k8s_kind, name)) = k8s_object(&file.text, line)
    {
        let kind = match k8s_kind.as_str() {
            "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" | "Pod" | "CronJob"
            | "Job" => "workload",
            "Service" => "endpoint",
            "Ingress" | "Gateway" | "HTTPRoute" => "gateway",
            "Namespace" => "namespace",
            _ => "config",
        };
        let label = format!("{k8s_kind} {name}");
        infer(
            &mut s,
            "",
            &label,
            Some(kind),
            &format!("Kubernetes kind `{k8s_kind}`"),
        );
        return s;
    }

    // Anything else (YAML, Dockerfile, nginx): the attached line is the symbol.
    let short: String = text.chars().take(60).collect();
    s.display = Some(short);
    s
}

/// `kind` and `metadata.name` of the Kubernetes object that starts at `line` (1-based).
fn k8s_object(text: &str, line: usize) -> Option<(String, String)> {
    let mut kind = None;
    let mut name = None;
    let mut in_metadata = false;
    for l in text.lines().skip(line.saturating_sub(1)).take(40) {
        if l.trim() == "---" {
            break;
        }
        if let Some(k) = l.strip_prefix("kind:") {
            kind.get_or_insert_with(|| k.trim().to_string());
        }
        if !l.starts_with(' ') {
            in_metadata = l.starts_with("metadata:");
            continue;
        }
        if in_metadata
            && name.is_none()
            && let Some(n) = l.strip_prefix("  name:")
        {
            name = Some(n.trim().trim_matches('"').to_string());
        }
    }
    Some((kind?, name?))
}

/// The title of a `test(…)`, `it(…)` or `describe(…)` call, with modifiers like `.only` or `.describe`.
pub fn test_title(text: &str) -> Option<String> {
    let t = text.trim_start();
    let head = ["test", "it", "describe"]
        .into_iter()
        .find(|h| t.starts_with(h))?;
    let mut rest = &t[head.len()..];
    while let Some(r) = rest.strip_prefix('.') {
        let word: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        if ![
            "only", "skip", "describe", "serial", "parallel", "fixme", "fail", "slow",
        ]
        .contains(&word.as_str())
        {
            return None;
        }
        rest = &r[word.len()..];
    }
    let rest = rest.trim_start().strip_prefix('(')?.trim_start();
    let quote = rest
        .chars()
        .next()
        .filter(|c| matches!(c, '\'' | '"' | '`'))?;
    let body = &rest[1..];
    let end = body.find(quote)?;
    Some(body[..end].to_string()).filter(|s| !s.is_empty())
}

/// The method or arrow-function field a TypeScript line declares, if any.
fn method_name(text: &str) -> Option<&str> {
    const MODIFIERS: &[&str] = &[
        "public ",
        "private ",
        "protected ",
        "static ",
        "async ",
        "readonly ",
        "override ",
        "get ",
        "set ",
    ];
    let mut t = text.trim();
    loop {
        let before = t;
        for m in MODIFIERS {
            if let Some(rest) = t.strip_prefix(m) {
                t = rest.trim_start();
            }
        }
        if t == before {
            break;
        }
    }
    let end = t
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .unwrap_or(t.len());
    let (name, rest) = t.split_at(end);
    const NOT_METHODS: &[&str] = &[
        "if", "for", "while", "switch", "catch", "return", "function", "new", "await", "yield",
        "typeof", "super", "this",
    ];
    if name.is_empty()
        || name.starts_with(|c: char| c.is_ascii_digit())
        || NOT_METHODS.contains(&name)
    {
        return None;
    }
    let rest = rest.trim_start();
    let rest = match rest.strip_prefix('<') {
        Some(generic) => generic
            .split_once('>')
            .map_or(generic, |(_, r)| r)
            .trim_start(),
        None => rest,
    };
    let is_method = rest.starts_with('(');
    let is_arrow_field = rest.starts_with('=') && !rest.starts_with("==") && rest.contains("=>");
    (is_method || is_arrow_field).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::{method_name, test_title};

    #[test]
    fn kubernetes_objects() {
        let text = "apiVersion: v1\nkind: Service\nmetadata:\n  name: docair-web\n  labels:\n    name: other\nspec: {}\n";
        assert_eq!(
            super::k8s_object(text, 1),
            Some(("Service".into(), "docair-web".into()))
        );
    }

    #[test]
    fn test_titles() {
        assert_eq!(
            test_title("test('a page', async () => {").as_deref(),
            Some("a page")
        );
        assert_eq!(
            test_title("  test.describe(\"web\", () => {").as_deref(),
            Some("web")
        );
        assert_eq!(test_title("it.only(`x y`, fn)").as_deref(), Some("x y"));
        assert_eq!(test_title("testing('x')"), None);
        assert_eq!(test_title("test.extend({})"), None);
    }

    #[test]
    fn typescript_members() {
        assert_eq!(
            method_name("async upload(files: File[]): Promise<void> {"),
            Some("upload")
        );
        assert_eq!(
            method_name("protected onDrop(event: DragEvent) {"),
            Some("onDrop")
        );
        assert_eq!(method_name("scan<T>(files: File[]) {"), Some("scan"));
        assert_eq!(
            method_name("readonly reset = () => this.files.set([]);"),
            Some("reset")
        );
        assert_eq!(method_name("if (x) {"), None);
        assert_eq!(method_name("readonly files = signal<File[]>([]);"), None);
    }
}

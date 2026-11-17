//! API descriptions, OpenAPI and AsyncAPI, read as facts (docs/IMPORTS.md).
//!
//! A description declares the routes a service answers and the topics an
//! application sends to and receives from, with what travels there. This
//! crate reads the files; the core matches the declarations with the code.

mod asyncapi;
mod model;
mod openapi;
mod tree;

pub use model::{Action, Api, Event, Format, Operation, join};
pub use tree::Problem;

use docair_core::adapter::{Adapter, Project};
use docair_core::config::ApiSettings;
use docair_core::facts::{
    Declare, Draw, Evidence, Facts, Inferred, Meet, Relation, Role, Symbol, Target, Verb,
};
use docair_core::ids::slug;
use docair_core::model::{Diagnostic, Http, SourceFile};

/// Which description a YAML or JSON text is, by its version key, without parsing it.
///
/// YAML: a top-level `openapi:`, `swagger:` or `asyncapi:` key. JSON: such a
/// key anywhere. Helm templates are never parsed, because they are not YAML.
pub fn sniff(text: &str) -> Option<Format> {
    const KEYS: [(&str, Format); 3] = [
        ("openapi", Format::OpenApi),
        ("swagger", Format::OpenApi),
        ("asyncapi", Format::AsyncApi),
    ];
    let text = text.trim_start_matches('\u{feff}');
    let start = text.trim_start();
    if start.starts_with('{') && !start.starts_with("{{") {
        return KEYS.into_iter().find_map(|(key, format)| {
            let quoted = format!("\"{key}\"");
            text.match_indices(&quoted)
                .any(|(i, _)| text[i + quoted.len()..].trim_start().starts_with(':'))
                .then_some(format)
        });
    }
    // A top-level key starts the line; it may be quoted.
    text.lines().find_map(|line| {
        let key_text = line.strip_prefix(['"', '\'']).unwrap_or(line);
        KEYS.into_iter().find_map(|(key, format)| {
            let rest = key_text.strip_prefix(key)?;
            let rest = rest.strip_prefix(['"', '\'']).unwrap_or(rest);
            rest.trim_start().starts_with(':').then_some(format)
        })
    })
}

/// Read a description: OpenAPI 3.0, 3.1 and 3.2 or Swagger 2.0, AsyncAPI 2.x and 3.x.
pub fn read(text: &str) -> Result<Api, Problem> {
    let format = sniff(text).ok_or_else(|| Problem {
        line: None,
        message: "not an OpenAPI or AsyncAPI description".into(),
    })?;
    let root = tree::parse(text)?;
    match format {
        Format::OpenApi => openapi::read(&root),
        Format::AsyncApi => asyncapi::read(&root),
    }
}

/***
 * Reads OpenAPI and AsyncAPI descriptions: what a service answers, what an application sends and receives.
 ***/
pub struct ApiAdapter;

impl Adapter for ApiAdapter {
    fn name(&self) -> &'static str {
        "apis"
    }

    fn handles(&self, file: &SourceFile) -> bool {
        matches!(file.lang.name, "yaml" | "json") && sniff(&file.text).is_some()
    }

    /***
     * Read every OpenAPI and AsyncAPI description of the project, with its `[[import.api]]` settings and its doc blocks.
     *
     * @out facts: Facts
     ***/
    fn project(&self, project: &Project) -> Facts {
        let mut facts = Facts::default();
        for file in project.files {
            if self.handles(file) {
                facts.extend(facts_of(file, project.config.api(&file.rel)));
            }
        }
        facts
    }
}

/// The facts of one description: the file as a symbol, and what it declares.
fn facts_of(file: &SourceFile, settings: Option<&ApiSettings>) -> Facts {
    let mut facts = Facts::default();
    let api = match read(&file.text) {
        Ok(api) => api,
        Err(p) => {
            facts.diagnostics.push(
                Diagnostic::new(
                    "W113",
                    format!("cannot read the API description: {}", p.message),
                )
                .at(&file.rel, p.line.unwrap_or(1)),
            );
            return facts;
        }
    };
    for p in &api.skipped {
        facts.diagnostics.push(
            Diagnostic::new("W113", format!("API description, not read: {}", p.message))
                .at(&file.rel, p.line.unwrap_or(api.line)),
        );
    }
    let key = format!("apis:{}", file.rel);
    let file_name = file.rel.rsplit('/').next().unwrap_or(&file.rel);
    let what = format!("{} {}", api.format.name(), api.version);
    let parents: Vec<String> = settings
        .and_then(|s| s.part_of.clone())
        .into_iter()
        .chain(api.part_of.clone())
        .collect();
    // A doc block right above the version key documents the description: then the
    // block is the node, and the description only says what it is.
    facts.symbols.push(Symbol {
        draw: Draw::Always,
        display: Some(format!("{what} {}", file.rel)),
        name: Some(file_name.to_string()),
        id: Some(id_of(&file.rel)),
        kind: Some(Inferred::new("api", what)),
        parents,
        docs: Some(match &api.description {
            Some(d) => format!("{}: {}", api.title, first_paragraph(d)),
            None => api.title.clone(),
        }),
        ..Symbol::at(key.clone(), &file.rel, api.line)
    });
    let base = settings.and_then(|s| s.base.as_deref());
    for op in &api.operations {
        let route = base.map_or_else(|| op.route(), |b| join(b, &op.path));
        let named = op
            .id
            .as_deref()
            .map(|id| format!(" ({id})"))
            .unwrap_or_default();
        facts.declares.push(Declare {
            by: key.clone(),
            at: Meet::Http(Http {
                method: op.method.clone(),
                path: route.clone(),
            }),
            payload: op.payload.clone(),
            reply: op.reply.clone(),
            name: op.id.clone(),
            role: Role::Serves,
            summary: op.summary.clone(),
            description: op.description.clone(),
            evidence: Evidence::new(op.line, format!("{} {route}{named}", op.method)),
        });
    }
    for ev in &api.events {
        let (role, verb, relation) = match ev.action {
            Action::Send => (Role::Sends, "sends to", Verb::Emits),
            Action::Receive => (Role::Receives, "receives from", Verb::Consumes),
        };
        // The described application's line to the topic; code inside the service that draws the same line wins (core).
        facts.relations.push(Relation {
            from: key.clone(),
            payload: ev.message.clone(),
            ..Relation::new(
                relation,
                Target::Ref(format!("topic:{}", ev.address)),
                Evidence::new(
                    ev.line,
                    format!("{}:{} · {verb} {}", file.rel, ev.line, ev.address),
                ),
            )
        });
        facts.declares.push(Declare {
            by: key.clone(),
            at: Meet::Topic(ev.address.clone()),
            payload: ev.message.clone(),
            reply: None,
            name: ev.id.clone(),
            role,
            summary: ev.summary.clone(),
            description: ev.description.clone(),
            evidence: Evidence::new(ev.line, format!("{verb} {}", ev.address)),
        });
    }
    facts
}

/// A node id from the file's path: `api/openapi.yaml` → `api.openapi`.
fn id_of(rel: &str) -> String {
    let stem = rel.rsplit_once('.').map_or(rel, |(s, _)| s);
    stem.split('/')
        .map(slug)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

fn first_paragraph(text: &str) -> String {
    text.split("\n\n")
        .next()
        .unwrap_or(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_are_known_by_their_version_key() {
        assert_eq!(sniff("# orders\nopenapi: 3.1.0\n"), Some(Format::OpenApi));
        assert_eq!(
            sniff("info:\n  title: x\nswagger: '2.0'\n"),
            Some(Format::OpenApi)
        );
        assert_eq!(sniff("\"asyncapi\": 3.0.0\n"), Some(Format::AsyncApi));
        assert_eq!(
            sniff("{\"info\": {}, \"openapi\" : \"3.0.0\"}"),
            Some(Format::OpenApi)
        );
        assert_eq!(sniff("apiVersion: apps/v1\nkind: Deployment\n"), None);
        assert_eq!(sniff("  openapi: nested, not top-level\n"), None);
        assert_eq!(sniff("{{- if .Values.openapi }}\nkind: ConfigMap\n"), None);
        assert_eq!(sniff("{\"name\": \"openapi\"}"), None);
    }

    #[test]
    fn ids_follow_the_path() {
        assert_eq!(id_of("api/openapi.yaml"), "api.openapi");
        assert_eq!(
            id_of("vendor/Payments-OpenAPI.json"),
            "vendor.payments-open-api"
        );
        assert_eq!(id_of("asyncapi.yml"), "asyncapi");
    }
}

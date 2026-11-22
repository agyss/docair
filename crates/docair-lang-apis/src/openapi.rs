//! OpenAPI 3.0, 3.1 and 3.2, and Swagger 2.0: the routes a service answers, with what travels there.

use crate::model::{Api, Format, Operation};
use crate::tree::{Node, Problem, ref_name, type_name};

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Read an OpenAPI or Swagger document.
pub fn read(root: &Node) -> Result<Api, Problem> {
    let (key, swagger) = match (root.get("openapi"), root.get("swagger")) {
        (Some(v), _) => (v, false),
        (None, Some(v)) => (v, true),
        _ => return Err(Problem::at(1, "no `openapi` or `swagger` version")),
    };
    let version = key.text().unwrap_or_default();
    let known = if swagger {
        version == "2.0"
    } else {
        ["3.0", "3.1", "3.2"]
            .iter()
            .any(|v| version == *v || version.starts_with(&format!("{v}.")))
    };
    if !known {
        return Err(Problem::at(
            key.line,
            format!("OpenAPI version `{version}` is not one docair reads (2.0, 3.0, 3.1, 3.2)"),
        ));
    }
    let mut api = Api::new(Format::OpenApi, version, key.line);
    let info = root.get("info");
    api.title = info
        .and_then(|i| i.get("title"))
        .and_then(Node::words)
        .unwrap_or_else(|| "API".to_string());
    api.description = info
        .and_then(|i| i.get("description"))
        .and_then(Node::words);
    api.part_of = root.get("x-docair-part-of").and_then(Node::words);

    let doc_base = if swagger {
        root.get("basePath")
            .and_then(Node::text)
            .map(|b| clean_base(&b))
            .unwrap_or_default()
    } else {
        servers_base(root.get("servers")).unwrap_or_default()
    };
    let Some(paths) = root.get("paths") else {
        return Ok(api);
    };
    for (path, path_key, item) in paths.entries() {
        if path.starts_with("x-") {
            continue;
        }
        let item = match item.get("$ref").and_then(Node::text) {
            Some(r) => match root.resolve(&r) {
                Some(target) => target,
                None => {
                    api.skipped.push(Problem::at(
                        path_key.line,
                        format!(
                            "`{path}` is a reference to another file (`{r}`), which is not read"
                        ),
                    ));
                    continue;
                }
            },
            None => item,
        };
        let path_base = servers_base(item.get("servers"));
        for (method, method_key, op) in item.entries() {
            let m = method.to_ascii_lowercase();
            if m == "query" || m == "additionaloperations" {
                api.skipped.push(Problem::at(
                    method_key.line,
                    format!("`{path}`: `{method}` (OpenAPI 3.2) is not read yet"),
                ));
                continue;
            }
            if !METHODS.contains(&m.as_str()) {
                continue;
            }
            let base = servers_base(op.get("servers"))
                .or_else(|| path_base.clone())
                .unwrap_or_else(|| doc_base.clone());
            let params = parameters(root, item.get("parameters"), op.get("parameters"));
            let payload = request(root, op, &params, swagger).or_else(|| param_names(&params));
            api.operations.push(Operation {
                method: m.to_ascii_uppercase(),
                path: path.clone(),
                base,
                id: op.get("operationId").and_then(Node::words),
                summary: op.get("summary").and_then(Node::words),
                description: op.get("description").and_then(Node::words),
                payload,
                reply: reply(root, op, swagger),
                line: method_key.line,
            });
        }
    }
    Ok(api)
}

/// The path part of the first server's URL, with its variables' defaults.
fn servers_base(servers: Option<&Node>) -> Option<String> {
    let first = servers?.items().first()?;
    let mut url = first.get("url").and_then(Node::text)?;
    if let Some(vars) = first.get("variables") {
        for (name, _, var) in vars.entries() {
            if let Some(default) = var.get("default").and_then(Node::text) {
                url = url.replace(&format!("{{{name}}}"), &default);
            }
        }
    }
    Some(url_path(&url))
}

/// `https://api.example.com/v1/` → `/v1`; `/api` → `/api`; `https://example.com` → ``.
fn url_path(url: &str) -> String {
    let url = url.split(['?', '#']).next().unwrap_or("");
    let after_host = |rest: &str| {
        rest.find('/')
            .map_or(String::new(), |j| rest[j..].to_string())
    };
    let path = match url.find("://") {
        Some(i) => after_host(&url[i + 3..]),
        None => match url.strip_prefix("//") {
            Some(rest) => after_host(rest),
            None => url.to_string(),
        },
    };
    clean_base(&path)
}

/// A base path without a trailing slash, with a leading one; empty for the root.
fn clean_base(path: &str) -> String {
    let p = path.trim().trim_end_matches('/');
    if p.is_empty() {
        String::new()
    } else if p.starts_with('/') {
        p.to_string()
    } else {
        format!("/{p}")
    }
}

/// A parameter after following its `$ref`.
struct Param<'a> {
    name: String,
    place: String,
    node: &'a Node,
}

/// The parameters of an operation: the path item's, then its own; its own win (same name and place).
fn parameters<'a>(
    root: &'a Node,
    path_level: Option<&'a Node>,
    op_level: Option<&'a Node>,
) -> Vec<Param<'a>> {
    let mut out: Vec<Param> = Vec::new();
    for list in [path_level, op_level].into_iter().flatten() {
        for p in list.items() {
            let p = p.follow(root);
            let (Some(name), Some(place)) = (
                p.get("name").and_then(Node::text),
                p.get("in").and_then(Node::text),
            ) else {
                continue;
            };
            out.retain(|q| !(q.name == name && q.place == place));
            out.push(Param {
                name,
                place,
                node: p,
            });
        }
    }
    out
}

/// What the request sends: the body's type (3.x `requestBody`, 2.0 the `in: body` parameter), or form fields.
fn request(root: &Node, op: &Node, params: &[Param], swagger: bool) -> Option<String> {
    if swagger {
        if let Some(body) = params.iter().find(|p| p.place == "body") {
            return body.node.get("schema").and_then(|s| schema_type(root, s));
        }
        let form: Vec<&str> = params
            .iter()
            .filter(|p| p.place == "formData")
            .map(|p| p.name.as_str())
            .collect();
        return (!form.is_empty()).then(|| form.join(", "));
    }
    let written = op.get("requestBody")?;
    let body = written.follow(root);
    body.get("content")
        .and_then(|c| content_type(root, c))
        .or_else(|| {
            written
                .get("$ref")
                .and_then(Node::text)
                .and_then(|r| ref_name(&r))
        })
}

/// What a successful answer carries: the first 2xx response with a schema.
fn reply(root: &Node, op: &Node, swagger: bool) -> Option<String> {
    let responses = op.get("responses")?;
    let mut ok: Vec<(String, &Node)> = responses
        .entries()
        .filter(|(code, _, _)| code.starts_with('2'))
        .map(|(code, _, r)| (code, r))
        .collect();
    ok.sort_by(|a, b| a.0.cmp(&b.0));
    ok.iter().find_map(|(_, r)| {
        let r = r.follow(root);
        if swagger {
            r.get("schema").and_then(|s| schema_type(root, s))
        } else {
            r.get("content").and_then(|c| content_type(root, c))
        }
    })
}

/// The type in a `content` map: JSON first, then any media type with a schema.
fn content_type(root: &Node, content: &Node) -> Option<String> {
    let entries: Vec<(String, &Node, &Node)> = content.entries().collect();
    let pick = entries
        .iter()
        .find(|(m, _, _)| m == "application/json")
        .or_else(|| entries.iter().find(|(m, _, _)| m.contains("json")))
        .or_else(|| entries.iter().find(|(_, _, v)| v.get("schema").is_some()))?;
    schema_type(root, pick.2.get("schema")?)
}

/// A schema's name; for an unnamed object with a few fields (a form, say), its field names.
fn schema_type(root: &Node, schema: &Node) -> Option<String> {
    type_name(schema).or_else(|| {
        let fields: Vec<String> = schema
            .follow(root)
            .get("properties")?
            .entries()
            .map(|(k, _, _)| k)
            .collect();
        (!fields.is_empty() && fields.len() <= 3).then(|| fields.join(", "))
    })
}

/// The names of the path parameters, then the query parameters: `id, status`.
fn param_names(params: &[Param]) -> Option<String> {
    let names: Vec<&str> = ["path", "query"]
        .iter()
        .flat_map(|place| {
            params
                .iter()
                .filter(move |p| p.place == *place)
                .map(|p| p.name.as_str())
        })
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::parse;

    /// Method, route, operationId, payload, reply, line.
    type Op = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        usize,
    );

    fn ops(yaml: &str) -> Vec<Op> {
        read(&parse(yaml).unwrap())
            .unwrap()
            .operations
            .into_iter()
            .map(|o| {
                (
                    o.method.clone(),
                    o.route(),
                    o.id,
                    o.payload,
                    o.reply,
                    o.line,
                )
            })
            .collect()
    }

    #[test]
    fn openapi_3_operations_with_base_types_and_lines() {
        let yaml = "openapi: 3.1.0
info: {title: Orders API, version: 1.0.0}
servers:
  - url: https://{env}.example.com/api/
    variables: {env: {default: prod}}
paths:
  /orders:
    post:
      operationId: createOrder
      requestBody:
        content:
          application/json:
            schema: {$ref: '#/components/schemas/CreateOrder'}
      responses:
        '201':
          description: created
          content:
            application/json:
              schema: {$ref: '#/components/schemas/Order'}
  /orders/{id}:
    parameters:
      - {name: id, in: path, required: true, schema: {type: string}}
    get:
      responses:
        200:
          description: one
          content:
            application/json:
              schema: {type: array, items: {$ref: '#/components/schemas/Order'}}
    delete:
      servers: [{url: /admin}]
      responses: {'204': {description: gone}}
";
        assert_eq!(
            ops(yaml),
            vec![
                (
                    "POST".into(),
                    "/api/orders".into(),
                    Some("createOrder".into()),
                    Some("CreateOrder".into()),
                    Some("Order".into()),
                    8
                ),
                (
                    "GET".into(),
                    "/api/orders/{id}".into(),
                    None,
                    Some("id".into()),
                    Some("Order[]".into()),
                    23
                ),
                (
                    "DELETE".into(),
                    "/admin/orders/{id}".into(),
                    None,
                    Some("id".into()),
                    None,
                    30
                ),
            ]
        );
    }

    #[test]
    fn swagger_2_body_parameters_and_base_path() {
        let json = r##"{
  "swagger": "2.0",
  "info": {"title": "Payments", "version": "1.0"},
  "basePath": "/v1/",
  "paths": {
    "/charges": {
      "post": {
        "operationId": "createCharge",
        "parameters": [{"name": "charge", "in": "body", "schema": {"$ref": "#/definitions/ChargeRequest"}}],
        "responses": {"201": {"description": "ok", "schema": {"$ref": "#/definitions/Charge"}}}
      },
      "get": {
        "parameters": [{"$ref": "#/parameters/limit"}],
        "responses": {"200": {"description": "ok", "schema": {"type": "array", "items": {"$ref": "#/definitions/Charge"}}}}
      }
    }
  },
  "parameters": {"limit": {"name": "limit", "in": "query", "type": "integer"}}
}"##;
        let got = ops(json);
        assert_eq!(got[0].1, "/v1/charges");
        assert_eq!(got[0].3.as_deref(), Some("ChargeRequest"));
        assert_eq!(got[0].4.as_deref(), Some("Charge"));
        assert_eq!(got[1].3.as_deref(), Some("limit"));
        assert_eq!(got[1].4.as_deref(), Some("Charge[]"));
    }

    #[test]
    fn what_is_not_read_is_said() {
        let api = read(&parse("openapi: 3.2.0\ninfo: {title: X}\npaths:\n  /a: {$ref: 'other.yaml#/paths/a'}\n  /b:\n    query: {}\n    get: {}\n").unwrap()).unwrap();
        assert_eq!(api.operations.len(), 1);
        let lines: Vec<Option<usize>> = api.skipped.iter().map(|p| p.line).collect();
        assert_eq!(lines, vec![Some(4), Some(6)]);
        assert!(read(&parse("openapi: 4.0.0\n").unwrap()).is_err());
    }

    #[test]
    fn server_urls_give_their_path() {
        assert_eq!(url_path("https://api.example.com/v1/"), "/v1");
        assert_eq!(url_path("https://api.example.com"), "");
        assert_eq!(url_path("/api"), "/api");
        assert_eq!(url_path("api/v2"), "/api/v2");
        assert_eq!(url_path("//cdn.example.com/x?y=1"), "/x");
        assert_eq!(url_path("/"), "");
    }
}

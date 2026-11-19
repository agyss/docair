//! What API descriptions declare, meeting the code (docs/IMPORTS.md).
//!
//! An OpenAPI description declares the routes a service answers. Each route
//! becomes an endpoint inside the description's node; then the description
//! moves into the service it describes, and every endpoint whose route a
//! handler of that service serves merges into the handler. What is left shows
//! what the description says and the code does not.

use std::collections::{HashMap, HashSet};

use crate::config::Config;
use crate::facts::{Declare, Facts, Meet, Role};
use crate::ids::slug;
use crate::model::{
    Diagnostic, Doc, Edge, EdgeKind, Following, Hints, Http, LineEvidence, Node, Origin, RawBlock,
    Tag,
};
use crate::resolve::split_payload;
use crate::route::Route;

/// The inferred tag that carries what a description declares for a node: `POST /api/orders (CreateOrder) -> Order`.
pub const DESCRIBED: &str = "described";
/// The inferred tag with the description's own name for a route: its `operationId`.
pub const OPERATION: &str = "operation";

/***
 * The routes the API descriptions declare, as endpoints inside their description.
 *
 * @in  facts: Facts[] from docair-core.adapter.collect
 * @out blocks: RawBlock[]
 ***/
pub fn blocks(facts: &[Facts], system: &str) -> Vec<RawBlock> {
    let ids: HashMap<&str, &str> = facts
        .iter()
        .flat_map(|f| &f.symbols)
        .filter_map(|s| Some((s.key.as_str(), s.id.as_deref()?)))
        .collect();
    let mut used: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for d in facts.iter().flat_map(|f| &f.declares) {
        if let Meet::Topic(address) = &d.at {
            if used.insert(format!("topic:{address}")) {
                out.push(topic_block(d, address, system));
            }
            continue;
        }
        let (Meet::Http(http), Role::Serves) = (&d.at, d.role) else {
            continue;
        };
        let Some(parent) = ids.get(d.by.as_str()) else {
            continue;
        };
        let file = d.by.split_once(':').map_or(d.by.as_str(), |(_, f)| f);
        let route = format!("{} {}", http.method, http.path);
        let own = d
            .name
            .as_deref()
            .map(slug)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| slug(&route));
        let mut id = format!("{parent}.{own}");
        let mut n = 2;
        while !used.insert(id.clone()) {
            id = format!("{parent}.{own}-{n}");
            n += 1;
        }
        let line = d.evidence.line;
        // `api/openapi.yaml:10 · POST /api/orders (createOrder)`: where, then what.
        let why = format!("{file}:{line} · {}", d.evidence.why);
        let why = why.as_str();
        let mut tags = vec![
            Tag::inferred("kind", "endpoint", line, why),
            Tag::inferred("http", &route, line, why),
            Tag::inferred(DESCRIBED, &described(d, &route), line, why),
        ];
        if let Some(name) = &d.name {
            tags.push(Tag::inferred(OPERATION, name, line, why));
        }
        out.push(RawBlock {
            file: file.to_string(),
            line,
            end_line: line,
            summary: d.summary.clone().unwrap_or_else(|| route.clone()),
            description: d.description.clone().unwrap_or_default(),
            tags,
            following: Some(Following {
                line,
                text: String::new(),
            }),
            hints: Hints {
                symbol: Some(match &d.name {
                    Some(name) => format!("{route} ({name})"),
                    None => route.clone(),
                }),
                name: Some(route),
                id: Some(id),
                part_of: vec![parent.to_string()],
                ..Hints::default()
            },
            origin: Origin::Discovered,
        });
    }
    out
}

/// A topic an AsyncAPI description declares: the node `@emits topic:…` resolves to, with the channel's text and message.
fn topic_block(d: &Declare, address: &str, system: &str) -> RawBlock {
    let file = d.by.split_once(':').map_or(d.by.as_str(), |(_, f)| f);
    let line = d.evidence.line;
    let why = format!("{file}:{line} · {}", d.evidence.why);
    let id = format!("topic:{address}");
    let mut tags = vec![Tag::inferred("kind", "topic", line, &why)];
    if let Some(message) = &d.payload {
        tags.push(Tag::inferred(
            DESCRIBED,
            &format!("{id} ({message})"),
            line,
            &why,
        ));
    }
    let text = d.description.clone().or_else(|| d.summary.clone());
    RawBlock {
        file: file.to_string(),
        line,
        end_line: line,
        summary: text.unwrap_or_default(),
        description: String::new(),
        tags,
        following: Some(Following {
            line,
            text: String::new(),
        }),
        hints: Hints {
            name: Some(address.to_string()),
            id: Some(id),
            // Topics sit at the top, like the ones `@emits` creates; not in the directory's node.
            part_of: vec![system.to_string()],
            ..Hints::default()
        },
        origin: Origin::Discovered,
    }
}

/// `POST /api/orders (CreateOrder) -> Order`: the tag syntax of a call, so `split_payload` reads it.
fn described(d: &Declare, route: &str) -> String {
    let mut v = route.to_string();
    if let Some(p) = &d.payload {
        v.push_str(&format!(" ({p})"));
    }
    if let Some(r) = &d.reply {
        v.push_str(&format!(" -> {r}"));
    }
    v
}

/***
 * Put each description into the service it describes, merge its endpoints into the code that serves them, and say where they disagree.
 *
 * A description goes where `[[import.api]] part_of` or `x-docair-part-of`
 * says, else into the closest service or job declared in its directory or
 * above. An endpoint merges into a handler inside that service whose `@http`
 * is the same route (parameter names aside), with or without the
 * description's base path; else into a function whose name is the
 * operation's `operationId`, and then W103 says that the routes differ. The
 * handler gets the operation as a doc and the `described` tag.
 *
 * W114: a route the description declares that no code in its service
 * serves. W115: a route the code serves that its description leaves out
 * (unless `[[import.api]] ignore` lists it).
 *
 * @in  nodes: Node[] from docair-core.build.build
 * @out nodes: Node[]
 ***/
pub fn settle(nodes: &mut Vec<Node>, system: &str, config: &Config, diags: &mut Vec<Diagnostic>) {
    let descriptions: Vec<usize> = (0..nodes.len())
        .filter(|&i| nodes[i].kind == "api")
        .collect();
    for &i in &descriptions {
        let file = source_file(&nodes[i]);
        let explicit = config.api(&file).and_then(|a| a.part_of.as_ref()).is_some()
            || nodes[i].part_of.as_deref().is_some_and(|p| p != system);
        if !explicit && let Some(owner) = owner_of(nodes, &file) {
            nodes[i].part_of = Some(owner);
        }
    }
    let mut gone: HashSet<String> = HashSet::new();
    let mut described: HashMap<String, Described> = HashMap::new();
    for &i in &descriptions {
        let desc = nodes[i].id.clone();
        let file = source_file(&nodes[i]);
        let endpoints: Vec<usize> = (0..nodes.len())
            .filter(|&e| {
                nodes[e].part_of.as_deref() == Some(desc.as_str())
                    && is_endpoint_of_a_description(&nodes[e])
            })
            .collect();
        if endpoints.is_empty() {
            continue;
        }
        let service = nodes[i].part_of.clone().filter(|p| p != system);
        let external = service
            .as_deref()
            .and_then(|s| nodes.iter().find(|n| n.id == s))
            .is_some_and(|n| n.kind == "external");
        // What is inside the service, besides what descriptions declare.
        let members: Vec<usize> = match &service {
            Some(s) => {
                let inside = within(nodes, s);
                (0..nodes.len())
                    .filter(|&m| {
                        inside.contains(nodes[m].id.as_str())
                            && nodes[m].kind != "api"
                            && !is_endpoint_of_a_description(&nodes[m])
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        let handlers: Vec<usize> = members
            .iter()
            .copied()
            .filter(|&m| nodes[m].http.is_some())
            .collect();
        let mut left: Vec<usize> = Vec::new();
        for &e in &endpoints {
            let by_route = handlers
                .iter()
                .copied()
                .find(|&h| same_route(&nodes[h], &nodes[e]));
            let by_name = || {
                let name = norm(&tag_value(&nodes[e], OPERATION)?);
                members
                    .iter()
                    .copied()
                    .find(|&m| norm(&nodes[m].name) == name)
            };
            let (h, renamed) = match by_route
                .map(|h| (h, false))
                .or_else(|| by_name().map(|m| (m, true)))
            {
                Some(found) => found,
                None => {
                    left.push(e);
                    continue;
                }
            };
            let (doc, mut tags) = carried(&nodes[e]);
            // The tags now sit on the handler: their lines are the handler's, their
            // reasons still say where in the description they come from.
            let own_line = nodes[h].source.as_ref().map_or(1, |s| s.line);
            for t in &mut tags {
                t.line = own_line;
            }
            if renamed {
                // The description has another route for this function: the block's
                // @http stays, and W103 (check) shows both, at the block's @http line.
                if let Some(t) = nodes[e].tags.iter().find(|t| t.name == "http") {
                    let op = tag_value(&nodes[e], OPERATION).unwrap_or_default();
                    let why = format!("{} ({op})", location(t));
                    let line = nodes[h]
                        .tags
                        .iter()
                        .find(|t| t.name == "http" && t.origin == Origin::Authored)
                        .map_or(own_line, |t| t.line);
                    tags.push(Tag::inferred("http", &t.value, line, &why));
                }
                if nodes[h].http.is_none() {
                    nodes[h].http = nodes[e].http.clone();
                }
            }
            nodes[h].docs.push(doc);
            nodes[h].tags.extend(tags);
            gone.insert(nodes[e].id.clone());
            if let Some(s) = &service {
                described
                    .entry(s.clone())
                    .or_default()
                    .matched
                    .insert(nodes[h].id.clone());
            }
        }
        if !handlers.is_empty() && !external {
            let s = service.clone().unwrap_or_default();
            for &e in &left {
                let n = &nodes[e];
                let route = tag_value(n, "http").unwrap_or_default();
                let op = tag_value(n, OPERATION)
                    .map(|o| format!(" ({o})"))
                    .unwrap_or_default();
                diags.push(
                    Diagnostic::new(
                        "W114",
                        format!("`{route}`{op} is in {file}, but no code in `{s}` serves it"),
                    )
                    .at(&file, n.source.as_ref().map_or(1, |r| r.line))
                    .node(&n.id),
                );
            }
            let d = described.entry(s).or_default();
            d.files.push(file.clone());
            d.ignore.extend(
                config
                    .api(&file)
                    .map(|a| a.ignore.clone())
                    .unwrap_or_default(),
            );
        } else if !external && let Some(hint) = served_elsewhere(nodes, &left, service.as_deref()) {
            let line = nodes[i].source.as_ref().map_or(1, |r| r.line);
            diags.push(
                Diagnostic::new(
                    "W114",
                    format!(
                        "{file} declares routes that no code in `{}` serves, but {hint}: set `part_of` in [[import.api]]",
                        service.as_deref().unwrap_or(system)
                    ),
                )
                .at(&file, line)
                .node(&desc),
            );
        }
        // Everything it declares is in the code: the handlers carry it now. A
        // description someone documented with a block stays.
        if left.is_empty()
            && nodes[i].origin == Origin::Discovered
            && !nodes
                .iter()
                .any(|n| n.part_of.as_deref() == Some(desc.as_str()) && !gone.contains(&n.id))
        {
            gone.insert(desc);
        }
    }
    // AsyncAPI: the description's lines to its topics, against the code of its service.
    let mut events: HashMap<String, Described> = HashMap::new();
    for &i in &descriptions {
        let declared: Vec<(String, String, usize)> = nodes[i]
            .tags
            .iter()
            .filter(|t| t.origin == Origin::Inferred && is_event(t))
            .map(|t| (t.name.clone(), topic_of(&t.value), t.line))
            .collect();
        if declared.is_empty() {
            continue;
        }
        let desc = nodes[i].id.clone();
        let file = source_file(&nodes[i]);
        let Some(service) = nodes[i].part_of.clone().filter(|p| p != system) else {
            continue;
        };
        let external = nodes
            .iter()
            .find(|n| n.id == service)
            .is_some_and(|n| n.kind == "external");
        let code: HashSet<(String, String)> = {
            let inside = within(nodes, &service);
            nodes
                .iter()
                .filter(|n| {
                    inside.contains(n.id.as_str())
                        && n.kind != "api"
                        && !is_endpoint_of_a_description(n)
                })
                .flat_map(|n| n.tags.iter().filter(|t| is_event(t)))
                .map(|t| (t.name.clone(), topic_of(&t.value)))
                .collect()
        };
        // A line the code already draws is the code's: the description's yields.
        nodes[i].tags.retain(|t| {
            !(t.origin == Origin::Inferred
                && is_event(t)
                && code.contains(&(t.name.clone(), topic_of(&t.value))))
        });
        if !code.is_empty() && !external {
            for (verb, topic, line) in &declared {
                if code.contains(&(verb.clone(), topic.clone())) {
                    continue;
                }
                let does = if verb == "emits" {
                    "sends to"
                } else {
                    "receives from"
                };
                diags.push(
                    Diagnostic::new(
                        "W114",
                        format!("{file} says `{service}` {does} `{topic}`, but no code in it does"),
                    )
                    .at(&file, *line)
                    .node(&desc),
                );
            }
            let d = events.entry(service.clone()).or_default();
            d.files.push(file.clone());
            d.matched
                .extend(declared.iter().map(|(v, t, _)| format!("{v} {t}")));
        }
        let lines_left = nodes[i]
            .tags
            .iter()
            .any(|t| t.origin == Origin::Inferred && is_event(t));
        if !lines_left
            && nodes[i].origin == Origin::Discovered
            && !nodes
                .iter()
                .any(|n| n.part_of.as_deref() == Some(desc.as_str()) && !gone.contains(&n.id))
        {
            gone.insert(desc);
        }
    }
    for (service, d) in &events {
        let inside = within(nodes, service);
        for n in nodes.iter().filter(|n| {
            inside.contains(n.id.as_str())
                && n.kind != "api"
                && !is_endpoint_of_a_description(n)
                && !gone.contains(&n.id)
        }) {
            for t in n.tags.iter().filter(|t| is_event(t)) {
                let topic = topic_of(&t.value);
                if d.matched.contains(&format!("{} {topic}", t.name)) {
                    continue;
                }
                let does = if t.name == "emits" {
                    "sends to"
                } else {
                    "receives from"
                };
                let file = source_file(n);
                diags.push(
                    Diagnostic::new(
                        "W115",
                        format!(
                            "`{}` {does} `{topic}`, but {} does not declare it",
                            n.id,
                            d.files.join(", ")
                        ),
                    )
                    .at(&file, t.line)
                    .node(&n.id),
                );
            }
        }
    }
    for (service, d) in &described {
        if d.files.is_empty() {
            continue;
        }
        let inside = within(nodes, service);
        for n in nodes.iter().filter(|n| {
            inside.contains(n.id.as_str())
                && n.http.is_some()
                && n.kind != "api"
                && !is_endpoint_of_a_description(n)
                && !gone.contains(&n.id)
        }) {
            let http = n.http.as_ref().expect("filtered");
            if d.matched.contains(&n.id) || listed(&d.ignore, http) {
                continue;
            }
            let (file, line) = n
                .source
                .as_ref()
                .map_or((String::new(), 1), |r| (r.file.clone(), r.line));
            diags.push(
                Diagnostic::new(
                    "W115",
                    format!(
                        "`{} {}` is served here, but {} does not declare it; add it there, or list it in [[import.api]] ignore",
                        http.method,
                        http.path,
                        d.files.join(", ")
                    ),
                )
                .at(&file, line)
                .node(&n.id),
            );
        }
    }
    nodes.retain(|n| !gone.contains(&n.id));
}

/// Per service: its descriptions that declare routes, the code they found, and the routes left out on purpose.
#[derive(Default)]
struct Described {
    files: Vec<String>,
    matched: HashSet<String>,
    ignore: Vec<String>,
}

/// Whether a tag sends to or receives from a topic.
fn is_event(t: &Tag) -> bool {
    t.name == "emits" || t.name == "consumes"
}

/// The topic a tag names, written the way the graph names it: `topic:order.created`.
fn topic_of(value: &str) -> String {
    let target = split_payload(value).target;
    let target = target.trim();
    if target.starts_with("topic:") {
        target.to_string()
    } else {
        format!("topic:{target}")
    }
}

fn source_file(n: &Node) -> String {
    n.source
        .as_ref()
        .map(|s| s.file.clone())
        .unwrap_or_default()
}

fn tag_value(n: &Node, name: &str) -> Option<String> {
    n.tags
        .iter()
        .find(|t| t.name == name)
        .map(|t| t.value.clone())
}

/// `api/openapi.yaml:43` from a reason `api/openapi.yaml:43 · PUT /api/orders/{id} (update_order)`.
fn location(t: &Tag) -> String {
    let why = t.reason.as_deref().unwrap_or("");
    why.split(" · ").next().unwrap_or(why).to_string()
}

/// The description file of a reason written by `blocks`: `api/openapi.yaml`.
fn description_file(t: &Tag) -> Option<String> {
    let loc = location(t);
    loc.rsplit_once(':').map(|(f, _)| f.to_string())
}

/// Names compared without case, `-` and `_`: `updateOrder` is `update_order`.
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether `[[import.api]] ignore` lists a route: `GET /healthz`.
fn listed(ignore: &[String], http: &Http) -> bool {
    ignore.iter().any(|entry| {
        entry.split_once(' ').is_some_and(|(method, path)| {
            same_http(
                http,
                &Http {
                    method: method.trim().to_string(),
                    path: path.trim().to_string(),
                },
            )
        })
    })
}

/// When a description's routes are served in another service: "3 of its routes are served in `billing`".
fn served_elsewhere(nodes: &[Node], endpoints: &[usize], here: Option<&str>) -> Option<String> {
    let index: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let program = |n: &Node| {
        let mut cur = n.part_of.as_deref();
        for _ in 0..64 {
            let p = index.get(cur?)?;
            if matches!(p.kind.as_str(), "service" | "job") {
                return Some(p.id.clone());
            }
            cur = p.part_of.as_deref();
        }
        None
    };
    let mut count: HashMap<String, usize> = HashMap::new();
    for &e in endpoints {
        let served: HashSet<String> = nodes
            .iter()
            .filter(|h| h.http.is_some() && !is_endpoint_of_a_description(h) && h.kind != "api")
            .filter(|h| same_route(h, &nodes[e]))
            .filter_map(program)
            .filter(|p| Some(p.as_str()) != here)
            .collect();
        for p in served {
            *count.entry(p).or_default() += 1;
        }
    }
    let (service, n) = count
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))?;
    Some(if n == 1 {
        format!("1 of them is served in `{service}`")
    } else {
        format!("{n} of them are served in `{service}`")
    })
}

/// Whether a node is an endpoint a description declared (not a handler).
fn is_endpoint_of_a_description(n: &Node) -> bool {
    n.origin == Origin::Discovered
        && n.kind == "endpoint"
        && n.tags.iter().any(|t| t.name == DESCRIBED)
}

/// The closest service or job declared in the file's directory or above it.
fn owner_of(nodes: &[Node], file: &str) -> Option<String> {
    let dir = parent_dir(file);
    nodes
        .iter()
        .filter(|n| matches!(n.kind.as_str(), "service" | "job") && n.origin != Origin::Discovered)
        .filter_map(|n| {
            let own = parent_dir(&n.source.as_ref()?.file);
            let holds = own.is_empty() || dir == own || dir.starts_with(&format!("{own}/"));
            holds.then_some((own.len(), n.id.clone()))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, id)| id)
}

fn parent_dir(file: &str) -> &str {
    file.rsplit_once('/').map_or("", |(d, _)| d)
}

/// The ids of a node and everything inside it.
fn within<'a>(nodes: &'a [Node], top: &str) -> HashSet<&'a str> {
    let mut out: HashSet<&str> = HashSet::new();
    if let Some(n) = nodes.iter().find(|n| n.id == top) {
        out.insert(n.id.as_str());
    }
    loop {
        let before = out.len();
        for n in nodes {
            if n.part_of.as_deref().is_some_and(|p| out.contains(p)) {
                out.insert(n.id.as_str());
            }
        }
        if out.len() == before {
            return out;
        }
    }
}

/// Same method, and the same route up to parameter names; the handler's route may lack the description's base path.
fn same_route(handler: &Node, endpoint: &Node) -> bool {
    match (&handler.http, &endpoint.http) {
        (Some(h), Some(e)) => same_http(h, e),
        _ => false,
    }
}

fn same_http(h: &Http, e: &Http) -> bool {
    if !h.method.eq_ignore_ascii_case(&e.method) {
        return false;
    }
    let (h, e) = (Route::parse(&h.path), Route::parse(&e.path));
    if h.same(&e) {
        return true;
    }
    // `/orders/{id}` served behind a base path `/api` the description puts in front. A handler
    // at `/` is no route behind a base path: it would match every route of the description.
    let extra = e.segments.len().saturating_sub(h.segments.len());
    extra > 0
        && !h.segments.is_empty()
        && Route {
            anchored: true,
            segments: e.segments[extra..].to_vec(),
        }
        .same(&h)
}

/// What an endpoint hands to the handler it merges into: the operation as a doc, and its inferred tags.
fn carried(endpoint: &Node) -> (Doc, Vec<Tag>) {
    let tag = |name: &str| endpoint.tags.iter().find(|t| t.name == name);
    let route = tag("http").map_or(String::new(), |t| t.value.clone());
    let mut md = format!("**{route}**");
    if let Some(op) = tag(OPERATION) {
        md.push_str(&format!(" · `{}`", op.value));
    }
    for text in [&endpoint.summary, &endpoint.description] {
        if !text.is_empty() && *text != route {
            md.push_str(&format!("\n\n{text}"));
        }
    }
    if let Some(d) = tag(DESCRIBED) {
        let v = split_payload(&d.value);
        match (v.sends, v.receives) {
            (Some(s), Some(r)) => md.push_str(&format!("\n\nTakes `{s}`, answers `{r}`.")),
            (Some(s), None) => md.push_str(&format!("\n\nTakes `{s}`.")),
            (None, Some(r)) => md.push_str(&format!("\n\nAnswers `{r}`.")),
            (None, None) => {}
        }
    }
    let doc = Doc {
        file: endpoint
            .source
            .as_ref()
            .map_or(String::new(), |s| s.file.clone()),
        anchor: tag(OPERATION).map(|t| t.value.clone()),
        markdown: md,
    };
    let tags = endpoint
        .tags
        .iter()
        .filter(|t| t.name == DESCRIBED || t.name == OPERATION)
        .cloned()
        .collect();
    (doc, tags)
}

/***
 * Give the calls that reach a described route the words of the description, and drop what nothing uses.
 *
 * A call resolved through `http:` that says nothing about what it sends
 * takes the request and the answer from the callee's `described` tag; the
 * description's words win over a code signature's. Endpoints of an external
 * part that nothing calls are dropped, so a vendored description adds only
 * the operations the code uses; a description node left empty goes too.
 *
 * @in  graph: Node[] + Edge[] from docair-core.resolve.resolve
 * @out graph: Node[] + Edge[]
 ***/
pub fn word_and_prune(nodes: &mut Vec<Node>, edges: &mut [Edge]) {
    let index: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    for e in edges.iter_mut() {
        if matches!(e.kind, EdgeKind::Emit | EdgeKind::Consume) && e.payload.is_none() {
            // What travels to or from a topic: the message its description names.
            let topic = if e.kind == EdgeKind::Emit {
                &e.to
            } else {
                &e.from
            };
            if let Some(tag) = index
                .get(topic.as_str())
                .and_then(|n| n.tags.iter().find(|t| t.name == DESCRIBED))
                && let Some(message) = split_payload(&tag.value).sends
            {
                e.label = e.label.take().or_else(|| Some(message.clone()));
                e.payload = Some(message);
                e.evidence = description_file(tag).map(LineEvidence::api);
            }
            continue;
        }
        if e.kind != EdgeKind::Call || !e.via.as_deref().is_some_and(|v| v.starts_with("http:")) {
            continue;
        }
        if (e.payload.is_some() || e.reply.is_some()) && e.evidence.is_none() {
            continue;
        }
        let Some(tag) = index
            .get(e.to.as_str())
            .and_then(|n| n.tags.iter().find(|t| t.name == DESCRIBED))
        else {
            continue;
        };
        let v = split_payload(&tag.value);
        if v.sends.is_none() && v.receives.is_none() {
            continue;
        }
        let words = match (&v.sends, &v.receives) {
            (Some(p), Some(r)) => format!("({p}) → {r}"),
            (Some(p), None) => format!("({p})"),
            (None, Some(r)) => format!("→ {r}"),
            (None, None) => unreachable!(),
        };
        if e.words_from_code() {
            // The code signature's words make way for the description's.
            e.label = None;
            e.evidence = None;
        }
        e.label = Some(match e.label.take() {
            Some(base) => format!("{base} {words}"),
            None => words,
        });
        e.payload = v.sends;
        e.reply = v.receives;
        e.evidence = description_file(tag).map(LineEvidence::api);
    }

    let touched: HashSet<&str> = edges
        .iter()
        .flat_map(|e| [e.from.as_str(), e.to.as_str()])
        .collect();
    let outside = |n: &Node| {
        let mut cur = n.part_of.as_deref();
        for _ in 0..64 {
            match cur.and_then(|p| index.get(p)) {
                Some(p) if p.kind == "external" => return true,
                Some(p) => cur = p.part_of.as_deref(),
                None => return false,
            }
        }
        false
    };
    let unused: HashSet<String> = nodes
        .iter()
        .filter(|n| {
            is_endpoint_of_a_description(n) && outside(n) && !touched.contains(n.id.as_str())
        })
        .map(|n| n.id.clone())
        .collect();
    // A description whose endpoints were all dropped goes too; one that never had any (AsyncAPI) stays.
    let emptied: HashSet<&str> = nodes
        .iter()
        .filter(|n| unused.contains(&n.id))
        .filter_map(|n| n.part_of.as_deref())
        .collect();
    let holding: HashSet<&str> = nodes
        .iter()
        .filter(|n| !unused.contains(&n.id))
        .filter_map(|n| n.part_of.as_deref())
        .collect();
    let empty: HashSet<String> = emptied
        .iter()
        .filter(|id| !holding.contains(*id) && !touched.contains(*id))
        .filter(|id| {
            index
                .get(*id)
                .is_some_and(|n| n.origin == Origin::Discovered)
        })
        .map(|id| id.to_string())
        .collect();
    nodes.retain(|n| !unused.contains(&n.id) && !empty.contains(&n.id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{Evidence, Symbol};
    use crate::model::Http;

    fn declare(method: &str, path: &str, name: Option<&str>) -> Declare {
        Declare {
            by: "apis:api/openapi.yaml".into(),
            at: Meet::Http(Http {
                method: method.into(),
                path: path.into(),
            }),
            payload: Some("CreateOrder".into()),
            reply: Some("Order".into()),
            name: name.map(String::from),
            role: Role::Serves,
            summary: Some("Place an order.".into()),
            description: None,
            evidence: Evidence::new(10, "openapi.yaml: POST /api/orders (createOrder)"),
        }
    }

    #[test]
    fn each_declared_route_is_an_endpoint_inside_its_description() {
        let facts = Facts {
            symbols: vec![Symbol {
                id: Some("api.openapi".into()),
                ..Symbol::at("apis:api/openapi.yaml", "api/openapi.yaml", 1)
            }],
            declares: vec![
                declare("POST", "/api/orders", Some("createOrder")),
                declare("GET", "/api/orders/{id}", None),
                declare("PUT", "/api/orders/{id}", Some("createOrder")),
            ],
            ..Facts::default()
        };
        let b = blocks(&[facts], "s");
        let ids: Vec<&str> = b.iter().map(|b| b.hints.id.as_deref().unwrap()).collect();
        assert_eq!(
            ids,
            [
                "api.openapi.create-order",
                "api.openapi.get-api-orders-id",
                "api.openapi.create-order-2"
            ]
        );
        assert_eq!(b[0].hints.part_of, vec!["api.openapi".to_string()]);
        assert_eq!(
            b[0].tag(DESCRIBED).unwrap().value,
            "POST /api/orders (CreateOrder) -> Order"
        );
        assert_eq!(b[0].tag("http").unwrap().value, "POST /api/orders");
    }

    /// A node as the graph file writes it.
    fn node(v: serde_json::Value) -> Node {
        let mut base = serde_json::json!({
            "name": "", "summary": "", "description": "", "docs": [], "tags": [], "origin": "authored"
        });
        base.as_object_mut()
            .unwrap()
            .extend(v.as_object().unwrap().clone());
        if base["name"] == "" {
            base["name"] = base["id"].clone();
        }
        serde_json::from_value(base).unwrap()
    }

    fn endpoint(id: &str, method: &str, path: &str, op: Option<&str>) -> Node {
        let why = format!("svc/openapi.yaml:7 · {method} {path}");
        let mut tags = vec![
            serde_json::json!({"name": "http", "value": format!("{method} {path}"), "line": 7, "origin": "inferred", "reason": why}),
            serde_json::json!({"name": "described", "value": format!("{method} {path} (In) -> Out"), "line": 7, "origin": "inferred", "reason": why}),
        ];
        if let Some(op) = op {
            tags.push(serde_json::json!({"name": "operation", "value": op, "line": 7, "origin": "inferred", "reason": why}));
        }
        node(serde_json::json!({
            "id": id, "kind": "endpoint", "part_of": "svc.openapi", "origin": "discovered",
            "source": {"file": "svc/openapi.yaml", "line": 7},
            "http": {"method": method, "path": path}, "tags": tags
        }))
    }

    fn handler(id: &str, name: &str, method: &str, path: &str) -> Node {
        node(serde_json::json!({
            "id": id, "name": name, "kind": "function", "part_of": "svc",
            "source": {"file": "svc/src/lib.rs", "line": 3},
            "http": {"method": method, "path": path},
            "tags": [{"name": "http", "value": format!("{method} {path}"), "line": 5, "origin": "authored"}]
        }))
    }

    fn world() -> Vec<Node> {
        vec![
            node(serde_json::json!({"id": "s", "kind": "system"})),
            node(
                serde_json::json!({"id": "svc", "kind": "service", "part_of": "s", "source": {"file": "svc/Cargo.toml", "line": 1}}),
            ),
            handler("svc.get", "get_thing", "GET", "/a/{x}"),
            handler("svc.put", "do_thing", "PUT", "/cc/{x}"),
            handler("svc.list", "list", "GET", "/a"),
            handler("svc.health", "health", "GET", "/healthz"),
            node(serde_json::json!({
                "id": "svc.openapi", "kind": "api", "part_of": "s", "origin": "discovered",
                "source": {"file": "svc/openapi.yaml", "line": 1}
            })),
            endpoint("svc.openapi.get", "GET", "/a/{id}", Some("getThing")),
            endpoint("svc.openapi.post", "POST", "/b", Some("makeThing")),
            endpoint("svc.openapi.put", "PUT", "/c/{x}", Some("doThing")),
        ]
    }

    fn codes(diags: &[Diagnostic]) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = diags
            .iter()
            .map(|d| (d.code.clone(), d.node.clone().unwrap_or_default()))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn descriptions_settle_into_their_service_and_say_where_they_disagree() {
        let mut nodes = world();
        let config: Config = toml::from_str(
            "[[import.api]]\nfile = \"svc/openapi.yaml\"\nignore = [\"GET /healthz\"]",
        )
        .unwrap();
        let mut diags = Vec::new();
        settle(&mut nodes, "s", &config, &mut diags);
        let get = |id: &str| nodes.iter().find(|n| n.id == id);
        // Placed by directory; the same route merges, parameter names aside.
        assert_eq!(get("svc.openapi").unwrap().part_of.as_deref(), Some("svc"));
        assert!(get("svc.openapi.get").is_none());
        assert_eq!(
            tag_value(get("svc.get").unwrap(), DESCRIBED).as_deref(),
            Some("GET /a/{id} (In) -> Out")
        );
        assert_eq!(get("svc.get").unwrap().docs.len(), 1);
        // By name: the description's route is added for W103 to compare, at the block's @http line.
        let put = get("svc.put").unwrap();
        let routes: Vec<(&str, usize)> = put
            .tags
            .iter()
            .filter(|t| t.name == "http")
            .map(|t| (t.value.as_str(), t.line))
            .collect();
        assert_eq!(routes, [("PUT /cc/{x}", 5), ("PUT /c/{x}", 5)]);
        // Left over: one declared route no code serves, one served route not declared.
        assert!(get("svc.openapi.post").is_some());
        assert_eq!(
            codes(&diags),
            [
                ("W114".to_string(), "svc.openapi.post".to_string()),
                ("W115".to_string(), "svc.list".to_string())
            ]
        );
    }

    #[test]
    fn a_description_whose_routes_are_served_elsewhere_says_so() {
        let mut nodes = world();
        for n in &mut nodes {
            if n.id.starts_with("svc.openapi")
                && let Some(s) = &mut n.source
            {
                s.file = "contracts/openapi.yaml".into();
            }
        }
        let mut diags = Vec::new();
        settle(&mut nodes, "s", &Config::default(), &mut diags);
        let api = nodes.iter().find(|n| n.id == "svc.openapi").unwrap();
        assert_eq!(api.part_of.as_deref(), Some("s"));
        assert_eq!(
            codes(&diags),
            [("W114".to_string(), "svc.openapi".to_string())]
        );
        assert!(
            diags[0].message.contains("1 of them is served in `svc`"),
            "{}",
            diags[0].message
        );
    }

    fn event(name: &str, value: &str, line: usize, origin: &str) -> serde_json::Value {
        serde_json::json!({"name": name, "value": value, "line": line, "origin": origin,
            "reason": format!("svc/asyncapi.yaml:{line} · {name} {value}")})
    }

    fn event_world(declared: Vec<serde_json::Value>) -> Vec<Node> {
        vec![
            node(serde_json::json!({"id": "s", "kind": "system"})),
            node(
                serde_json::json!({"id": "svc", "kind": "service", "part_of": "s", "source": {"file": "svc/Cargo.toml", "line": 1}}),
            ),
            node(
                serde_json::json!({"id": "svc.send", "kind": "function", "part_of": "svc",
                "source": {"file": "svc/src/lib.rs", "line": 3}, "tags": [event("emits", "topic:a", 6, "authored")]}),
            ),
            node(
                serde_json::json!({"id": "svc.audit", "kind": "function", "part_of": "svc",
                "source": {"file": "svc/src/lib.rs", "line": 10}, "tags": [event("emits", "topic:z", 12, "authored")]}),
            ),
            node(
                serde_json::json!({"id": "svc.asyncapi", "kind": "api", "part_of": "s", "origin": "discovered",
                "source": {"file": "svc/asyncapi.yaml", "line": 1}, "tags": declared}),
            ),
        ]
    }

    #[test]
    fn a_description_s_events_yield_to_the_code_and_say_where_they_disagree() {
        let mut nodes = event_world(vec![
            event("emits", "topic:a (A)", 5, "inferred"),
            event("consumes", "topic:b (B)", 9, "inferred"),
        ]);
        let mut diags = Vec::new();
        settle(&mut nodes, "s", &Config::default(), &mut diags);
        let api = nodes.iter().find(|n| n.id == "svc.asyncapi").unwrap();
        assert_eq!(api.part_of.as_deref(), Some("svc"));
        let lines: Vec<&str> = api.tags.iter().map(|t| t.value.as_str()).collect();
        assert_eq!(lines, ["topic:b (B)"], "the line the code draws yields");
        assert_eq!(
            codes(&diags),
            [
                ("W114".to_string(), "svc.asyncapi".to_string()),
                ("W115".to_string(), "svc.audit".to_string())
            ]
        );
        assert_eq!(diags[0].line.max(diags[1].line), Some(12));
    }

    #[test]
    fn a_description_the_code_says_all_of_goes() {
        let mut nodes = event_world(vec![event("emits", "topic:a (A)", 5, "inferred")]);
        let mut diags = Vec::new();
        settle(&mut nodes, "s", &Config::default(), &mut diags);
        assert!(nodes.iter().all(|n| n.id != "svc.asyncapi"));
    }

    #[test]
    fn lines_to_a_topic_carry_its_message() {
        let mut nodes = vec![node(serde_json::json!({
            "id": "topic:a", "kind": "topic",
            "tags": [{"name": "described", "value": "topic:a (A)", "line": 5, "origin": "inferred",
                      "reason": "svc/asyncapi.yaml:5 · sends to a"}]
        }))];
        let mut edges: Vec<Edge> = vec![
            serde_json::from_value(serde_json::json!({
                "from": "svc.send", "to": "topic:a", "kind": "emit", "tag": "@emits topic:a",
                "source": {"file": "svc/src/lib.rs", "line": 6}, "origin": "authored"
            }))
            .unwrap(),
        ];
        word_and_prune(&mut nodes, &mut edges);
        assert_eq!(edges[0].payload.as_deref(), Some("A"));
        assert_eq!(edges[0].label.as_deref(), Some("A"));
        assert_eq!(edges[0].words_from_api(), Some("svc/asyncapi.yaml"));
    }

    #[test]
    fn a_documented_description_stays_when_everything_in_it_merged() {
        let mut nodes = world();
        nodes.retain(|n| n.id != "svc.openapi.post" && n.id != "svc.openapi.put");
        for n in &mut nodes {
            if n.id == "svc.openapi" {
                n.origin = Origin::Authored;
            }
        }
        let mut diags = Vec::new();
        settle(&mut nodes, "s", &Config::default(), &mut diags);
        assert!(nodes.iter().any(|n| n.id == "svc.openapi"));
        assert!(nodes.iter().all(|n| n.id != "svc.openapi.get"));
    }

    #[test]
    fn ignore_lists_routes_by_method_and_path() {
        let h = |m: &str, p: &str| Http {
            method: m.into(),
            path: p.into(),
        };
        let ignore = vec!["GET /healthz".to_string(), "post /api/x/{id}".to_string()];
        assert!(listed(&ignore, &h("GET", "/healthz")));
        assert!(listed(&ignore, &h("POST", "/api/x/:key")));
        assert!(!listed(&ignore, &h("PUT", "/healthz")));
        assert_eq!(norm("updateOrder"), norm("update_order"));
    }

    #[test]
    fn routes_match_up_to_parameter_names_and_base_paths() {
        let node = |path: &str| -> Node {
            serde_json::from_value(serde_json::json!({
                "id": "x", "kind": "function", "name": "x", "summary": "", "description": "",
                "http": {"method": "GET", "path": path},
                "docs": [], "tags": [], "origin": "authored"
            }))
            .unwrap()
        };
        assert!(same_route(
            &node("/api/orders/{orderId}"),
            &node("/api/orders/{id}")
        ));
        assert!(same_route(&node("/orders/:id"), &node("/api/orders/{id}")));
        assert!(!same_route(
            &node("/api/order/{id}"),
            &node("/api/orders/{id}")
        ));
        assert!(!same_route(&node("/api/orders"), &node("/api/orders/{id}")));
    }

    #[test]
    fn a_handler_at_the_root_takes_no_other_route() {
        let http = |method: &str, path: &str| Http {
            method: method.into(),
            path: path.into(),
        };
        assert!(same_http(&http("GET", "/"), &http("GET", "/")));
        assert!(!same_http(&http("GET", "/"), &http("GET", "/api/version")));
        assert!(same_http(
            &http("GET", "/orders/{id}"),
            &http("GET", "/api/orders/{id}")
        ));
    }
}

//! Stage 4: blocks become nodes (SPEC §3, §5).

use std::collections::{HashMap, HashSet};

use crate::ids::{is_valid_id, slug};
use crate::model::{Diagnostic, Doc, Hints, Http, MdDoc, Node, Origin, RawBlock, SourceRef, Tag};

/// Per-node facts that build needs but the graph does not keep.
struct Pending {
    hints: Hints,
    file: String,
    code_line: Option<usize>,
}

/***
 * Turn blocks into nodes: ids, kinds, parents, and attached Markdown.
 *
 * Ids come from `@id`, else the adapter's hint, else `<file-stem>.<symbol>`.
 * Parents come from `@part-of`, else the enclosing item, the adapter's module
 * path, the nearest directory named like a node, and finally the system.
 *
 * @flow scan 9 "blocks become nodes"
 * @in   blocks: RawBlock[] from docair-core.merge.merge
 * @in   docs: MdDoc[] from docair-core.markdown.read_markdown
 * @out  nodes: Node[]
 ***/
pub fn build(
    system: &str,
    blocks: Vec<RawBlock>,
    docs: &[MdDoc],
    files: &[String],
    diags: &mut Vec<Diagnostic>,
) -> Vec<Node> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut pending: Vec<Pending> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    let md_blocks = docs.iter().flat_map(|d| d.blocks.iter().cloned());
    for b in blocks.into_iter().chain(md_blocks) {
        let Some(id) = node_id(&b, diags) else {
            continue;
        };
        if let Some(&first) = index.get(&id) {
            // A node found in the code that is already declared: the declaration wins.
            if b.origin == Origin::Discovered {
                continue;
            }
            let at = nodes[first]
                .source
                .as_ref()
                .map(|s| format!("{}:{}", s.file, s.line))
                .unwrap_or_default();
            diags.push(
                Diagnostic::new(
                    "E002",
                    format!("duplicate id `{id}` (first defined at {at})"),
                )
                .at(&b.file, b.line)
                .node(&id),
            );
            continue;
        }
        let http = b.tag("http").and_then(|t| match parse_http(&t.value) {
            Some(h) => Some(h),
            None => {
                diags.push(
                    Diagnostic::new("E006", format!("malformed @http `{}`", t.value))
                        .at(&b.file, t.line)
                        .node(&id),
                );
                None
            }
        });
        index.insert(id.clone(), nodes.len());
        nodes.push(Node {
            name: b
                .hints
                .name
                .clone()
                .unwrap_or_else(|| id.rsplit('.').next().unwrap_or(&id).to_string()),
            kind: b
                .tag("kind")
                .map_or_else(|| "unknown".to_string(), |t| t.value.clone()),
            summary: b.summary.clone(),
            description: b.description.clone(),
            part_of: None,
            // The part runs to its code's end, or its block's when it has no code (docair-zrr.2).
            source: Some(SourceRef {
                file: b.file.clone(),
                line: b.line,
                end: b.hints.end.or((b.end_line >= b.line).then_some(b.end_line)),
            }),
            symbol: b.hints.symbol.clone(),
            http,
            deployed_as: b.tag("deployed-as").map(|t| t.value.clone()),
            docs: Vec::new(),
            tags: b.tags.clone(),
            origin: b.origin,
            test: None,
            signature: b.hints.signature.clone(),
            runs_in: None,
            code_from: None,
            id,
        });
        pending.push(Pending {
            hints: b.hints,
            file: b.file,
            code_line: b.following.map(|f| f.line),
        });
    }

    if !index.contains_key(system) {
        index.insert(system.to_string(), nodes.len());
        nodes.push(implicit_node(system, "system", None));
        pending.push(Pending {
            hints: Hints::default(),
            file: String::new(),
            code_line: None,
        });
    }

    // README.md files document their directory as a group node; create the
    // group first so that other nodes can be placed inside it.
    for doc in docs.iter().filter(|d| is_readme(&d.file)) {
        let dir = parent_dir(&doc.file);
        let id = if dir.is_empty() {
            system.to_string()
        } else {
            slug(last_segment(dir))
        };
        if !index.contains_key(&id) && doc.blocks.is_empty() && !doc.rest.is_empty() {
            index.insert(id.clone(), nodes.len());
            let mut group = implicit_node(
                &id,
                "module",
                Some(SourceRef {
                    file: doc.file.clone(),
                    line: 1,
                    end: None,
                }),
            );
            group.origin = Origin::Authored;
            group.summary = doc
                .rest
                .lines()
                .next()
                .unwrap_or("")
                .trim_start_matches('#')
                .trim()
                .to_string();
            nodes.push(group);
            pending.push(Pending {
                hints: Hints::default(),
                file: doc.file.clone(),
                code_line: None,
            });
        }
    }

    assign_parents(system, &mut nodes, &pending, &index, diags);
    attach_docs(system, &mut nodes, &pending, &index, docs, files, diags);
    drop_empty_modules(&mut nodes);
    nodes
}

/// A module found in the code that nothing ended up in says nothing: leave it out
/// (a TypeScript file whose declarations are all documented elsewhere).
fn drop_empty_modules(nodes: &mut Vec<Node>) {
    loop {
        let parents: HashSet<String> = nodes.iter().filter_map(|n| n.part_of.clone()).collect();
        let before = nodes.len();
        nodes.retain(|n| {
            !(n.origin == Origin::Discovered
                && n.kind == "module"
                && n.docs.is_empty()
                && !parents.contains(&n.id))
        });
        if nodes.len() == before {
            break;
        }
    }
}

fn node_id(b: &RawBlock, diags: &mut Vec<Diagnostic>) -> Option<String> {
    if let Some(t) = b.tags.iter().find(|t| t.name == "id") {
        if is_valid_id(&t.value) {
            return Some(t.value.clone());
        }
        diags.push(
            Diagnostic::new(
                "E004",
                format!(
                    "malformed id `{}`: use lowercase [a-z0-9_-] segments joined with `.`",
                    t.value
                ),
            )
            .at(&b.file, t.line),
        );
        return None;
    }
    if let Some(id) = &b.hints.id {
        return Some(id.clone());
    }
    match (&b.following, &b.hints.name) {
        (Some(_), Some(name)) => Some(format!("{}.{}", slug(file_stem(&b.file)), slug(name))),
        (Some(f), None) => {
            diags.push(
                Diagnostic::new(
                    "E004",
                    format!("cannot derive an id from `{}`: add @id", f.text),
                )
                .at(&b.file, b.line),
            );
            None
        }
        (None, _) => {
            diags.push(Diagnostic::new("E004", "a free block needs an @id").at(&b.file, b.line));
            None
        }
    }
}

fn assign_parents(
    system: &str,
    nodes: &mut [Node],
    pending: &[Pending],
    index: &HashMap<String, usize>,
    diags: &mut Vec<Diagnostic>,
) {
    let by_code_line: HashMap<(&str, usize), usize> = pending
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.code_line.map(|l| ((p.file.as_str(), l), i)))
        .collect();

    for i in 0..nodes.len() {
        if nodes[i].id == system {
            continue;
        }
        let own = nodes[i].id.clone();
        let authored = nodes[i]
            .tags
            .iter()
            .find(|t| t.name == "part-of" && t.origin == Origin::Authored)
            .cloned();
        let parent = match authored {
            Some(t) if index.contains_key(&t.value) && t.value != own => t.value,
            Some(t) => {
                let file = nodes[i]
                    .source
                    .as_ref()
                    .map_or("", |s| s.file.as_str())
                    .to_string();
                diags.push(
                    Diagnostic::new("E001", format!("unresolved ref `{}` in @part-of", t.value))
                        .at(&file, t.line)
                        .node(&own),
                );
                system.to_string()
            }
            None => {
                let p = &pending[i];
                let line = p.code_line.unwrap_or(0);
                let exists = |c: &String| index.contains_key(c) && *c != own;
                let from_item = p
                    .hints
                    .parent_line
                    .and_then(|l| by_code_line.get(&(p.file.as_str(), l)))
                    .map(|&j| (nodes[j].id.clone(), "enclosing item"));
                // A function is no box for other parts unless a block says so: a method of
                // `Config` skips a function a block named `….config`, and goes to the module.
                let is_function =
                    |c: &String| index.get(c).is_some_and(|&j| nodes[j].kind == "function");
                let from_hints = || {
                    p.hints
                        .part_of
                        .iter()
                        .find(|c| exists(c) && !is_function(c))
                        .map(|c| (c.clone(), "module path"))
                };
                let from_dirs = || {
                    dir_candidates(&p.file)
                        .into_iter()
                        .find(exists)
                        .map(|c| (c, "directory name"))
                };
                match from_item.or_else(from_hints).or_else(from_dirs) {
                    Some((parent, why)) => {
                        nodes[i]
                            .tags
                            .push(Tag::inferred("part-of", &parent, line, why));
                        parent
                    }
                    None => system.to_string(),
                }
            }
        };
        // `@runs-in`: the code comes from `parent`, but it runs, and is drawn, there (SPEC §3.1).
        let runs_in = nodes[i]
            .tags
            .iter()
            .find(|t| t.name == "runs-in" && t.origin == Origin::Authored)
            .cloned();
        let parent = match runs_in {
            Some(t) if index.contains_key(&t.value) && t.value != own => {
                nodes[i].code_from = Some(parent);
                nodes[i].runs_in = Some(t.value.clone());
                t.value
            }
            Some(t) => {
                let file = nodes[i]
                    .source
                    .as_ref()
                    .map_or("", |s| s.file.as_str())
                    .to_string();
                diags.push(
                    Diagnostic::new("E001", format!("unresolved ref `{}` in @runs-in", t.value))
                        .at(&file, t.line)
                        .node(&own),
                );
                parent
            }
            None => parent,
        };
        nodes[i].part_of = Some(parent);
    }

    // E003: break @part-of cycles at the node where the walk returns.
    for i in 0..nodes.len() {
        let mut seen = HashSet::new();
        let mut cur = nodes[i].part_of.clone();
        seen.insert(nodes[i].id.clone());
        while let Some(p) = cur {
            if !seen.insert(p.clone()) {
                let id = nodes[i].id.clone();
                let file = nodes[i]
                    .source
                    .as_ref()
                    .map_or("", |s| s.file.as_str())
                    .to_string();
                let line = nodes[i].source.as_ref().map_or(0, |s| s.line);
                diags.push(
                    Diagnostic::new("E003", format!("@part-of cycle through `{id}`"))
                        .at(&file, line)
                        .node(&id),
                );
                nodes[i].part_of = Some(system.to_string());
                break;
            }
            cur = index.get(&p).and_then(|&j| nodes[j].part_of.clone());
        }
    }
}

fn attach_docs(
    system: &str,
    nodes: &mut [Node],
    pending: &[Pending],
    index: &HashMap<String, usize>,
    docs: &[MdDoc],
    files: &[String],
    diags: &mut Vec<Diagnostic>,
) {
    for doc in docs {
        for (anchor, markdown, line) in &doc.sections {
            match index.get(anchor) {
                Some(&j) => nodes[j].docs.push(Doc {
                    file: doc.file.clone(),
                    anchor: Some(anchor.clone()),
                    markdown: markdown.clone(),
                }),
                None => diags.push(
                    Diagnostic::new(
                        "W105",
                        format!("Markdown anchor `{{#{anchor}}}` points to no node"),
                    )
                    .at(&doc.file, *line),
                ),
            }
        }
        if doc.rest.is_empty() {
            continue;
        }
        let target = if is_readme(&doc.file) {
            let dir = parent_dir(&doc.file);
            let id = if dir.is_empty() {
                system.to_string()
            } else {
                slug(last_segment(dir))
            };
            index
                .get(&id)
                .copied()
                .or_else(|| first_declared(doc, index))
        } else {
            match sibling_code_file(&doc.file, files) {
                Some(code) => {
                    let t = file_node(&code, nodes, pending);
                    if t.is_none() {
                        diags.push(
                            Diagnostic::new(
                                "W105",
                                format!("no node in `{code}` to attach this Markdown to"),
                            )
                            .at(&doc.file, 1),
                        );
                    }
                    t
                }
                None => None,
            }
        };
        if let Some(j) = target {
            nodes[j].docs.push(Doc {
                file: doc.file.clone(),
                anchor: None,
                markdown: doc.rest.clone(),
            });
        }
    }

    // Explicit `@doc path.md[#anchor]` links.
    for node in nodes.iter_mut() {
        let file = node
            .source
            .as_ref()
            .map_or(String::new(), |s| s.file.clone());
        let links: Vec<Tag> = node.tags_named("doc").cloned().collect();
        for t in links {
            let (path, anchor) = match t.value.split_once('#') {
                Some((p, a)) => (p, Some(a)),
                None => (t.value.as_str(), None),
            };
            let target = normalize(&format!("{}/{}", parent_dir(&file), path));
            let found = docs
                .iter()
                .find(|d| d.file == target)
                .and_then(|d| match anchor {
                    Some(a) => d
                        .sections
                        .iter()
                        .find(|(id, _, _)| id == a)
                        .map(|(_, md, _)| md.clone()),
                    None => Some(d.rest.clone()),
                });
            let anchor = anchor.map(str::to_string);
            let already = node
                .docs
                .iter()
                .any(|d| d.file == target && d.anchor == anchor);
            match found {
                Some(_) if already => {}
                Some(markdown) => node.docs.push(Doc {
                    file: target,
                    anchor,
                    markdown,
                }),
                None => diags.push(
                    Diagnostic::new("E001", format!("unresolved ref `{}` in @doc", t.value))
                        .at(&file, t.line)
                        .node(&node.id),
                ),
            }
        }
    }
}

/// The node a whole code file stands for: the item that declares the file
/// (Rust `mod x;`), else the first node in the file that is not inside another one.
fn file_node(code: &str, nodes: &[Node], pending: &[Pending]) -> Option<usize> {
    if let Some(i) = pending
        .iter()
        .position(|p| p.hints.declares_file.as_deref() == Some(code))
    {
        return Some(i);
    }
    let in_file: HashSet<&str> = nodes
        .iter()
        .zip(pending)
        .filter(|(_, p)| p.file == code)
        .map(|(n, _)| n.id.as_str())
        .collect();
    nodes.iter().zip(pending).position(|(n, p)| {
        p.file == code
            && n.part_of
                .as_deref()
                .is_none_or(|parent| !in_file.contains(parent))
    })
}

fn first_declared(doc: &MdDoc, index: &HashMap<String, usize>) -> Option<usize> {
    doc.blocks.iter().find_map(|b| {
        b.tags
            .iter()
            .find(|t| t.name == "id")
            .and_then(|t| index.get(&t.value).copied())
    })
}

fn implicit_node(id: &str, kind: &str, source: Option<SourceRef>) -> Node {
    Node {
        id: id.to_string(),
        kind: kind.to_string(),
        name: id.to_string(),
        summary: String::new(),
        description: String::new(),
        part_of: None,
        source,
        symbol: None,
        http: None,
        deployed_as: None,
        docs: Vec::new(),
        tags: Vec::new(),
        origin: Origin::Inferred,
        test: None,
        signature: None,
        runs_in: None,
        code_from: None,
    }
}

pub fn parse_http(value: &str) -> Option<Http> {
    let mut parts = value.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?;
    let ok = method.chars().all(|c| c.is_ascii_alphabetic())
        && path.starts_with('/')
        && parts.next().is_none();
    ok.then(|| Http {
        method: method.to_ascii_uppercase(),
        path: path.to_string(),
    })
}

fn is_readme(file: &str) -> bool {
    last_segment(file).eq_ignore_ascii_case("readme.md")
}

fn sibling_code_file(md: &str, files: &[String]) -> Option<String> {
    let stem = md.strip_suffix(".md")?;
    files
        .iter()
        .find(|f| !f.ends_with(".md") && f.rsplit_once('.').is_some_and(|(s, _)| s == stem))
        .cloned()
}

fn file_stem(file: &str) -> &str {
    let name = last_segment(file);
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

fn last_segment(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// Directory names from the innermost outwards, as id segments.
fn dir_candidates(file: &str) -> Vec<String> {
    let dir = parent_dir(file);
    if dir.is_empty() {
        return Vec::new();
    }
    dir.rsplit('/')
        .map(slug)
        .filter(|s| !s.is_empty())
        .collect()
}

fn normalize(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Following;

    fn block(file: &str, line: usize, tags: &[(&str, &str)], name: Option<&str>) -> RawBlock {
        RawBlock {
            file: file.into(),
            line,
            end_line: line + 2,
            summary: format!("block at {line}"),
            description: String::new(),
            tags: tags
                .iter()
                .map(|(n, v)| Tag::authored(n, v, line))
                .collect(),
            following: Some(Following {
                line: line + 3,
                text: "fn x() {}".into(),
            }),
            hints: Hints {
                name: name.map(str::to_string),
                ..Hints::default()
            },
            origin: Origin::Authored,
        }
    }

    #[test]
    fn ids_parents_and_diagnostics() {
        let blocks = vec![
            block(
                "apps/orders-api/orders.rs",
                1,
                &[("id", "orders-api"), ("kind", "service")],
                None,
            ),
            block(
                "apps/orders-api/orders.rs",
                10,
                &[("kind", "function")],
                Some("create_order"),
            ),
            block(
                "apps/orders-api/orders.rs",
                20,
                &[("id", "orders-api")],
                None,
            ),
            block("x.rs", 30, &[("id", "Bad.Id")], None),
            block("x.rs", 40, &[("part-of", "nowhere")], Some("y")),
        ];
        let mut d = Vec::new();
        let nodes = build("shop", blocks, &[], &[], &mut d);
        let find = |id: &str| nodes.iter().find(|n| n.id == id).unwrap();
        assert_eq!(
            find("orders.create_order").part_of.as_deref(),
            Some("orders-api"),
            "directory rule"
        );
        assert_eq!(find("x.y").part_of.as_deref(), Some("shop"));
        assert_eq!(find("shop").kind, "system");
        let codes: Vec<&str> = d.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, vec!["E002", "E004", "E001"]);
    }

    #[test]
    fn runs_in_draws_the_code_where_it_runs() {
        let blocks = vec![
            block("a.md", 1, &[("id", "browser")], None),
            block("a.md", 5, &[("id", "container")], None),
            block(
                "a.md",
                9,
                &[
                    ("id", "app"),
                    ("part-of", "container"),
                    ("runs-in", "browser"),
                ],
                None,
            ),
            block("a.md", 13, &[("id", "app.panel"), ("part-of", "app")], None),
            block("a.md", 17, &[("id", "lost"), ("runs-in", "nowhere")], None),
        ];
        let mut d = vec![];
        let nodes = build("shop", blocks, &[], &[], &mut d);
        let find = |id: &str| nodes.iter().find(|n| n.id == id).unwrap();
        assert_eq!(find("app").part_of.as_deref(), Some("browser"));
        assert_eq!(find("app").code_from.as_deref(), Some("container"));
        assert_eq!(find("app").runs_in.as_deref(), Some("browser"));
        assert_eq!(
            find("app.panel").part_of.as_deref(),
            Some("app"),
            "its parts go along"
        );
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("@runs-in"), "{:?}", d);
    }

    #[test]
    fn markdown_sections_and_readme() {
        let blocks = vec![block("src/orders.rs", 1, &[("id", "orders.create")], None)];
        let docs = vec![
            MdDoc {
                file: "src/orders.md".into(),
                rest: "About orders.".into(),
                sections: vec![("orders.create".into(), "## Create".into(), 3)],
                blocks: vec![],
            },
            MdDoc {
                file: "src/README.md".into(),
                rest: "# Sources\nAll code.".into(),
                sections: vec![],
                blocks: vec![],
            },
        ];
        let files = vec!["src/orders.rs".to_string(), "src/orders.md".to_string()];
        let mut d = Vec::new();
        let nodes = build("shop", blocks, &docs, &files, &mut d);
        let create = nodes.iter().find(|n| n.id == "orders.create").unwrap();
        assert_eq!(create.docs.len(), 2, "anchored section + sibling file");
        let group = nodes.iter().find(|n| n.id == "src").unwrap();
        assert_eq!(group.summary, "Sources");
        assert_eq!(
            create.part_of.as_deref(),
            Some("src"),
            "README group contains the code"
        );
        assert!(d.is_empty(), "{d:?}");
    }
}

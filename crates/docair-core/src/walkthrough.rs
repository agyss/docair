//! Recorded walkthroughs: read, bound to the graph, written (SPEC §10.3).
//!
//! A walkthrough file names nodes by id and edges by their ends, never by
//! index, so it survives a rescan. Binding checks every ref against the
//! current graph and turns edge refs into edge indices (`path`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ids::slug;
use crate::model::{Diagnostic, Edge, Node, WALKTHROUGH_SCHEMA, Walkthrough};

/// One ref that did not bind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    UnknownNode {
        step: usize,
        id: String,
    },
    UnknownEdge {
        step: usize,
        from: String,
        to: String,
    },
    EmptyStep {
        step: usize,
        title: String,
    },
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::UnknownNode { step, id } => {
                write!(f, "step {}: node `{id}` is not in the graph", step + 1)
            }
            Problem::UnknownEdge { step, from, to } => write!(
                f,
                "step {}: there is no edge between `{from}` and `{to}`",
                step + 1
            ),
            Problem::EmptyStep { step, title } => {
                write!(
                    f,
                    "step {} (\"{title}\") names nothing in the graph",
                    step + 1
                )
            }
        }
    }
}

/// The id of a walkthrough: the slug of its question, at most 60 characters.
pub fn id_for(question: &str) -> String {
    let mut id = slug(&question.to_lowercase());
    if id.len() > 60 {
        let cut = id[..60].rfind('-').unwrap_or(60);
        id.truncate(cut);
    }
    if id.is_empty() {
        "walkthrough".to_string()
    } else {
        id
    }
}

/***
 * Bind a walkthrough to the graph: keep the refs that exist, turn edge refs into edge indices.
 *
 * An edge ref binds to every edge `from → to`, else to every edge `to → from`.
 * A step with nothing left is dropped.
 *
 * @id   docair-core.walkthrough.bind
 * @flow ask 6 "docair checks every id against the graph"
 * @in   walkthrough: Walkthrough
 * @out  walkthrough: bound steps + problems
 ***/
pub fn bind(wt: &mut Walkthrough, nodes: &[Node], edges: &[Edge]) -> Vec<Problem> {
    let ids: HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let mut problems = Vec::new();
    let mut kept = Vec::new();
    for (k, mut step) in std::mem::take(&mut wt.steps).into_iter().enumerate() {
        step.nodes.retain(|id| {
            let ok = ids.contains(id.as_str());
            if !ok {
                problems.push(Problem::UnknownNode {
                    step: k,
                    id: id.clone(),
                });
            }
            ok
        });
        let mut seen = HashSet::new();
        step.nodes.retain(|id| seen.insert(id.clone()));
        step.path.clear();
        step.edges.retain(|r| {
            let forward: Vec<usize> = (0..edges.len())
                .filter(|&i| edges[i].from == r.from && edges[i].to == r.to)
                .collect();
            let found = if forward.is_empty() {
                (0..edges.len())
                    .filter(|&i| edges[i].from == r.to && edges[i].to == r.from)
                    .collect()
            } else {
                forward
            };
            if found.is_empty() {
                problems.push(Problem::UnknownEdge {
                    step: k,
                    from: r.from.clone(),
                    to: r.to.clone(),
                });
                return false;
            }
            for i in found {
                if !step.path.contains(&i) {
                    step.path.push(i);
                }
            }
            true
        });
        if step.nodes.is_empty() && step.path.is_empty() {
            problems.push(Problem::EmptyStep {
                step: k,
                title: step.title.clone(),
            });
            continue;
        }
        kept.push(step);
    }
    wt.steps = kept;
    problems
}

/***
 * Read every recorded walkthrough and bind it to the graph; stale refs become W109, unreadable files W110.
 *
 * @id     docair-core.walkthrough.load
 * @flow   scan 11 "recorded walkthroughs are bound to the graph"
 * @reads  walkthrough-files -> Walkthrough[]
 * @calls  docair-core.walkthrough.bind "checks the refs against the graph"
 ***/
pub fn load(
    root: &Path,
    dir: &str,
    nodes: &[Node],
    edges: &[Edge],
    diags: &mut Vec<Diagnostic>,
) -> Vec<Walkthrough> {
    let folder = root.join(dir);
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let rel = format!(
            "{}/{}",
            dir.trim_end_matches('/'),
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<Walkthrough>(&t).map_err(|e| e.to_string()));
        let mut wt = match parsed {
            Ok(wt) if wt.schema == WALKTHROUGH_SCHEMA => wt,
            Ok(wt) => {
                diags.push(
                    Diagnostic::new(
                        "W110",
                        format!(
                            "not a walkthrough: schema `{}`, expected `{WALKTHROUGH_SCHEMA}`",
                            wt.schema
                        ),
                    )
                    .at(&rel, 1),
                );
                continue;
            }
            Err(e) => {
                diags.push(Diagnostic::new("W110", format!("not a walkthrough: {e}")).at(&rel, 1));
                continue;
            }
        };
        for p in bind(&mut wt, nodes, edges) {
            diags
                .push(Diagnostic::new("W109", format!("walkthrough `{}`, {p}", wt.id)).at(&rel, 1));
        }
        wt.file = Some(rel);
        out.push(wt);
    }
    out
}

/***
 * Record a walkthrough as `<dir>/<id>.json`, with its refs but without the bound indices.
 *
 * @id     docair-core.walkthrough.save
 * @flow   ask 7 "the walkthrough is recorded"
 * @writes walkthrough-files (Walkthrough)
 ***/
pub fn save(root: &Path, dir: &str, wt: &Walkthrough) -> Result<PathBuf> {
    let folder = root.join(dir);
    std::fs::create_dir_all(&folder).with_context(|| format!("creating {}", folder.display()))?;
    let path = folder.join(format!("{}.json", wt.id));
    let mut stored = wt.clone();
    stored.file = None;
    for s in &mut stored.steps {
        s.path.clear();
    }
    let json = serde_json::to_string_pretty(&stored).expect("walkthroughs serialize") + "\n";
    std::fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EdgeKind, EdgeRef, Origin, SourceRef, WalkStep};

    fn node(id: &str) -> Node {
        Node {
            id: id.into(),
            kind: "service".into(),
            name: id.into(),
            summary: String::new(),
            description: String::new(),
            part_of: None,
            source: None,
            symbol: None,
            http: None,
            deployed_as: None,
            docs: vec![],
            tags: vec![],
            origin: Origin::Authored,
            test: None,
            signature: None,
            runs_in: None,
            code_from: None,
        }
    }

    fn edge(from: &str, to: &str) -> Edge {
        Edge {
            from: from.into(),
            to: to.into(),
            kind: EdgeKind::Call,
            label: None,
            via: None,
            payload: None,
            reply: None,
            text: None,
            carries: vec![],
            from_code: false,
            from_api: None,
            tag: "@calls".into(),
            source: SourceRef {
                file: "x".into(),
                line: 1,
            },
            origin: Origin::Authored,
        }
    }

    fn step(title: &str, nodes: &[&str], edges: &[(&str, &str)]) -> WalkStep {
        WalkStep {
            title: title.into(),
            text: String::new(),
            nodes: nodes.iter().map(|s| s.to_string()).collect(),
            edges: edges
                .iter()
                .map(|(f, t)| EdgeRef {
                    from: f.to_string(),
                    to: t.to_string(),
                })
                .collect(),
            path: vec![],
        }
    }

    fn walkthrough(steps: Vec<WalkStep>) -> Walkthrough {
        Walkthrough {
            schema: WALKTHROUGH_SCHEMA.into(),
            id: "q".into(),
            question: "q?".into(),
            title: "Q".into(),
            summary: String::new(),
            recorded: None,
            steps,
            file: None,
        }
    }

    #[test]
    fn binds_edges_by_their_ends_in_either_direction() {
        let nodes = [node("a"), node("b"), node("c")];
        let edges = [edge("a", "b"), edge("c", "b")];
        let mut wt = walkthrough(vec![step("s", &["a", "b"], &[("a", "b"), ("b", "c")])]);
        let problems = bind(&mut wt, &nodes, &edges);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            wt.steps[0].path,
            vec![0, 1],
            "b → c binds to the edge c → b"
        );
    }

    #[test]
    fn unknown_refs_are_left_out_and_empty_steps_dropped() {
        let nodes = [node("a"), node("b")];
        let edges = [edge("a", "b")];
        let mut wt = walkthrough(vec![
            step("one", &["a", "ghost"], &[("a", "ghost")]),
            step("two", &["ghost"], &[]),
        ]);
        let problems = bind(&mut wt, &nodes, &edges);
        assert_eq!(wt.steps.len(), 1);
        assert_eq!(wt.steps[0].nodes, vec!["a"]);
        assert!(wt.steps[0].edges.is_empty());
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert!(matches!(problems[3], Problem::EmptyStep { step: 1, .. }));
    }

    #[test]
    fn saved_files_keep_refs_not_indices_and_load_binds_again() {
        let dir = std::env::temp_dir().join(format!("docair-wt-{}", std::process::id()));
        let nodes = [node("a"), node("b")];
        let edges = [edge("a", "b")];
        let mut wt = walkthrough(vec![step("s", &["a"], &[("a", "b")])]);
        bind(&mut wt, &nodes, &edges);
        let path = save(&dir, "walks", &wt).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("\"path\""), "{text}");
        let mut diags = vec![];
        let loaded = load(
            &dir,
            "walks",
            &nodes,
            &[edge("x", "y"), edge("a", "b")],
            &mut diags,
        );
        assert_eq!(loaded[0].steps[0].path, vec![1], "bound to the new index");
        assert_eq!(loaded[0].file.as_deref(), Some("walks/q.json"));
        std::fs::write(dir.join("walks/bad.json"), "{").unwrap();
        let mut diags = vec![];
        load(&dir, "walks", &[node("a")], &[], &mut diags);
        let codes: Vec<_> = diags.iter().map(|d| d.code.as_str()).collect();
        assert!(
            codes.contains(&"W110") && codes.contains(&"W109"),
            "{codes:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ids_are_short_slugs_of_the_question() {
        assert_eq!(
            id_for("How does Terraform provision the cluster?"),
            "how-does-terraform-provision-the-cluster"
        );
        assert!(id_for(&"why ".repeat(40)).len() <= 60);
        assert_eq!(id_for("???"), "walkthrough");
    }
}

//! What two commits change in the system's flows (docs/CHANGES.md): the changed parts, and the
//! slices through them compared at A and at B by their seeds, as JSON (`docair.changes/0`) and as
//! a short Markdown summary.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::diff::GraphDiff;
use crate::model::{Graph, Reach};
use crate::slice::{SeedKind, Slice};

pub const SCHEMA: &str = "docair.changes/0";

/// The lines of a file that changed between the commits: at A (`old`) and at B (`new`), 1-based.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinesTouched {
    /// The file at A (none for a new file) and at B (none for a removed one).
    pub old_file: Option<String>,
    pub new_file: Option<String>,
    pub old: Vec<usize>,
    pub new: Vec<usize>,
}

/// One flow through a changed part, compared at A and at B.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowChange {
    /// `flow:checkout`, `data.order`, `walkthrough:upload`, a route's part.
    pub seed: String,
    /// `changed`, `new` (only at B) or `gone` (only at A).
    pub how: String,
    pub parts_added: Vec<String>,
    pub parts_removed: Vec<String>,
    pub lines_added: Vec<String>,
    pub lines_removed: Vec<String>,
    /// The changed parts on it.
    pub through: Vec<String>,
    /// False when a line of the change only may reach its part (one of, could): it may be a false alarm.
    pub certain: bool,
    /// The seed it had at A, when its seed was renamed and it was matched by its parts (docair-zrr.32).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renamed_from: Option<String>,
}

/// What two commits change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changes {
    pub schema: String,
    pub a: String,
    pub b: String,
    /// B is the working tree, not a commit: asking about the pair again leaves B out.
    #[serde(default)]
    pub working_tree: bool,
    /// The commit compared from when it is not A itself: where B split from A, the merge base
    /// (docs/EDITOR.md §2, docair-p2c.1), so a branch shows what it adds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub graph: GraphDiff,
    /// The parts the change touches, at B's ids (A's for removed parts).
    pub parts: Vec<String>,
    pub flows: Vec<FlowChange>,
}

/// The seed of a slice as one word: `flow:checkout`, `walkthrough:x`, or the part's id.
fn seed_key(s: &Slice) -> String {
    match s.seed.kind {
        SeedKind::Flow => format!("flow:{}", s.seed.id),
        SeedKind::Walkthrough => format!("walkthrough:{}", s.seed.id),
        _ => s.seed.id.clone(),
    }
}

/// The parts the changed lines fall in: for each line, the smallest part whose source holds it.
fn parts_at(graph: &Graph, file: &str, lines: &[usize]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for &l in lines {
        let mut best: Option<(usize, &str)> = None;
        for n in &graph.nodes {
            let Some(s) = n.source.as_ref().filter(|s| s.file == file) else {
                continue;
            };
            let end = s.end.unwrap_or(s.line);
            if s.line <= l && l <= end && best.is_none_or(|(span, _)| end - s.line < span) {
                best = Some((end - s.line, n.id.as_str()));
            }
        }
        if let Some((_, id)) = best {
            out.insert(id.to_string());
        }
    }
    out
}

/***
 * Compare the flows of two graphs: the changed parts are the graph diff's (added, removed, changed, moved) and the parts the changed lines fall in; every scenario, walkthrough, datum and route slice through one of them is compared at A and at B by its seed, by the parts it holds and its lines' ids.
 *
 * @id    docair-core.changes.changes
 * @calls docair-core.slice.all "works out the slices at A and at B"
 ***/
pub fn changes(
    a: &Graph,
    b: &Graph,
    graph: GraphDiff,
    lines: &[LinesTouched],
    names: (&str, &str),
) -> Changes {
    let moved: HashMap<&str, &str> = graph
        .moved
        .iter()
        .map(|m| (m.from.as_str(), m.to.as_str()))
        .collect();
    let at_b = |id: &str| moved.get(id).map_or(id.to_string(), |s| s.to_string());
    let mut parts: BTreeSet<String> = BTreeSet::new();
    parts.extend(graph.added.iter().cloned());
    parts.extend(graph.removed.iter().cloned());
    parts.extend(graph.changed.iter().map(|c| c.id.clone()));
    parts.extend(graph.moved.iter().map(|m| m.to.clone()));
    for c in lines {
        if let Some(f) = &c.old_file {
            parts.extend(parts_at(a, f, &c.old).iter().map(|id| at_b(id)));
        }
        if let Some(f) = &c.new_file {
            parts.extend(parts_at(b, f, &c.new));
        }
    }
    // Containers (the system, a crate) hold every changed line; a part's own lines say more.
    let system = [a.system.as_str(), b.system.as_str()];
    parts.retain(|p| !system.contains(&p.as_str()));
    let flows_of = |g: &Graph| -> BTreeMap<String, Slice> {
        crate::slice::all(g)
            .into_iter()
            .filter(|s| s.seed.kind != SeedKind::Part)
            .map(|s| (seed_key(&s), s))
            .collect()
    };
    let (fa, fb) = (flows_of(a), flows_of(b));
    // A slice's parts at B's ids, and its lines' ids as they read at B (moved parts renamed).
    let a_ids: HashMap<String, String> = {
        let mut edges = a.edges.clone();
        for e in &mut edges {
            e.from = at_b(&e.from);
            e.to = at_b(&e.to);
        }
        crate::ids::edge_ids(&mut edges);
        a.edges
            .iter()
            .zip(edges)
            .map(|(o, n)| (o.id.clone(), n.id))
            .collect()
    };
    let parts_of = |s: &Slice, from_a: bool| -> BTreeSet<String> {
        s.parts
            .iter()
            .map(|p| if from_a { at_b(p) } else { p.clone() })
            .collect()
    };
    let lines_of = |s: &Slice, g: &Graph, from_a: bool| -> BTreeSet<String> {
        s.lines
            .iter()
            .filter_map(|l| g.edges.get(l.edge))
            .map(|e| {
                if from_a {
                    a_ids.get(&e.id).cloned().unwrap_or_else(|| e.id.clone())
                } else {
                    e.id.clone()
                }
            })
            .collect()
    };
    let uncertain = |s: &Slice, changed: &BTreeSet<String>, g: &Graph| {
        s.lines.iter().any(|l| {
            matches!(l.reach, Some(Reach::OneOf | Reach::Could))
                && g.edges.get(l.edge).is_some_and(|e| changed.contains(&e.id))
        })
    };
    // A flow whose seed was renamed is the same flow (docair-zrr.32): one only A has and one only B has,
    // of the same kind, each the other's only candidate, sharing at least half of each one's parts.
    let only_a: Vec<&String> = fa.keys().filter(|k| !fb.contains_key(*k)).collect();
    let only_b: Vec<&String> = fb.keys().filter(|k| !fa.contains_key(*k)).collect();
    let shares = |ka: &String, kb: &String| {
        let (sa, sb) = (&fa[ka], &fb[kb]);
        if sa.seed.kind != sb.seed.kind {
            return false;
        }
        let (pa, pb) = (parts_of(sa, true), parts_of(sb, false));
        let both = pa.intersection(&pb).count();
        both > 0 && both * 2 >= pa.len() && both * 2 >= pb.len()
    };
    let mut renamed: BTreeMap<&String, &String> = BTreeMap::new();
    for kb in &only_b {
        let from: Vec<&&String> = only_a.iter().filter(|ka| shares(ka, kb)).collect();
        if let [ka] = from.as_slice()
            && only_b.iter().filter(|k| shares(ka, k)).count() == 1
        {
            renamed.insert(kb, ka);
        }
    }
    let was: BTreeSet<&String> = renamed.values().copied().collect();
    let mut flows = Vec::new();
    let seeds: BTreeSet<&String> = fa
        .keys()
        .chain(fb.keys())
        .filter(|k| !was.contains(k))
        .collect();
    for seed in seeds {
        let from = renamed.get(seed).copied();
        let (sa, sb) = (fa.get(from.unwrap_or(seed)), fb.get(seed));
        let pa = sa.map(|s| parts_of(s, true)).unwrap_or_default();
        let pb = sb.map(|s| parts_of(s, false)).unwrap_or_default();
        let through: Vec<String> = parts
            .iter()
            .filter(|p| pa.contains(*p) || pb.contains(*p))
            .cloned()
            .collect();
        if through.is_empty() && sa.is_some() && sb.is_some() && from.is_none() {
            continue;
        }
        let la = sa.map(|s| lines_of(s, a, true)).unwrap_or_default();
        let lb = sb.map(|s| lines_of(s, b, false)).unwrap_or_default();
        let how = match (sa, sb) {
            (Some(_), None) => "gone",
            (None, Some(_)) => "new",
            _ => "changed",
        };
        let lines_added: Vec<String> = lb.difference(&la).cloned().collect();
        let lines_removed: Vec<String> = la.difference(&lb).cloned().collect();
        let parts_added: Vec<String> = pb.difference(&pa).cloned().collect();
        let parts_removed: Vec<String> = pa.difference(&pb).cloned().collect();
        if how == "changed"
            && lines_added.is_empty()
            && lines_removed.is_empty()
            && parts_added.is_empty()
            && parts_removed.is_empty()
            && through.is_empty()
            && from.is_none()
        {
            continue;
        }
        let changed_lines: BTreeSet<String> =
            lines_added.iter().chain(&lines_removed).cloned().collect();
        let certain = !(sb.is_some_and(|s| uncertain(s, &changed_lines, b))
            || sa.is_some_and(|s| uncertain(s, &changed_lines, a)));
        flows.push(FlowChange {
            seed: seed.clone(),
            how: how.to_string(),
            parts_added,
            parts_removed,
            lines_added,
            lines_removed,
            through,
            certain,
            renamed_from: from.cloned(),
        });
    }
    Changes {
        schema: SCHEMA.to_string(),
        a: names.0.to_string(),
        b: names.1.to_string(),
        working_tree: false,
        base: None,
        graph,
        parts: parts.into_iter().collect(),
        flows,
    }
}

/// Where B left A, when the comparison starts there and not at A (docs/EDITOR.md §2), in Markdown:
/// *Compared from where `feature` left `main`: `3f2a9c1d0e4b`.*
pub fn split_from(c: &Changes) -> Option<String> {
    let base = c.base.as_deref()?;
    let b = if c.working_tree {
        "the working tree".to_string()
    } else {
        format!("`{}`", c.b)
    };
    Some(format!(
        "Compared from where {b} left `{}`: `{}`.",
        c.a,
        &base[..base.len().min(12)]
    ))
}

/***
 * The changes as a short Markdown summary: what docair reads first when it changed, one line per changed flow, the moves and the parts that changed.
 *
 * @id docair-core.changes.markdown
 ***/
pub fn markdown(c: &Changes) -> String {
    let mut out = format!("## Changes from `{}` to `{}`\n\n", c.a, c.b);
    if let Some(line) = split_from(c) {
        out.push_str(&format!("{line}\n\n"));
    }
    // Each commit is read with its own docair.toml: what changed in it comes first (docair-zrr.31).
    if !c.graph.toml.is_empty() {
        out.push_str(&format!(
            "- **docair.toml changed**, so some differences may come from how docair read the code:\n\n  ```diff\n{}\n  ```\n",
            c.graph.toml
                .iter()
                .map(|l| format!("  {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    for r in &c.graph.reading {
        out.push_str(&format!("- **The reading changed:** {r}\n"));
    }
    if c.flows.is_empty() {
        out.push_str("No flow changes.\n");
    }
    for f in &c.flows {
        let mut what = Vec::new();
        if !f.parts_added.is_empty() {
            what.push(format!(
                "now through {}",
                f.parts_added
                    .iter()
                    .map(|p| format!("`{p}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !f.parts_removed.is_empty() {
            what.push(format!(
                "no longer through {}",
                f.parts_removed
                    .iter()
                    .map(|p| format!("`{p}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if what.is_empty() && !f.through.is_empty() {
            what.push(format!(
                "{} changed",
                f.through
                    .iter()
                    .map(|p| format!("`{p}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let sure = if f.certain {
            ""
        } else {
            " (rests on a line that may not go there)"
        };
        let how = match f.how.as_str() {
            "new" => "new flow",
            "gone" => "flow gone",
            _ => "changed",
        };
        out.push_str(&format!(
            "- **{}**: {how}; {}{sure}\n",
            f.seed,
            if what.is_empty() {
                "its lines changed".to_string()
            } else {
                what.join("; ")
            }
        ));
    }
    if !c.graph.moved.is_empty() {
        out.push_str(&format!(
            "\nMoved or renamed, the same body: {}.\n",
            c.graph
                .moved
                .iter()
                .map(|m| format!("`{}` → `{}`", m.from, m.to))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let changed: Vec<String> = c
        .graph
        .changed
        .iter()
        .map(|x| format!("`{}` ({})", x.id, x.what.join(", ")))
        .collect();
    if !changed.is_empty() {
        out.push_str(&format!("\nChanged parts: {}.\n", changed.join(", ")));
    }
    out
}

/// The changed lines of a unified diff with no context (`git diff -U0`), by file.
pub fn changed_lines(unified: &str) -> Vec<LinesTouched> {
    let mut out: Vec<LinesTouched> = Vec::new();
    let path = |s: &str| {
        (s != "/dev/null").then(|| {
            s.trim_start_matches("a/")
                .trim_start_matches("b/")
                .to_string()
        })
    };
    for l in unified.lines() {
        if let Some(old) = l.strip_prefix("--- ") {
            out.push(LinesTouched {
                old_file: path(old.trim()),
                ..LinesTouched::default()
            });
        } else if let Some(new) = l.strip_prefix("+++ ") {
            if let Some(c) = out.last_mut() {
                c.new_file = path(new.trim());
            }
        } else if let Some(h) = l.strip_prefix("@@ ")
            && let Some(c) = out.last_mut()
        {
            // `@@ -12,3 +14,0 @@`: three lines gone at A from 12, none added at B.
            let mut parts = h.split_whitespace();
            let range = |s: Option<&str>, sign: char| -> Vec<usize> {
                let Some(s) = s.and_then(|s| s.strip_prefix(sign)) else {
                    return Vec::new();
                };
                let (start, count) = match s.split_once(',') {
                    Some((a, b)) => (a.parse().unwrap_or(0), b.parse().unwrap_or(0)),
                    None => (s.parse().unwrap_or(0), 1),
                };
                if count == 0 {
                    // Nothing on this side: the line it sits after counts, so a part that gained lines shows.
                    return if start > 0 { vec![start] } else { Vec::new() };
                }
                (start..start + count).collect()
            };
            c.old.extend(range(parts.next(), '-'));
            c.new.extend(range(parts.next(), '+'));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scenario renamed between the commits is one flow, matched by its parts, not one gone and one
    /// new (docair-zrr.32).
    #[test]
    fn a_renamed_flow_is_matched_by_its_parts() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../docair-adapters/tests/golden/shop.graph.json"
        );
        let a: Graph = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut b = a.clone();
        b.flows
            .iter_mut()
            .find(|f| f.name == "checkout")
            .unwrap()
            .name = "buy".into();
        let c = changes(&a, &b, GraphDiff::default(), &[], ("A", "B"));
        let buy = c.flows.iter().find(|f| f.seed == "flow:buy");
        assert!(
            !c.flows.iter().any(|f| f.seed == "flow:checkout"),
            "not gone: {:#?}",
            c.flows
        );
        let buy = buy.unwrap_or_else(|| panic!("{:#?}", c.flows));
        assert_eq!(buy.how, "changed");
        assert_eq!(buy.renamed_from.as_deref(), Some("flow:checkout"));
    }

    #[test]
    fn a_unified_diff_gives_the_changed_lines_of_each_file() {
        let d = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2 +2 @@\n-a\n+b\n@@ -10,0 +11,2 @@\n+c\n+d\ndiff --git a/new.rs b/new.rs\n--- /dev/null\n+++ b/new.rs\n@@ -0,0 +1,3 @@\n";
        let c = changed_lines(d);
        assert_eq!(c.len(), 2);
        assert_eq!(
            (c[0].old_file.as_deref(), c[0].new_file.as_deref()),
            (Some("src/lib.rs"), Some("src/lib.rs"))
        );
        assert_eq!(c[0].old, [2, 10]);
        assert_eq!(c[0].new, [2, 11, 12]);
        assert_eq!(
            (c[1].old_file.as_deref(), c[1].new.as_slice()),
            (None, &[1, 2, 3][..])
        );
    }
}

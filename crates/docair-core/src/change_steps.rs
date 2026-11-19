//! The steps of a change (docs/CHANGES.md §8, docs/EXPLAIN.md §8, docair-8l6.12): a changed flow
//! before and after as one picture, what each of its parts and lines is marked as, and its changes in
//! steps with stable ids, the same for the page's walk over them and for the model's words on them.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::changes::{Changes, FlowChange};
use crate::diff::GraphDiff;
use crate::layer_view::{LayerLine, LayerPart, LayerPicture, slice_picture};
use crate::model::Graph;

/// A's ids of the parts that moved or were renamed, and their ids at B (the graph diff's moves).
pub type IdMoves<'c> = HashMap<&'c str, &'c str>;

/// The graph diff's moves, A's id to B's.
pub fn id_moves(diff: &GraphDiff) -> IdMoves<'_> {
    diff.moved
        .iter()
        .map(|m| (m.from.as_str(), m.to.as_str()))
        .collect()
}

/// A flow's slice picture in one version, by its seed; none when that version has no such flow.
pub fn flow_picture(graph: &Graph, seed: &str) -> Option<LayerPicture> {
    let found = crate::slice::find(graph, seed)?;
    let s = crate::slice::slice(graph, &found)?;
    Some(slice_picture(graph, &s))
}

/// Both versions of a flow as one: B's graph with what only A has added, and one picture of both.
#[derive(Debug, Clone)]
pub struct Merged {
    pub graph: Graph,
    pub pic: LayerPicture,
}

/***
 * Both versions of a flow as one picture (docs/CHANGES.md §8, docair-zrr.18): B's graph with the parts and lines only A has added, A's moved parts under their ids at B (docair-zrr.21), and the slice's picture of both, a part or line both have once; a flow only one version has is that version's alone.
 *
 * The lines before and after leave a shared part and meet at a shared part again, so the picture forks where the flow goes another way and rejoins where it is the same.
 ***/
pub fn merged(
    a: Option<(&Graph, &LayerPicture)>,
    b: Option<(&Graph, &LayerPicture)>,
    moves: &IdMoves,
) -> Option<Merged> {
    let at_b = |id: &str| {
        moves
            .get(id)
            .map_or_else(|| id.to_string(), |to| to.to_string())
    };
    let (mut graph, mut pic) = match (a, b) {
        (_, Some((g, p))) => (g.clone(), p.clone()),
        (Some((g, p)), None) => (
            Graph {
                nodes: Vec::new(),
                edges: Vec::new(),
                ..g.clone()
            },
            LayerPicture {
                parts: Vec::new(),
                lines: Vec::new(),
                ..p.clone()
            },
        ),
        (None, None) => return None,
    };
    let Some((a_graph, a_pic)) = a else {
        return Some(Merged { graph, pic });
    };
    // A's own parts, under their ids at B.
    let known: HashSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    for n in &a_graph.nodes {
        let id = at_b(&n.id);
        if !known.contains(&id) {
            let mut n = n.clone();
            n.id = id;
            n.part_of = n.part_of.as_deref().map(at_b);
            graph.nodes.push(n);
        }
    }
    // A's own lines, their ids as they read at B; every line of A, where it is in the merged graph.
    let mut renamed = a_graph.edges.clone();
    for e in &mut renamed {
        e.from = at_b(&e.from);
        e.to = at_b(&e.to);
    }
    crate::ids::edge_ids(&mut renamed);
    let mut edge_at: HashMap<String, usize> = graph
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.clone(), i))
        .collect();
    let mut a_edge = Vec::with_capacity(renamed.len());
    for e in renamed {
        let i = match edge_at.get(&e.id) {
            Some(&i) => i,
            None => {
                edge_at.insert(e.id.clone(), graph.edges.len());
                graph.edges.push(e);
                graph.edges.len() - 1
            }
        };
        a_edge.push(i);
    }
    // The parts of A's picture: one B has too is opened or closed as either needs.
    let mut part_at: HashMap<String, usize> = pic
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.clone(), i))
        .collect();
    for p in &a_pic.parts {
        let id = at_b(&p.id);
        match part_at.get(&id) {
            Some(&i) => {
                let q = &mut pic.parts[i];
                q.open |= p.open;
                q.closed = !q.open && (q.closed || p.closed);
                q.holds = if q.closed { q.holds.max(p.holds) } else { 0 };
                q.more = q.more.max(p.more);
            }
            None => {
                part_at.insert(id.clone(), pic.parts.len());
                pic.parts.push(LayerPart {
                    id,
                    parent: p.parent.as_deref().map(at_b),
                    ..p.clone()
                });
            }
        }
    }
    // The lines of A's picture B does not draw, by the id of the first line each stands for.
    let first = |g: &Graph, l: &LayerLine| {
        l.members
            .first()
            .and_then(|&m| g.edges.get(m))
            .map(|e| e.id.clone())
    };
    let drawn: HashSet<String> = pic.lines.iter().filter_map(|l| first(&graph, l)).collect();
    let in_merged = |m: &usize| a_edge.get(*m).copied().unwrap_or(*m);
    for l in &a_pic.lines {
        let line = LayerLine {
            from: at_b(&l.from),
            to: at_b(&l.to),
            members: l.members.iter().map(in_merged).collect(),
            via: l.via.iter().map(|v| at_b(v)).collect(),
            says: l.says.as_ref().map(|s| s.iter().map(in_merged).collect()),
            ..l.clone()
        };
        if first(&graph, &line).is_some_and(|k| !drawn.contains(&k)) {
            pic.lines.push(line);
        }
    }
    Some(Merged { graph, pic })
}

/// What in a part counts as its code; a change in anything else (`summary`, `tags`, `name`) is a change in its words.
pub const CODE: [&str; 4] = ["body", "kind", "route", "part_of"];

/// The parts both versions have that changed inside: in their code, or only in their words (docs/CHANGES.md §2, §8, docair-zrr.23).
#[derive(Debug, Clone, Default)]
pub struct ChangedInside<'c> {
    pub code: HashSet<&'c str>,
    pub words: HashSet<&'c str>,
}

pub fn changed_inside(diff: &GraphDiff) -> ChangedInside<'_> {
    let mut inside = ChangedInside::default();
    for c in &diff.changed {
        if c.what.iter().any(|w| CODE.contains(&w.as_str())) {
            inside.code.insert(c.id.as_str());
        } else if !c.what.is_empty() {
            inside.words.insert(c.id.as_str());
        }
    }
    inside
}

/// How the one picture marks a part: only after, only before, its code changed, or the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartMark {
    Added,
    Removed,
    Code,
    Same,
}

/// How the one picture marks a line: only after, only before, or the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineMark {
    Added,
    Removed,
    Same,
}

/// What each part (by id) and each graph line (by id) of a flow's one picture is marked as.
#[derive(Debug, Clone, Default)]
pub struct FlowMarks {
    pub parts: HashMap<String, PartMark>,
    pub lines: HashMap<String, LineMark>,
}

/***
 * What each part and line of a flow's one picture is marked as (docs/CHANGES.md §8): added only after, removed only before, code a part both have whose code changed (an open box too), the same the rest, a part whose words alone changed among them. A picture's line takes the mark of its lines: added if one is, else removed if one is; every graph line it stands for gets that mark.
 ***/
pub fn marks(m: &Merged, f: &FlowChange, code: &HashSet<&str>) -> FlowMarks {
    fn set(v: &[String]) -> HashSet<&str> {
        v.iter().map(String::as_str).collect()
    }
    let (added, removed, through) = (set(&f.parts_added), set(&f.parts_removed), set(&f.through));
    let (lines_added, lines_removed) = (set(&f.lines_added), set(&f.lines_removed));
    let mut out = FlowMarks::default();
    for p in &m.pic.parts {
        let id = p.id.as_str();
        let mark = if added.contains(id) {
            PartMark::Added
        } else if removed.contains(id) {
            PartMark::Removed
        } else if through.contains(id) && code.contains(id) {
            PartMark::Code
        } else {
            PartMark::Same
        };
        out.parts.insert(p.id.clone(), mark);
    }
    for l in &m.pic.lines {
        let ids: Vec<&str> = l
            .members
            .iter()
            .filter_map(|&e| m.graph.edges.get(e))
            .map(|e| e.id.as_str())
            .collect();
        let mark = if ids.iter().any(|i| lines_added.contains(i)) {
            LineMark::Added
        } else if ids.iter().any(|i| lines_removed.contains(i)) {
            LineMark::Removed
        } else {
            LineMark::Same
        };
        for i in ids {
            out.lines.entry(i.to_string()).or_insert(mark);
        }
    }
    out
}

/// What a step of a change is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// A stretch of the path: new and gone parts joined by new or gone lines.
    Path,
    /// A line alone, new or gone, between two parts both versions have.
    Line,
    /// The parts of one box whose code changed.
    Code,
    /// An open box whose own block changed, not the parts drawn in it.
    Box,
}

/// A line of a step: the graph line's id (A's lines as they read at B, moved parts under their ids at B) and its ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepLine {
    pub id: String,
    pub from: String,
    pub to: String,
}

/// One step of a walk over a flow's changes: what it is about, its parts and its lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeStep {
    /// Stable: `<seed>#path:<id>`, `<seed>#line:<from>-><to>`, `<seed>#code:<box>`, `<seed>#box:<id>` (docs/EXPLAIN.md §8).
    pub id: String,
    pub kind: StepKind,
    /// The parts only after, by their ids at B.
    pub added: Vec<String>,
    /// The parts only before, by their ids at A.
    pub removed: Vec<String>,
    /// The parts both have whose code changed (or the open box itself, for a `box` step).
    pub code: Vec<String>,
    /// The parts both have where the change forks off and rejoins the flow.
    pub ends: Vec<String>,
    pub lines_added: Vec<String>,
    pub lines_removed: Vec<String>,
    /// The step's lines with their ends, added and removed together.
    pub lines: Vec<StepLine>,
    /// The box its parts are drawn in, for a step told box by box or a `code` step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_box: Option<String>,
}

impl ChangeStep {
    /// Every part the step is about: its new, gone and changed parts and its ends.
    pub fn parts(&self) -> impl Iterator<Item = &String> {
        self.added
            .iter()
            .chain(&self.removed)
            .chain(&self.code)
            .chain(&self.ends)
    }
}

/// A stretch of the path while steps are worked out: its parts and its lines, by their places in the picture.
type StepDraft = (Vec<usize>, Vec<usize>);

/// A step holds at most this many new or gone parts; a stretch with more is told box by box.
pub const MAX_STEP_PARTS: usize = 8;

/***
 * A flow's changes in steps (docs/CHANGES.md §8, docair-zrr.19, zrr.25), the unchanged rest left out: one per stretch of the path (new and gone parts joined by new or gone lines, with the shared parts where they fork off and rejoin; a shared part is an end, never a joint), a gone stretch and a new one that share an end as one step (the fork), a stretch with more than 8 new or gone parts told box by box; then one per box whose parts changed in their code, an open box whose own block changed alone. Path steps first, then code steps, each in the order of the picture's parts. A new or gone flow has none: the whole picture is the change.
 *
 * Each step's id is stable (docs/EXPLAIN.md §8), so the model's words on it are kept by id.
 ***/
pub fn steps(m: &Merged, f: &FlowChange, marks: &FlowMarks) -> Vec<ChangeStep> {
    if f.how != "changed" {
        return Vec::new();
    }
    let n = m.pic.parts.len();
    let at: HashMap<&str, usize> = m
        .pic
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.as_str(), i))
        .collect();
    let mark = |i: usize| {
        marks
            .parts
            .get(&m.pic.parts[i].id)
            .copied()
            .unwrap_or(PartMark::Same)
    };
    let line_mark = |l: &LayerLine| {
        l.members
            .first()
            .and_then(|&e| m.graph.edges.get(e))
            .and_then(|e| marks.lines.get(&e.id))
            .copied()
            .unwrap_or(LineMark::Same)
    };
    let on_path = |i: usize| matches!(mark(i), PartMark::Added | PartMark::Removed);
    let mut up: Vec<usize> = (0..n).collect();
    fn root(up: &mut [usize], mut i: usize) -> usize {
        while up[i] != i {
            up[i] = up[up[i]];
            i = up[i];
        }
        i
    }
    // A new or gone line joins the new and gone parts it links; a shared part is only an end.
    let mut changed_lines = Vec::new();
    for (k, l) in m.pic.lines.iter().enumerate() {
        if let (true, Some(&a), Some(&b)) = (
            line_mark(l) != LineMark::Same,
            at.get(l.from.as_str()),
            at.get(l.to.as_str()),
        ) {
            if on_path(a) && on_path(b) {
                let (ra, rb) = (root(&mut up, a), root(&mut up, b));
                up[ra] = rb;
            }
            changed_lines.push((k, a, b));
        }
    }
    let mut stretches: HashMap<usize, StepDraft> = HashMap::new();
    for &(k, a, b) in &changed_lines {
        // Its stretch: the one its new or gone end is in; a line between two shared parts is one of its own.
        let key = match (on_path(a), on_path(b)) {
            (true, _) => root(&mut up, a),
            (_, true) => root(&mut up, b),
            _ => n + k,
        };
        let e = stretches.entry(key).or_default();
        e.1.push(k);
        for i in [a, b] {
            if !e.0.contains(&i) {
                e.0.push(i);
            }
        }
    }
    // A new or gone part no changed line reaches is a change of its own; an open box shows its parts.
    for (i, p) in m.pic.parts.iter().enumerate() {
        if on_path(i) && !p.open {
            let e = stretches.entry(root(&mut up, i)).or_default();
            if !e.0.contains(&i) {
                e.0.push(i);
            }
        }
    }
    let mut stretches: Vec<(usize, StepDraft)> = stretches.into_iter().collect();
    stretches.sort_by_key(|(key, _)| *key);
    // A stretch with many new or gone parts is told box by box, each with its lines and ends.
    let mut split: Vec<(Vec<usize>, Vec<usize>, Option<String>)> = Vec::new();
    for (_, (parts, lines)) in stretches {
        if parts.iter().filter(|&&i| on_path(i)).count() <= MAX_STEP_PARTS {
            split.push((parts, lines, None));
            continue;
        }
        let mut by_box: Vec<(Option<String>, Vec<usize>, Vec<usize>)> = Vec::new();
        for &i in parts.iter().filter(|&&i| on_path(i)) {
            let key = m.pic.parts[i].parent.clone();
            match by_box.iter_mut().find(|(k, _, _)| *k == key) {
                Some((_, v, _)) => v.push(i),
                None => by_box.push((key, vec![i], Vec::new())),
            }
        }
        for (_, group, own) in &mut by_box {
            for &k in &lines {
                let l = &m.pic.lines[k];
                let (Some(&a), Some(&b)) = (at.get(l.from.as_str()), at.get(l.to.as_str())) else {
                    continue;
                };
                if group.contains(&a) || group.contains(&b) {
                    own.push(k);
                    for i in [a, b] {
                        if !group.contains(&i) && !on_path(i) {
                            group.push(i);
                        }
                    }
                }
            }
        }
        split.extend(by_box.into_iter().map(|(b, g, l)| (g, l, b)));
    }
    // A fork: what is gone and what replaced it, where they meet the same shared part, are one step;
    // a gone line alone counts as gone, a new line alone as new (a function put in between, or taken out).
    let kinds = |(parts, lines): (&[usize], &[usize])| {
        let has = |w: PartMark| parts.iter().any(|&i| mark(i) == w);
        let has_line = |w: LineMark| lines.iter().any(|&k| line_mark(&m.pic.lines[k]) == w);
        (
            has(PartMark::Added) || has_line(LineMark::Added),
            has(PartMark::Removed) || has_line(LineMark::Removed),
        )
    };
    let ends_of = |parts: &[usize]| -> HashSet<usize> {
        parts.iter().copied().filter(|&i| !on_path(i)).collect()
    };
    let mut k = 0;
    while k < split.len() {
        if kinds((&split[k].0, &split[k].1)) == (false, true) {
            let gone_ends = ends_of(&split[k].0);
            let pair = (0..split.len()).find(|&j| {
                j != k
                    && kinds((&split[j].0, &split[j].1)) == (true, false)
                    && !ends_of(&split[j].0).is_disjoint(&gone_ends)
            });
            if let Some(j) = pair {
                let (parts, lines, _) = split.remove(k);
                let j = if j > k { j - 1 } else { j };
                for i in parts {
                    if !split[j].0.contains(&i) {
                        split[j].0.push(i);
                    }
                }
                split[j].1.extend(lines);
                continue;
            }
        }
        k += 1;
    }
    let in_stretch: HashSet<usize> = split.iter().flat_map(|s| s.0.clone()).collect();
    // The parts changed in their code, by the box they are in; an open box whose own block changed alone.
    let mut boxes: Vec<(Option<String>, Vec<usize>, bool)> = Vec::new();
    for i in (0..n).filter(|&i| mark(i) == PartMark::Code && !in_stretch.contains(&i)) {
        let p = &m.pic.parts[i];
        if p.open {
            boxes.push((Some(p.id.clone()), vec![i], true));
            continue;
        }
        match boxes.iter_mut().find(|(k, _, own)| *k == p.parent && !own) {
            Some((_, v, _)) => v.push(i),
            None => boxes.push((p.parent.clone(), vec![i], false)),
        }
    }
    let id = |i: usize| m.pic.parts[i].id.clone();
    let seed = &f.seed;
    let mut out: Vec<ChangeStep> = Vec::new();
    let mut path: Vec<(Vec<usize>, Vec<usize>, Option<String>)> = split;
    path.sort_by_key(|(parts, _, _)| parts.iter().min().copied());
    for (mut parts, mut lines, in_box) in path {
        parts.sort_unstable();
        lines.sort_unstable();
        let of = |w: PartMark| -> Vec<String> {
            parts
                .iter()
                .filter(|&&i| mark(i) == w)
                .map(|&i| id(i))
                .collect()
        };
        let step_lines: Vec<(LineMark, StepLine)> = lines
            .iter()
            .map(|&k| &m.pic.lines[k])
            .flat_map(|l| {
                let mark = line_mark(l);
                l.members
                    .iter()
                    .filter_map(|&e| m.graph.edges.get(e))
                    .map(move |e| {
                        (
                            mark,
                            StepLine {
                                id: e.id.clone(),
                                from: e.from.clone(),
                                to: e.to.clone(),
                            },
                        )
                    })
            })
            .collect();
        let (added, removed) = (of(PartMark::Added), of(PartMark::Removed));
        let mut on: Vec<&String> = added.iter().chain(&removed).collect();
        on.sort();
        let (kind, key) = match (on.first(), step_lines.first()) {
            (Some(first), _) => (StepKind::Path, format!("{seed}#path:{first}")),
            (None, Some((_, l))) => (StepKind::Line, format!("{seed}#line:{}->{}", l.from, l.to)),
            (None, None) => continue,
        };
        out.push(ChangeStep {
            id: key,
            kind,
            added,
            removed,
            code: of(PartMark::Code),
            ends: of(PartMark::Same),
            lines_added: step_lines
                .iter()
                .filter(|(w, _)| *w == LineMark::Added)
                .map(|(_, l)| l.id.clone())
                .collect(),
            lines_removed: step_lines
                .iter()
                .filter(|(w, _)| *w == LineMark::Removed)
                .map(|(_, l)| l.id.clone())
                .collect(),
            lines: step_lines.into_iter().map(|(_, l)| l).collect(),
            in_box,
        });
    }
    for (key, parts, own) in boxes {
        let ids: Vec<String> = parts.iter().map(|&i| id(i)).collect();
        let (kind, step_id) = if own {
            (StepKind::Box, format!("{seed}#box:{}", ids[0]))
        } else {
            (
                StepKind::Code,
                format!("{seed}#code:{}", key.as_deref().unwrap_or("")),
            )
        };
        out.push(ChangeStep {
            id: step_id,
            kind,
            added: Vec::new(),
            removed: Vec::new(),
            code: ids,
            ends: Vec::new(),
            lines_added: Vec::new(),
            lines_removed: Vec::new(),
            lines: Vec::new(),
            in_box: if own { None } else { key },
        });
    }
    // Ids stay unique, should two steps ever be keyed alike.
    let mut seen: HashMap<String, usize> = HashMap::new();
    for s in &mut out {
        let k = seen.entry(s.id.clone()).or_default();
        *k += 1;
        if *k > 1 {
            s.id = format!("{} #{k}", s.id);
        }
    }
    out
}

/***
 * A changed flow's steps, from the two graphs and what `changes` found (docs/EXPLAIN.md §8): the flow before and after as one picture, its marks, and its steps; empty for a new or gone flow, or a seed neither version has.
 *
 * @calls docair-core.change_steps.merged "both versions as one picture"
 * @calls docair-core.change_steps.steps "the changes in steps"
 ***/
pub fn change_steps(a: &Graph, b: &Graph, changes: &Changes, flow: &FlowChange) -> Vec<ChangeStep> {
    let (pa, pb) = (flow_picture(a, &flow.seed), flow_picture(b, &flow.seed));
    let moves = id_moves(&changes.graph);
    let Some(m) = merged(
        pa.as_ref().map(|p| (a, p)),
        pb.as_ref().map(|p| (b, p)),
        &moves,
    ) else {
        return Vec::new();
    };
    let inside = changed_inside(&changes.graph);
    let marked = marks(&m, flow, &inside.code);
    steps(&m, flow, &marked)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shop as the adapters read it, from their golden graph.
    fn shop() -> Graph {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../docair-adapters/tests/golden/shop.graph.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// A flow's change as `changes` reports it, from its two pictures.
    fn change(seed: &str, a: (&Graph, &LayerPicture), b: (&Graph, &LayerPicture)) -> FlowChange {
        let ids = |p: &LayerPicture| -> HashSet<String> {
            p.parts.iter().map(|p| p.id.clone()).collect()
        };
        let lines = |(g, p): (&Graph, &LayerPicture)| -> HashSet<String> {
            p.lines
                .iter()
                .flat_map(|l| l.members.iter().map(|&m| g.edges[m].id.clone()))
                .collect()
        };
        let less = |x: &HashSet<String>, y: &HashSet<String>| -> Vec<String> {
            let mut v: Vec<String> = x.difference(y).cloned().collect();
            v.sort();
            v
        };
        FlowChange {
            seed: seed.into(),
            how: "changed".into(),
            parts_added: less(&ids(b.1), &ids(a.1)),
            parts_removed: less(&ids(a.1), &ids(b.1)),
            lines_added: less(&lines(b), &lines(a)),
            lines_removed: less(&lines(a), &lines(b)),
            through: Vec::new(),
            certain: true,
        }
    }

    /// docair-zrr.18: before and after are one picture, a part both have once; what only B has is
    /// added, what only A had removed, the rest the same.
    #[test]
    fn before_and_after_are_one_picture() {
        let after = shop();
        let mut before = after.clone();
        before
            .edges
            .retain(|e| !(e.from == "orders.create" && e.to == "orders-db"));
        let (pa, pb) = (
            flow_picture(&before, "data.order").unwrap(),
            flow_picture(&after, "data.order").unwrap(),
        );
        let f = change("data.order", (&before, &pa), (&after, &pb));
        assert!(
            f.parts_added.contains(&"orders-db".to_string()),
            "only B writes the order"
        );
        let m = merged(Some((&before, &pa)), Some((&after, &pb)), &IdMoves::new()).unwrap();
        for p in pa.parts.iter().chain(&pb.parts) {
            assert_eq!(
                m.pic.parts.iter().filter(|q| q.id == p.id).count(),
                1,
                "{} once",
                p.id
            );
        }
        let marked = marks(&m, &f, &HashSet::new());
        assert_eq!(marked.parts["orders-db"], PartMark::Added);
        assert!(
            marked
                .parts
                .values()
                .filter(|w| **w == PartMark::Same)
                .count()
                >= 3
        );
        assert!(marked.lines.values().any(|w| *w == LineMark::Added));
        // The other way round, the part is only before.
        let g = change("data.order", (&after, &pb), (&before, &pa));
        let m = merged(Some((&after, &pb)), Some((&before, &pa)), &IdMoves::new()).unwrap();
        assert_eq!(
            marks(&m, &g, &HashSet::new()).parts["orders-db"],
            PartMark::Removed
        );
    }

    /// docair-zrr.21: a part renamed between the versions is one part, under its name at B, and its
    /// lines are the same lines.
    #[test]
    fn a_renamed_part_is_drawn_once() {
        let after = shop();
        let mut before = after.clone();
        let old = "orders.create-old";
        for n in &mut before.nodes {
            if n.id == "orders.create" {
                n.id = old.into();
            }
        }
        for e in &mut before.edges {
            for end in [&mut e.from, &mut e.to] {
                if end == "orders.create" {
                    *end = old.into();
                }
            }
        }
        crate::ids::edge_ids(&mut before.edges);
        let (pa, pb) = (
            flow_picture(&before, "data.order").unwrap(),
            flow_picture(&after, "data.order").unwrap(),
        );
        let moves: IdMoves = [(old, "orders.create")].into_iter().collect();
        let m = merged(Some((&before, &pa)), Some((&after, &pb)), &moves).unwrap();
        assert!(
            m.pic.parts.iter().all(|p| p.id != old),
            "no part under its old name"
        );
        assert_eq!(m.pic.lines.len(), pb.lines.len(), "its lines are B's lines");
        assert_eq!(
            m.graph.edges.len(),
            after.edges.len(),
            "no line of A is added"
        );
    }

    /// docair-zrr.19, 8l6.12: the shop's one new line is one path step, with a stable id, the new
    /// store, the part it is reached from as its end, and the line.
    #[test]
    fn a_new_line_is_one_step_with_a_stable_id() {
        let after = shop();
        let mut before = after.clone();
        before
            .edges
            .retain(|e| !(e.from == "orders.create" && e.to == "orders-db"));
        let (pa, pb) = (
            flow_picture(&before, "data.order").unwrap(),
            flow_picture(&after, "data.order").unwrap(),
        );
        let f = change("data.order", (&before, &pa), (&after, &pb));
        let m = merged(Some((&before, &pa)), Some((&after, &pb)), &IdMoves::new()).unwrap();
        let s = steps(&m, &f, &marks(&m, &f, &HashSet::new()));
        assert_eq!(s.len(), 1, "{s:?}");
        assert_eq!(s[0].id, "data.order#path:orders-db");
        assert_eq!(s[0].kind, StepKind::Path);
        assert_eq!(s[0].added, vec!["orders-db".to_string()]);
        assert!(!s[0].ends.is_empty(), "where it forks off");
        assert!(!s[0].lines_added.is_empty() && s[0].lines.iter().any(|l| l.to == "orders-db"));
        // Asked again, the same id: the model's words on it are kept by it.
        assert_eq!(
            steps(&m, &f, &marks(&m, &f, &HashSet::new()))[0].id,
            s[0].id
        );
    }

    /// docair-zrr.26: a function put in between, its new lines, and the direct line it replaced are
    /// one step, the fork; taken out again, the same the other way round.
    #[test]
    fn a_function_put_in_between_is_one_step_with_the_line_it_replaced() {
        let before = shop();
        let mut after = before.clone();
        let direct = after
            .edges
            .iter()
            .position(|e| e.from == "orders.create" && e.to == "orders-db")
            .expect("the shop writes the order");
        let mut node = after
            .nodes
            .iter()
            .find(|n| n.id == "orders.create")
            .unwrap()
            .clone();
        node.id = "orders.validate".into();
        node.name = "validate_order".into();
        after.nodes.push(node);
        let mut to_validate = after.edges[direct].clone();
        to_validate.to = "orders.validate".into();
        let mut to_db = after.edges[direct].clone();
        to_db.from = "orders.validate".into();
        after.edges.remove(direct);
        after.edges.push(to_validate);
        after.edges.push(to_db);
        crate::ids::edge_ids(&mut after.edges);
        for (a, b, added) in [(&before, &after, true), (&after, &before, false)] {
            let (pa, pb) = (
                flow_picture(a, "data.order").unwrap(),
                flow_picture(b, "data.order").unwrap(),
            );
            let f = change("data.order", (a, &pa), (b, &pb));
            let m = merged(Some((a, &pa)), Some((b, &pb)), &IdMoves::new()).unwrap();
            let s = steps(&m, &f, &marks(&m, &f, &HashSet::new()));
            assert_eq!(s.len(), 1, "one fork: {s:#?}");
            let (new, gone) = if added {
                (&s[0].added, &s[0].lines_removed)
            } else {
                (&s[0].removed, &s[0].lines_added)
            };
            assert_eq!(new, &vec!["orders.validate".to_string()]);
            assert!(
                gone.iter()
                    .any(|l| l.starts_with("orders.create -") && l.contains("-> orders-db")),
                "{gone:?}"
            );
            assert!(
                s[0].ends.contains(&"orders.create".to_string())
                    && s[0].ends.contains(&"orders-db".to_string())
            );
        }
    }

    /// docair-zrr.22, zrr.23: a part whose code changed is a code step of its box; an open box whose
    /// own block changed is a box step; a part whose words alone changed is no step.
    #[test]
    fn code_is_a_step_and_words_are_not() {
        use crate::diff::ChangedItem;
        let after = shop();
        let pb = flow_picture(&after, "data.order").unwrap();
        let open_box = pb.parts.iter().find(|p| p.open).unwrap().id.clone();
        let leaves: Vec<String> = pb
            .parts
            .iter()
            .filter(|p| !p.open)
            .map(|p| p.id.clone())
            .take(2)
            .collect();
        let (code, words) = (leaves[0].clone(), leaves[1].clone());
        let item = |id: &str, what: &[&str]| ChangedItem {
            id: id.into(),
            what: what.iter().map(|w| w.to_string()).collect(),
        };
        let diff = GraphDiff {
            changed: vec![
                item(&code, &["body", "summary"]),
                item(&words, &["summary", "tags"]),
                item(&open_box, &["body"]),
            ],
            ..GraphDiff::default()
        };
        let f = FlowChange {
            seed: "data.order".into(),
            how: "changed".into(),
            parts_added: vec![],
            parts_removed: vec![],
            lines_added: vec![],
            lines_removed: vec![],
            through: vec![code.clone(), words.clone(), open_box.clone()],
            certain: true,
        };
        let inside = changed_inside(&diff);
        assert!(inside.code.contains(code.as_str()) && inside.words.contains(words.as_str()));
        let m = merged(None, Some((&after, &pb)), &IdMoves::new()).unwrap();
        let marked = marks(&m, &f, &inside.code);
        assert_eq!(marked.parts[&code], PartMark::Code);
        assert_eq!(
            marked.parts[&words],
            PartMark::Same,
            "words alone are listed, not marked"
        );
        let s = steps(&m, &f, &marked);
        assert!(
            s.iter()
                .any(|s| s.kind == StepKind::Box && s.id == format!("data.order#box:{open_box}"))
        );
        assert!(
            s.iter()
                .any(|s| s.kind == StepKind::Code && s.code.contains(&code))
        );
        assert!(s.iter().all(|s| !s.parts().any(|p| *p == words)));
    }
}

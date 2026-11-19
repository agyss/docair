//! The review (docs/EXPLAIN.md §9, docair-8l6.18): the pieces of a whole change, worked out once
//! across every flow; the intent the AI reads beside them (commit messages, changed docs, doc
//! blocks whose words changed); and a change's own picture, made of its pieces only.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::change_steps::{self, ChangeStep, IdMoves, Merged, StepKind};
use crate::changes::{Changes, FlowChange};
use crate::layer_view::{LayerPicture, slice_picture};
use crate::model::Graph;
use crate::slice::{SeedKind, Slice, SliceLine, SliceSeed};

/// A commit message's body goes to the AI cut at this many characters (docs/EXPLAIN.md §9.2).
pub const MAX_BODY: usize = 2_000;
/// A Markdown file's changed lines go cut at this many characters.
pub const MAX_DOC: usize = 2_000;
/// All Markdown files' changed lines together go cut at this many characters.
pub const MAX_DOCS: usize = 12_000;

/***
 * A piece of the whole change: a change step (CHANGES.md §8) of one picture of the whole change, a stretch of new and gone parts and lines, a line alone, the parts of one box whose code changed, or an open box whose own block changed; named without a flow (`path:<first new or gone part>`, `line:<from>-><to>`, `code:<box>`, `box:<id>`), with the changed flows it touches.
 ***/
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Piece {
    #[serde(flatten)]
    pub step: ChangeStep,
    /// The changed flows it touches, by seed; none when no flow goes through it.
    pub flows: Vec<String>,
}

impl Piece {
    pub fn id(&self) -> &str {
        &self.step.id
    }
}

/// A step's id without its flow: `data.order#path:orders-db` → `path:orders-db`.
fn key_of(step_id: &str) -> &str {
    step_id.split_once('#').map_or(step_id, |(_, k)| k)
}

fn push_new<T: PartialEq + Clone>(into: &mut Vec<T>, from: &[T]) {
    for x in from {
        if !into.contains(x) {
            into.push(x.clone());
        }
    }
}

/// The two ends of a graph line, by its id: `<from> -<kind>-> <to>`, with ` "<label>"` or ` #2` after it (docair-svh).
pub fn line_ends(id: &str) -> Option<(&str, &str)> {
    let (from, rest) = id.split_once(" -")?;
    let (_, to) = rest.split_once("-> ")?;
    let to = to.split(" \"").next()?.split(" #").next()?;
    Some((from, to))
}

/***
 * The pieces of the whole change (docs/EXPLAIN.md §9.2): the change steps (CHANGES.md §8) of one picture of the whole change, its new, gone and code-changed parts and the ends of its new and gone lines at A and at B, marked by the graph diff (new means new to the system, not to a flow), so a change that shows up in many flows is one piece, and a part no flow goes through is a piece too. Each piece's id names no flow (`path:<first new or gone part>`, `line:<from>-><to>`, `code:<box>`, `box:<id>`) and it lists the changed flows it touches. The path's pieces first, then the code's.
 *
 * @calls docair-core.change_steps.steps "the whole change in steps"
 ***/
pub fn pieces(a: &Graph, b: &Graph, changes: &Changes) -> Vec<Piece> {
    let g = &changes.graph;
    let inside = change_steps::changed_inside(g);
    let mut code: Vec<String> = inside.code.iter().map(|s| s.to_string()).collect();
    code.sort();
    let whole = FlowChange {
        seed: "review".into(),
        how: "changed".into(),
        parts_added: g.added.clone(),
        parts_removed: g.removed.clone(),
        lines_added: g.lines_added.clone(),
        lines_removed: g.lines_removed.clone(),
        through: code.clone(),
        certain: true,
    };
    let moves: IdMoves = change_steps::id_moves(g);
    let back: HashMap<&str, &str> = moves.iter().map(|(from, to)| (*to, *from)).collect();
    let has = |gr: &Graph, id: &str| gr.nodes.iter().any(|n| n.id == id);
    let (mut at_a, mut at_b): (HashSet<String>, HashSet<String>) = (HashSet::new(), HashSet::new());
    at_b.extend(g.added.iter().cloned());
    at_a.extend(g.removed.iter().cloned());
    let ends = g
        .lines_added
        .iter()
        .chain(&g.lines_removed)
        .filter_map(|l| line_ends(l))
        .flat_map(|(x, y)| [x, y]);
    for id in code.iter().map(String::as_str).chain(ends) {
        if has(b, id) {
            at_b.insert(id.to_string());
        }
        let old = back.get(id).copied().unwrap_or(id);
        if has(a, old) {
            at_a.insert(old.to_string());
        }
    }
    let system = [a.system.as_str(), b.system.as_str()];
    at_a.retain(|id| !system.contains(&id.as_str()));
    at_b.retain(|id| !system.contains(&id.as_str()));
    let Some(m) = picture_of(a, b, &moves, &at_a, &at_b) else {
        return Vec::new();
    };
    let marked = change_steps::marks(&m, &whole, &inside.code);
    // The flows each piece touches: those whose changes hold one of its parts or lines.
    let touches = |s: &ChangeStep| -> Vec<String> {
        changes
            .flows
            .iter()
            .filter(|f| {
                let parts = f
                    .parts_added
                    .iter()
                    .chain(&f.parts_removed)
                    .chain(&f.through);
                let lines = f.lines_added.iter().chain(&f.lines_removed);
                parts
                    .into_iter()
                    .any(|p| s.added.contains(p) || s.removed.contains(p) || s.code.contains(p))
                    || lines
                        .into_iter()
                        .any(|l| s.lines.iter().any(|x| &x.id == l))
            })
            .map(|f| f.seed.clone())
            .collect()
    };
    let mut out: Vec<Piece> = change_steps::steps(&m, &whole, &marked)
        .into_iter()
        .map(|step| {
            let flows = touches(&step);
            Piece {
                step: ChangeStep {
                    id: key_of(&step.id).to_string(),
                    ..step
                },
                flows,
            }
        })
        .collect();
    // By module, so a module's new, gone and edited pieces go to the model together; the path's first.
    out.sort_by_key(|p| {
        (
            module_of(&p.step),
            matches!(p.step.kind, StepKind::Code | StepKind::Box),
            p.step.id.clone(),
        )
    });
    out
}

/// The module a piece is in, for its order: its box, else its id without its kind and last segment.
fn module_of(s: &ChangeStep) -> String {
    let id = s.id.split_once(':').map_or(s.id.as_str(), |(_, rest)| rest);
    s.in_box
        .clone()
        .unwrap_or_else(|| id.rsplit_once('.').map_or(id, |(m, _)| m).to_string())
}

/// The module an id is in: the id without its last segment (`docair-core.review.pieces` → `docair-core.review`).
fn parent_id(id: &str) -> &str {
    id.rsplit_once('.').map_or(id, |(m, _)| m)
}

/// The modules a piece is in: its box, and the modules of its own new, gone and edited parts; for a line alone, of its ends.
pub fn modules_of(p: &Piece) -> HashSet<String> {
    let s = &p.step;
    let mut out: HashSet<String> = s.in_box.iter().cloned().collect();
    let own = s.added.iter().chain(&s.removed).chain(&s.code);
    out.extend(own.map(|id| parent_id(id).to_string()));
    if out.is_empty() {
        for l in &s.lines {
            out.insert(parent_id(&l.from).to_string());
            out.insert(parent_id(&l.to).to_string());
        }
    }
    out
}

/***
 * The other edits that belong with a change (docs/EXPLAIN.md §9.4, docair-8l6.41): a piece no change holds goes with the change that has the most pieces in a module it is in, when that change has more there than every other; else it stays an other edit. A rule docair can stand behind: the model groups by meaning, this only by where the code is.
 ***/
pub fn attach_by_module(
    pieces: &[Piece],
    held: &[Vec<String>],
    other: &[String],
) -> (Vec<Vec<String>>, Vec<String>) {
    let by_id: HashMap<&str, &Piece> = pieces.iter().map(|p| (p.id(), p)).collect();
    let modules = |id: &str| by_id.get(id).map(|p| modules_of(p)).unwrap_or_default();
    let held_modules: Vec<Vec<HashSet<String>>> = held
        .iter()
        .map(|ids| ids.iter().map(|id| modules(id)).collect())
        .collect();
    let mut also: Vec<Vec<String>> = vec![Vec::new(); held.len()];
    let mut rest = Vec::new();
    for o in other {
        let mine = modules(o);
        let score: Vec<usize> = held_modules
            .iter()
            .map(|ms| ms.iter().filter(|m| !m.is_disjoint(&mine)).count())
            .collect();
        let best = score.iter().copied().max().unwrap_or(0);
        let winners: Vec<usize> = (0..score.len()).filter(|&k| score[k] == best).collect();
        match winners.as_slice() {
            [k] if best > 0 => also[*k].push(o.clone()),
            _ => rest.push(o.clone()),
        }
    }
    (also, rest)
}

/// One picture of some parts at A and at B and the lines between them (`change_steps::merged`).
fn picture_of(
    a: &Graph,
    b: &Graph,
    moves: &IdMoves,
    at_a: &HashSet<String>,
    at_b: &HashSet<String>,
) -> Option<Merged> {
    let pic = |g: &Graph, parts: &HashSet<String>| -> Option<LayerPicture> {
        (!parts.is_empty()).then(|| slice_picture(g, &slice_of(g, parts)))
    };
    let (pa, pb) = (pic(a, at_a), pic(b, at_b));
    change_steps::merged(
        pa.as_ref().map(|p| (a, p)),
        pb.as_ref().map(|p| (b, p)),
        moves,
    )
}

/// A commit between A and B, as the AI reads it: `commit:<short sha>`, its subject and its body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitNote {
    pub id: String,
    pub subject: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
}

/// A Markdown file that changed between A and B: `doc:<path>`, and its added and removed lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocChange {
    pub id: String,
    pub path: String,
    /// Its changed lines, each with `+` or `-` in front, cut at `MAX_DOC`.
    pub lines: String,
}

/// A doc block whose words alone changed: the part, and its summary before and after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordsChange {
    pub part: String,
    pub before: String,
    pub after: String,
}

/// What was meant, beside what changed (docs/EXPLAIN.md §9.2): the commit messages, the changed docs, the doc blocks whose words changed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent {
    pub commits: Vec<CommitNote>,
    /// B is the working tree: what is not committed has no message.
    #[serde(default)]
    pub uncommitted: bool,
    pub docs: Vec<DocChange>,
    pub words: Vec<WordsChange>,
}

impl Intent {
    /// The ids the AI may cite as a source: every commit and doc.
    pub fn sources(&self) -> HashSet<&str> {
        self.commits
            .iter()
            .map(|c| c.id.as_str())
            .chain(self.docs.iter().map(|d| d.id.as_str()))
            .collect()
    }
}

/// Cut text at a char boundary, saying so.
pub fn cut(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// The doc blocks whose words alone changed (`summary`, `tags`, `name`; docair-zrr.23): each part with its summary at A and at B.
pub fn words_changed(a: &Graph, b: &Graph, changes: &Changes) -> Vec<WordsChange> {
    let inside = change_steps::changed_inside(&changes.graph);
    let summary = |g: &Graph, id: &str| {
        g.nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| n.summary.clone())
            .unwrap_or_default()
    };
    let mut ids: Vec<&str> = inside.words.into_iter().collect();
    ids.sort_unstable();
    ids.into_iter()
        .map(|id| WordsChange {
            part: id.to_string(),
            before: cut(&summary(a, id), 400),
            after: cut(&summary(b, id), 400),
        })
        .filter(|w| w.before != w.after)
        .collect()
}

/// A slice made of some parts of a graph and the lines between them: what a change's picture draws.
fn slice_of(graph: &Graph, parts: &HashSet<String>) -> Slice {
    let mut ordered: Vec<String> = graph
        .nodes
        .iter()
        .filter(|n| parts.contains(&n.id))
        .map(|n| n.id.clone())
        .collect();
    ordered.dedup();
    let lines = graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| parts.contains(&e.from) && parts.contains(&e.to) && e.from != e.to)
        .map(|(i, e)| SliceLine {
            edge: i,
            from: e.from.clone(),
            to: e.to.clone(),
            kind: e.kind,
            reach: None,
            how: None,
        })
        .collect();
    Slice {
        schema: crate::slice::SCHEMA.to_string(),
        seed: SliceSeed {
            kind: SeedKind::Part,
            id: ordered.first().cloned().unwrap_or_default(),
        },
        ends: Vec::new(),
        parts: ordered,
        steps: Vec::new(),
        lines,
        boxes: Vec::new(),
        data: Vec::new(),
        covers: None,
    }
}

/***
 * A change's own picture (docs/EXPLAIN.md §9.5, docair-8l6.33): its pieces' parts at A and at B and the lines between them, nothing else, as one picture like a flow's (`change_steps::merged`); and the change as a flow change, so it is marked like one (new, gone, changed in its code).
 *
 * @calls docair-core.change_steps.merged "both versions as one picture"
 ***/
pub fn change_picture(
    a: &Graph,
    b: &Graph,
    changes: &Changes,
    seed: &str,
    pieces: &[&Piece],
) -> Option<(Merged, FlowChange)> {
    let moves: IdMoves = change_steps::id_moves(&changes.graph);
    let back: HashMap<&str, &str> = moves.iter().map(|(from, to)| (*to, *from)).collect();
    let mut at_a: HashSet<String> = HashSet::new();
    let mut at_b: HashSet<String> = HashSet::new();
    let mut f = FlowChange {
        seed: seed.to_string(),
        how: "changed".into(),
        parts_added: Vec::new(),
        parts_removed: Vec::new(),
        lines_added: Vec::new(),
        lines_removed: Vec::new(),
        through: Vec::new(),
        certain: true,
    };
    let has = |g: &Graph, id: &str| g.nodes.iter().any(|n| n.id == id);
    for p in pieces {
        let s = &p.step;
        push_new(&mut f.parts_added, &s.added);
        push_new(&mut f.parts_removed, &s.removed);
        push_new(&mut f.lines_added, &s.lines_added);
        push_new(&mut f.lines_removed, &s.lines_removed);
        push_new(&mut f.through, &s.code);
        at_b.extend(s.added.iter().cloned());
        at_a.extend(s.removed.iter().cloned());
        for id in s.code.iter().chain(&s.ends) {
            if has(b, id) {
                at_b.insert(id.clone());
            }
            let old = back.get(id.as_str()).copied().unwrap_or(id);
            if has(a, old) {
                at_a.insert(old.to_string());
            }
        }
    }
    if at_a.is_empty() && at_b.is_empty() {
        return None;
    }
    let m = picture_of(a, b, &moves, &at_a, &at_b)?;
    Some((m, f))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{ChangedItem, GraphDiff};

    fn shop() -> Graph {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../docair-adapters/tests/golden/shop.graph.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    /// The shop before it wrote the order, and the change as `changes` would find it.
    fn a_new_write() -> (Graph, Graph, Changes) {
        let after = shop();
        let mut before = after.clone();
        let at = before
            .edges
            .iter()
            .position(|e| e.from == "orders.create" && e.to == "orders-db")
            .unwrap();
        let line = before.edges.remove(at).id;
        // Writing the order is a new tag of `orders.create`: its tags changed.
        let graph = GraphDiff {
            lines_added: vec![line],
            changed: vec![ChangedItem {
                id: "orders.create".into(),
                what: vec!["tags".into()],
            }],
            ..GraphDiff::default()
        };
        let lines: Vec<crate::changes::LinesTouched> = Vec::new();
        let changes = crate::changes::changes(&before, &after, graph, &lines, ("A", "B"));
        (before, after, changes)
    }

    /// docair-8l6.18: a change that shows up in several flows is one piece, which says which flows it
    /// touches; its id names no flow.
    #[test]
    fn a_change_in_several_flows_is_one_piece() {
        let (before, after, changes) = a_new_write();
        assert!(
            changes.flows.len() >= 2,
            "{:?}",
            changes.flows.iter().map(|f| &f.seed).collect::<Vec<_>>()
        );
        let ps = pieces(&before, &after, &changes);
        let writes: Vec<&Piece> = ps
            .iter()
            .filter(|p| {
                p.step
                    .lines_added
                    .iter()
                    .any(|l| l.contains("orders.create") && l.contains("orders-db"))
            })
            .collect();
        assert_eq!(writes.len(), 1, "{ps:#?}");
        assert!(
            !writes[0].id().contains('#'),
            "the id names no flow: {}",
            writes[0].id()
        );
        assert!(
            writes[0].flows.len() >= 2,
            "the flows it touches: {:?}",
            writes[0].flows
        );
    }

    /// docair-8l6.18: a changed part no flow goes through is a piece of its box; nothing is missing.
    #[test]
    fn a_part_no_flow_goes_through_is_a_piece_of_its_box() {
        let after = shop();
        let mut before = after.clone();
        before.nodes.retain(|n| n.id != "orders.list");
        before
            .edges
            .retain(|e| e.from != "orders.list" && e.to != "orders.list");
        let graph = GraphDiff {
            added: vec!["orders.list".into()],
            changed: vec![ChangedItem {
                id: "shop-ui.cart.total".into(),
                what: vec!["body".into()],
            }],
            ..GraphDiff::default()
        };
        let changes = Changes {
            schema: crate::changes::SCHEMA.into(),
            a: "A".into(),
            b: "B".into(),
            working_tree: false,
            graph,
            parts: vec![],
            flows: vec![],
        };
        let ps = pieces(&before, &after, &changes);
        let all: HashSet<&String> = ps.iter().flat_map(|p| p.step.parts()).collect();
        assert!(all.contains(&"orders.list".to_string()), "{ps:#?}");
        assert!(all.contains(&"shop-ui.cart.total".to_string()), "{ps:#?}");
        assert!(ps.iter().all(|p| p.flows.is_empty()));
    }

    /// docair-8l6.19: the words of a doc block that changed, before and after.
    #[test]
    fn words_that_changed_are_read_before_and_after() {
        let before = shop();
        let mut after = before.clone();
        for n in &mut after.nodes {
            if n.id == "orders.create" {
                n.summary = "Store a new order, checked first.".into();
            }
        }
        let changes = Changes {
            schema: crate::changes::SCHEMA.into(),
            a: "A".into(),
            b: "B".into(),
            working_tree: false,
            graph: GraphDiff {
                changed: vec![ChangedItem {
                    id: "orders.create".into(),
                    what: vec!["summary".into()],
                }],
                ..GraphDiff::default()
            },
            parts: vec![],
            flows: vec![],
        };
        let w = words_changed(&before, &after, &changes);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].part, "orders.create");
        assert!(w[0].after.contains("checked first") && !w[0].before.contains("checked"));
        assert_eq!(cut("abcdef", 3), "abc…");
    }

    /// docair-8l6.41: an other edit goes with the change that has the most pieces in its module;
    /// one in a module no change has, or two changes have alike, stays an other edit.
    #[test]
    fn other_edits_go_with_the_change_of_their_module() {
        let piece = |id: &str, code: &[&str]| Piece {
            step: ChangeStep {
                id: id.into(),
                kind: StepKind::Code,
                added: vec![],
                removed: vec![],
                code: code.iter().map(|c| c.to_string()).collect(),
                ends: vec![],
                lines_added: vec![],
                lines_removed: vec![],
                lines: vec![],
                in_box: None,
            },
            flows: vec![],
        };
        let pieces = vec![
            piece("a1", &["x.review.pieces"]),
            piece("a2", &["x.review.intent"]),
            piece("b1", &["x.diff.loose"]),
            piece("o1", &["x.review.cut"]),
            piece("o2", &["x.diff.exact"]),
            piece("o3", &["x.server.job"]),
        ];
        let held = vec![
            vec!["a1".to_string(), "a2".to_string()],
            vec!["b1".to_string()],
        ];
        let other: Vec<String> = ["o1", "o2", "o3"].iter().map(|s| s.to_string()).collect();
        let (also, rest) = attach_by_module(&pieces, &held, &other);
        assert_eq!(also, vec![vec!["o1".to_string()], vec!["o2".to_string()]]);
        assert_eq!(rest, ["o3"], "a module no change has: still an other edit");
    }

    /// docair-8l6.33: a change's picture holds its pieces' parts, and marks the new line.
    #[test]
    fn a_change_is_drawn_from_its_pieces_only() {
        let (before, after, changes) = a_new_write();
        let ps = pieces(&before, &after, &changes);
        let write = ps
            .iter()
            .find(|p| p.step.lines_added.iter().any(|l| l.contains("orders-db")))
            .unwrap();
        let (m, f) = change_picture(&before, &after, &changes, "review/1", &[write]).unwrap();
        let ids: HashSet<&str> = m.pic.parts.iter().map(|p| p.id.as_str()).collect();
        for p in write.step.parts() {
            assert!(ids.contains(p.as_str()), "{p}: {ids:?}");
        }
        let marked = change_steps::marks(&m, &f, &HashSet::new());
        assert!(
            marked
                .lines
                .values()
                .any(|w| *w == change_steps::LineMark::Added)
        );
        assert!(
            m.pic.parts.len()
                < change_steps::flow_picture(&after, "data.order")
                    .unwrap()
                    .parts
                    .len()
                    + 5
        );
    }
}

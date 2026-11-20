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

/// A changed part's text goes to the AI cut at this many characters a version (docair-8l6.47).
pub const MAX_PART_TEXT: usize = 4_000;
/// A piece's parts' texts go cut at this many characters together, then the rest is counted.
pub const MAX_PIECE_TEXT: usize = 20_000;
/// A changed file no part covers goes as its changed lines, cut at this many characters a file.
pub const MAX_FILE: usize = 2_000;
/// All such files together go cut at this many characters.
pub const MAX_FILES: usize = 10_000;
/// A commit message's body goes to the AI cut at this many characters (docs/EXPLAIN.md §9.2).
pub const MAX_BODY: usize = 2_000;
/// A Markdown file's changed lines go cut at this many characters.
pub const MAX_DOC: usize = 2_000;
/// All Markdown files' changed lines together go cut at this many characters.
pub const MAX_DOCS: usize = 12_000;

/// The kinds of what the AI found off (docs/EXPLAIN.md §9.1, docair-8l6.62), in the order a reader sees them,
/// each with its heading: the clear contradictions first, then what nothing mentions, then the risks.
pub const OFF_KINDS: [(&str, &str); 3] = [
    ("contradicts", "Said, but the code does otherwise"),
    ("unsaid", "Done, but said nowhere"),
    ("risk", "Looks wrong or risky"),
];

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

/// A changed part's whole text, its doc block and comments with it, before and after (docair-8l6.47): none at A for a new part, none at B for a gone one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartText {
    pub part: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

/// What was meant, beside what changed (docs/EXPLAIN.md §9.2): the commit messages, the changed docs, the doc blocks whose words changed; and all of what changed, to read: each changed part's whole text, and the files no part covers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intent {
    pub commits: Vec<CommitNote>,
    /// B is the working tree: what is not committed has no message.
    #[serde(default)]
    pub uncommitted: bool,
    pub docs: Vec<DocChange>,
    pub words: Vec<WordsChange>,
    /// Every new, gone and changed part's text before and after (docair-8l6.47).
    #[serde(default)]
    pub code: Vec<PartText>,
    /// The files that changed but hold no part, as their changed lines: `file:<path>`.
    #[serde(default)]
    pub files: Vec<DocChange>,
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

/// Is this line a comment, a doc line or an attribute that belongs to the part below it?
fn leads_into(line: &str) -> bool {
    let t = line.trim_start();
    [
        "///", "//!", "//", "/*", "*", "#[", "#!", "@", "<!--", "-->", "#",
    ]
    .iter()
    .any(|k| t.starts_with(k))
}

/***
 * A part's whole text in a file (docair-8l6.47): its source range, from its doc block (or its first line) to its last, with the comments, doc lines and attributes right above it; none without a known end.
 ***/
pub fn part_text(text: &str, node: &crate::model::Node) -> Option<String> {
    let src = node.source.as_ref()?;
    let end = src.end?;
    let lines: Vec<&str> = text.lines().collect();
    if src.line == 0 || end > lines.len() || end < src.line {
        return None;
    }
    let mut start = src.line - 1;
    while start > 0
        && src.line - start < 40
        && leads_into(lines[start - 1])
        && !lines[start - 1].trim().is_empty()
    {
        start -= 1;
    }
    Some(lines[start..end].join("\n"))
}

/***
 * Every new, gone and changed part's text before and after (docair-8l6.47), the files read with `read_a` and `read_b` (a path relative to the project → its text at A, at B): a gone part at A only, a new part at B only, a moved one under its id at B with its text at A; each version cut at `MAX_PART_TEXT`.
 ***/
pub fn part_texts(
    a: &Graph,
    b: &Graph,
    changes: &Changes,
    read_a: &dyn Fn(&str) -> Option<String>,
    read_b: &dyn Fn(&str) -> Option<String>,
) -> Vec<PartText> {
    let g = &changes.graph;
    let back: HashMap<&str, &str> = g
        .moved
        .iter()
        .map(|m| (m.to.as_str(), m.from.as_str()))
        .collect();
    let mut files_a: HashMap<String, Option<String>> = HashMap::new();
    let mut files_b: HashMap<String, Option<String>> = HashMap::new();
    let at = |graph: &Graph,
              id: &str,
              files: &mut HashMap<String, Option<String>>,
              read: &dyn Fn(&str) -> Option<String>| {
        let n = graph.nodes.iter().find(|n| n.id == id)?;
        let file = n.source.as_ref()?.file.clone();
        let text = files.entry(file.clone()).or_insert_with(|| read(&file));
        text.as_deref()
            .and_then(|t| part_text(t, n))
            .map(|t| cut(&t, MAX_PART_TEXT))
    };
    let mut ids: Vec<&str> = g
        .added
        .iter()
        .chain(&g.removed)
        .chain(g.changed.iter().map(|c| &c.id))
        .chain(g.moved.iter().map(|m| &m.to))
        .map(String::as_str)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let system = [a.system.as_str(), b.system.as_str()];
    let mut out = Vec::new();
    for id in ids.into_iter().filter(|id| !system.contains(id)) {
        let old = back.get(id).copied().unwrap_or(id);
        let before = at(a, old, &mut files_a, read_a);
        let after = at(b, id, &mut files_b, read_b);
        if before.is_some() || after.is_some() {
            out.push(PartText {
                part: id.to_string(),
                before,
                after,
            });
        }
    }
    out
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

    /// docair-8l6.47: a part's whole text: its range and the comments, doc lines and attributes
    /// right above it, not the blank line or the code before those.
    #[test]
    fn a_parts_text_holds_its_comments() {
        let text = "fn other() {}\n\n/// Total the cart.\n/// In cents.\n#[inline]\npub fn total() -> u32 {\n    // every item costs 100\n    1 * 100\n}\nfn after() {}\n";
        let mut n: crate::model::Node = serde_json::from_value(serde_json::json!({
            "id": "x.total", "kind": "function", "name": "total", "summary": "", "description": "",
            "source": { "file": "src/lib.rs", "line": 6, "end": 9 }, "docs": [], "tags": [], "origin": "discovered"
        }))
        .unwrap();
        let t = part_text(text, &n).unwrap();
        assert!(
            t.starts_with("/// Total the cart.")
                && t.contains("#[inline]")
                && t.contains("// every item costs 100"),
            "{t}"
        );
        assert!(!t.contains("fn other") && !t.contains("fn after"), "{t}");
        n.source.as_mut().unwrap().end = None;
        assert_eq!(part_text(text, &n), None, "no end, no text");
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

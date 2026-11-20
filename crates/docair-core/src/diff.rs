//! What two graphs of a project differ in: parts and lines added, removed and changed, and the
//! parts that only moved or were renamed (docs/CHANGES.md §3).
//!
//! Parts are compared by id and lines by their ids (docair-svh). A part gone at B and a part new
//! at B with the same body (its fingerprint) are one part that moved: its lines are compared as
//! if it had kept its id, so a rename alone changes no line.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::model::{Edge, Graph, Node, Origin};

/// The fingerprint of a part's body: its code from its first code line to its end, without the
/// doc block above it, comments, spacing, and the names of the graph's parts (`names`: `fn load`
/// and `fn read` with the same body are the same, and so is a caller of a renamed part; which
/// part it calls is in its lines). None for a part without an end line or code.
pub fn fingerprint(
    text: &str,
    node: &Node,
    names: &std::collections::HashSet<String>,
) -> Option<String> {
    let source = node.source.as_ref()?;
    let end = source.end?;
    let lines: Vec<&str> = text.lines().collect();
    if source.line == 0 || end > lines.len() || end < source.line {
        return None;
    }
    let mut body: Vec<&str> = lines[source.line - 1..end].to_vec();
    // The doc block above the code is the part's words, not its body.
    if let Some(first) = body.first().map(|l| l.trim_start()) {
        let close = if first.starts_with("/***") {
            Some("***/")
        } else if first.starts_with("#***") {
            Some("#***")
        } else if first.starts_with("<!--***") {
            Some("***-->")
        } else if first.starts_with("{{/***") {
            Some("***/}}")
        } else {
            None
        };
        if let Some(close) = close {
            let at = body
                .iter()
                .skip(1)
                .position(|l| l.contains(close))
                .map(|k| k + 2);
            body = at.map_or(Vec::new(), |k| body[k.min(body.len())..].to_vec());
        }
    }
    let name = node.name.as_str();
    // Two hashes: `loose` leaves out the names of the graph's parts, so a part that moved or was
    // renamed, or calls one that was, keeps it; `exact` keeps every word. A part that kept its id
    // changed its body only when both differ (docair-zrr.29).
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut e: u64 = 0xcbf2_9ce4_8422_2325;
    let mut words = 0usize;
    for l in body {
        let t = l.trim();
        if t.is_empty()
            || t.starts_with("//")
            || t.starts_with('#')
            || t.starts_with("/*")
            || t.starts_with('*')
        {
            continue;
        }
        for w in t
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|w| !w.is_empty())
        {
            for b in w.bytes().chain([0]) {
                e ^= b as u64;
                e = e.wrapping_mul(0x0100_0000_01b3);
            }
            let w = if (!name.is_empty() && w == name) || names.contains(w) {
                "\u{1}"
            } else {
                w
            };
            for b in w.bytes().chain([0]) {
                h ^= b as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            words += 1;
        }
        // The punctuation too, so `a + b` and `a - b` differ.
        for c in t
            .chars()
            .filter(|c| !c.is_alphanumeric() && !c.is_whitespace() && *c != '_')
        {
            for b in c.to_string().bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
                e ^= b as u64;
                e = e.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    (words > 0).then(|| format!("{h:016x}/{e:016x}"))
}

/// A fingerprint's hash for finding moves: the names of the graph's parts left out.
pub fn loose(fp: &str) -> &str {
    fp.split('/').next().unwrap_or(fp)
}

/// A fingerprint's hash of every word: the same text is the same body, whatever the graph's names.
pub fn exact(fp: &str) -> &str {
    fp.rsplit('/').next().unwrap_or(fp)
}

/***
 * The fingerprints of a graph's parts, their files read with `read` (a path relative to the root → its text).
 *
 * @id docair-core.diff.fingerprints
 ***/
pub fn fingerprints(
    graph: &Graph,
    read: &dyn Fn(&str) -> Option<String>,
) -> HashMap<String, String> {
    let names: std::collections::HashSet<String> = graph
        .nodes
        .iter()
        .map(|n| n.name.clone())
        .filter(|n| n.len() > 1)
        .collect();
    let mut texts: HashMap<String, Option<String>> = HashMap::new();
    let mut out = HashMap::new();
    for n in &graph.nodes {
        let Some(src) = n.source.as_ref().filter(|s| s.end.is_some()) else {
            continue;
        };
        let text = texts
            .entry(src.file.clone())
            .or_insert_with(|| read(&src.file));
        if let Some(fp) = text.as_deref().and_then(|t| fingerprint(t, n, &names)) {
            out.insert(n.id.clone(), fp);
        }
    }
    out
}

/// A part at A that is at B under another id, with the same body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveMatch {
    pub from: String,
    pub to: String,
}

/// What changed in a part that is at A and at B: `body`, `summary`, `tags`, `kind`, `name`, `part_of`, `route`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedItem {
    pub id: String,
    pub what: Vec<String>,
}

/// What two graphs differ in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphDiff {
    /// docair or its settings differ: a difference may come from the reading, not the code.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reading: Vec<String>,
    /// The lines of docair.toml that changed from A to B, each with `-` or `+` in front: each commit is
    /// read with its own, so these are shown first (docair-zrr.31).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub toml: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<ChangedItem>,
    pub moved: Vec<MoveMatch>,
    pub lines_added: Vec<String>,
    pub lines_removed: Vec<String>,
    pub lines_changed: Vec<ChangedItem>,
}

impl GraphDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.changed.is_empty()
            && self.moved.is_empty()
            && self.lines_added.is_empty()
            && self.lines_removed.is_empty()
            && self.lines_changed.is_empty()
    }
}

/// A part's tags as authors wrote them, by name and value (what the code says is in its lines).
fn written_tags(n: &Node) -> BTreeSet<(String, String)> {
    n.tags
        .iter()
        .filter(|t| t.origin == Origin::Authored && t.name != "id")
        .map(|t| {
            (
                t.name.clone(),
                t.value.split_whitespace().collect::<Vec<_>>().join(" "),
            )
        })
        .collect()
}

/// What differs in a part between A and B.
fn part_changes(a: &Node, b: &Node, fa: Option<&String>, fb: Option<&String>) -> Vec<String> {
    let mut what = Vec::new();
    // Its body changed when its text did and more than the names of parts in it (docair-zrr.29):
    // the same text is no change, nor is a caller of a part that was renamed.
    let differ = |h: fn(&str) -> &str| fa.map(|f| h(f)) != fb.map(|f| h(f));
    if differ(exact) && differ(loose) {
        what.push("body".to_string());
    }
    if a.kind != b.kind {
        what.push("kind".into());
    }
    if a.name != b.name {
        what.push("name".into());
    }
    if a.summary.trim() != b.summary.trim() || a.description.trim() != b.description.trim() {
        what.push("summary".into());
    }
    if a.part_of != b.part_of {
        what.push("part_of".into());
    }
    if a.http != b.http {
        what.push("route".into());
    }
    if written_tags(a) != written_tags(b) {
        what.push("tags".into());
    }
    what
}

/// What differs in a line between A and B.
fn line_changes(a: &Edge, b: &Edge) -> Vec<String> {
    let mut what = Vec::new();
    if a.payload != b.payload || a.reply != b.reply {
        what.push("carries".to_string());
    }
    if a.label != b.label || a.text != b.text {
        what.push("words".into());
    }
    if a.via != b.via {
        what.push("via".into());
    }
    if a.reach != b.reach {
        what.push("reach".into());
    }
    what
}

/***
 * Compare two graphs of a project: parts by id, lines by id, and a part gone at A and new at B with the same fingerprint as one part that moved. `fa` and `fb` are the parts' fingerprints at A and at B (`fingerprint`).
 *
 * @id docair-core.diff.diff
 ***/
pub fn diff(
    a: &Graph,
    b: &Graph,
    fa: &HashMap<String, String>,
    fb: &HashMap<String, String>,
) -> GraphDiff {
    let mut out = GraphDiff::default();
    if a.generator != b.generator {
        out.reading.push(format!(
            "docair: {} at A, {} at B",
            a.generator, b.generator
        ));
    }
    let at_a: BTreeMap<&str, &Node> = a.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let at_b: BTreeMap<&str, &Node> = b.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut removed: Vec<&str> = at_a
        .keys()
        .filter(|id| !at_b.contains_key(*id))
        .copied()
        .collect();
    let mut added: Vec<&str> = at_b
        .keys()
        .filter(|id| !at_a.contains_key(*id))
        .copied()
        .collect();
    // Moves: the same body, gone at A and new at B; each fingerprint matched once.
    let mut moved: HashMap<String, String> = HashMap::new();
    for r in removed.clone() {
        let Some(fp) = fa.get(r).map(|f| loose(f)) else {
            continue;
        };
        let candidates: Vec<&str> = added
            .iter()
            .copied()
            .filter(|x| fb.get(*x).map(|f| loose(f)) == Some(fp))
            .collect();
        if let [one] = candidates.as_slice() {
            moved.insert(r.to_string(), one.to_string());
            out.moved.push(MoveMatch {
                from: r.to_string(),
                to: one.to_string(),
            });
            added.retain(|x| x != one);
            removed.retain(|x| *x != r);
        }
    }
    out.added = added.iter().map(|s| s.to_string()).collect();
    out.removed = removed.iter().map(|s| s.to_string()).collect();
    for (id, na) in &at_a {
        let (bid, nb) = match moved.get(*id) {
            Some(to) => (to.as_str(), at_b[to.as_str()]),
            None => match at_b.get(id) {
                Some(nb) => (*id, *nb),
                None => continue,
            },
        };
        let mut what = part_changes(na, nb, fa.get(*id), fb.get(bid));
        if moved.contains_key(*id) {
            // A move keeps its body; its id and name are what moved.
            what.retain(|w| {
                !matches!(w.as_str(), "body" | "name" | "part_of" | "tags" | "summary")
            });
        }
        if !what.is_empty() {
            out.changed.push(ChangedItem {
                id: bid.to_string(),
                what,
            });
        }
    }
    // Lines: A's ids as they would read with the moved parts' new ids.
    let mut a_lines: Vec<Edge> = a.edges.clone();
    for e in &mut a_lines {
        if let Some(to) = moved.get(&e.from) {
            e.from = to.clone();
        }
        if let Some(to) = moved.get(&e.to) {
            e.to = to.clone();
        }
    }
    crate::ids::edge_ids(&mut a_lines);
    // A type's uses are not drawn, so they are no change of a picture (docs/ADAPTERS.md §5.8).
    let la: BTreeMap<&str, &Edge> = a_lines
        .iter()
        .filter(|e| e.kind.drawn())
        .map(|e| (e.id.as_str(), e))
        .collect();
    let lb: BTreeMap<&str, &Edge> = b
        .edges
        .iter()
        .filter(|e| e.kind.drawn())
        .map(|e| (e.id.as_str(), e))
        .collect();
    out.lines_removed = la
        .keys()
        .filter(|id| !lb.contains_key(*id))
        .map(|s| s.to_string())
        .collect();
    out.lines_added = lb
        .keys()
        .filter(|id| !la.contains_key(*id))
        .map(|s| s.to_string())
        .collect();
    for (id, ea) in &la {
        if let Some(eb) = lb.get(id) {
            let what = line_changes(ea, eb);
            if !what.is_empty() {
                out.lines_changed.push(ChangedItem {
                    id: id.to_string(),
                    what,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SourceRef;

    fn node(id: &str, name: &str, line: usize, end: usize) -> Node {
        serde_json::from_value(serde_json::json!({
            "id": id, "kind": "function", "name": name, "summary": "", "description": "", "part_of": "s",
            "source": { "file": "src/lib.rs", "line": line, "end": end }, "docs": [], "tags": [], "origin": "discovered"
        }))
        .unwrap()
    }

    #[test]
    fn a_body_keeps_its_fingerprint_when_renamed_or_moved() {
        let none = std::collections::HashSet::new();
        let a = "/***\n * Load it.\n * @id x.load\n ***/\npub fn load(p: &str) -> u8 {\n    // read it\n    p.len() as u8\n}\n";
        let b = "\n\n/***\n * Read it, renamed.\n * @id x.read\n ***/\npub fn read(p: &str) -> u8 {\n    p.len()   as u8\n}\n";
        let fa = fingerprint(a, &node("x.load", "load", 1, 8), &none).unwrap();
        let fb = fingerprint(b, &node("x.read", "read", 3, 9), &none).unwrap();
        assert_eq!(
            loose(&fa),
            loose(&fb),
            "the same body, renamed, moved down, comments and spacing aside"
        );
        let c = "pub fn load(p: &str) -> u8 {\n    p.len() as u8 + 1\n}\n";
        assert_ne!(
            fa,
            fingerprint(c, &node("x.load", "load", 1, 3), &none).unwrap(),
            "another body"
        );
        let mut no_end = node("x.load", "load", 1, 3);
        no_end.source = Some(SourceRef {
            file: "src/lib.rs".into(),
            line: 1,
            end: None,
        });
        assert_eq!(fingerprint(c, &no_end, &none), None);
    }

    /// docair-zrr.29: a body that did not change keeps its exact hash when the other graph has a
    /// new part named like one of its words (`routes`); only the loose hash, for moves, leaves names out.
    #[test]
    fn an_unchanged_body_does_not_change_when_a_part_of_its_words_name_is_added() {
        let text = "pub fn plan(routes: u8) -> u8 {\n    routes + 1\n}\n";
        let n = node("x.plan", "plan", 1, 3);
        let before = std::collections::HashSet::new();
        let after: std::collections::HashSet<String> = ["routes".to_string()].into_iter().collect();
        let (fa, fb) = (
            fingerprint(text, &n, &before).unwrap(),
            fingerprint(text, &n, &after).unwrap(),
        );
        assert_eq!(exact(&fa), exact(&fb), "the text is the same");
        assert_ne!(
            loose(&fa),
            loose(&fb),
            "the loose hash leaves the new name out"
        );
        let (pa, pb) = (n.clone(), n);
        assert!(!part_changes(&pa, &pb, Some(&fa), Some(&fb)).contains(&"body".to_string()));
    }
}

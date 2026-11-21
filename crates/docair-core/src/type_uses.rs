//! How a part uses a type, in the words of a `@uses` tag (docs/ADAPTERS.md §5.8).

use std::collections::{HashMap, HashSet};

use crate::model::{Edge, EdgeKind, Node};

/// One way a part uses a type, in the order the words name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TypeUse {
    Makes,
    Takes,
    Returns,
    Holds,
    Implements,
    Matches,
    /// A field read: named by the field.
    Reads,
    /// A field set: named by the field.
    Sets,
    /// Anything else: a `let`'s type, a cast, inside a macro.
    Uses,
}

/// Every way one part uses one type, and the fields it reads and sets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeUses {
    ways: Vec<TypeUse>,
    reads: Vec<String>,
    sets: Vec<String>,
}

impl TypeUses {
    /// One more use; `field` names the field read or set.
    pub fn add(&mut self, way: TypeUse, field: &str) {
        let list = match way {
            TypeUse::Reads => &mut self.reads,
            TypeUse::Sets => &mut self.sets,
            _ => {
                if !self.ways.contains(&way) {
                    self.ways.push(way);
                }
                return;
            }
        };
        if !list.iter().any(|f| f == field) {
            list.push(field.to_string());
        }
    }

    /// The words of the tag: `makes, reads command`; "uses" only when nothing says more.
    pub fn words(&self) -> String {
        let mut ways = self.ways.clone();
        ways.sort();
        if ways.len() > 1 || !self.reads.is_empty() || !self.sets.is_empty() {
            ways.retain(|w| *w != TypeUse::Uses);
        }
        let mut words: Vec<String> = ways
            .iter()
            .map(|w| {
                match w {
                    TypeUse::Makes => "makes",
                    TypeUse::Takes => "takes",
                    TypeUse::Returns => "returns",
                    TypeUse::Holds => "holds",
                    TypeUse::Implements => "implements",
                    TypeUse::Matches => "matches",
                    TypeUse::Reads | TypeUse::Sets | TypeUse::Uses => "uses",
                }
                .to_string()
            })
            .collect();
        if !self.reads.is_empty() {
            words.push(format!("reads {}", self.reads.join(", ")));
        }
        if !self.sets.is_empty() {
            words.push(format!("sets {}", self.sets.join(", ")));
        }
        words.join(", ")
    }
}

/// Whether a node is a type: a struct, an enum, a trait, a type alias, an interface or a class.
fn is_type(n: &Node) -> bool {
    n.symbol
        .as_deref()
        .and_then(|s| s.split_whitespace().next())
        .is_some_and(|w| {
            matches!(
                w,
                "struct" | "enum" | "union" | "trait" | "type" | "interface" | "class"
            )
        })
}

/***
 * Mark the uses another line already brings to their part as listed, so only the others are drawn (docair-4wk.43, docs/ADAPTERS.md §5.8).
 *
 * A use of type T by part f is brought when T, or a type that holds T, is
 * in f's own signature (f takes or returns it), is f's own type (f is one
 * of its methods), or is in the signature of a part f has a call or a data
 * line with. A use between two types is listed too: it is drawn in the
 * type's own section (docair-4wk.44). The rest, a value no line brings
 * (`static ALL` makes the adapters, `main` makes `Cli` by `Cli::parse()`), is
 * drawn.
 ***/
pub fn list_brought(nodes: &[Node], edges: &mut [Edge]) {
    let index: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let typed = |id: &str| index.get(id).is_some_and(|n| is_type(n));
    let words = |e: &Edge| -> Vec<String> {
        e.payload
            .as_deref()
            .unwrap_or("")
            .split(", ")
            .map(|w| w.split(' ').next().unwrap_or("").to_string())
            .collect()
    };
    // What each type holds, and what each part's signature carries.
    let mut holds: HashMap<String, Vec<String>> = HashMap::new();
    let mut signature: HashMap<String, Vec<String>> = HashMap::new();
    for e in edges.iter().filter(|e| e.kind == EdgeKind::Use) {
        if typed(&e.from) {
            holds.entry(e.from.clone()).or_default().push(e.to.clone());
        } else if words(e).iter().any(|w| w == "takes" || w == "returns") {
            signature
                .entry(e.from.clone())
                .or_default()
                .push(e.to.clone());
        }
    }
    let reach = |start: &[String]| -> HashSet<String> {
        let mut seen: HashSet<String> = start.iter().cloned().collect();
        let mut todo: Vec<String> = start.to_vec();
        while let Some(t) = todo.pop() {
            for u in holds.get(&t).into_iter().flatten() {
                if seen.insert(u.clone()) {
                    todo.push(u.clone());
                }
            }
        }
        seen
    };
    // The parts each part has a call or a data line with.
    let mut peers: HashMap<String, HashSet<String>> = HashMap::new();
    for e in edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Call | EdgeKind::Data))
    {
        peers
            .entry(e.from.clone())
            .or_default()
            .insert(e.to.clone());
        peers
            .entry(e.to.clone())
            .or_default()
            .insert(e.from.clone());
    }
    let mut brought: HashMap<String, HashSet<String>> = HashMap::new();
    for e in edges.iter_mut().filter(|e| e.kind == EdgeKind::Use) {
        if typed(&e.from) {
            e.listed = true;
            continue;
        }
        let have = brought.entry(e.from.clone()).or_insert_with(|| {
            let mut start: Vec<String> = signature.get(&e.from).cloned().unwrap_or_default();
            if let Some(own) = index
                .get(e.from.as_str())
                .and_then(|n| n.part_of.as_deref())
                .filter(|p| typed(p))
            {
                start.push(own.to_string());
            }
            for p in peers.get(&e.from).into_iter().flatten() {
                start.extend(signature.get(p).cloned().unwrap_or_default());
            }
            reach(&start)
        });
        e.listed = have.contains(&e.to);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words name each way once, in a fixed order, the fields after them; "uses" only alone.
    #[test]
    fn the_words_say_each_way_once() {
        let mut u = TypeUses::default();
        u.add(TypeUse::Reads, "command");
        u.add(TypeUse::Uses, "");
        u.add(TypeUse::Makes, "");
        u.add(TypeUse::Reads, "command");
        u.add(TypeUse::Sets, "end");
        u.add(TypeUse::Makes, "");
        assert_eq!(u.words(), "makes, reads command, sets end");
        let mut alone = TypeUses::default();
        alone.add(TypeUse::Uses, "");
        assert_eq!(alone.words(), "uses");
    }
}

//! How a part uses a type, in the words of a `@uses` tag (docs/ADAPTERS.md §5.8).

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

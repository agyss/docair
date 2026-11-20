//! Doc blocks meet the facts the adapters read (docs/ADAPTERS.md §6).

use std::collections::HashMap;

use crate::facts::{Draw, Facts, Meet, Reach, Relation, Request, Serve, Symbol, Target};
use crate::model::{Diagnostic, Following, Hints, Origin, RawBlock, SourceFile, Tag};

/// The routes and relations of each symbol, by symbol key.
struct ByKey<'a> {
    serves: HashMap<&'a str, Vec<&'a Serve>>,
    requests: HashMap<&'a str, Vec<&'a Request>>,
    relations: HashMap<&'a str, Vec<&'a Relation>>,
    /// The id of the node each drawn symbol became.
    ids: HashMap<&'a str, String>,
}

impl<'a> ByKey<'a> {
    fn new(facts: &'a [Facts]) -> Self {
        let mut serves: HashMap<&str, Vec<&Serve>> = HashMap::new();
        let mut requests: HashMap<&str, Vec<&Request>> = HashMap::new();
        let mut relations: HashMap<&str, Vec<&Relation>> = HashMap::new();
        for f in facts {
            for s in &f.serves {
                serves.entry(s.by.as_str()).or_default().push(s);
            }
            for r in &f.requests {
                requests.entry(r.by.as_str()).or_default().push(r);
            }
            for r in &f.relations {
                relations.entry(r.from.as_str()).or_default().push(r);
            }
        }
        ByKey {
            serves,
            requests,
            relations,
            ids: HashMap::new(),
        }
    }

    /// What the code says a symbol serves and uses, as inferred tags: routes first, then relations.
    ///
    /// A relation to a symbol that became no node is left out.
    fn tags(&self, key: &str) -> Vec<Tag> {
        let serves = self.serves.get(key).into_iter().flatten().filter_map(|s| {
            let e = &s.evidence;
            match &s.at {
                Meet::Http(_) => Some(Tag::inferred("http", &s.at.to_string(), e.line, &e.why)),
                // Code that serves a topic is where its messages arrive.
                Meet::Topic(_) => {
                    Some(Tag::inferred("consumes", &s.at.to_string(), e.line, &e.why))
                }
                // What sets a variable shows as the lines to the code that reads it (env::link).
                Meet::Env(_) => None,
            }
        });
        let relations = self
            .relations
            .get(key)
            .into_iter()
            .flatten()
            .filter_map(|r| {
                let e = &r.evidence;
                let to = match &r.to {
                    Target::Ref(id) => id.clone(),
                    Target::Symbol(k) => self.ids.get(k.as_str())?.clone(),
                };
                let mut tag = Tag::inferred(r.verb.tag(), &r.value_to(&to), e.line, &e.why);
                tag.reach = (r.reach != Reach::Certain).then_some(r.reach);
                Some(tag)
            });
        // Requests show as `@requests` until step 6 makes them lines (docs/ADAPTERS.md §7).
        let requests = self.requests.get(key).into_iter().flatten().map(|r| {
            let e = &r.evidence;
            let payload = r
                .payload
                .as_deref()
                .map(|p| format!(" ({p})"))
                .unwrap_or_default();
            let reply = r
                .reply
                .as_deref()
                .map(|t| format!(" -> {t}"))
                .unwrap_or_default();
            Tag::inferred(
                "requests",
                &format!("{}{payload}{reply}", r.at),
                e.line,
                &e.why,
            )
        });
        serves.chain(requests).chain(relations).collect()
    }
}

fn hints_of(s: &Symbol) -> Hints {
    Hints {
        symbol: s.display.clone(),
        name: s.name.clone(),
        id: s.id.clone(),
        part_of: s.parents.clone(),
        parent_line: s.parent_line,
        declares_file: s.declares_file.clone(),
        signature: s.signature.clone(),
        end: s.end,
    }
}

fn kind_tag(s: &Symbol) -> Option<Tag> {
    s.kind
        .as_ref()
        .map(|k| Tag::inferred("kind", &k.value, s.line, &k.why))
}

/// A doc comment as summary (its first paragraph, on one line) and description (the rest).
fn summary_and_description(docs: &str) -> (String, String) {
    let docs = docs.trim();
    let (first, rest) = docs
        .split_once("\n\n")
        .map_or((docs, ""), |(a, b)| (a, b.trim()));
    let summary = first.split_whitespace().collect::<Vec<_>>().join(" ");
    (summary, rest.to_string())
}

/// The first word of a symbol, for an id: `fn`, `struct`, `static`.
fn keyword(s: &Symbol) -> String {
    let word = s
        .display
        .as_deref()
        .and_then(|d| d.split_whitespace().next())
        .unwrap_or("item");
    crate::ids::slug(word.trim_end_matches('!'))
}

/// Whether a symbol is a type: a struct, an enum, a trait, a type alias, an interface or a class.
fn is_type(s: &Symbol) -> bool {
    matches!(
        keyword(s).as_str(),
        "struct" | "enum" | "union" | "trait" | "type" | "interface" | "class"
    )
}

/***
 * New ids for code items that would share one with another item of their file (docair-4wk.38).
 *
 * `static ALL` and `fn all` both slug to `all`; a block may give a method the
 * id of its type (`@id docair-server.config` on `Config::from_env`). The one
 * a block names keeps the id, else a type, else the first; the other gets
 * its kind after it (`all-static`), or a number when both are of one kind
 * (`foo-2`). Before, the other was left out with its lines.
 ***/
fn file_ids<'a>(blocks: &[RawBlock], p: &Placed<'a>) -> HashMap<&'a str, String> {
    let mut taken: HashMap<String, &str> = HashMap::new();
    let mut renamed: HashMap<&'a str, String> = HashMap::new();
    for (b, syms) in blocks.iter().zip(&p.attached) {
        let identity = syms.iter().copied().find(|s| s.has_identity());
        if let (Some(id), Some(s)) = (block_id(b, identity), syms.first()) {
            taken.entry(id).or_insert(&s.key);
        }
    }
    let mut alone: Vec<&Symbol> = p.alone.clone();
    alone.sort_by_key(|s| !is_type(s));
    for s in alone {
        let Some(id) = &s.id else {
            continue;
        };
        let Some(&first) = taken.get(id.as_str()) else {
            taken.insert(id.clone(), &s.key);
            continue;
        };
        // A file's module stands for the node a block declares with its id (a TypeScript file).
        if first == s.key || s.draw == Draw::Alone {
            continue;
        }
        let same_kind = p
            .alone
            .iter()
            .chain(p.attached.iter().flatten())
            .find(|o| o.key == first)
            .is_some_and(|o| keyword(o) == keyword(s));
        let mut new = if same_kind {
            format!("{id}-2")
        } else {
            format!("{id}-{}", keyword(s))
        };
        let mut n = 2;
        while taken.contains_key(&new) {
            n += 1;
            new = format!("{id}-{n}");
        }
        taken.insert(new.clone(), &s.key);
        renamed.insert(s.key.as_str(), new);
    }
    renamed
}

/// A node that no block documents, as a block of its own.
fn discovered(s: &Symbol, by_key: &ByKey) -> RawBlock {
    let mut tags: Vec<Tag> = kind_tag(s).into_iter().collect();
    tags.extend(by_key.tags(&s.key));
    let (summary, description) = summary_and_description(s.docs.as_deref().unwrap_or(""));
    RawBlock {
        file: s.file.clone(),
        line: s.line,
        end_line: s.line,
        summary,
        description,
        tags,
        following: Some(Following {
            line: s.line,
            text: String::new(),
        }),
        hints: Hints {
            id: by_key.ids.get(s.key.as_str()).cloned().or(s.id.clone()),
            ..hints_of(s)
        },
        origin: Origin::Discovered,
    }
}

/// Where the symbols of one file go: to a block, to a node of their own, or nowhere.
struct Placed<'a> {
    /// For each block of the file, the symbols it documents.
    attached: Vec<Vec<&'a Symbol>>,
    /// Symbols that become nodes of their own, in order.
    alone: Vec<&'a Symbol>,
}

fn place<'a>(blocks: &[RawBlock], symbols: Vec<&'a Symbol>) -> Placed<'a> {
    let mut attached: Vec<Vec<&Symbol>> = vec![Vec::new(); blocks.len()];
    let mut alone = Vec::new();
    for s in symbols {
        let at = (s.draw != Draw::Alone)
            .then(|| {
                blocks.iter().position(|b| {
                    b.following
                        .as_ref()
                        .is_some_and(|f| s.attach_from <= f.line && f.line <= s.line)
                })
            })
            .flatten();
        match (at, s.draw) {
            (Some(i), _) => attached[i].push(s),
            (None, Draw::Always | Draw::Alone) => alone.push(s),
            (None, Draw::Documented) => {}
        }
    }
    Placed { attached, alone }
}

/// The id a block's node will have: its authored `@id`, else the id the code gives it.
fn block_id(b: &RawBlock, identity: Option<&Symbol>) -> Option<String> {
    b.tags
        .iter()
        .find(|t| t.name == "id" && t.origin == Origin::Authored)
        .map(|t| t.value.clone())
        .or_else(|| identity.and_then(|s| s.id.clone()))
}

/***
 * Attach each doc block to the symbols it documents and add what the code says; symbols drawn without a block become blocks of their own.
 *
 * A block documents a symbol when its code line lies between the symbol's
 * `attach_from` and its line. What the symbol is (symbol, name, id, kind,
 * parents) comes from the first adapter that says it; the block's text wins
 * over the code's own comments. The routes and relations of every attached
 * symbol follow, in adapter order. Inferred tags never replace authored ones:
 * build and resolve prefer what the author wrote. Symbols drawn without a
 * block (`Draw::Always`, `Draw::Alone`) become discovered blocks after the
 * file's own blocks.
 *
 * @flow  scan 8 "blocks meet the code"
 * @in    facts: Facts[] from docair-core.adapter.collect
 * @out   blocks: RawBlock[]
 ***/
pub fn merge(
    files: &[SourceFile],
    blocks: Vec<Vec<RawBlock>>,
    facts: &[Facts],
    diags: &mut Vec<Diagnostic>,
) -> Vec<Vec<RawBlock>> {
    let mut by_key = ByKey::new(facts);
    let mut by_file: HashMap<&str, Vec<&Symbol>> = HashMap::new();
    for f in facts {
        diags.extend(f.diagnostics.iter().cloned());
        for s in &f.symbols {
            by_file.entry(s.file.as_str()).or_default().push(s);
        }
    }

    // First where every symbol goes, so that relations can name the node a symbol became.
    let placed: Vec<Placed> = files
        .iter()
        .zip(&blocks)
        .map(|(file, file_blocks)| {
            let symbols = by_file.remove(file.rel.as_str()).unwrap_or_default();
            place(file_blocks, symbols)
        })
        .collect();
    // A type that got a new id takes its methods along: their parent and ids were its old id.
    let mut moved: HashMap<(&str, String), String> = HashMap::new();
    let mut renamed: HashMap<&str, String> = HashMap::new();
    for (file_blocks, p) in blocks.iter().zip(&placed) {
        let here = file_ids(file_blocks, p);
        for s in &p.alone {
            if let (Some(new), Some(old)) = (here.get(s.key.as_str()), &s.id)
                && is_type(s)
            {
                moved.insert((s.file.as_str(), old.clone()), new.clone());
            }
        }
        renamed.extend(here);
        for s in p.alone.iter().chain(p.attached.iter().flatten()) {
            let (Some(id), Some(parent)) = (&s.id, s.parents.first()) else {
                continue;
            };
            if let Some(new) = moved.get(&(s.file.as_str(), parent.clone()))
                && s.parent_line.is_some()
                && let Some(rest) = id.strip_prefix(&format!("{parent}."))
            {
                renamed.insert(s.key.as_str(), format!("{new}.{rest}"));
            }
        }
        for (b, syms) in file_blocks.iter().zip(&p.attached) {
            let identity = syms.iter().copied().find(|s| s.has_identity());
            let authored = b
                .tags
                .iter()
                .find(|t| t.name == "id" && t.origin == Origin::Authored)
                .map(|t| t.value.clone());
            let id = authored
                .or_else(|| identity.and_then(|s| renamed.get(s.key.as_str()).cloned()))
                .or_else(|| block_id(b, identity));
            if let Some(id) = id {
                for s in syms {
                    by_key.ids.insert(s.key.as_str(), id.clone());
                }
            }
        }
        for s in &p.alone {
            if let Some(id) = renamed.get(s.key.as_str()).or(s.id.as_ref()) {
                by_key.ids.insert(s.key.as_str(), id.clone());
            }
        }
    }
    let parents = |s: &Symbol| -> Vec<String> {
        let mut ps = s.parents.clone();
        if s.parent_line.is_some()
            && let Some(first) = ps.first_mut()
            && let Some(new) = moved.get(&(s.file.as_str(), first.clone()))
        {
            *first = new.clone();
        }
        ps
    };

    blocks
        .into_iter()
        .zip(placed)
        .map(|(mut file_blocks, p)| {
            for (b, syms) in file_blocks.iter_mut().zip(&p.attached) {
                if let Some(s) = syms.iter().find(|s| s.has_identity()) {
                    b.hints = Hints {
                        id: renamed.get(s.key.as_str()).cloned().or(s.id.clone()),
                        part_of: parents(s),
                        ..hints_of(s)
                    };
                    b.tags.extend(kind_tag(s));
                    if b.summary.is_empty()
                        && let Some(docs) = &s.docs
                    {
                        (b.summary, b.description) = summary_and_description(docs);
                    }
                }
                for s in syms {
                    b.tags.extend(by_key.tags(&s.key));
                }
            }
            file_blocks.extend(p.alone.iter().map(|s| {
                let mut b = discovered(s, &by_key);
                b.hints.part_of = parents(s);
                b
            }));
            file_blocks
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::{Evidence, Inferred, Verb};
    use crate::lang::language_for;
    use crate::model::Http;
    use std::path::PathBuf;

    fn file(rel: &str) -> SourceFile {
        SourceFile {
            path: PathBuf::from(rel),
            rel: rel.to_string(),
            lang: language_for("rs").unwrap(),
            text: String::new(),
        }
    }

    fn block(line: usize, code_line: usize) -> RawBlock {
        RawBlock {
            file: "a.rs".into(),
            line,
            end_line: line,
            summary: String::new(),
            description: String::new(),
            tags: vec![Tag::authored("http", "POST /a", line)],
            following: Some(Following {
                line: code_line,
                text: String::new(),
            }),
            hints: Hints::default(),
            origin: Origin::Authored,
        }
    }

    #[test]
    fn the_first_adapter_says_what_a_symbol_is_and_every_one_adds_its_relations() {
        let named = |key: &str, name: &str| Symbol {
            display: Some(format!("fn {name}")),
            name: Some(name.into()),
            kind: Some(Inferred::new("function", "test")),
            ..Symbol::at(key, "a.rs", 3)
        };
        let mut first = Facts {
            symbols: vec![named("one", "create"), Symbol::at("unattached", "a.rs", 9)],
            ..Facts::default()
        };
        first.serves.push(Serve {
            by: "one".into(),
            at: Meet::Http(Http {
                method: "POST".into(),
                path: "/b".into(),
            }),
            evidence: Evidence::new(3, "#[post]"),
        });
        let mut second = Facts {
            symbols: vec![
                named("two", "other"),
                Symbol {
                    draw: Draw::Alone,
                    id: Some("found".into()),
                    kind: Some(Inferred::new("config", "found alone")),
                    docs: Some("Found in the code.".into()),
                    ..Symbol::at("three", "a.rs", 7)
                },
            ],
            ..Facts::default()
        };
        second.relations.push(Relation {
            from: "two".into(),
            ..Relation::new(
                Verb::Calls,
                Target::Ref("x".into()),
                Evidence::new(3, "call"),
            )
        });

        let mut d = Vec::new();
        let out = merge(
            &[file("a.rs")],
            vec![vec![block(1, 3)]],
            &[first, second],
            &mut d,
        );
        let [b, found] = &out[0][..] else {
            panic!("{out:?}")
        };
        assert_eq!(b.hints.name.as_deref(), Some("create"));
        let tags: Vec<_> = b
            .tags
            .iter()
            .map(|t| (t.name.as_str(), t.value.as_str(), t.origin))
            .collect();
        assert_eq!(
            tags,
            [
                ("http", "POST /a", Origin::Authored),
                ("kind", "function", Origin::Inferred),
                ("http", "POST /b", Origin::Inferred),
                ("calls", "x", Origin::Inferred),
            ]
        );
        assert_eq!(found.origin, Origin::Discovered);
        assert_eq!(found.summary, "Found in the code.");
        assert_eq!(found.hints.id.as_deref(), Some("found"));
    }
}

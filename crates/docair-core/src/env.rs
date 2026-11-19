//! Environment variables: the code that reads one, and what sets it (docs/ADAPTERS.md §5.7).

use crate::facts::{Evidence, Facts, Meet, Reach, Relation, Serve, Target, Verb};

/***
 * Give every read of an environment variable a line from what sets it: a ConfigMap or Secret key, or a Deployment's own value.
 *
 * A read and a setting meet by the variable's name, like a request and a
 * route. The variables one part reads from one setter make one line
 * (`@reads k8s.cm.docair-server (DOCAIR_MAX_FILES, DOCAIR_MAX_UPLOAD_MB)`).
 * A variable that several things set reaches one of them; a read that
 * nothing in the project sets stays a `@requests env:NAME` on the part.
 *
 * @id  docair-core.env.link
 ***/
pub fn link(all: &mut [Facts]) {
    // The code reads in one adapter's facts, the settings in another's (infra).
    let serves: Vec<Serve> = all
        .iter()
        .flat_map(|f| &f.serves)
        .filter(|s| matches!(s.at, Meet::Env(_)))
        .cloned()
        .collect();
    for facts in all {
        link_one(facts, &serves);
    }
}

/// One line from a setter to a reader, with the variables it sets.
struct Line {
    pair: (String, String),
    names: Vec<String>,
    reach: Reach,
    evidence: Evidence,
}

fn link_one(facts: &mut Facts, serves: &[Serve]) {
    let mut kept = Vec::new();
    // (reader, setter) → the variables, the reach, and where the first is read.
    let mut lines: Vec<Line> = Vec::new();
    for r in std::mem::take(&mut facts.requests) {
        let Meet::Env(name) = &r.at else {
            kept.push(r);
            continue;
        };
        let mut setters: Vec<&str> = serves
            .iter()
            .filter(|s| s.at == r.at)
            .map(|s| s.by.as_str())
            .collect();
        setters.sort_unstable();
        setters.dedup();
        if setters.is_empty() {
            kept.push(r);
            continue;
        }
        let reach = if setters.len() > 1 {
            Reach::OneOf
        } else {
            Reach::Certain
        };
        for setter in setters {
            let pair = (r.by.clone(), setter.to_string());
            match lines.iter_mut().find(|l| l.pair == pair) {
                Some(l) => {
                    if !l.names.contains(name) {
                        l.names.push(name.clone());
                    }
                    if reach == Reach::OneOf {
                        l.reach = Reach::OneOf;
                    }
                }
                None => lines.push(Line {
                    pair,
                    names: vec![name.clone()],
                    reach,
                    evidence: r.evidence.clone(),
                }),
            }
        }
    }
    facts.requests = kept;
    for Line {
        pair: (reader, setter),
        mut names,
        reach,
        evidence,
    } in lines
    {
        names.sort();
        facts.relations.push(
            Relation {
                from: reader,
                ..Relation::new(Verb::Reads, Target::Symbol(setter), evidence)
            }
            .payload(Some(names.join(", ")))
            .reach(reach),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facts::Request;

    fn read(by: &str, name: &str) -> Request {
        Request {
            by: by.into(),
            at: Meet::Env(name.into()),
            payload: None,
            reply: None,
            evidence: Evidence::new(3, format!("std::env::var(\"{name}\")")),
        }
    }

    fn set(by: &str, name: &str) -> Serve {
        Serve {
            by: by.into(),
            at: Meet::Env(name.into()),
            evidence: Evidence::new(1, "envFrom configMapRef"),
        }
    }

    #[test]
    fn a_read_meets_what_sets_the_variable() {
        let mut facts = Facts {
            requests: vec![
                read("config", "DOCAIR_MAX_FILES"),
                read("config", "DOCAIR_MAX_UPLOAD_MB"),
                read("config", "DOCAIR_AI_API_KEY"),
                read("main", "DOCAIR_BIND"),
                read("main", "SHARED"),
            ],
            serves: vec![
                set("cm", "DOCAIR_MAX_FILES"),
                set("cm", "DOCAIR_MAX_UPLOAD_MB"),
                set("secret", "DOCAIR_AI_API_KEY"),
                set("cm", "SHARED"),
                set("other-cm", "SHARED"),
            ],
            ..Facts::default()
        };
        link(std::slice::from_mut(&mut facts));
        let lines: Vec<(String, String, String, Reach)> = facts
            .relations
            .iter()
            .map(|r| {
                let Target::Symbol(to) = &r.to else {
                    unreachable!()
                };
                (
                    r.from.clone(),
                    to.clone(),
                    r.payload.clone().unwrap_or_default(),
                    r.reach,
                )
            })
            .collect();
        assert_eq!(
            lines,
            [
                (
                    "config".into(),
                    "cm".into(),
                    "DOCAIR_MAX_FILES, DOCAIR_MAX_UPLOAD_MB".into(),
                    Reach::Certain
                ),
                (
                    "config".into(),
                    "secret".into(),
                    "DOCAIR_AI_API_KEY".into(),
                    Reach::Certain
                ),
                ("main".into(), "cm".into(), "SHARED".into(), Reach::OneOf),
                (
                    "main".into(),
                    "other-cm".into(),
                    "SHARED".into(),
                    Reach::OneOf
                ),
            ]
        );
        // Nothing sets DOCAIR_BIND: it stays a request.
        assert_eq!(facts.requests.len(), 1);
        assert_eq!(facts.requests[0].at, Meet::Env("DOCAIR_BIND".into()));
    }
}

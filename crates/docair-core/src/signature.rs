//! Function signatures as wording for edges: what a call passes and gets back (SPEC §4.2, §6).

use crate::model::Signature;

/// At most this many parameter names go on a line.
const MAX_PARAMS: usize = 4;
/// Longer types are cut.
const MAX_TYPE: usize = 40;

/// Wrappers that say how a value comes back, not what it is.
const WRAPPERS: &[&str] = &[
    "Result",
    "Option",
    "Box",
    "Arc",
    "Rc",
    "Cow",
    "Json",
    "Promise",
    "Observable",
    "Signal",
    "Future",
    "Ref",
    "RefMut",
    "Mutex",
    "RwLock",
];

/// `Result<Json<Reply>, ApiError>` → `Reply`, `Vec<Change>` → `Change[]`, `()` → none.
pub fn simplify_type(t: &str) -> Option<String> {
    let mut t = t.trim().trim_start_matches("->").trim().to_string();
    for _ in 0..8 {
        let before = t.clone();
        for prefix in [
            "&'static ",
            "&mut ",
            "&",
            "mut ",
            "impl ",
            "dyn ",
            "readonly ",
        ] {
            if let Some(rest) = t.strip_prefix(prefix) {
                t = rest.trim().to_string();
            }
        }
        // Drop module paths outside generics: `std::path::PathBuf` → `PathBuf`.
        let generics = t.find('<').unwrap_or(t.len());
        if let Some(cut) = t[..generics].rfind("::") {
            t = t[cut + 2..].to_string();
        }
        if let Some(open) = t.find('<')
            && t.ends_with('>')
        {
            let head = &t[..open];
            let inner = &t[open + 1..t.len() - 1];
            let first = first_generic(inner);
            if WRAPPERS.contains(&head) {
                t = first.to_string();
            } else if head == "Vec" || head == "VecDeque" || head == "HashSet" || head == "BTreeSet"
            {
                t = format!(
                    "{}[]",
                    simplify_type(first).unwrap_or_else(|| first.to_string())
                );
            }
        }
        if t == before {
            break;
        }
    }
    let t = t.trim();
    if t.is_empty() || matches!(t, "()" | "void" | "undefined" | "never" | "Self" | "None") {
        return None;
    }
    if t.chars().count() > MAX_TYPE {
        let cut: String = t.chars().take(MAX_TYPE - 1).collect();
        return Some(format!("{cut}…"));
    }
    Some(t.to_string())
}

/// The first type argument of `A, B<C, D>, E` at the top level.
fn first_generic(inner: &str) -> &str {
    let mut depth = 0;
    for (i, c) in inner.char_indices() {
        match c {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' | ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => return inner[..i].trim(),
            _ => {}
        }
    }
    inner.trim()
}

/// Split `a: A, b: B<C, D>` at its top-level commas.
pub fn split_top(list: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for c in list.chars() {
        match c {
            '<' | '(' | '[' | '{' => depth += 1,
            '>' | ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if c == ',' && depth == 0 {
            out.push(std::mem::take(&mut cur).trim().to_string());
        } else {
            cur.push(c);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// A TypeScript or JavaScript declaration line: `async ask(question: string, …): Observable<Reply> {`,
/// `function setHidden(hide, show) {`, `const scan = (files: File[]) => …`.
pub fn ts_signature(line: &str) -> Option<Signature> {
    let open = line.find('(')?;
    // Only declarations: the text before `(` is a name, perhaps with modifiers, `function` or `= async`.
    let head = line[..open].trim();
    let ok = head.split_whitespace().all(|w| {
        w.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$' || c == '=' || c == '*')
    });
    const STATEMENTS: &[&str] = &[
        "if", "for", "while", "switch", "catch", "return", "else", "do", "with", "new", "await",
        "typeof", "throw", "yield", "delete", "void",
    ];
    let first = head.split_whitespace().next().unwrap_or("");
    if !ok || STATEMENTS.contains(&first) || (head.is_empty() && !line.contains("=>")) {
        return None;
    }
    let mut depth = 0;
    let mut close = None;
    for (i, c) in line[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let (params, types): (Vec<String>, Vec<String>) = split_top(&line[open + 1..close])
        .into_iter()
        .filter_map(|p| {
            let p = p.trim_start_matches("...").trim();
            let p = ["public ", "private ", "protected ", "readonly "]
                .iter()
                .fold(p, |p, m| p.strip_prefix(m).unwrap_or(p));
            let name: String = if p.starts_with('{') || p.starts_with('[') {
                p.split([':', '=']).next().unwrap_or(p).trim().to_string()
            } else {
                p.split([':', '=', '?'])
                    .next()
                    .unwrap_or(p)
                    .trim()
                    .to_string()
            };
            // `files: File[] = []`: the type between the colon and a default.
            let ty = (!p.starts_with('{') && !p.starts_with('['))
                .then(|| p.split_once(':'))
                .flatten()
                .and_then(|(_, t)| simplify_type(t.split('=').next().unwrap_or(t).trim()))
                .unwrap_or_default();
            (!name.is_empty() && name != "this").then_some((name, ty))
        })
        .unzip();
    let rest = line[close + 1..].trim_start();
    let returns = rest.strip_prefix(':').and_then(|r| {
        let end = r
            .find(" {")
            .or_else(|| r.find("=>"))
            .or_else(|| r.find('{'))
            .unwrap_or(r.len());
        simplify_type(r[..end].trim().trim_end_matches(';'))
    });
    let types = if types.iter().all(String::is_empty) {
        Vec::new()
    } else {
        types
    };
    Some(Signature {
        params,
        returns,
        types,
    })
}

impl Signature {
    /// The call's payload and reply as wording: `(question, graph) `, `Reply`.
    pub fn wording(&self) -> (Option<String>, Option<String>) {
        self.wording_typed(&|_| false)
    }

    /// As `wording`, with a parameter's type where it is one of the project's (`is_project_type`)
    /// and its name does not say it: `draw(g: Graph)`, but `scan(graph)` (docair-4wk.45).
    pub fn wording_typed(
        &self,
        is_project_type: &dyn Fn(&str) -> bool,
    ) -> (Option<String>, Option<String>) {
        let words: Vec<String> = self
            .params
            .iter()
            .enumerate()
            .map(
                |(i, name)| match self.types.get(i).filter(|t| !t.is_empty()) {
                    Some(t) if is_project_type(base_name(t)) && !says_type(name, base_name(t)) => {
                        format!("{name}: {t}")
                    }
                    _ => name.clone(),
                },
            )
            .collect();
        let params = if words.len() > MAX_PARAMS {
            format!("{}, …", words[..MAX_PARAMS - 1].join(", "))
        } else {
            words.join(", ")
        };
        ((!params.is_empty()).then_some(params), self.returns.clone())
    }
}

/// The type's own name in a simplified type: `Change` of `Change[]`, `Graph` of `Graph`.
pub fn base_name(t: &str) -> &str {
    t.trim_end_matches("[]")
        .rsplit("::")
        .next()
        .unwrap_or(t)
        .split('<')
        .next()
        .unwrap_or(t)
        .trim()
}

/// Whether a parameter's name says its type: it holds the type's last word (`graph`, `old_graph`
/// for `Graph`; `diff` for `GraphDiff`), so the type need not be written beside it.
fn says_type(name: &str, ty: &str) -> bool {
    let words = |s: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        for c in s.chars() {
            if c == '_' || c == '-' {
                out.push(std::mem::take(&mut cur));
            } else if c.is_uppercase() && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
                cur.extend(c.to_lowercase());
            } else {
                cur.extend(c.to_lowercase());
            }
        }
        out.push(cur);
        out.retain(|w| !w.is_empty());
        out
    };
    let last = words(ty).pop().unwrap_or_default();
    let name = words(name);
    name.iter()
        .any(|w| *w == last || w.trim_end_matches('s') == last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_lose_their_wrappers() {
        assert_eq!(
            simplify_type("Result<Json<Reply>, ApiError>").as_deref(),
            Some("Reply")
        );
        assert_eq!(
            simplify_type("anyhow::Result<Graph>").as_deref(),
            Some("Graph")
        );
        assert_eq!(simplify_type("Vec<Change>").as_deref(), Some("Change[]"));
        assert_eq!(
            simplify_type("Option<&'static LanguageSpec>").as_deref(),
            Some("LanguageSpec")
        );
        assert_eq!(
            simplify_type("Result<(Reply, Vec<String>)>").as_deref(),
            Some("(Reply, Vec<String>)")
        );
        assert_eq!(
            simplify_type("std::path::PathBuf").as_deref(),
            Some("PathBuf")
        );
        assert_eq!(simplify_type("Promise<void>"), None);
        assert_eq!(simplify_type("Result<()>"), None);
        assert_eq!(
            simplify_type("impl IntoResponse").as_deref(),
            Some("IntoResponse")
        );
    }

    #[test]
    fn typescript_declarations() {
        let s = ts_signature("  ask(question: string, graph: object, history: Turn[], context?: string): Observable<Reply> {").unwrap();
        assert_eq!(s.params, vec!["question", "graph", "history", "context"]);
        assert_eq!(s.returns.as_deref(), Some("Reply"));
        let s = ts_signature("protected async apply(entry: ChatEntry): Promise<void> {").unwrap();
        assert_eq!((s.params.len(), s.returns), (1, None));
        let s = ts_signature("function setHidden(hide, show) {").unwrap();
        assert_eq!(s.params, vec!["hide", "show"]);
        let s = ts_signature("const scan = async ({ page }, ...rest) => {").unwrap();
        assert_eq!(s.params, vec!["{ page }", "rest"]);
        assert!(
            ts_signature("if (x) {").is_none(),
            "statements are no declarations"
        );
        assert!(ts_signature("return foo(bar);").is_none());
    }

    #[test]
    fn wording_keeps_lines_short() {
        let s = Signature {
            params: ["a", "b", "c", "d", "e"].map(String::from).to_vec(),
            returns: Some("R".into()),
            types: vec![],
        };
        assert_eq!(s.wording(), (Some("a, b, c, …".into()), Some("R".into())));
        let s = Signature {
            params: vec![],
            returns: None,
            types: vec![],
        };
        assert_eq!(s.wording(), (None, None));
    }

    /// A parameter whose name does not say its project type shows the type (docair-4wk.45).
    #[test]
    fn a_name_that_does_not_say_its_type_shows_it() {
        let s = Signature {
            params: ["g", "graph", "diff", "a", "root"]
                .map(String::from)
                .to_vec(),
            returns: None,
            types: ["Graph", "Graph", "GraphDiff", "GraphDiff", "Path"]
                .map(String::from)
                .to_vec(),
        };
        let ours = |t: &str| ["Graph", "GraphDiff"].contains(&t);
        assert_eq!(
            s.wording_typed(&ours).0.as_deref(),
            Some("g: Graph, graph, diff, …")
        );
        let s = Signature {
            params: ["changes", "x"].map(String::from).to_vec(),
            returns: None,
            types: ["ChangeStep[]", "Path"].map(String::from).to_vec(),
        };
        assert_eq!(
            s.wording_typed(&|t| t == "ChangeStep").0.as_deref(),
            Some("changes: ChangeStep[], x"),
            "a type outside the project (Path) is not written"
        );
        assert_eq!(
            ts_signature("ask(question: string, g: Graph = {}): Reply {").map(|s| s.types),
            Some(vec!["string".to_string(), "Graph".to_string()])
        );
    }
}

//! Node id rules (SPEC §3.1): lowercase `[a-z0-9_-]` segments joined with `.`.

/// Turn a symbol or file name into one id segment: `RustAdapter` → `rust-adapter`.
pub fn slug(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            if c.is_ascii_uppercase() {
                let prev = i.checked_sub(1).map(|j| chars[j]);
                let next_lower = chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
                let boundary = match prev {
                    Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                    Some(p) if p.is_ascii_uppercase() => next_lower,
                    _ => false,
                };
                if boundary && !out.is_empty() && !out.ends_with('-') {
                    out.push('-');
                }
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Whether an authored `@id` follows the id rules.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('.').all(|seg| {
            !seg.is_empty()
                && seg
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("RustAdapter"), "rust-adapter");
        assert_eq!(slug("create_order"), "create_order");
        assert_eq!(slug("LANGUAGES"), "languages");
        assert_eq!(slug("HTMLParser"), "html-parser");
        assert_eq!(slug("orders.service"), "orders-service");
        assert_eq!(slug("docair-core"), "docair-core");
    }

    #[test]
    fn ids() {
        assert!(is_valid_id("orders.create"));
        assert!(is_valid_id("docair-core.extract.parse_body"));
        assert!(!is_valid_id("Orders.create"));
        assert!(!is_valid_id("orders..create"));
        assert!(!is_valid_id("topic:order.created"));
    }
}

/***
 * Give every line of the graph its id: `<from> -<kind>-> <to>`, with ` "<label>"` when two lines share that, and `#2`, `#3` only for true twins, in the graph's order (docair-svh).
 *
 * The id says what the line is, not where it stands, so it is the same after a scan that changed something else.
 ***/
pub fn edge_ids(edges: &mut [crate::model::Edge]) {
    use std::collections::HashMap;
    let kind = |e: &crate::model::Edge| {
        serde_json::to_value(e.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    };
    let base: Vec<String> = edges
        .iter()
        .map(|e| format!("{} -{}-> {}", e.from, kind(e), e.to))
        .collect();
    let mut shared: HashMap<&str, usize> = HashMap::new();
    for b in &base {
        *shared.entry(b.as_str()).or_default() += 1;
    }
    let named: Vec<String> = edges
        .iter()
        .zip(&base)
        .map(|(e, b)| match (&e.label, shared[b.as_str()] > 1) {
            (Some(l), true) => format!("{b} \"{l}\""),
            _ => b.clone(),
        })
        .collect();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (e, id) in edges.iter_mut().zip(named) {
        let n = seen.entry(id.clone()).or_default();
        *n += 1;
        e.id = if *n == 1 { id } else { format!("{id} #{n}") };
    }
}

/// What an id names on the page: a part or datum, a line, a walk's step, a diagnostic, a glossary term.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdTarget<'g> {
    Part(&'g crate::model::Node),
    /// A line, with its index in `Graph::edges`.
    Line(usize, &'g crate::model::Edge),
    /// The n-th step (1-based) of the walk a seed starts: `flow:checkout/2`, `data.order/1`.
    Step(&'g str, usize),
    Diagnostic(&'g crate::model::Diagnostic),
    Term(&'g crate::model::Term),
}

/***
 * Find what an id names, for the CLI, the server and the page alike: a node id, a line's id, `<walk seed>/<n>` for a step, `<code>@<node>` (and `#2` …) for a diagnostic, `term:<id>` for a glossary entry.
 *
 * @id docair-core.ids.find
 ***/
pub fn find<'g>(graph: &'g crate::model::Graph, id: &'g str) -> Option<IdTarget<'g>> {
    if let Some(n) = graph.nodes.iter().find(|n| n.id == id) {
        return Some(IdTarget::Part(n));
    }
    if let Some((i, e)) = graph.edges.iter().enumerate().find(|(_, e)| e.id == id) {
        return Some(IdTarget::Line(i, e));
    }
    if let Some(t) = id.strip_prefix("term:") {
        return graph
            .glossary
            .iter()
            .find(|x| x.id == t)
            .map(IdTarget::Term);
    }
    if let Some((code, rest)) = id.split_once('@')
        && code.len() == 4
        && code.starts_with(['E', 'W'])
    {
        let (node, nth) = match rest.rsplit_once(" #") {
            Some((n, k)) => (n, k.parse::<usize>().ok()?),
            None => (rest, 1),
        };
        return graph
            .diagnostics
            .iter()
            .filter(|d| d.code == code && d.node.as_deref() == Some(node))
            .nth(nth - 1)
            .map(IdTarget::Diagnostic);
    }
    if let Some((seed, n)) = id.rsplit_once('/')
        && let Ok(n) = n.parse::<usize>()
        && n >= 1
        && crate::slice::find(graph, seed).is_some()
    {
        return Some(IdTarget::Step(seed, n));
    }
    None
}

/// A diagnostic's id: `<code>@<node>`, and `#2`, `#3` for more of the same code on that node.
pub fn diagnostic_id(graph: &crate::model::Graph, index: usize) -> Option<String> {
    let d = graph.diagnostics.get(index)?;
    let node = d.node.as_deref()?;
    let nth = graph.diagnostics[..index]
        .iter()
        .filter(|x| x.code == d.code && x.node.as_deref() == Some(node))
        .count()
        + 1;
    Some(if nth == 1 {
        format!("{}@{node}", d.code)
    } else {
        format!("{}@{node} #{nth}", d.code)
    })
}

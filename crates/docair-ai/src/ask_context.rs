//! What a question points at (docair-5n2): parts, lines, data and terms the reader picked
//! anywhere on the page, as typed refs, turned into words for the model.

use docair_core::layer_view::{LayerKey, LayerPicture, picture, section_picture, slice_picture};
use docair_core::model::{Edge, Graph, Node};

/// The words for one node: its id, kind and name, and what it does.
fn node_words(what: &str, n: &Node) -> String {
    let summary = n.summary.trim();
    let mut s = format!("- {what} `{}` ({}, \"{}\")", n.id, n.kind, n.name);
    if !summary.is_empty() {
        s.push_str(": ");
        s.push_str(summary.lines().next().unwrap_or(""));
    }
    s
}

/// The words for one line of the graph: its ends by name, its kind, and what it says or carries.
fn edge_words(graph: &Graph, e: &Edge) -> String {
    let name = |id: &str| {
        graph
            .nodes
            .iter()
            .find(|n| n.id == id)
            .map_or(id.to_string(), |n| n.name.clone())
    };
    let mut s = format!(
        "- line `{}` → `{}` ({}: {} → {})",
        e.from,
        e.to,
        kind_word(e),
        name(&e.from),
        name(&e.to)
    );
    let says: Vec<&str> = [e.text.as_deref(), e.label.as_deref(), e.payload.as_deref()]
        .into_iter()
        .flatten()
        .filter(|t| !t.trim().is_empty())
        .collect();
    if !says.is_empty() {
        s.push_str(": ");
        s.push_str(&says.join(" · "));
    }
    s
}

fn kind_word(e: &Edge) -> String {
    serde_json::to_value(e.kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "line".to_string())
}

/// The picture an address names: `layer/<key>`, `box/<id>`, `slice/<seed>`.
fn picture_at(graph: &Graph, address: &str) -> Option<LayerPicture> {
    let (what, rest) = address.split_once('/')?;
    match what {
        "layer" => {
            let key: LayerKey =
                serde_json::from_value(serde_json::Value::String(rest.to_string())).ok()?;
            picture(graph, key, &[], None)
        }
        "box" => section_picture(graph, rest, None).map(|(_, p)| p),
        "slice" => {
            let seed = docair_core::slice::find(graph, rest)?;
            let s = docair_core::slice::slice(graph, &seed)?;
            Some(slice_picture(graph, &s))
        }
        _ => None,
    }
}

/***
 * Turn what a question points at into words for the model, and say which refs named nothing.
 *
 * A ref is `part:<id>`, `data:<id>`, `term:<word>`, `edge:<line id>`, `line:<k>@<address>` (the k-th line of the
 * picture at that address, as the page numbers them) or `edges:<i>,<j>` (lines of the graph by
 * index); a bare id is a part, as the page sent before. Each line of the answer names one thing;
 * a line in a picture says every line of the graph it stands for.
 *
 * @id      docair-ai.ask_context.about
 * @calls   docair-core.layer_view.picture "finds the line a ref names in its picture"
 ***/
pub fn about(graph: &Graph, refs: &[String]) -> (Option<String>, Vec<String>) {
    let mut out: Vec<String> = Vec::new();
    let mut problems = Vec::new();
    let node = |id: &str| graph.nodes.iter().find(|n| n.id == id);
    for r in refs.iter().map(|r| r.trim()).filter(|r| !r.is_empty()) {
        let (kind, rest) = r
            .split_once(':')
            .filter(|(k, _)| ["part", "data", "term", "line", "edges", "edge"].contains(k))
            .unwrap_or(("part", r));
        let said = match kind {
            "part" | "data" => node(rest).map(|n| {
                vec![node_words(
                    if n.kind == "data" { "datum" } else { "part" },
                    n,
                )]
            }),
            "term" => {
                let w = rest.to_lowercase();
                graph
                    .glossary
                    .iter()
                    .find(|t| {
                        t.term.to_lowercase() == w
                            || t.id == w
                            || t.aliases.iter().any(|a| a.to_lowercase() == w)
                    })
                    .map(|t| vec![format!("- term \"{}\": {}", t.term, t.summary.trim())])
            }
            // `edge:<id>`: a line by its id, `shop-ui.cart -call-> orders.create` (docair-svh).
            "edge" => match docair_core::ids::find(graph, rest) {
                Some(docair_core::ids::IdTarget::Line(_, e)) => Some(vec![edge_words(graph, e)]),
                _ => None,
            },
            "edges" => {
                let lines: Vec<String> = rest
                    .split(',')
                    .filter_map(|i| i.trim().parse::<usize>().ok())
                    .filter_map(|i| graph.edges.get(i))
                    .map(|e| edge_words(graph, e))
                    .collect();
                (!lines.is_empty()).then_some(lines)
            }
            _ => rest.split_once('@').and_then(|(k, address)| {
                let k: usize = k.parse().ok()?;
                let pic = picture_at(graph, address)?;
                let l = pic.lines.get(k)?;
                let lines: Vec<String> = l
                    .members
                    .iter()
                    .filter_map(|&i| graph.edges.get(i))
                    .map(|e| edge_words(graph, e))
                    .collect();
                (!lines.is_empty()).then_some(lines)
            }),
        };
        match said {
            Some(lines) => {
                for l in lines {
                    if !out.contains(&l) {
                        out.push(l);
                    }
                }
            }
            None => problems.push(format!(
                "the question points at `{r}`, which names nothing in the graph; left out"
            )),
        }
    }
    ((!out.is_empty()).then(|| out.join("\n")), problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use docair_core::model::{EdgeKind, Term};

    fn graph() -> Graph {
        let mut g: Graph = serde_json::from_value(serde_json::json!({
            "schema": "docair.graph/v0", "system": "shop", "generator": "test", "nodes": [], "edges": [], "flows": [], "diagnostics": []
        }))
        .unwrap();
        let node = |id: &str, kind: &str, name: &str, summary: &str| -> Node {
            serde_json::from_value(serde_json::json!({
                "id": id, "kind": kind, "name": name, "summary": summary, "description": "", "part_of": "shop",
                "docs": [], "tags": [], "origin": "authored"
            }))
            .unwrap()
        };
        g.nodes = vec![
            node("shop", "system", "shop", ""),
            node("cart", "component", "Cart", "The cart page."),
            node(
                "orders.create",
                "function",
                "create_order",
                "Store a new order.\n\nMore.",
            ),
            node("data.order", "data", "order", "What the shopper buys."),
        ];
        let mut e: Edge = serde_json::from_value(serde_json::json!({
            "from": "cart", "to": "orders.create", "kind": "call", "tag": "calls",
            "source": { "file": "cart.ts", "line": 1 }, "origin": "authored"
        }))
        .unwrap();
        e.kind = EdgeKind::Call;
        e.label = Some("POST /api/orders".to_string());
        g.edges = vec![e];
        docair_core::ids::edge_ids(&mut g.edges);
        g.glossary = vec![
            serde_json::from_value::<Term>(serde_json::json!({
                "id": "order", "term": "order", "aliases": ["orders"], "summary": "What the shopper buys.",
                "markdown": "What the shopper buys.", "source": { "file": "GLOSSARY.md", "line": 1 }
            }))
            .unwrap(),
        ];
        g
    }

    #[test]
    fn each_ref_becomes_words_and_unknown_ones_are_named() {
        let g = graph();
        let refs: Vec<String> = [
            "orders.create",
            "data:data.order",
            "term:Orders",
            "edges:0",
            "part:ghost",
            "line:9@layer/apps",
            "edge:cart -call-> orders.create",
            "edge:nope",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (said, problems) = about(&g, &refs);
        let said = said.unwrap();
        assert!(
            said.contains(
                "- part `orders.create` (function, \"create_order\"): Store a new order."
            ),
            "{said}"
        );
        assert!(!said.contains("More."), "only the summary's first line");
        assert!(
            said.contains("- datum `data.order` (data, \"order\"): What the shopper buys."),
            "{said}"
        );
        assert!(
            said.contains("- term \"order\": What the shopper buys."),
            "{said}"
        );
        assert!(
            said.contains(
                "- line `cart` → `orders.create` (call: Cart → create_order): POST /api/orders"
            ),
            "{said}"
        );
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("part:ghost"));
        assert_eq!(about(&g, &[]).0, None);
    }
}

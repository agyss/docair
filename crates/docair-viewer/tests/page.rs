//! The documentation page of the e2e fixtures (docs/SLICES.md §4): every address has its picture,
//! every packed picture unpacks to a drawing, no link leads nowhere, and the same graph gives the
//! same page.

use std::collections::HashSet;
use std::path::Path;

use base64::Engine as _;
use docair_core::model::Graph;
use docair_core::{Options, scan};
use serde_json::Value;

fn graph(name: &str) -> Graph {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../e2e/fixtures")
        .join(name);
    scan(&root, docair_adapters::all(), &Options::default()).unwrap()
}

/// The JSON in `<script id="<id>">`.
fn script(page: &str, id: &str) -> Value {
    let open = format!(r#"<script id="{id}" type="application/json">"#);
    let at = page.find(&open).expect("the script is in the page") + open.len();
    let end = at + page[at..].find("</script>").expect("the script ends");
    serde_json::from_str(&page[at..end]).expect("it is JSON")
}

fn unpack(b64: &str) -> String {
    let flat = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .expect("base64");
    String::from_utf8(miniz_oxide::inflate::decompress_to_vec(&flat).expect("raw deflate"))
        .expect("UTF-8")
}

fn page_of(name: &str) -> (Graph, String) {
    let g = graph(name);
    let page = docair_viewer::doc_page::render_page_keeping(&g, None);
    (g, page)
}

#[test]
fn every_address_has_a_picture_and_every_picture_unpacks() {
    for name in ["shop", "platform", "apis", "answer", "fold"] {
        let (g, page) = page_of(name);
        assert!(
            !page.contains("{{docair:"),
            "{name}: every placeholder is filled"
        );
        assert!(
            !page.contains("cytoscape") && !page.contains("elkjs"),
            "{name}: no layout library"
        );
        let ix = script(&page, "docair-index");
        let packed = script(&page, "docair-packed");
        let packed = packed.as_array().unwrap();
        let pictures = ix["pictures"].as_object().unwrap();
        assert!(
            pictures.contains_key(ix["first"].as_str().unwrap()),
            "{name}: the first picture exists"
        );
        let mut used = HashSet::new();
        for (addr, p) in pictures {
            for k in ["svg", "on", "off"] {
                if let Some(i) = p[k].as_u64() {
                    used.insert(i as usize);
                }
            }
            assert!(
                p["svg"].is_u64() || (p["on"].is_u64() && p["off"].is_u64()),
                "{name}: {addr} has its picture"
            );
            let crumbs = p["crumbs"].as_array().unwrap();
            assert_eq!(
                crumbs.last().unwrap()[1],
                Value::from(addr.as_str()),
                "{name}: {addr}'s crumbs end at it"
            );
        }
        let panels = ix["panels"].as_u64().unwrap() as usize;
        assert_eq!(
            panels,
            packed.len() - 2,
            "{name}: the panels and the walks are packed last"
        );
        assert_eq!(
            used.len(),
            packed.len() - 2,
            "{name}: every other packed text is a picture shown somewhere"
        );
        // Each drawing: a picture of its own, and every closed box in it opens to a section.
        for b64 in &packed[..panels] {
            let svg = unpack(b64.as_str().unwrap());
            assert!(
                svg.starts_with("<svg ") && svg.contains("class=\"docair-pic\""),
                "{name}: a drawing"
            );
            for chunk in svg.split("<g class=\"part ").skip(1) {
                let head = &chunk[..chunk.find('>').unwrap()];
                if head.contains("closed") {
                    let id = head
                        .split("data-id=\"")
                        .nth(1)
                        .unwrap()
                        .split('"')
                        .next()
                        .unwrap();
                    assert!(
                        pictures.contains_key(&format!("box/{id}")),
                        "{name}: the closed box {id} opens to its section"
                    );
                }
            }
        }
        // Each part's home is one of the pictures, and draws it.
        for (id, home) in ix["home"].as_object().unwrap() {
            assert!(
                pictures.contains_key(home.as_str().unwrap()),
                "{name}: {id}'s home {home} exists"
            );
        }
        assert!(
            g.nodes
                .iter()
                .filter(|n| n.kind != "data" && n.id != g.system)
                .all(|n| ix["home"].get(&n.id).is_some()),
            "{name}: every part has a picture that draws it"
        );
    }
}

#[test]
fn the_layers_are_links_and_the_same_graph_gives_the_same_page() {
    let (g, page) = page_of("shop");
    for l in docair_core::layer_view::layers(&g) {
        let key = serde_json::to_value(l.key).unwrap();
        let key = key.as_str().unwrap();
        assert!(
            page.contains(&format!(r##"data-layer="{key}" href="#layer/{key}""##)),
            "a link to {key}"
        );
    }
    let again = docair_viewer::doc_page::render_page_keeping(&g, None);
    assert!(page == again, "the same graph, the same page");
    // Kept layouts give the same page as layouts made anew.
    let keep = std::env::temp_dir().join(format!("docair-page-test-{}", std::process::id()));
    let first = docair_viewer::doc_page::render_page_keeping(&g, Some(&keep));
    let kept = docair_viewer::doc_page::render_page_keeping(&g, Some(&keep));
    let _ = std::fs::remove_dir_all(&keep);
    assert!(first == page && kept == page, "kept layouts change nothing");
}

#[test]
fn the_panels_say_what_the_viewer_said() {
    let (g, page) = page_of("shop");
    let ix = script(&page, "docair-index");
    let packed = script(&page, "docair-packed");
    let panels: Value = serde_json::from_str(&unpack(
        packed[ix["panels"].as_u64().unwrap() as usize]
            .as_str()
            .unwrap(),
    ))
    .unwrap();
    // Every part has its details, its tip and a row in the tree.
    for n in g.nodes.iter().filter(|n| n.id != g.system) {
        assert!(
            panels["details"][&n.id]
                .as_str()
                .is_some_and(|d| if n.kind == "data" {
                    d.contains(r#"data-test="data-title""#)
                } else {
                    d.contains(&format!(r#"<span class="id">{}</span>"#, n.id))
                }),
            "{} has details",
            n.id
        );
        assert!(panels["tips"][&n.id].is_string(), "{} has a tip", n.id);
        if n.kind != "data" {
            assert!(
                page.contains(&format!(r#"<li data-id="{}""#, n.id)),
                "{} has a row in the tree",
                n.id
            );
        }
    }
    let create = panels["details"]["orders.create"].as_str().unwrap();
    assert!(
        create.contains("Tested by (1)"),
        "the details list the tests that cover it"
    );
    assert!(
        create.contains(r##"href="#part/shop-ui.cart""##),
        "a line's other end is a link to it"
    );
    assert!(
        create.contains(r#"data-hide="orders.create""#)
            && create.contains(r#"data-show="orders.create" hidden"#),
        "it can be hidden, and shown again"
    );
    let api = panels["details"]["orders-api"].as_str().unwrap();
    assert!(
        api.contains(r##"href="#box/orders-api" data-test="zoom-in""##),
        "a box links to its section"
    );
    assert!(api.contains("Talks to ("), "a box says what it talks to");
    // Every line of the graph has its tip; the search words hold names.
    assert_eq!(panels["lines"].as_array().unwrap().len(), g.edges.len());
    assert!(
        panels["search"]["orders.create"]
            .as_str()
            .unwrap()
            .contains("create_order")
    );
    // The switch, the legend and the diagnostics are in the page from the start.
    assert!(
        page.contains(r#"data-test="tests-toggle""#)
            && page.contains(r#"<span class="tests-off" hidden>"#)
    );
    assert!(page.contains(r#"<h4>Nodes</h4>"#) && page.contains("var(--k-service-bg)"));
    assert!(page.contains(r#"id="btn-diag""#) && page.contains("0 issues"));
}

#[test]
fn the_glossary_and_the_data_are_written_into_the_page() {
    let (_, page) = page_of("shop");
    let ix = script(&page, "docair-index");
    let packed = script(&page, "docair-packed");
    let p: Value = serde_json::from_str(&unpack(
        packed[ix["panels"].as_u64().unwrap() as usize]
            .as_str()
            .unwrap(),
    ))
    .unwrap();
    let create = p["details"]["orders.create"].as_str().unwrap();
    assert_eq!(
        create.matches(r#"class="term" data-term="order""#).count(),
        1,
        "a term is marked at its first mention"
    );
    assert!(
        create.contains(r#"<span class="data-ref inline" data-data="data.order""#),
        "the data a line's words name are chips"
    );
    let pop = p["pops"]["term:order"].as_str().unwrap();
    assert!(
        pop.contains(r#"data-test="pop-open""#)
            && pop.contains(r#"href="docair-doc:docs/orders.md#how-an-order-moves""#),
        "the popup opens the entry, and Read more the doc"
    );
    assert!(
        p["terms"]["order"]
            .as_str()
            .unwrap()
            .contains(r#"data-test="term-title""#),
        "each term has its entry"
    );
    assert!(
        p["glossary"]
            .as_str()
            .unwrap()
            .contains(r#"data-test="glossary-search""#),
        "the glossary's list can be searched"
    );
    assert!(
        p["docs"]["docs/orders.md"]
            .as_str()
            .unwrap()
            .contains(r#"data-test="reader-close""#),
        "the docs Read more opens are in the page"
    );
    let datum = p["details"]["data.order"].as_str().unwrap();
    assert!(
        datum.contains(r#"data-test="data-title""#)
            && datum.contains(r##"href="#slice/data.order" data-test="follow-data""##),
        "a datum's details are its page, with Follow"
    );
    assert!(
        p["dataList"]
            .as_str()
            .unwrap()
            .contains(r#"data-test="data-list""#)
    );
    assert!(
        page.contains(r#"data-test="glossary-button""#)
            && page.contains(r#"data-test="data-button""#)
    );
    assert_eq!(
        ix["pictures"]["slice/data.order"]["datum"],
        Value::from("data.order"),
        "a datum's slice says what it follows"
    );
}

#[test]
fn a_walk_is_one_picture_with_its_steps() {
    let (_, page) = page_of("shop");
    let ix = script(&page, "docair-index");
    let packed = script(&page, "docair-packed");
    let walks: Value = serde_json::from_str(&unpack(
        packed[ix["walks"].as_u64().unwrap() as usize]
            .as_str()
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(
        ix["pictures"]["slice/flow:checkout"]["walk"],
        Value::from(true),
        "the scenario's slice is a walk"
    );
    let steps = walks["slice/flow:checkout"]["steps"].as_array().unwrap();
    assert_eq!(steps.len(), 3);
    for (k, st) in steps.iter().enumerate() {
        let card = st["card"].as_str().unwrap();
        assert!(
            card.contains(r#"data-test="step-card""#)
                && card.contains(&format!("step {} of 3", k + 1)),
            "step {k}'s card"
        );
        assert!(
            st["camera"].as_array().is_some_and(|b| b.len() == 4),
            "step {k} has a camera box"
        );
        assert!(
            !st["parts"].as_array().unwrap().is_empty(),
            "step {k} lights parts"
        );
    }
    let second = &steps[1];
    assert!(
        second["card"]
            .as_str()
            .unwrap()
            .contains(r#"data-test="hops""#),
        "a step with lines says what happens"
    );
    assert_eq!(
        second["tokens"][0]["word"],
        Value::from("clicks Buy"),
        "a token carries the message's word"
    );
    assert!(
        second["lines"]
            .as_array()
            .unwrap()
            .contains(&second["tokens"][0]["line"]),
        "the token runs on a line the step lights"
    );
    assert!(
        walks["slice/data.order"]["steps"]
            .as_array()
            .is_some_and(|s| !s.is_empty()),
        "a datum's way is a walk too"
    );
    assert!(
        page.contains(r##"<a class="tour-chip" href="#slice/flow:checkout""##),
        "the footer lists the walks"
    );
    assert!(
        page.contains("Walk me through"),
        "the details start with the walks"
    );
}

/// The page's script stays thin (docs/SLICES.md §7): its budget is 1,000 lines.
#[test]
fn the_script_stays_within_its_budget() {
    let script = include_str!("../assets/page.js");
    let lines = script.lines().count();
    assert!(
        lines <= 1000,
        "page.js has {lines} lines; its budget is 1,000"
    );
    for library in ["cytoscape", "elk.", "dagre"] {
        assert!(
            !script.to_lowercase().contains(library),
            "page.js uses no layout or drawing library ({library})"
        );
    }
}

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
        // The pictures, then the panels and the walks, then what moving a part by hand needs: each
        // picture's layout input and layout, and the router last.
        let panels = ix["panels"].as_u64().unwrap() as usize;
        let at = |k: &str| ix[k].as_u64().unwrap() as usize;
        assert_eq!(
            (at("walks"), at("moves"), at("router")),
            (panels + 1, panels + 2, packed.len() - 1),
            "{name}: the packed texts in their order"
        );
        assert_eq!(
            packed.len(),
            2 * panels + 3,
            "{name}: one move for each picture"
        );
        assert_eq!(
            used.len(),
            panels,
            "{name}: every packed drawing is a picture shown somewhere"
        );
        for i in 0..panels {
            let moves: Value =
                serde_json::from_str(&unpack(packed[at("moves") + i].as_str().unwrap())).unwrap();
            assert!(
                moves["input"].is_object() && moves["layout"].is_object(),
                "{name}: picture {i} moves with its layout input and layout"
            );
        }
        let router = base64::engine::general_purpose::STANDARD
            .decode(packed[at("router")].as_str().unwrap())
            .unwrap();
        let router = miniz_oxide::inflate::decompress_to_vec(&router).unwrap();
        assert!(
            router.starts_with(b"\0asm"),
            "{name}: the router is WebAssembly"
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
    let tips = panels["lines"].as_object().unwrap();
    assert_eq!(tips.len(), g.edges.len());
    assert!(
        g.edges.iter().all(|e| tips.contains_key(&e.id)),
        "each line's tip goes by its id"
    );
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
    assert!(
        page.contains("<h4>Parts</h4>")
            && page.contains(r#"<g class="part k-service""#)
            && page.contains("What the forms say"),
        "the legend draws each kind in its shape and explains the forms"
    );
    assert!(
        page.contains(r#"<div class="row" data-in=".k-service">"#)
            && page.contains(r#"<div class="row" data-in=".e-call">"#)
            && page.contains(r#"<details class="group forms">"#),
        "each row of the legend names what it explains, for the page to show only what a picture holds; the forms fold away"
    );
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
    // What a reader can ask about: a part, a datum, a term; the page sends them as typed refs.
    for (html, rf) in [
        (create, "part:orders.create"),
        (datum, "data:data.order"),
        (p["terms"]["order"].as_str().unwrap(), "term:order"),
    ] {
        assert!(
            html.contains(&format!(r#"data-ref="{rf}""#)) && html.contains("✦ Ask about this"),
            "{rf} can be asked about"
        );
    }
    assert!(
        page.contains(
            r#"<div class="context" data-test="context" hidden>about <span class="refs">"#
        ),
        "the chat shows what it is asked about"
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
    // docair-7h5: the walks are a header button next to Tests and the panel it opens; no footer.
    assert!(
        page.contains(r#"data-test="tests-toggle""#)
            && page.contains(r#"<button class="btn" id="btn-walks" data-test="walks-button""#)
            && page.find(r#"data-test="tests-toggle""#) < page.find(r#"data-test="walks-button""#)
            && !page.contains(r#"<footer id="flow">"#)
            && !page.contains("tour-chip"),
        "the walks are a button next to Tests"
    );
    let panels: Value = serde_json::from_str(&unpack(
        packed[ix["panels"].as_u64().unwrap() as usize]
            .as_str()
            .unwrap(),
    ))
    .unwrap();
    assert!(
        panels["walks"].as_str().unwrap().contains(
            r##"<a class="tour" href="#slice/flow:checkout" data-walk="slice/flow:checkout">"##
        ),
        "the walks' panel lists them"
    );
    // docair-jjk: the side panels' widths are dragged at a handle each.
    assert!(
        page.contains(r#"data-test="tree-handle""#)
            && page.contains(r#"data-test="details-handle""#)
    );
    assert!(
        page.contains("Walk me through"),
        "the details start with the walks"
    );
}

/// The page's script stays thin (docs/SLICES.md §7): its budget is 1,050 lines (docair-702.37).
#[test]
fn the_script_stays_within_its_budget() {
    let script = include_str!("../assets/page.js");
    let lines = script.lines().count();
    assert!(
        lines <= 1050,
        "page.js has {lines} lines; its budget is 1,050"
    );
    for library in ["cytoscape", "elk.", "dagre"] {
        assert!(
            !script.to_lowercase().contains(library),
            "page.js uses no layout or drawing library ({library})"
        );
    }
}

/// A slice keeps its words at their size (docair-702.27, docair-tqy): a datum passed along 30
/// functions of one box is placed at scale 1 in its slice, so every word keeps its 11 to 12 units
/// and a walk's camera (at least 0.8 px per unit) shows it at 8.8 px or more. Laid out as a layer
/// is, the same box shrinks what it holds. (On docair's own data.graph, data.page and
/// data.question, the smallest word went from 3.2 to 11 units.)
#[test]
fn a_slice_keeps_its_words_readable() {
    let dir = std::env::temp_dir().join(format!("docair-slice-words-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut md = String::from(
        "# Big\n\n```docair\nWhat is passed along.\n\n@id         data.order\n@kind       data\n@carried-as Order\n```\n\n```docair\nThe service.\n\n@id      big\n@kind    service\n```\n",
    );
    for i in 1..=30 {
        md.push_str(&format!(
            "\n```docair\nStep {i}.\n\n@id      big.f{i}\n@kind    function\n@part-of big\n"
        ));
        if i < 30 {
            md.push_str(&format!("@calls   big.f{} (Order)\n", i + 1));
        }
        md.push_str("```\n");
    }
    std::fs::write(dir.join("README.md"), md).unwrap();
    let g = scan(&dir, docair_adapters::all(), &Options::default()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let seed = docair_core::slice::find(&g, "data.order").expect("the datum");
    let s = docair_core::slice::slice(&g, &seed).expect("its slice");
    let pic = docair_core::layer_view::slice_picture(&g, &s);
    let input = docair_viewer::layout_input::input(&g, &pic, &docair_layout::Monospace::default());
    let placed = docair_layout::place(&input, &docair_viewer::layout_input::options_for(&pic));
    assert!(
        placed.scale.len() >= 30 && placed.scale.iter().all(|&k| k >= 1.0),
        "the slice keeps its words: {:?}",
        placed.scale
    );
    let shrunk = docair_layout::place(&input, &docair_layout::Options::default());
    assert!(
        shrunk.scale.iter().any(|&k| k < 1.0),
        "laid out as a layer, the box shrinks what it holds"
    );
}

/// Tidy in the page (docs/ADAPTERS.md §6.1): a tag the code repeats shows as the header's count,
/// the banner, a ticked entry in the panel with the code that says the same, and a mark in the
/// part's Tags tab; what needs docair serve waits for it. Where nothing repeats, the button says so.
#[test]
fn the_page_offers_to_tidy_the_tags_the_code_repeats() {
    let dir = std::env::temp_dir().join(format!("docair-tidy-page-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("docair.toml"), "system = \"tidyp\"\n").unwrap();
    let block = |id: &str, calls: &str| {
        format!(
            "/***\n * {id}.\n *\n * @id      app.{id}\n * @kind    function\n * @part-of tidyp\n{calls} ***/\n"
        )
    };
    std::fs::write(
        dir.join("src").join("lib.rs"),
        format!(
            "{}pub fn run() {{\n    load();\n}}\n\n{}pub fn load() {{}}\n",
            block("run", " * @calls   app.load\n"),
            block("load", "")
        ),
    )
    .unwrap();
    let g = scan(&dir, docair_adapters::all(), &Options::default()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let page = docair_viewer::doc_page::render_page_keeping(&g, None);
    assert!(
        page.contains(r#"data-test="tidy-button""#) && page.contains("✂ Tidy · 1"),
        "the header counts the tags to tidy"
    );
    assert!(
        page.contains(r#"data-test="tidy-banner" data-count="1""#)
            && page.contains("1 tag repeats what the code says"),
        "the banner says so"
    );
    let ix = script(&page, "docair-index");
    let packed = script(&page, "docair-packed");
    let p: Value = serde_json::from_str(&unpack(
        packed[ix["panels"].as_u64().unwrap() as usize]
            .as_str()
            .unwrap(),
    ))
    .unwrap();
    let tidy = p["tidy"].as_str().unwrap();
    assert!(
        tidy.contains(
            r#"<input type="checkbox" checked data-node="app.run" data-tag="@calls app.load">"#
        ) && tidy.contains("the code says <code>@calls app.load</code> · src/lib.rs:"),
        "the panel lists the tag, ticked, with the code that says the same: {tidy}"
    );
    assert!(
        tidy.contains(r#"data-test="tidy-apply" data-served hidden"#)
            && tidy.contains(r#"data-command="docair tidy --apply""#),
        "Apply waits for docair serve; the command is there without it"
    );
    let run = p["details"]["app.run"].as_str().unwrap();
    assert!(
        run.contains(r#"data-test="tidy-mark""#) && run.contains("✂ the code says this"),
        "the Tags tab marks the tag"
    );
    let (_, shop) = page_of("shop");
    assert!(
        shop.contains("✂ Tidy ✓") && !shop.contains(r#"data-test="tidy-banner""#),
        "where nothing repeats, the button says so and no banner shows"
    );
}

/// The changes section (docs/CHANGES.md §7 and §8, docair-zrr.7, zrr.18): a changed flow is listed
/// and opens as one still picture of its slice before and after, with what it now goes through
/// marked green, what stayed faded, a part that only moved not marked; a move is listed apart.
#[test]
fn a_changed_flow_shows_before_and_after() {
    use docair_core::changes::{Changes, FlowChange};
    use docair_core::diff::{GraphDiff, MoveMatch};
    let after = graph("shop");
    let mut before = after.clone();
    let at = before
        .edges
        .iter()
        .position(|e| e.from == "orders.create" && e.to == "orders-db")
        .expect("the shop writes the order");
    let line = before.edges.remove(at).id;
    let changes = Changes {
        schema: docair_core::changes::SCHEMA.to_string(),
        a: "0123456789abcdef0123456789abcdef01234567".to_string(),
        b: "work".to_string(),
        working_tree: false,
        graph: GraphDiff {
            moved: vec![MoveMatch {
                from: "orders.make".to_string(),
                to: "orders.create".to_string(),
            }],
            ..GraphDiff::default()
        },
        parts: vec!["orders.create".to_string()],
        flows: vec![FlowChange {
            seed: "data.order".to_string(),
            how: "changed".to_string(),
            parts_added: vec!["orders-db".to_string()],
            parts_removed: vec![],
            lines_added: vec![line],
            lines_removed: vec![],
            through: vec!["orders.create".to_string()],
            certain: false,
        }],
    };
    let changed = docair_viewer::doc_page::Changed {
        before: &before,
        changes: &changes,
        findings: None,
        code: &[],
    };
    let page = docair_viewer::doc_page::render_page_with(&after, None, Some(&changed));
    assert!(page.contains(r#"data-test="changes-button""#) && page.contains("Δ Changes · 1"));
    let ix = script(&page, "docair-index");
    let packed = script(&page, "docair-packed");
    let p = &ix["pictures"]["changes/data.order"];
    assert_eq!(
        p["still"],
        Value::from(true),
        "a change is not moved by hand"
    );
    assert_eq!(p["change"], Value::from("data.order"));
    let svg = unpack(
        packed[p["svg"].as_u64().unwrap() as usize]
            .as_str()
            .unwrap(),
    );
    assert_eq!(
        svg.matches("<svg ").count(),
        1,
        "before and after are one picture"
    );
    assert!(svg.contains(r#"class="docair-pic changes""#));
    assert!(
        svg.contains(r#"<g class="part k-store ch-added" data-id="orders-db">"#),
        "what the flow now goes through is marked"
    );
    assert!(
        svg.contains(r#"ch-same" data-id="orders.create">"#),
        "a part that only moved is not marked as changed inside"
    );
    assert!(
        svg.matches(" ch-same\" data-id").count() >= 3,
        "what stayed is faded"
    );
    let panels: Value = serde_json::from_str(&unpack(
        packed[ix["panels"].as_u64().unwrap() as usize]
            .as_str()
            .unwrap(),
    ))
    .unwrap();
    let list = panels["changes"].as_str().unwrap();
    assert!(
        list.contains(r##"href="#changes/data.order">Datum order</a>"##)
            && list.contains("uncertain")
            && list.contains("<code>orders.make</code> → <code>orders.create</code>"),
        "the list links the flow, marks it uncertain, and lists the move apart: {list}"
    );
    // docair-8l6.57: every flow of the system, folded, each to its picture; the changed one to its change too.
    let all = &list[list.find(r#"data-test="all-flows""#).expect("all flows")..];
    assert!(
        all.contains("<summary>All ")
            && all.contains(" flows of the system</summary>")
            && all.contains("<h4>Data · ")
            && all.contains(r##"<a class="link" href="#slice/data.order">order</a> <span class="badge ch-changed">changed</span> <a class="link" href="#changes/data.order" data-test="flow-changes">"##)
            && all.contains(r##"href="#slice/flow:"##),
        "{all}"
    );
    let details = panels["flowChanges"]["data.order"].as_str().unwrap();
    assert!(
        details.contains("Now goes through") && details.contains("orders-db"),
        "the flow's details say what it now goes through"
    );
}

/// The review in the page (docs/EXPLAIN.md §9.5, docair-8l6.22): the AI's changes lead the changes'
/// list, each opens at #review/<n> on its own picture, walked through its pieces with the model's
/// words; its details lead with how, why (with its sources), what to check and where it disagrees,
/// then the flows it touches. Without a review, the list says the AI writes it and offers a button.
#[test]
fn a_review_shows_its_changes_on_their_own_pictures() {
    use docair_core::changes::{Changes, FlowChange};
    use docair_core::diff::{ChangedItem, GraphDiff};
    let after = graph("shop");
    let before = after.clone();
    let flow = |seed: &str| FlowChange {
        seed: seed.to_string(),
        how: "changed".to_string(),
        parts_added: vec![],
        parts_removed: vec![],
        lines_added: vec![],
        lines_removed: vec![],
        through: vec!["orders.create".to_string()],
        certain: true,
    };
    let changes = Changes {
        schema: docair_core::changes::SCHEMA.to_string(),
        a: "0123456789abcdef0123456789abcdef01234567".to_string(),
        b: "fedcba9876543210fedcba9876543210fedcba98".to_string(),
        working_tree: false,
        graph: GraphDiff {
            changed: vec![ChangedItem {
                id: "orders.create".to_string(),
                what: vec!["body".to_string()],
            }],
            ..GraphDiff::default()
        },
        parts: vec!["orders.create".to_string()],
        flows: vec![flow("data.order"), flow("flow:checkout")],
    };
    // The piece the review is about, by the id docair gives it.
    let pieces = docair_core::review::pieces(&before, &after, &changes);
    assert_eq!(pieces.len(), 1, "one piece: the code of orders.create");
    let piece = pieces[0].id().to_string();
    let recorded = serde_json::json!({ "by": "model", "model": "fake-model", "at": "2026-11-19" });
    let findings = serde_json::json!({
        "schema": "docair.explained/2",
        "a": "01234567", "b": "fedcba98",
        "links": [{ "a": "orders.make", "b": "orders.create", "why": "the same work, renamed", "recorded": recorded }],
        "changes": [{
            "title": "Placing an order now checks the cart first.",
            "how": [{ "piece": piece.clone(), "words": "Here `orders.create` now checks the cart before it stores it." }],
            "why": "An order with an empty cart is turned away before it is stored.",
            "off": [
                { "source": "doc", "where": "README.md", "says": "The cart checks itself before it posts.",
                  "code": "The cart is checked in `orders.create`, not in the cart.", "parts": ["orders.create"] },
                "Nothing tests an empty cart."
            ],
            "check": "That `orders.create` still answers fast.",
            "recorded": recorded
        }],
        "problems": [], "left": [], "requests": 1
    });
    // docair-8l6.49: the changed part's whole text before and after, to get down to the code.
    let code = vec![docair_core::review::PartText {
        part: "orders.create".to_string(),
        before: Some("/// Place an order.\npub fn create() {\n    store();\n}".to_string()),
        after: Some(
            "/// Place an order.\npub fn create() {\n    check_cart();\n    store();\n}"
                .to_string(),
        ),
    }];
    let render = |findings: Option<&Value>| {
        let changed = docair_viewer::doc_page::Changed {
            before: &before,
            changes: &changes,
            findings,
            code: &code,
        };
        docair_viewer::doc_page::render_page_with(&after, None, Some(&changed))
    };
    let page = render(Some(&findings));
    assert!(
        page.contains("Δ Changes · 1"),
        "the button counts the review's changes"
    );
    let ix = script(&page, "docair-index");
    let packed = script(&page, "docair-packed");
    let unpacked = |k: &str| -> Value {
        serde_json::from_str(&unpack(
            packed[ix[k].as_u64().unwrap() as usize].as_str().unwrap(),
        ))
        .unwrap()
    };
    let panels = unpacked("panels");
    let list = panels["changes"].as_str().unwrap();
    assert!(
        list.contains(r#"data-test="review-list""#)
            && list.contains(r##"href="#review/1" data-test="review-change">Placing an order now checks the cart first.</a>"##)
            && !list.contains("review-needs-ai"),
        "the review leads the list: {list}"
    );
    assert!(
        list.contains(r#"data-test="changes-list""#)
            && list.contains("The flows the changes touch · 2 of the system's"),
        "the flows follow, with how many the system has: {list}"
    );
    assert!(
        list.contains("<code>orders.make</code> at A is <code>orders.create</code> at B: the same work, renamed"),
        "the model's links are listed"
    );
    let p = &ix["pictures"]["review/1"];
    assert!(
        p["walk"] == true && p["still"] == true && p["change"] == "review/1",
        "{p}"
    );
    let walks = unpacked("walks");
    let steps = walks["review/1"]["steps"].as_array().unwrap();
    // docair-8l6.43, 8l6.54, 8l6.55: it opens on what and why under the change's whole title, once;
    // the AI's why, what it found off, what to check; then the how.
    assert_eq!(steps.len(), 2);
    let intro = steps[0]["card"].as_str().unwrap();
    assert!(
        intro.contains(r#"<span class="card-glyph">Δ</span><b>Placing an "#)
            && intro.contains("</span> first.</b>")
            && intro.matches("Placing an").count() == 1
            && intro.contains(r#"<div class="card-step">What and why</div>"#)
            && intro.contains(r#"data-test="review-why""#)
            && intro.contains(r#"data-test="review-off""#)
            && intro.contains(r#"<dl class="off-item" data-test="off-item" data-source="doc"><dt>Said</dt><dd data-test="off-said"><span class="off-who"><code>README.md</code>:</span> The cart checks itself before it posts.</dd><dt>The code</dt><dd data-test="off-code">The cart is checked in "#)
            // docair-8l6.61: where the code does it, a link to the part, and its code folded.
            && intro.contains(r#"<div class="off-parts" data-test="off-parts">in <code class="node-ref" data-node="orders.create" tabindex="0" title="orders.create">"#)
            && intro.contains(r#"</div><details class="piece-code" data-test="piece-code"><summary>The code · 1 part</summary>"#)
            && intro.contains(r#"data-source="none"><dt>Said</dt><dd data-test="off-said"><span class="muted">Nothing: no commit, doc or comment mentions it.</span></dd><dt>The code</dt><dd data-test="off-code">Nothing tests an empty cart.</dd>"#)
            && intro.contains(r#"data-test="review-check""#)
            && !intro.contains("review-ok")
            && !intro.contains("commit:")
            && !intro.contains("This change"),
        "what and why first: {intro}"
    );
    let at = |t: &str| intro.find(&format!(r#"data-test="{t}""#)).unwrap();
    assert!(
        at("review-why") < at("review-off") && at("review-off") < at("review-check"),
        "{intro}"
    );
    assert_eq!(steps[1]["id"], Value::from(piece));
    let card = steps[1]["card"].as_str().unwrap();
    assert!(
        card.contains("How · 1 of 1")
            && card.contains(r#"data-test="step-words""#)
            && card.contains("before it stores it")
            && card.contains("Code changed in"),
        "the model's words on its piece, over docair's: {card}"
    );
    // Its details hold only what the card does not: which change, the flows, who wrote it.
    let details = panels["flowChanges"]["review/1"].as_str().unwrap();
    for test in ["review-meta", "review-flows"] {
        assert!(
            details.contains(&format!(r#"data-test="{test}""#)),
            "{test}: {details}"
        );
    }
    for gone in [
        "review-title",
        "review-why",
        "review-off",
        "review-check",
        "review-how",
        "piece-code",
        "Placing an order",
    ] {
        assert!(!details.contains(gone), "{gone} is on the card: {details}");
    }
    assert!(details.contains("change 1 of 1") && details.contains("by fake-model · 2026-11-19"));
    // Down to the code, on the piece's card: its part, file and line, its text as a line diff, folded.
    assert!(
        card.contains(r#"data-test="piece-code"><summary>The code · 1 part</summary>"#)
            && card.contains(r#"<span class="same">  /// Place an order.</span>"#)
            && card.contains(r#"<span class="add">+     check_cart();</span>"#)
            && !card.contains(r#"class="del""#),
        "{card}"
    );
    assert!(
        details.contains(r##"href="#changes/data.order""##),
        "the flows it touches link to them"
    );
    // Without a review: the AI writes it, the command says how, docair serve offers a button.
    let page = render(None);
    let ix = script(&page, "docair-index");
    let packed = script(&page, "docair-packed");
    let panels: Value = serde_json::from_str(&unpack(
        packed[ix["panels"].as_u64().unwrap() as usize]
            .as_str()
            .unwrap(),
    ))
    .unwrap();
    let list = panels["changes"].as_str().unwrap();
    assert!(
        list.contains(r#"data-test="review-needs-ai""#)
            && list.contains("docair explain 0123456789abcdef0123456789abcdef01234567 fedcba9876543210fedcba9876543210fedcba98")
            && list.contains(r#"data-test="explain" data-served hidden data-a="0123456789abcdef0123456789abcdef01234567" data-b="fedcba9876543210fedcba9876543210fedcba98""#),
        "the review needs the AI: {list}"
    );
}

//! The graphs of the e2e fixtures, compared with checked-in golden files.
//!
//! A change of what docair makes of a project shows here as a diff. When the
//! change is intended, run `DOCAIR_BLESS=1 cargo test -p docair-adapters` and
//! review the golden files' diff.

use std::path::{Path, PathBuf};

use docair_core::emit::to_json;
use docair_core::model::Graph;
use docair_core::{Options, scan};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../e2e/fixtures")
        .join(name)
}

fn graph(name: &str) -> Graph {
    scan(&fixture(name), docair_adapters::all(), &Options::default()).unwrap()
}

fn golden(name: &str) {
    let actual = to_json(&graph(name));
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.graph.json"));
    if std::env::var_os("DOCAIR_BLESS").is_some() {
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    if actual != expected {
        let out = std::env::temp_dir().join(format!("docair-{name}.graph.json"));
        std::fs::write(&out, &actual).unwrap();
        panic!(
            "the graph of e2e/fixtures/{name} changed: compare {} with {}; if intended, rerun with DOCAIR_BLESS=1",
            path.display(),
            out.display()
        );
    }
}

#[test]
fn shop_graph_is_unchanged() {
    golden("shop");
}

#[test]
fn platform_graph_is_unchanged() {
    golden("platform");
}

#[test]
fn apis_graph_is_unchanged() {
    golden("apis");
}

/// Compare what docair makes of a fixture with `tests/golden/<name>.<what>.json`; `DOCAIR_BLESS=1` rewrites it.
fn golden_file(name: &str, what: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.{what}.json"));
    if std::env::var_os("DOCAIR_BLESS").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    if actual != expected {
        let out = std::env::temp_dir().join(format!("docair-{name}.{what}.json"));
        std::fs::write(&out, actual).unwrap();
        panic!(
            "the {what} of e2e/fixtures/{name} changed: compare {} with {}; if intended, rerun with DOCAIR_BLESS=1",
            path.display(),
            out.display()
        );
    }
}

/// Every slice of a fixture (docs/SLICES.md §3).
fn golden_slices(name: &str) {
    golden_file(
        name,
        "slices",
        &docair_core::slice::to_json(&docair_core::slice::all(&graph(name))),
    );
}

/// What every layer of a fixture draws, nothing opened (SPEC §3.4).
fn golden_layers(name: &str) {
    use docair_core::layer_view::{layers, picture, to_json};
    let g = graph(name);
    let pictures: Vec<_> = layers(&g)
        .iter()
        .filter_map(|l| picture(&g, l.key, &[], None))
        .collect();
    golden_file(name, "layers", &to_json(&pictures));
}

/// The section of every box of a fixture (docs/SLICES.md §4).
fn golden_sections(name: &str) {
    use docair_core::layer_view::{section_picture, to_json};
    let g = graph(name);
    let boxes: std::collections::BTreeSet<&str> = g
        .nodes
        .iter()
        .filter_map(|n| n.part_of.as_deref())
        .collect();
    let sections: Vec<_> = boxes
        .iter()
        .filter_map(|b| section_picture(&g, b, None))
        .map(|(_, p)| p)
        .collect();
    golden_file(name, "sections", &to_json(&sections));
}

/// The messages of every step of every slice with steps (viewer.js walkMessages, in core).
fn golden_messages(name: &str) {
    use docair_core::slice::{all, messages, to_json};
    let g = graph(name);
    let each: Vec<_> = all(&g)
        .into_iter()
        .filter(|s| !s.steps.is_empty())
        .map(|s| (s.seed.clone(), messages(&g, &s)))
        .collect();
    golden_file(name, "messages", &to_json(&each));
}

#[test]
fn shop_messages_are_unchanged() {
    golden_messages("shop");
}

#[test]
fn answer_messages_are_unchanged() {
    golden_messages("answer");
}

/// A walk keeps one call stack: an answer comes back after the work it waits for (SPEC §7.3).
#[test]
fn a_walks_answers_come_back_after_the_work_they_wait_for() {
    use docair_core::slice::{find, messages, slice};
    let g = graph("answer");
    let s = slice(&g, &find(&g, "data.summary").unwrap()).unwrap();
    let kinds: Vec<Vec<String>> = messages(&g, &s)
        .iter()
        .map(|ms| {
            ms.iter()
                .map(|m| format!("{} {}", m.kind, m.who.join(" ")))
                .collect()
        })
        .collect();
    assert_eq!(
        kinds,
        [
            Vec::<String>::new(),
            vec!["return summarize answers ask".to_string()],
            vec!["call ask calls model".to_string()],
            vec!["call report calls mail".to_string()],
        ],
        "made, brought back in an answer, sent on; mailing the report is a stretch of its own"
    );
}

#[test]
fn shop_sections_are_unchanged() {
    golden_sections("shop");
}

#[test]
fn platform_sections_are_unchanged() {
    golden_sections("platform");
}

#[test]
fn apis_sections_are_unchanged() {
    golden_sections("apis");
}

#[test]
fn fold_sections_are_unchanged() {
    golden_sections("fold");
}

/// A box's section holds the box one level down and folds what its lines join to (SPEC §3.4).
#[test]
fn a_section_is_its_box_one_level_down_and_its_context() {
    use docair_core::layer_view::{LayerKey, section_picture};
    let g = graph("shop");
    let (key, p) = section_picture(&g, "orders-api", None).unwrap();
    assert_eq!(key, LayerKey::Apps);
    let ids: Vec<&str> = p.parts.iter().map(|x| x.id.as_str()).collect();
    assert!(
        ids.contains(&"orders-api") && ids.contains(&"orders.create"),
        "{ids:?}"
    );
    assert!(
        ids.contains(&"orders-db"),
        "what its lines join to comes as context: {ids:?}"
    );
    assert!(p.parts.iter().any(|x| x.id == "orders-api" && x.open));
    assert!(
        section_picture(&g, "orders.create", None).is_none(),
        "a part that holds nothing has no section"
    );
}

#[test]
fn shop_layers_are_unchanged() {
    golden_layers("shop");
}

#[test]
fn platform_layers_are_unchanged() {
    golden_layers("platform");
}

#[test]
fn apis_layers_are_unchanged() {
    golden_layers("apis");
}

#[test]
fn answer_layers_are_unchanged() {
    golden_layers("answer");
}

#[test]
fn fold_layers_are_unchanged() {
    golden_layers("fold");
}

/// At Code, parts docair only found in the code wait until their box is opened: the box says how many it
/// leaves out, and opened it draws them, a module of such parts standing closed (SPEC §3.4).
#[test]
fn parts_found_only_in_the_code_wait_until_their_box_is_opened() {
    use docair_core::layer_view::{LayerKey, picture};
    let g = graph("fold");
    let shut = picture(&g, LayerKey::Code, &[], None).unwrap();
    let crate_box = shut.parts.iter().find(|p| p.id == "billing").unwrap();
    assert!(crate_box.open && crate_box.more == 2, "{crate_box:?}");
    let opened = picture(&g, LayerKey::Code, &["billing".to_string()], None).unwrap();
    let ids: Vec<&str> = opened.parts.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "billing",
            "billing.bill",
            "billing.money",
            "billing.tax",
            "billing.fee"
        ]
    );
    let money = opened
        .parts
        .iter()
        .find(|p| p.id == "billing.money")
        .unwrap();
    assert!(money.closed && money.holds == 3, "{money:?}");
}

#[test]
fn shop_slices_are_unchanged() {
    golden_slices("shop");
}

#[test]
fn answer_slices_are_unchanged() {
    golden_slices("answer");
}

#[test]
fn platform_slices_are_unchanged() {
    golden_slices("platform");
}

#[test]
fn apis_slices_are_unchanged() {
    golden_slices("apis");
}

/// A datum's slice walks its way as Follow does (SPEC §3.6): made, brought back inside an answer,
/// sent on, and a line its way cannot reach as a stretch of its own.
#[test]
fn a_datum_slice_follows_its_way_there_and_back() {
    use docair_core::slice::{CarryWay, SeedKind, find, slice};
    let g = graph("answer");
    let seed = find(&g, "data.summary").unwrap();
    assert_eq!(seed.kind, SeedKind::Data);
    let s = slice(&g, &seed).unwrap();
    let walk: Vec<(String, Option<CarryWay>, bool, bool)> = s
        .steps
        .iter()
        .map(|st| (st.nodes[0].clone(), st.way, st.inside, st.apart))
        .collect();
    assert_eq!(
        walk,
        [
            ("app.summarize".to_string(), None, false, false),
            ("app.ask".to_string(), Some(CarryWay::Back), true, false),
            ("model".to_string(), Some(CarryWay::There), false, false),
            ("mail".to_string(), Some(CarryWay::There), false, true),
        ]
    );
    let covers = s.covers.unwrap();
    assert_eq!((covers.lines, covers.reached, covers.apart), (2, 1, 1));
    let ends: Vec<&str> = s.ends.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ends, ["app.summarize", "model", "mail"]);
}

/// API descriptions become parts of the graph (e2e/fixtures/apis, docs/IMPORTS.md).
#[test]
fn api_descriptions_are_parts_where_they_belong() {
    let g = graph("apis");
    // Exactly the drift the fixture plants (docs/IMPORTS.md): a misspelled route,
    // a declared route no code serves, a served route the description leaves out,
    // and an event the mailer's description receives that no code handles.
    let mut drift: Vec<(String, String)> = g
        .diagnostics
        .iter()
        .map(|d| (d.code.clone(), d.node.clone().unwrap_or_default()))
        .collect();
    drift.sort();
    assert_eq!(
        drift,
        [
            ("W103".to_string(), "orders.update".to_string()),
            ("W114".to_string(), "api.openapi.cancel-order".to_string()),
            ("W114".to_string(), "mailer.asyncapi".to_string()),
            ("W115".to_string(), "orders.list".to_string()),
        ]
    );
    let node = |id: &str| {
        g.nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| panic!("no node {id}"))
    };
    for id in ["api.openapi", "mailer.asyncapi", "vendor.payments-openapi"] {
        assert_eq!(node(id).kind, "api", "{id}");
    }
    // [[import.api]] part_of, then x-docair-part-of, else the service declared in its directory.
    assert_eq!(
        node("vendor.payments-openapi").part_of.as_deref(),
        Some("payments")
    );
    assert_eq!(node("mailer.asyncapi").part_of.as_deref(), Some("mailer"));
    assert_eq!(node("api.openapi").part_of.as_deref(), Some("orders-api"));
    assert_eq!(node("api.openapi").name, "openapi.yaml");
    assert_eq!(
        node("api.openapi").summary,
        "Orders API: The orders a shopper places."
    );
}

/// Declared routes merge into the handlers that serve them; the rest stay endpoints (docs/IMPORTS.md).
#[test]
fn declared_routes_meet_their_handlers() {
    let g = graph("apis");
    let has = |id: &str| g.nodes.iter().any(|n| n.id == id);
    let described = |id: &str| {
        g.nodes
            .iter()
            .find(|n| n.id == id)
            .and_then(|n| n.tags.iter().find(|t| t.name == "described"))
            .map(|t| t.value.clone())
    };
    // The same route, also when the parameter has another name.
    assert_eq!(
        described("orders.create").as_deref(),
        Some("POST /api/orders (CreateOrder) -> Order")
    );
    assert_eq!(
        described("orders.get").as_deref(),
        Some("GET /api/orders/{id} (id) -> Order")
    );
    assert!(!has("api.openapi.create-order") && !has("api.openapi.get-order"));
    // A route no handler serves stays inside the description; one named like a
    // function merges into it although the routes differ (W103 says so).
    assert!(has("api.openapi.cancel-order") && !has("api.openapi.update_order"));
    assert_eq!(
        described("orders.update").as_deref(),
        Some("PUT /api/orders/{id} (Order) -> Order")
    );
    // Of a vendored description, only what the code calls.
    assert!(has("vendor.payments-openapi.create-charge"));
    assert!(!has("vendor.payments-openapi.list-charges"));
    assert!(!has("vendor.payments-openapi.create-refund"));
    // Callers carry the description's words.
    let label = |to: &str| {
        g.edges
            .iter()
            .find(|e| e.from == "shop-ui.cart" && e.to == to)
            .and_then(|e| e.label.clone())
    };
    assert_eq!(
        label("orders.create").as_deref(),
        Some("(CreateOrder) → Order")
    );
    assert_eq!(
        label("vendor.payments-openapi.create-charge").as_deref(),
        Some("(ChargeRequest) → Charge")
    );
}

/// AsyncAPI channels are the topics the code sends to; the code's own lines win (docs/IMPORTS.md).
#[test]
fn declared_topics_meet_their_code() {
    let g = graph("apis");
    let node = |id: &str| g.nodes.iter().find(|n| n.id == id);
    let created = node("topic:order.created").unwrap();
    assert_eq!(created.summary, "A new order was stored.");
    assert_eq!(created.part_of.as_deref(), Some("orders"));
    // Everything api/asyncapi.yaml says, the code says too: its node goes.
    assert!(node("api.asyncapi").is_none());
    let line = |from: &str, to: &str| {
        g.edges
            .iter()
            .find(|e| e.from == from && e.to == to)
            .map(|e| (e.label.clone(), e.from_api.clone()))
    };
    // One line each way, from the code, carrying the message the description names.
    assert_eq!(
        line("orders.create", "topic:order.created"),
        Some((
            Some("OrderCreated".to_string()),
            Some("api/asyncapi.yaml".to_string())
        ))
    );
    assert!(line("topic:order.created", "mailer.confirm").is_some());
    assert!(line("topic:order.created", "mailer.asyncapi").is_none());
    // What only the description says stays its own line.
    assert!(line("topic:order.cancelled", "mailer.asyncapi").is_some());
}

/// The whole chain from infrastructure code without one doc block (e2e/fixtures/platform).
#[test]
fn terraform_and_charts_tell_how_things_are_made_and_routed() {
    let g = graph("platform");
    assert!(g.diagnostics.is_empty(), "{:?}", g.diagnostics);
    let part_of = |id: &str| {
        g.nodes
            .iter()
            .find(|n| n.id == id)
            .and_then(|n| n.part_of.clone())
    };
    assert_eq!(part_of("k8s.cluster.dev").as_deref(), Some("platform"));
    assert_eq!(part_of("k8s.ns.shop").as_deref(), Some("k8s.cluster.dev"));
    assert_eq!(part_of("k8s.release.shop").as_deref(), Some("k8s.ns.shop"));
    assert_eq!(
        part_of("k8s.deploy.api").as_deref(),
        Some("k8s.release.shop")
    );
    assert_eq!(
        part_of("tf.helm_release.shop").as_deref(),
        Some("terraform")
    );
    let edge = |from: &str, to: &str| {
        g.edges
            .iter()
            .find(|e| e.from == from && e.to == to)
            .map(|e| {
                (
                    format!("{:?}", e.kind).to_lowercase(),
                    e.label.clone().unwrap_or_default(),
                )
            })
    };
    assert_eq!(
        edge("tf.kind_cluster.dev", "k8s.cluster.dev").unwrap().0,
        "create"
    );
    assert_eq!(
        edge("tf.helm_release.shop", "k8s.release.shop").unwrap(),
        ("create".into(), "helm install: chart shop".into())
    );
    assert_eq!(
        edge("tf.helm_release.ingress", "tf.helm_release.shop")
            .unwrap()
            .1,
        "runs after"
    );
    assert_eq!(
        edge("tf.kind_cluster.dev", "tf.helm_release.shop")
            .unwrap()
            .1,
        "cluster access"
    );
    assert_eq!(
        edge("k8s.release.traefik", "k8s.ingress.shop").unwrap().1,
        "requests (ingress class traefik)"
    );
    assert_eq!(
        edge("k8s.ingress.shop", "k8s.svc.api").unwrap(),
        ("route".into(), "HTTP /api, port 80".into())
    );
    assert_eq!(
        edge("k8s.svc.web", "k8s.deploy.web").unwrap(),
        ("select".into(), "port 8080".into())
    );
    assert!(
        edge("k8s.svc.api", "k8s.deploy.web").is_none(),
        "selectors pick their own workload"
    );
}

/// One call stack for the whole walk: a call's answer goes back only after the work it waits for,
/// even when that work is a later step, and a line from a part not on the stack closes no call.
#[test]
fn a_call_stays_open_while_its_callee_works_in_later_steps() {
    use docair_core::slice::{Slice, SliceStep, find, messages, slice};
    let g = graph("shop");
    let mut s = slice(&g, &find(&g, "flow:checkout").unwrap()).unwrap();
    let at = |from: &str, to: &str| {
        g.edges
            .iter()
            .position(|e| e.from == from && e.to == to)
            .unwrap_or_else(|| panic!("no line {from} -> {to}"))
    };
    let step = |lines: Vec<usize>| SliceStep {
        nodes: Vec::new(),
        lines,
        made: false,
        way: None,
        inside: false,
        apart: false,
        label: None,
    };
    let words = |s: &Slice| -> Vec<Vec<String>> {
        messages(&g, s)
            .iter()
            .map(|ms| {
                ms.iter()
                    .map(|m| format!("{} {}>{}", m.kind, g.edges[m.edge].from, g.edges[m.edge].to))
                    .collect()
            })
            .collect()
    };
    s.steps = vec![
        step(vec![at("shop-ui.cart", "orders.create")]),
        step(vec![at("orders.create", "orders-db")]),
    ];
    assert_eq!(
        words(&s),
        [
            vec!["call shop-ui.cart>orders.create"],
            vec![
                "write orders.create>orders-db",
                "return shop-ui.cart>orders.create"
            ],
        ]
    );
    s.steps = vec![
        step(vec![at("shop-ui.cart", "orders.create")]),
        step(vec![at("tf.cluster", "cluster")]),
        step(vec![at("orders.create", "orders-db")]),
    ];
    assert_eq!(
        words(&s),
        [
            vec!["call shop-ui.cart>orders.create"],
            vec!["create tf.cluster>cluster"],
            vec![
                "write orders.create>orders-db",
                "return shop-ui.cart>orders.create"
            ],
        ]
    );
}

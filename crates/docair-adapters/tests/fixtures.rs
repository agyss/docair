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
            .map(|e| (e.label.clone(), e.words_from_api().map(str::to_string)))
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

/// A ref into code by the sub-part form is a stale ref: W122, and the line still goes to the part (docair-6u3).
#[test]
fn a_sub_part_of_code_is_a_stale_ref() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("docair.toml"), "system = \"s\"\n").unwrap();
    std::fs::write(
        dir.path().join("README.md"),
        "# s\n\n```docair\nThe app.\n\n@id      app\n@kind    module\n@part-of s\n```\n\n```docair\nThe store.\n\n@id      db\n@kind    store\n@part-of s\n```\n\n```docair\nA test.\n\n@id      t\n@kind    test\n@part-of s\n@covers  app.gone\n@writes  db.orders\n```\n",
    )
    .unwrap();
    let g = docair_core::scan(
        dir.path(),
        docair_adapters::all(),
        &docair_core::Options {
            lenient: true,
            ..Default::default()
        },
    )
    .unwrap();
    let w: Vec<&str> = g
        .diagnostics
        .iter()
        .filter(|d| d.code == "W122")
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].contains("`app.gone`") && w[0].contains("no part `gone`"),
        "{w:?}"
    );
    assert!(
        g.edges.iter().any(|e| e.from == "t" && e.to == "app"),
        "the line still goes to the part"
    );
}

/// A scenario steps along a request line the code sends, not only along hand-written lines (docair-4wk.15).
#[test]
fn a_walk_steps_along_a_request() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("docair.toml"), "system = \"s\"\n").unwrap();
    std::fs::write(
        dir.path().join("README.md"),
        "# s\n\n```docair\nThe page.\n\n@id       page\n@kind     component\n@part-of  s\n@requests POST /api/orders\n@flow     buy 1 \"the page sends the order\"\n```\n\n```docair\nThe handler.\n\n@id      api\n@kind    function\n@part-of s\n@http    POST /api/orders\n@flow    buy 2 \"the API takes it\"\n```\n",
    )
    .unwrap();
    let g = docair_core::scan(
        dir.path(),
        docair_adapters::all(),
        &docair_core::Options {
            lenient: true,
            ..Default::default()
        },
    )
    .unwrap();
    let w104: Vec<&str> = g
        .diagnostics
        .iter()
        .filter(|d| d.code == "W104")
        .map(|d| d.message.as_str())
        .collect();
    assert!(w104.is_empty(), "{w104:?}");
    let buy = g.flows.iter().find(|f| f.name == "buy").unwrap();
    let step2 = &buy.steps[1];
    assert_eq!(step2.path.len(), 1, "{buy:?}");
    let e = &g.edges[step2.path[0]];
    assert_eq!((e.from.as_str(), e.to.as_str()), ("page", "api"));
}

/// A store that says where it is gets a certain line from code that writes the path out, with no block on the function (docair-4wk.25).
#[test]
fn a_store_with_a_path_is_read_by_the_code_that_names_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("docair.toml"), "system = \"s\"\n").unwrap();
    std::fs::write(
        dir.path().join("README.md"),
        "# s\n\n```docair\nThe prices, one file per shop.\n\n@id      prices\n@kind    store\n@part-of s\n@path    data/*.json\n```\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"s\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src").join("lib.rs"),
        "pub fn prices() -> String {\n    std::fs::read_to_string(\"data/shop.json\").unwrap_or_default()\n}\n",
    )
    .unwrap();
    let g = docair_core::scan(
        dir.path(),
        docair_adapters::all(),
        &docair_core::Options {
            lenient: true,
            untrusted: true,
        },
    )
    .unwrap();
    let read = g
        .edges
        .iter()
        .find(|e| e.from == "prices" && e.kind == docair_core::model::EdgeKind::Read)
        .unwrap_or_else(|| panic!("{:#?}", g.edges));
    assert!(read.to.ends_with("prices"), "{}", read.to);
    assert!(read.reach.is_none(), "certain");
}

/// A Service reaches the program its pods run: the Deployment's image, the Dockerfile Terraform builds it from, the crate that Dockerfile builds (docair-4wk.26).
#[test]
fn a_service_reaches_the_program_its_pods_run() {
    let dir = tempfile::tempdir().unwrap();
    let w = |rel: &str, text: &str| {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    w("docair.toml", "system = \"s\"\n");
    w(
        "app/Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    w("app/src/main.rs", "fn main() {}\n");
    w(
        "docker/app.Dockerfile",
        "FROM rust:1 AS build\nCOPY app ./app\nRUN cargo build --release -p app\nFROM debian\nCOPY --from=build /x /y\n",
    );
    w(
        "infra/images.tf",
        "locals {\n  app_image = \"app:${var.tag}\"\n}\n\nresource \"terraform_data\" \"images\" {\n  provisioner \"local-exec\" {\n    command = \"docker build -f docker/app.Dockerfile -t ${local.app_image} .\"\n  }\n}\n",
    );
    w("chart/Chart.yaml", "name: app\n");
    w(
        "chart/values.yaml",
        "image:\n  repository: app\n  tag: dev\n",
    );
    w(
        "chart/templates/deploy.yaml",
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: app\n  labels:\n    app: app\nspec:\n  template:\n    spec:\n      containers:\n        - name: app\n          image: \"{{ .Values.image.repository }}:{{ .Values.image.tag }}\"\n",
    );
    w(
        "chart/templates/svc.yaml",
        "apiVersion: v1\nkind: Service\nmetadata:\n  name: app\nspec:\n  selector:\n    app: app\n  ports:\n    - port: 80\n      targetPort: 8080\n",
    );
    let g = docair_core::scan(
        dir.path(),
        docair_adapters::all(),
        &docair_core::Options {
            lenient: true,
            untrusted: true,
        },
    )
    .unwrap();
    let to: Vec<&str> = g
        .edges
        .iter()
        .filter(|e| e.from.ends_with("svc.app") && e.kind == docair_core::model::EdgeKind::Call)
        .map(|e| e.to.as_str())
        .collect();
    assert_eq!(
        to,
        ["app.main"],
        "{:#?}",
        g.edges.iter().map(|e| (&e.from, &e.to)).collect::<Vec<_>>()
    );
}

/// With scip-typescript installed, a TypeScript call only the compiler can resolve is a certain line (docair-4wk.14). Skipped where web/node_modules is not installed.
#[test]
fn a_typescript_call_comes_from_scip_typescript() {
    let modules = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/node_modules");
    if !modules.join(".bin/scip-typescript").is_file() {
        eprintln!("skipped: no scip-typescript in web/node_modules");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let w = |rel: &str, text: &str| {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    w(
        "docair.toml",
        "system = \"s\"\n\n[code]\nrun_indexers = true\n",
    );
    w(
        "app/package.json",
        "{ \"name\": \"app\", \"version\": \"0.0.0\" }\n",
    );
    w(
        "app/tsconfig.json",
        "{ \"compilerOptions\": { \"strict\": true }, \"include\": [\"src\"] }\n",
    );
    w(
        "app/src/a.ts",
        "export function helper(): number {\n  return 1;\n}\n",
    );
    w(
        "app/src/b.ts",
        "import * as m from './a';\n\nexport function run(): number {\n  return m.helper();\n}\n",
    );
    std::os::unix::fs::symlink(
        modules.canonicalize().unwrap(),
        dir.path().join("app/node_modules"),
    )
    .unwrap();
    let cache = tempfile::tempdir().unwrap();
    // SAFETY: tests in this file set no other environment; the cache is this test's own.
    unsafe { std::env::set_var("DOCAIR_CACHE", cache.path()) };
    let g = docair_core::scan(
        dir.path(),
        docair_adapters::all(),
        &docair_core::Options::default(),
    )
    .unwrap();
    let call = g
        .edges
        .iter()
        .find(|e| e.from.ends_with("run") && e.to.ends_with("helper"))
        .unwrap_or_else(|| {
            panic!(
                "{:#?}",
                g.edges.iter().map(|e| (&e.from, &e.to)).collect::<Vec<_>>()
            )
        });
    assert!(call.reach.is_none(), "certain");
}

/// Every line has an id, unique in its graph, and the same after a scan that changed something else (docair-svh).
#[test]
fn every_line_has_a_unique_stable_id() {
    for name in ["shop", "platform", "apis", "fold", "answer", "words"] {
        let g = graph(name);
        let mut ids: Vec<&str> = g.edges.iter().map(|e| e.id.as_str()).collect();
        assert!(
            ids.iter().all(|i| !i.is_empty()),
            "{name}: a line without an id"
        );
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "{name}: two lines share an id");
        for (i, e) in g.edges.iter().enumerate() {
            assert_eq!(
                docair_core::ids::find(&g, &e.id),
                Some(docair_core::ids::IdTarget::Line(i, e)),
                "{name}: {} finds its line",
                e.id
            );
        }
    }
    // Stable: an unrelated part added to the shop keeps every line's id.
    let before = graph("shop");
    let dir = tempfile::tempdir().unwrap();
    let shop = dir.path().join("shop");
    copy_dir(&fixture_dir("shop"), &shop);
    let readme = shop.join("README.md");
    let text = std::fs::read_to_string(&readme).unwrap();
    std::fs::write(&readme, format!("```docair\nA new part.\n\n@id      extra\n@kind    external\n@part-of shop\n@calls   shopper\n```\n\n{text}")).unwrap();
    let after = docair_core::scan(
        &shop,
        docair_adapters::all(),
        &docair_core::Options {
            lenient: true,
            ..Default::default()
        },
    )
    .unwrap();
    for e in &before.edges {
        assert!(
            after.edges.iter().any(|x| x.id == e.id),
            "{} kept its id",
            e.id
        );
    }
}

fn fixture_dir(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../e2e/fixtures")
        .join(name)
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

/// Two scans compared: a renamed part with the same body is a move (its caller's line too), a changed body is a change, a new part and its line are added (docair-zrr.4).
#[test]
fn two_graphs_differ_in_moves_changes_and_new_parts() {
    let scan_of = |lib: &str| {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("docair.toml"), "system = \"s\"\n").unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"s\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), lib).unwrap();
        let g = docair_core::scan(
            dir.path(),
            docair_adapters::all(),
            &docair_core::Options {
                lenient: true,
                ..Default::default()
            },
        )
        .unwrap();
        let root = dir.path().to_path_buf();
        let fp =
            docair_core::diff::fingerprints(&g, &|p| std::fs::read_to_string(root.join(p)).ok());
        (g, fp, dir)
    };
    let (a, fa, _da) = scan_of(
        "pub fn load() -> u8 {\n    1\n}\n\npub fn run() -> u8 {\n    load() + 1\n}\n\npub fn save(x: u8) {\n    let _ = x;\n}\n",
    );
    let (b, fb, _db) = scan_of(
        "pub fn read() -> u8 {\n    1\n}\n\npub fn run() -> u8 {\n    read() + 1\n}\n\npub fn save(x: u8) {\n    let _ = x + 1;\n}\n\npub fn extra() {\n    save(2);\n}\n",
    );
    let d = docair_core::diff::diff(&a, &b, &fa, &fb);
    let moved: Vec<(&str, &str)> = d
        .moved
        .iter()
        .map(|m| (m.from.as_str(), m.to.as_str()))
        .collect();
    assert_eq!(moved, [("s.load", "s.read")], "{d:#?}");
    assert_eq!(d.added, ["s.extra"], "{d:#?}");
    assert!(d.removed.is_empty(), "{d:#?}");
    let changed: Vec<(&str, Vec<&str>)> = d
        .changed
        .iter()
        .map(|c| (c.id.as_str(), c.what.iter().map(String::as_str).collect()))
        .collect();
    assert_eq!(
        changed,
        [("s.save", vec!["body"])],
        "the caller of the renamed part is no change: {d:#?}"
    );
    assert_eq!(d.lines_added, ["s.extra -call-> s.save"], "{d:#?}");
    assert!(
        d.lines_removed.is_empty(),
        "run -> load is run -> read: {d:#?}"
    );
}

/// Two items of one file whose names give one id keep their own parts and lines: the one a block
/// names keeps the id, else the type; the other gets its kind after it (docair-4wk.38).
#[test]
fn two_items_with_one_id_keep_their_own_parts() {
    let dir = std::env::temp_dir().join(format!("docair-one-id-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/lib.rs"),
        "pub struct Uploads;\n\npub struct Config {\n    pub uploads: Uploads,\n}\n\nimpl Config {\n    /***\n     * Read the settings.\n     *\n     * @id      lib.config\n     ***/\n    pub fn from_env() -> Config {\n        Config { uploads: Uploads }\n    }\n\n    pub fn size(&self) -> usize {\n        0\n    }\n}\n\npub enum Family {\n    Request,\n}\n\npub fn family() -> Family {\n    Family::Request\n}\n\npub struct ApiError;\n\nimpl From<std::io::Error> for ApiError {\n    fn from(_: std::io::Error) -> Self {\n        ApiError\n    }\n}\n\nimpl From<anyhow::Error> for ApiError {\n    fn from(_: anyhow::Error) -> Self {\n        ApiError\n    }\n}\n\nimpl From<tokio::task::JoinError> for ApiError {\n    fn from(_: tokio::task::JoinError) -> Self {\n        ApiError\n    }\n}\n",
    )
    .unwrap();
    let g = scan(&dir, docair_adapters::all(), &Options::default()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let node = |id: &str| {
        g.nodes.iter().find(|n| n.id == id).unwrap_or_else(|| {
            panic!(
                "{id}: {:?}",
                g.nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
            )
        })
    };
    assert_eq!(
        node("lib.config").symbol.as_deref(),
        Some("fn from_env"),
        "the block's id"
    );
    assert_eq!(
        node("lib.config-struct").symbol.as_deref(),
        Some("struct Config")
    );
    assert_eq!(
        node("lib.config-struct.size").part_of.as_deref(),
        Some("lib.config-struct"),
        "its methods go along"
    );
    assert_eq!(
        node("lib.family").symbol.as_deref(),
        Some("enum Family"),
        "the type keeps it"
    );
    assert_eq!(node("lib.family-fn").symbol.as_deref(), Some("fn family"));
    // Overloads say what tells them apart, each of them (docair-4wk.41).
    for id in [
        "lib.api-error.from-io-error",
        "lib.api-error.from-anyhow-error",
        "lib.api-error.from-join-error",
    ] {
        assert_eq!(node(id).symbol.as_deref(), Some("fn from"), "{id}");
    }
    let has = |from: &str, kind: &str, to: &str| {
        g.edges
            .iter()
            .any(|e| e.from == from && e.to == to && format!("{:?}", e.kind).to_lowercase() == kind)
    };
    assert!(
        has("lib.config-struct", "use", "lib.uploads"),
        "Config keeps its lines: {:?}",
        g.edges
    );
    assert!(
        has("lib.family-fn", "use", "lib.family"),
        "family returns a Family: {:?}",
        g.edges
    );
}

/// A use no line brings is drawn (main makes Cli); one a signature brings is listed; a use between
/// two types is drawn in the type's own section, not on its layer (docair-4wk.43, 4wk.44).
#[test]
fn uses_are_drawn_where_they_bring_something() {
    let dir = std::env::temp_dir().join(format!("docair-drawn-uses-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/lib.rs"),
        "pub struct Cli {\n    pub command: Command,\n}\n\nimpl Cli {\n    pub fn parse() -> Cli {\n        Cli { command: Command::Build }\n    }\n}\n\npub enum Command {\n    Build,\n}\n\npub fn run(cli: Cli) {\n    match cli.command {\n        Command::Build => {}\n    }\n}\n\npub fn main() {\n    let cli = Cli { command: Command::Build };\n    let _ = cli.command;\n}\n\npub fn start() {\n    run(Cli::parse());\n}\n",
    )
    .unwrap();
    let g = scan(&dir, docair_adapters::all(), &Options::default()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let id = |name: &str| {
        g.nodes
            .iter()
            .find(|n| n.name == name)
            .map(|n| n.id.clone())
            .unwrap_or_else(|| panic!("{name}"))
    };
    let usage = |from: &str, to: &str| {
        g.edges
            .iter()
            .find(|e| {
                e.from == id(from) && e.to == id(to) && e.kind == docair_core::model::EdgeKind::Use
            })
            .unwrap_or_else(|| panic!("{from} uses {to}: {:?}", g.edges))
    };
    assert!(
        usage("run", "Cli").listed,
        "run takes Cli: its callers' line brings it"
    );
    assert!(
        usage("run", "Command").listed,
        "Cli, which run takes, holds Command"
    );
    assert!(
        !usage("main", "Cli").listed,
        "main makes Cli, and no line brings it"
    );
    assert!(
        usage("Cli", "Command").listed,
        "between two types: not on the layer"
    );
    let (_, section) =
        docair_core::layer_view::section_picture(&g, &id("Cli"), None).expect("Cli's section");
    let holds = usage("Cli", "Command") as *const _;
    assert!(
        section
            .lines
            .iter()
            .flat_map(|l| &l.members)
            .any(|&i| std::ptr::eq(&g.edges[i], holds)),
        "Cli's own section draws that it holds Command: {:?}",
        section.lines
    );
}

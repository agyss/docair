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

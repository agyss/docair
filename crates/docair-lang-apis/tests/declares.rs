//! What the API descriptions of e2e/fixtures/apis declare, one sorted line each (docs/IMPORTS.md).

use std::path::Path;

use docair_core::adapter::{Adapter, Project};
use docair_core::facts::{Facts, Role};
use docair_core::{config, discover};
use docair_lang_apis::ApiAdapter;

fn fixture_facts() -> Facts {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../e2e/fixtures/apis");
    let config = config::load(&root).unwrap();
    let files = discover::discover(&root, &config).unwrap();
    let blocks = vec![Vec::new(); files.len()];
    ApiAdapter.project(&Project {
        system: "orders",
        config: &config,
        files: &files,
        blocks: &blocks,
    })
}

#[test]
fn the_fixture_declares_its_routes_and_topics() {
    let facts = fixture_facts();
    assert!(facts.diagnostics.is_empty(), "{:?}", facts.diagnostics);
    let mut lines: Vec<String> = facts
        .declares
        .iter()
        .map(|d| {
            let file = d.by.trim_start_matches("apis:");
            let role = match d.role {
                Role::Serves => "serves",
                Role::Sends => "sends",
                Role::Receives => "receives",
            };
            let payload = d
                .payload
                .as_deref()
                .map(|p| format!(" ({p})"))
                .unwrap_or_default();
            let reply = d
                .reply
                .as_deref()
                .map(|r| format!(" -> {r}"))
                .unwrap_or_default();
            let name = d
                .name
                .as_deref()
                .map(|n| format!(" [{n}]"))
                .unwrap_or_default();
            format!(
                "{file}:{} {role} {}{payload}{reply}{name}",
                d.evidence.line, d.at
            )
        })
        .collect();
    lines.sort();
    assert_eq!(lines.join("\n"), EXPECTED.trim());
}

const EXPECTED: &str = "
api/asyncapi.yaml:17 sends topic:order.created (OrderCreated) [sendOrderCreated]
api/openapi.yaml:10 serves POST /api/orders (CreateOrder) -> Order [createOrder]
api/openapi.yaml:33 serves GET /api/orders/{id} (id) -> Order [getOrder]
api/openapi.yaml:43 serves PUT /api/orders/{id} (Order) -> Order [update_order]
api/openapi.yaml:58 serves DELETE /api/orders/{id} (id) [cancelOrder]
mailer/asyncapi.yaml:13 receives topic:order.created (OrderCreated) [receiveOrderCreated]
mailer/asyncapi.yaml:19 receives topic:order.cancelled (OrderCancelled) [receiveOrderCancelled]
vendor/payments-openapi.json:12 serves POST /v1/charges (ChargeRequest) -> Charge [createCharge]
vendor/payments-openapi.json:22 serves GET /v1/charges (limit) -> Charge[] [listCharges]
vendor/payments-openapi.json:31 serves POST /v1/refunds (Refund) -> Refund [createRefund]
";

/// A broken description is reported with its line; the scan goes on.
#[test]
fn a_broken_description_says_where() {
    use docair_core::lang::language_for;
    use docair_core::model::SourceFile;
    let file = SourceFile {
        path: "broken/openapi.yaml".into(),
        rel: "broken/openapi.yaml".into(),
        lang: language_for("yaml").unwrap(),
        text: "openapi: 3.0.0\ninfo: {title: X}\npaths:\n  /x: [unclosed\n".into(),
    };
    let config = docair_core::config::Config::default();
    let files = vec![file];
    let blocks = vec![Vec::new()];
    let facts = ApiAdapter.project(&Project {
        system: "s",
        config: &config,
        files: &files,
        blocks: &blocks,
    });
    assert!(facts.symbols.is_empty() && facts.declares.is_empty());
    let d = &facts.diagnostics[0];
    assert_eq!((d.code.as_str(), d.line), ("W113", Some(4)));
}

/// A doc block right above the version key documents the description: it attaches to the block.
#[test]
fn a_documented_description_takes_its_block() {
    use docair_core::lang::language_for;
    use docair_core::model::SourceFile;
    let text = "#***\n# The orders API, as the client team sees it.\n#\n# @part-of orders-api\n#***\nopenapi: 3.1.0\ninfo: {title: Orders}\npaths: {}\n";
    let file = SourceFile {
        path: "api/openapi.yaml".into(),
        rel: "api/openapi.yaml".into(),
        lang: language_for("yaml").unwrap(),
        text: text.into(),
    };
    let mut diags = Vec::new();
    let blocks = vec![docair_core::extract::extract(&file, &mut diags)];
    let config = docair_core::config::Config::default();
    let files = vec![file];
    let facts = ApiAdapter.project(&Project {
        system: "s",
        config: &config,
        files: &files,
        blocks: &blocks,
    });
    assert_eq!(facts.symbols.len(), 1);
    assert_eq!(facts.symbols[0].draw, docair_core::facts::Draw::Always);
    assert_eq!(facts.symbols[0].line, 6);
}

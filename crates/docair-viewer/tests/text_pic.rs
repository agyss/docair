//! Pictures drawn as text (docs/PULL_REQUESTS.md §3): every slice of the shop, from the layout its
//! SVG is drawn from.

use std::path::Path;

use docair_core::layer_view::slice_picture;
use docair_core::{Options, scan};
use docair_layout::text::columns;
use docair_viewer::{layout_input, text_pic};

fn shop() -> docair_core::model::Graph {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../e2e/fixtures/shop");
    scan(&root, docair_adapters::all(), &Options::default()).unwrap()
}

/// Each slice of the shop as text: every part's name shows whole, one arrow per drawn line, and
/// only characters every common monospace font has.
#[test]
fn every_slice_of_the_shop_draws_as_text() {
    let g = shop();
    // Box drawing and arrows only from WGL4; the parts' own words may hold anything.
    let allowed = |c: char| {
        let drawing =
            ('\u{2500}'..='\u{257f}').contains(&c) || ('\u{25a0}'..='\u{25ff}').contains(&c);
        !drawing || "─│┌┐└┘├┤┬┴┼═║╔╗╚╝╠╣╦╩╬►◄▲▼".contains(c)
    };
    let mut drawn = 0;
    for s in docair_core::slice::all(&g) {
        let pic = slice_picture(&g, &s);
        let laid = layout_input::lay_out(&g, &pic);
        let text = text_pic::draw(&g, &pic, &laid, None);
        if std::env::var_os("DOCAIR_SHOW").is_some() {
            println!("── {}\n{text}", s.seed.id);
        }
        for ch in text.chars().filter(|c| *c != '\n') {
            assert!(allowed(ch), "{}: {ch:?} is not in every font", s.seed.id);
        }
        for p in &pic.parts {
            let name = g
                .nodes
                .iter()
                .find(|n| n.id == p.id)
                .map_or(p.id.as_str(), |n| n.name.as_str());
            assert!(
                text.contains(name),
                "{}: {name} shows whole\n{text}",
                s.seed.id
            );
        }
        let arrows = text.chars().filter(|c| "►◄▲▼".contains(*c)).count();
        let routed = laid.routes.lines.iter().filter(|r| r.len() >= 2).count();
        assert!(
            arrows <= routed && (routed == 0 || arrows > 0),
            "{}: {arrows} arrows for {routed} lines\n{text}",
            s.seed.id
        );
        let widest = text.lines().map(columns).max().unwrap_or(0);
        assert!(widest > 0, "{}: something is drawn", s.seed.id);
        drawn += 1;
    }
    assert!(drawn > 3, "the shop has slices: {drawn}");
}

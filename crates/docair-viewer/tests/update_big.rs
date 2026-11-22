//! A big picture's layout updated after a small change (docs/REGENERATE.md §5): docair's own Code
//! layer, from the graph `docair build .` wrote. Run by hand, it needs that graph and takes a while:
//! `cargo test --release -p docair-viewer --test update_big -- --ignored --nocapture`.

use std::path::Path;
use std::time::Instant;

use docair_core::layer_view::{LayerKey, picture};
use docair_core::model::Graph;
use docair_viewer::layout_input;

#[test]
#[ignore]
fn docair_s_code_layer_updated_after_one_new_function() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.docair/docair.graph.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("no {}: run docair build . first", path.display());
        return;
    };
    let g1: Graph = serde_json::from_str(&text).unwrap();
    let mut g2 = g1.clone();
    let host = g1
        .nodes
        .iter()
        .find(|n| n.id == "docair-viewer.doc_page.render_page")
        .expect("a function to call the new one")
        .clone();
    let mut new = host.clone();
    new.id = "docair-viewer.doc_page.timing_sample".into();
    new.name = "timing_sample".into();
    g2.nodes.push(new);
    let mut e = g1
        .edges
        .iter()
        .find(|e| e.from == host.id)
        .expect("a line from it")
        .clone();
    e.to = "docair-viewer.doc_page.timing_sample".into();
    e.id = format!("{} -call-> {}", host.id, e.to);
    g2.edges.push(e);
    for (g, name) in [(&g1, "old"), (&g2, "new")] {
        let p = picture(g, LayerKey::Code, &[], None).unwrap();
        eprintln!("{name}: {} parts, {} lines", p.parts.len(), p.lines.len());
    }
    let (p1, p2) = (
        picture(&g1, LayerKey::Code, &[], None).unwrap(),
        picture(&g2, LayerKey::Code, &[], None).unwrap(),
    );
    let font = docair_layout::Monospace::default();
    let (i1, i2) = (
        layout_input::input(&g1, &p1, &font),
        layout_input::input(&g2, &p2, &font),
    );
    let opts = layout_input::options_for(&p1);
    let t = Instant::now();
    // The old layout, laid out once (minutes for this picture) and kept beside the build output under
    // a name made of its input alone, so a change of the layout code does not lay it out again.
    let name = {
        let text = serde_json::to_string(&i1).unwrap();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in text.bytes() {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
        format!("{h:016x}.json")
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/update_big");
    let path = dir.join(name);
    let old: docair_layout::Layout = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap(),
        Err(_) => {
            let l = docair_layout::lay_out(&i1, &opts);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, serde_json::to_string(&l).unwrap()).unwrap();
            l
        }
    };
    eprintln!("old layout: {:.1} s", t.elapsed().as_secs_f64());
    let t = Instant::now();
    let up = docair_layout::update_layout(&i1, &old, &i2, &opts);
    eprintln!(
        "updated: {:.2} s, {}",
        t.elapsed().as_secs_f64(),
        if up.is_some() {
            "done"
        } else {
            "none: from scratch"
        }
    );
    let up = up.expect("one new function is an update");
    // A line kept keeps its way: the same corners left to right; a band below a new part moves it
    // down, or stretches it, but leaves its x where they were. Lines are matched by their ends.
    let ends = |i: &docair_layout::Input, j: usize| {
        let l = &i.lines[j];
        (i.parts[l.from].id.clone(), i.parts[l.to].id.clone())
    };
    let mut was: std::collections::HashMap<(String, String), Vec<usize>> = Default::default();
    for j in 0..i1.lines.len() {
        was.entry(ends(&i1, j)).or_default().push(j);
    }
    let xs = |r: &[(f64, f64)]| {
        r.iter()
            .map(|p| (p.0 * 10.0).round() as i64)
            .collect::<Vec<_>>()
    };
    let mut same = 0;
    for (j, route) in up.routes.lines.iter().enumerate() {
        if let Some(olds) = was.get_mut(&ends(&i2, j))
            && !olds.is_empty()
        {
            let o = olds.remove(0);
            if xs(route) == xs(&old.routes.lines[o]) {
                same += 1;
            }
        }
    }
    eprintln!("lines kept as they were: {same} of {}", i2.lines.len());
    // Every line is drawn: the ones not kept are routed again, none is lost.
    let drawn = up.routes.lines.iter().filter(|r| r.len() >= 2).count();
    eprintln!(
        "lines drawn: {drawn} of {}, {} without a way of their own",
        i2.lines.len(),
        up.routes.none
    );
    assert_eq!(up.routes.lines.len(), i2.lines.len());
    assert_eq!(drawn, i2.lines.len(), "every line has its way");
    assert!(
        same * 10 >= i2.lines.len() * 9,
        "nine in ten lines keep their ways"
    );
}

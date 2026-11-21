//! The page kept between saves (docs/REGENERATE.md §2): the pictures taken over from the last page
//! are the ones a page made from nothing works out, and only what a change touched is laid out.

use std::path::Path;

use docair_core::model::{Edge, EdgeKind, Graph, Node};
use docair_core::{Options, scan};
use docair_viewer::doc_page::render_page_kept;
use docair_viewer::page_state::PageState;

fn shop() -> Graph {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../e2e/fixtures/shop");
    scan(&root, docair_adapters::all(), &Options::default()).unwrap()
}

/// The shop with one function more in the box of `orders.create`, called by it.
fn with_audit(g: &Graph) -> Graph {
    let mut g = g.clone();
    let create = g
        .nodes
        .iter()
        .find(|n| n.id == "orders.create")
        .unwrap()
        .clone();
    let mut audit: Node = create.clone();
    audit.id = "orders.audit".into();
    audit.name = "audit".into();
    audit.summary = "Record the order for the auditors.".into();
    g.nodes.push(audit);
    let call = g
        .edges
        .iter()
        .find(|e| e.from == "orders.create" && e.kind == EdgeKind::Call)
        .cloned()
        .unwrap_or_else(|| g.edges[0].clone());
    let mut e: Edge = call;
    e.from = "orders.create".into();
    e.to = "orders.audit".into();
    e.kind = EdgeKind::Call;
    e.id = "orders.create -call-> orders.audit".into();
    g.edges.push(e);
    g
}

/// The shop with `f` done to it.
fn changed(g: &Graph, f: impl Fn(&mut Graph)) -> Graph {
    let mut g = g.clone();
    f(&mut g);
    g
}

fn node<'g>(g: &'g mut Graph, id: &str) -> &'g mut Node {
    g.nodes.iter_mut().find(|n| n.id == id).unwrap()
}

/// More kinds of change: a part renamed, a part gone with its lines, a part moved to another box, a
/// datum renamed. After each, the kept page's pictures are a new page's.
#[test]
fn after_each_kind_of_change_the_kept_pictures_are_a_new_page_s() {
    let keep = tempfile::tempdir().unwrap();
    let g1 = shop();
    let some_box = g1
        .nodes
        .iter()
        .find(|n| {
            n.id != "orders-api"
                && g1.nodes.iter().any(|k| k.part_of.as_deref() == Some(&n.id))
                && n.id != g1.system
        })
        .map(|n| n.id.clone())
        .unwrap();
    let datum = g1
        .nodes
        .iter()
        .find(|n| n.kind == "data")
        .map(|n| n.id.clone());
    let mut cases: Vec<(&str, Graph)> = vec![
        (
            "renamed",
            changed(&g1, |g| {
                node(g, "orders.create").name = "create order".into()
            }),
        ),
        (
            "gone",
            changed(&g1, |g| {
                g.nodes.retain(|n| n.id != "orders.create");
                g.edges
                    .retain(|e| e.from != "orders.create" && e.to != "orders.create");
            }),
        ),
        (
            "moved",
            changed(&g1, |g| {
                node(g, "orders.create").part_of = Some(some_box.clone())
            }),
        ),
    ];
    if let Some(d) = datum {
        cases.push((
            "datum renamed",
            changed(&g1, |g| node(g, &d).name = "Purchase".into()),
        ));
    }
    for (what, g2) in cases {
        let mut state = PageState::default();
        render_page_kept(&g1, Some(keep.path()), None, &mut state);
        render_page_kept(&g2, Some(keep.path()), None, &mut state);
        // A renamed part leaves the pictures without it alone; in the small shop, a part gone or
        // moved touches every picture.
        if what == "renamed" {
            assert!(state.counts.taken_over > 0, "{what}: {:?}", state.counts);
        }
        let kept = pictures(&state);
        let mut fresh = PageState::default();
        render_page_kept(&g2, Some(keep.path()), None, &mut fresh);
        let new = pictures(&fresh);
        assert_eq!(
            kept.keys().collect::<Vec<_>>(),
            new.keys().collect::<Vec<_>>(),
            "{what}"
        );
        for (id, pic) in new {
            assert_eq!(
                kept[&id], pic,
                "{what}: {id} is the picture a new page works out"
            );
        }
    }
}

/// docs/REGENERATE.md §8: serve's layouts, saved and read back, are a build's base: the same
/// pictures take them as they were, a changed one is updated from them, and a state of other layout
/// code is not used.
#[test]
fn saved_layouts_are_a_checked_base_for_a_build() {
    let keep = tempfile::tempdir().unwrap();
    let saved = tempfile::tempdir().unwrap();
    let g1 = shop();
    let mut served = PageState::default();
    render_page_kept(&g1, Some(keep.path()), None, &mut served);
    served.save(saved.path()).unwrap();
    // Saved again with nothing new: the same files.
    let files = |d: &Path| std::fs::read_dir(d.join("layouts")).unwrap().count();
    let n = files(saved.path());
    served.save(saved.path()).unwrap();
    assert_eq!(files(saved.path()), n);

    // A build of the same code: every layout taken as it was.
    let mut base = PageState::load(saved.path()).unwrap();
    render_page_kept(&g1, Some(keep.path()), None, &mut base);
    let c = base.counts;
    assert_eq!(c.taken_over, 0, "a build works out every picture: {c:?}");
    assert_eq!(c.kept, c.worked_out, "{c:?}");
    assert_eq!((c.updated, c.laid_out), (0, 0), "{c:?}");

    // A build after a change: the touched pictures updated from serve's layouts.
    let mut base = PageState::load(saved.path()).unwrap();
    render_page_kept(&with_audit(&g1), Some(keep.path()), None, &mut base);
    assert!(base.counts.updated > 0, "{:?}", base.counts);

    // Layouts of other layout code are no base.
    let state = saved.path().join("state.json");
    let text = std::fs::read_to_string(&state).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut other = v.clone();
    other["code"] = serde_json::json!("0");
    std::fs::write(&state, other.to_string()).unwrap();
    assert!(PageState::load(saved.path()).is_err());
    assert!(PageState::load(&saved.path().join("nothing")).is_err());
}

fn pictures(state: &PageState) -> std::collections::BTreeMap<String, String> {
    state
        .pictures()
        .map(|(id, pic)| (id.to_string(), serde_json::to_string(pic).unwrap()))
        .collect()
}

#[test]
fn a_kept_page_takes_over_only_the_pictures_a_change_did_not_touch() {
    let keep = tempfile::tempdir().unwrap();
    let g1 = shop();
    let mut state = PageState::default();
    render_page_kept(&g1, Some(keep.path()), None, &mut state);
    assert_eq!(
        state.counts.taken_over, 0,
        "the first page works out every picture"
    );
    let n = state.counts.worked_out;
    assert!(n > 10, "the shop has pictures: {n}");

    // The same graph again: every section and slice taken over, every layout kept.
    render_page_kept(&g1, Some(keep.path()), None, &mut state);
    assert!(state.counts.taken_over > 0, "{:?}", state.counts);
    assert_eq!(state.counts.laid_out, 0, "{:?}", state.counts);
    assert_eq!(state.counts.updated, 0, "{:?}", state.counts);

    // One new function: what it touched is worked out again and updated; the rest taken over.
    let g2 = with_audit(&g1);
    render_page_kept(&g2, Some(keep.path()), None, &mut state);
    let c = state.counts;
    assert!(c.taken_over > 0, "{c:?}");
    assert!(
        c.updated > 0,
        "a picture with the new function is updated: {c:?}"
    );
    let kept_pictures = pictures(&state);

    // The same pictures as a page made from nothing.
    let mut fresh = PageState::default();
    render_page_kept(&g2, Some(keep.path()), None, &mut fresh);
    assert_eq!(
        kept_pictures.keys().collect::<Vec<_>>(),
        pictures(&fresh).keys().collect::<Vec<_>>()
    );
    for (id, pic) in pictures(&fresh) {
        assert_eq!(
            kept_pictures[&id], pic,
            "{id} is the picture a new page works out"
        );
    }
    assert!(
        kept_pictures.values().any(|p| p.contains("orders.audit")),
        "the new function is drawn"
    );
}

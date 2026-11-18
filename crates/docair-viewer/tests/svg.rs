//! The pictures of the e2e fixtures drawn as SVG (docs/SLICES.md §6), compared with checked-in
//! golden files: every layer of each fixture, nothing opened, and every slice of the shop.
//!
//! A change of how docair draws shows here as a diff. When the change is intended, run
//! `DOCAIR_BLESS=1 cargo test -p docair-viewer` and review the golden files' diff.

use std::path::{Path, PathBuf};

use docair_core::layer_view::{LayerPicture, layers, picture, slice_picture};
use docair_core::model::Graph;
use docair_core::{Options, scan};
use docair_viewer::{layout_input, svg};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../e2e/fixtures")
        .join(name)
}

fn graph(name: &str) -> Graph {
    scan(&fixture(name), docair_adapters::all(), &Options::default()).unwrap()
}

/// Compare a drawing with `tests/golden/<file>`; `DOCAIR_BLESS=1` rewrites it.
fn golden(file: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(file);
    if std::env::var_os("DOCAIR_BLESS").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    if actual != expected {
        let out = std::env::temp_dir().join(format!("docair-{file}"));
        std::fs::write(&out, actual).unwrap();
        panic!(
            "the picture {file} changed: compare {} with {}; if intended, rerun with DOCAIR_BLESS=1",
            path.display(),
            out.display()
        );
    }
}

/// Every element closes, in order: the drawing is well-formed XML.
fn assert_well_formed(svg: &str) {
    let mut open: Vec<&str> = Vec::new();
    let mut rest = svg;
    while let Some(at) = rest.find('<') {
        rest = &rest[at + 1..];
        let end = rest.find('>').expect("a tag ends");
        let tag = &rest[..end];
        rest = &rest[end + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or_default();
        if let Some(closing) = tag.strip_prefix('/') {
            assert_eq!(open.pop(), Some(closing.trim()), "tags close in order");
        } else if !tag.ends_with('/') {
            open.push(name);
        }
    }
    assert!(open.is_empty(), "every tag closes: {open:?} left open");
}

/// Draw a picture, check that every part and line of it is in the drawing, and compare it with its golden file.
fn check(g: &Graph, p: &LayerPicture, file: &str) {
    let laid = layout_input::lay_out(g, p);
    let drawn = svg::draw(g, p, &laid);
    assert_well_formed(&drawn);
    for part in &p.parts {
        let id = part.id.replace('&', "&amp;").replace('"', "&quot;");
        assert!(
            drawn.contains(&format!(r#"data-id="{id}""#)),
            "{file} draws {}",
            part.id
        );
    }
    let lines = drawn.matches(r#"<g class="line "#).count();
    assert_eq!(lines, p.lines.len(), "{file} draws every line");
    assert!(
        !drawn.contains('#'),
        "{file} takes its colours from classes"
    );
    golden(file, &drawn);
}

fn layers_of(name: &str) {
    let g = graph(name);
    for l in layers(&g) {
        let Some(p) = picture(&g, l.key, &[], None) else {
            continue;
        };
        let key = serde_json::to_value(l.key).unwrap();
        check(&g, &p, &format!("{name}.{}.svg", key.as_str().unwrap()));
    }
}

fn slices_of(name: &str) {
    let g = graph(name);
    for s in docair_core::slice::all(&g) {
        let p = slice_picture(&g, &s);
        let seed: String = s
            .seed
            .id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "._-".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        check(&g, &p, &format!("{name}.slice.{seed}.svg"));
    }
}

#[test]
fn shop_pictures_are_unchanged() {
    layers_of("shop");
    slices_of("shop");
}

#[test]
fn platform_pictures_are_unchanged() {
    layers_of("platform");
}

#[test]
fn apis_pictures_are_unchanged() {
    layers_of("apis");
}

#[test]
fn answer_pictures_are_unchanged() {
    layers_of("answer");
}

#[test]
fn fold_pictures_are_unchanged() {
    layers_of("fold");
}

#[test]
fn a_picture_of_its_own_has_its_colours() {
    let g = graph("shop");
    let l = layers(&g)[0].key;
    let p = picture(&g, l, &[], None).unwrap();
    let laid = layout_input::lay_out(&g, &p);
    let file = svg::standalone(&g, &p, &laid);
    assert_well_formed(&file);
    assert!(file.starts_with("<?xml"));
    assert!(file.contains("--k-service-bg"), "the page's colours");
    assert!(
        file.contains("prefers-color-scheme: dark"),
        "light and dark"
    );
    assert!(file.contains(".docair-pic .sh"), "the pictures' stylesheet");
    assert!(file.contains(r#"<rect class="bg""#), "a background");
}

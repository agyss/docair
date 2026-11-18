//! The documentation as one page (docs/SLICES.md §4, step 5): every picture a reader can reach,
//! laid out and drawn by the CLI and packed into one HTML file, and a thin script that unpacks
//! and shows each by its `#` address. Until step 5.5 writes it by default, `DOCAIR_PAGE=rust`
//! asks for it.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use base64::Engine as _;
use docair_core::layer_view::{
    LayerInfo, LayerKey, LayerPicture, layers, picture, section_picture, slice_picture,
};
use docair_core::model::Graph;
use docair_core::slice::{SeedKind, SliceSeed};
use serde_json::{Map, Value, json};

use crate::{
    escape_text, fill, layout_input, layout_store, page_panels, page_walks, script_js, script_json,
    svg,
};

const TEMPLATE: &str = include_str!("../assets/page.html");
const SCRIPT: &str = include_str!("../assets/page.js");
const PAGE_CSS: &str = include_str!("../assets/page.css");
const CHAT_CSS: &str = include_str!("../assets/chat.css");

/// How hard each picture is packed (miniz_oxide's levels run from 0 to 10).
const PACK_LEVEL: u8 = 6;

/// One picture a reader can reach: its address, its name, the layer it is drawn at, the way back
/// up, and what it shows with tests and without them.
struct Spot {
    addr: String,
    title: String,
    layer: Option<LayerKey>,
    crumbs: Vec<(String, String)>,
    /// A datum's slice: the datum it follows.
    datum: Option<String>,
    /// Indices into the pictures to draw: one, or one with tests and one without.
    shown: SpotPictures,
}

enum SpotPictures {
    One(usize),
    Two { on: usize, off: usize },
}

/// Where docair keeps what it can make again: `$DOCAIR_CACHE`, else the user's cache folder
/// (the same place as docair-scip's kept code index).
fn cache_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("DOCAIR_CACHE") {
        return d.into();
    }
    if let Some(d) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(d).join("docair");
    }
    if let Some(d) = std::env::var_os("HOME") {
        return PathBuf::from(d).join(".cache").join("docair");
    }
    std::env::temp_dir().join("docair-cache")
}

fn key_name(key: LayerKey) -> String {
    serde_json::to_value(key)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The pictures of every spot, each once: a spot's two pictures are one when tests change nothing.
struct Spots {
    spots: Vec<Spot>,
    pictures: Vec<LayerPicture>,
    /// For each picture, the walk it plays: its seed and slice.
    walks: Vec<Option<(SliceSeed, docair_core::slice::Slice)>>,
}

impl Spots {
    fn add(
        &mut self,
        addr: String,
        title: String,
        layer: Option<LayerKey>,
        crumbs: Vec<(String, String)>,
        on: LayerPicture,
        off: Option<LayerPicture>,
    ) {
        let first = self.pictures.len();
        self.walks.resize_with(first, || None);
        self.pictures.push(on);
        let shown = match off {
            Some(off) if off != self.pictures[first] => {
                self.pictures.push(off);
                SpotPictures::Two {
                    on: first,
                    off: first + 1,
                }
            }
            _ => SpotPictures::One(first),
        };
        self.spots.push(Spot {
            addr,
            title,
            layer,
            crumbs,
            datum: None,
            shown,
        });
    }
}

fn layer_crumb(infos: &[LayerInfo], key: LayerKey) -> (String, String) {
    let name = infos.iter().find(|l| l.key == key).map_or("", |l| l.name);
    (name.to_string(), format!("layer/{}", key_name(key)))
}

/// A slice's address: a datum's or a part's id, `flow:<name>`, `walkthrough:<id>`.
fn slice_addr(seed: &SliceSeed) -> String {
    match seed.kind {
        SeedKind::Flow => format!("slice/flow:{}", seed.id),
        SeedKind::Walkthrough => format!("slice/walkthrough:{}", seed.id),
        _ => format!("slice/{}", seed.id),
    }
}

/***
 * Every picture a reader can reach (docs/SLICES.md §4): each layer, a section for each box, a slice for each seed; a layer or a section that holds tests twice, with them and without.
 *
 * @calls docair-core.layer_view.layers "lists the layers"
 * @calls docair-core.layer_view.picture "works out what a layer draws"
 * @calls docair-core.layer_view.section_picture "works out a box's section"
 * @calls docair-core.slice.seeds "lists the seeds"
 * @calls docair-core.slice.slice "works out each slice"
 * @calls docair-core.layer_view.slice_picture "makes each slice a picture"
 ***/
fn spots(graph: &Graph) -> Spots {
    let mut out = Spots {
        spots: Vec::new(),
        pictures: Vec::new(),
        walks: Vec::new(),
    };
    let infos = layers(graph);
    let both = |f: &dyn Fn(Option<bool>) -> Option<LayerPicture>| -> Option<(LayerPicture, Option<LayerPicture>)> {
        let shown = f(None)?;
        let other = f(Some(!graph.view.tests));
        Some((shown, other))
    };
    for l in &infos {
        let Some((on, off)) = both(&|t| picture(graph, l.key, &[], t)) else {
            continue;
        };
        out.add(
            format!("layer/{}", key_name(l.key)),
            l.name.to_string(),
            Some(l.key),
            vec![layer_crumb(&infos, l.key)],
            on,
            off,
        );
    }
    let nodes: std::collections::HashMap<&str, &docair_core::model::Node> =
        graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let name = |id: &str| nodes.get(id).map_or(id.to_string(), |n| n.name.clone());
    // Only a part that holds parts has a section; each is worked out on its own, side by side.
    let holds: std::collections::HashSet<&str> = graph
        .nodes
        .iter()
        .filter(|n| n.kind != "data")
        .filter_map(|n| n.part_of.as_deref())
        .collect();
    let boxes: Vec<&docair_core::model::Node> = graph
        .nodes
        .iter()
        .filter(|n| n.id != graph.system && holds.contains(n.id.as_str()))
        .collect();
    let sections = side_by_side(&boxes, |n| {
        let (key, on) = section_picture(graph, &n.id, None)?;
        let off = section_picture(graph, &n.id, Some(!graph.view.tests)).map(|(_, p)| p);
        Some((key, on, off))
    });
    for (n, found) in boxes.iter().zip(sections) {
        let Some((key, on, off)) = found else {
            continue;
        };
        // The way up: the layer, then each box around this one.
        let mut up: Vec<(String, String)> = Vec::new();
        let mut at = n.part_of.as_deref();
        while let Some(p) = at.filter(|p| *p != graph.system) {
            up.push((name(p), format!("box/{p}")));
            at = nodes.get(p).and_then(|q| q.part_of.as_deref());
        }
        up.reverse();
        let mut crumbs = vec![layer_crumb(&infos, key)];
        crumbs.extend(up);
        crumbs.push((n.name.clone(), format!("box/{}", n.id)));
        out.add(
            format!("box/{}", n.id),
            n.name.clone(),
            Some(key),
            crumbs,
            on,
            off,
        );
    }
    let seeds = docair_core::slice::seeds(graph);
    let slices = side_by_side(&seeds, |seed| {
        let s = docair_core::slice::slice(graph, seed)?;
        let pic = slice_picture(graph, &s);
        Some((pic, page_walks::is_walk(seed).then_some(s)))
    });
    for (seed, found) in seeds.iter().zip(slices) {
        let Some((pic, walk)) = found else {
            continue;
        };
        let title = match seed.kind {
            SeedKind::Data => format!("Datum {}", name(&seed.id)),
            SeedKind::Route => format!("Route {}", name(&seed.id)),
            SeedKind::Flow => format!("Scenario {}", seed.id),
            SeedKind::Walkthrough => format!("Walkthrough {}", seed.id),
            SeedKind::Part => name(&seed.id),
        };
        let addr = slice_addr(seed);
        out.add(
            addr.clone(),
            title.clone(),
            None,
            vec![(title, addr)],
            pic,
            None,
        );
        if seed.kind == SeedKind::Data
            && let Some(s) = out.spots.last_mut()
        {
            s.datum = Some(seed.id.clone());
        }
        if let Some(w) = walk {
            out.walks.truncate(out.pictures.len() - 1);
            out.walks.push(Some((seed.clone(), w)));
        }
    }
    out.walks.resize_with(out.pictures.len(), || None);
    out
}

/// How many pictures are worked out at once: `DOCAIR_JOBS`, else the cores, at most 4. Each big
/// picture's lines are routed in memory of their own, so this bounds the memory: docair's own page peaks at
/// about 420 MB with 4 at once, 900 MB with 32, and takes as long, as its Code layer is most of it.
fn workers() -> usize {
    std::env::var("DOCAIR_JOBS")
        .ok()
        .and_then(|j| j.parse::<usize>().ok())
        .filter(|&j| j > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map_or(4, |n| n.get())
                .min(4)
        })
}

/// `f` of each item, worked out side by side; the results in the items' order.
fn side_by_side<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let out: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    let workers = workers().min(items.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else {
                        break;
                    };
                    *out[i].lock().expect("no worker panics") = Some(f(item));
                }
            });
        }
    });
    out.into_iter()
        .map(|m| {
            m.into_inner()
                .expect("no worker panics")
                .expect("every item is worked out")
        })
        .collect()
}

/// Lay out, draw and pack each picture, the biggest first, on every core; kept layouts are taken from `keep`.
/// Each picture packed, and the steps of the walks among them.
struct DrawnPictures {
    packed: Vec<String>,
    walks: Vec<Option<Value>>,
}

fn draw_all(
    graph: &Graph,
    pictures: &[LayerPicture],
    walks: &[Option<(SliceSeed, docair_core::slice::Slice)>],
    keep: Option<&Path>,
) -> DrawnPictures {
    let linker = crate::page_glossary::Linker::new(graph);
    let steps: Vec<Mutex<Option<Value>>> = pictures.iter().map(|_| Mutex::new(None)).collect();
    let mut order: Vec<usize> = (0..pictures.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(pictures[i].lines.len() + pictures[i].parts.len()));
    let next = AtomicUsize::new(0);
    let packed: Vec<Mutex<String>> = pictures.iter().map(|_| Mutex::new(String::new())).collect();
    let workers = workers().min(pictures.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&i) = order.get(k) else {
                        break;
                    };
                    let pic = &pictures[i];
                    let walk = walks.get(i).and_then(Option::as_ref);
                    // A walk's lines point ahead in its layout.
                    let ahead = walk
                        .map(|(_, s)| page_walks::walked(graph, pic, s))
                        .unwrap_or_default();
                    let laid = keep
                        .and_then(|dir| {
                            layout_store::lay_out_kept_with(graph, pic, &ahead, dir)
                                .ok()
                                .map(|(l, _)| l)
                        })
                        .unwrap_or_else(|| layout_input::lay_out_with(graph, pic, &ahead));
                    *packed[i].lock().expect("no worker panics") =
                        pack(&svg::draw(graph, pic, &laid));
                    if let Some((seed, slice)) = walk {
                        *steps[i].lock().expect("no worker panics") =
                            Some(page_walks::steps(graph, pic, &laid, seed, slice, &linker));
                    }
                }
            });
        }
    });
    DrawnPictures {
        packed: packed
            .into_iter()
            .map(|m| m.into_inner().expect("no worker panics"))
            .collect(),
        walks: steps
            .into_iter()
            .map(|m| m.into_inner().expect("no worker panics"))
            .collect(),
    }
}

/// Text as the page carries it: deflated, then base64.
fn pack(text: &str) -> String {
    let flat = miniz_oxide::deflate::compress_to_vec(text.as_bytes(), PACK_LEVEL);
    base64::engine::general_purpose::STANDARD.encode(flat)
}

/// For each part, the picture `#part/<id>` shows it in: its box's section, else a layer that
/// draws it (Apps first), else its own slice.
fn homes(graph: &Graph, spots: &Spots) -> Map<String, Value> {
    let drawn = |addr: &str, id: &str| {
        spots
            .spots
            .iter()
            .find(|s| s.addr == addr)
            .is_some_and(|s| {
                let i = match s.shown {
                    SpotPictures::One(i) | SpotPictures::Two { on: i, .. } => i,
                };
                spots.pictures[i].parts.iter().any(|p| p.id == id)
            })
    };
    let mut layer_order: Vec<String> = spots
        .spots
        .iter()
        .filter(|s| s.addr.starts_with("layer/"))
        .map(|s| s.addr.clone())
        .collect();
    layer_order.sort_by_key(|a| a != "layer/apps");
    let mut out = Map::new();
    for n in &graph.nodes {
        if n.id == graph.system {
            continue;
        }
        let section = n.part_of.as_deref().map(|p| format!("box/{p}"));
        let home = section
            .filter(|a| drawn(a, &n.id))
            .or_else(|| layer_order.iter().find(|a| drawn(a, &n.id)).cloned())
            .or_else(|| {
                let own = format!("slice/{}", n.id);
                spots.spots.iter().any(|s| s.addr == own).then_some(own)
            });
        if let Some(h) = home {
            out.insert(n.id.clone(), Value::String(h));
        }
    }
    out
}

/***
 * The documentation as one page: every picture laid out, drawn and packed, the header and the layer links, and the thin script that shows them.
 *
 * Layouts are kept under `$DOCAIR_CACHE/layouts` (else the user's cache folder), so a picture that did not change is not laid out again.
 *
 * @calls docair-viewer.doc_page.spots "works out every picture"
 * @calls docair-viewer.layout_store.lay_out_kept "lays each picture out, or takes its kept layout"
 * @calls docair-viewer.svg.draw "draws each picture"
 ***/
pub fn render_page(graph: &Graph) -> String {
    render_page_keeping(graph, Some(&cache_dir().join("layouts")))
}

/// As `render_page`, with layouts kept in `keep`, or none kept.
pub fn render_page_keeping(graph: &Graph, keep: Option<&Path>) -> String {
    let all = spots(graph);
    let drawn = draw_all(graph, &all.pictures, &all.walks, keep);
    let mut packed = drawn.packed;
    let sections: std::collections::HashSet<String> = all
        .spots
        .iter()
        .filter_map(|s| s.addr.strip_prefix("box/").map(str::to_string))
        .collect();
    let panels = page_panels::panels(graph, &sections);
    // The panels and the walks' steps go with the pictures, packed the same way, last.
    packed.push(pack(&panels.data.to_string()));
    let panels_at = packed.len() - 1;
    let picture_of = |s: &Spot| match s.shown {
        SpotPictures::One(i) | SpotPictures::Two { on: i, .. } => i,
    };
    let mut walk_steps = Map::new();
    let mut walk_list: Vec<(String, SeedKind, String, usize)> = Vec::new();
    for s in &all.spots {
        let i = picture_of(s);
        if let (Some(Some(steps)), Some(Some((seed, _)))) = (drawn.walks.get(i), all.walks.get(i)) {
            let title = steps["title"].as_str().unwrap_or(&s.title).to_string();
            walk_list.push((
                s.addr.clone(),
                seed.kind,
                title,
                steps["steps"].as_array().map_or(0, Vec::len),
            ));
            walk_steps.insert(s.addr.clone(), steps.clone());
        }
    }
    packed.push(pack(&Value::Object(walk_steps).to_string()));
    let walks_at = packed.len() - 1;
    let (walk_bar, tours) = page_walks::walk_list(graph, &walk_list);
    let infos = layers(graph);
    let mut pictures = Map::new();
    for s in &all.spots {
        let mut p = json!({
            "title": s.title,
            "crumbs": s.crumbs.iter().map(|(l, a)| json!([l, a])).collect::<Vec<_>>(),
        });
        if let Some(k) = s.layer {
            p["layer"] = json!(key_name(k));
        }
        if let Some(d) = &s.datum {
            p["datum"] = json!(d);
        }
        if drawn.walks.get(picture_of(s)).is_some_and(Option::is_some) {
            p["walk"] = json!(true);
        }
        match s.shown {
            SpotPictures::One(i) => p["svg"] = json!(i),
            SpotPictures::Two { on, off } => {
                // `on` is the picture as [view] tests says; the switch shows the other.
                let (with, without) = if graph.view.tests {
                    (on, off)
                } else {
                    (off, on)
                };
                p["on"] = json!(with);
                p["off"] = json!(without);
            }
        }
        pictures.insert(s.addr.clone(), p);
    }
    let first = ["layer/apps"]
        .into_iter()
        .map(str::to_string)
        .chain(
            all.spots
                .iter()
                .filter(|s| s.addr.starts_with("layer/"))
                .map(|s| s.addr.clone()),
        )
        .find(|a| pictures.contains_key(a))
        .unwrap_or_default();
    let index = json!({
        "system": graph.system,
        "tests": graph.view.tests,
        "first": first,
        "pictures": pictures,
        "home": homes(graph, &all),
        "panels": panels_at,
        "walks": walks_at,
        // The words the script puts around what the CLI wrote.
        "words": { "more": "… {n} more" },
    });
    let links: String = infos
        .iter()
        .filter(|l| pictures.contains_key(&format!("layer/{}", key_name(l.key))))
        .map(|l| {
            let key = key_name(l.key);
            format!(
                r##"<a class="seg" data-layer="{key}" href="#layer/{key}" title="{}: {}">{}</a>"##,
                escape_text(l.name),
                escape_text(l.ask),
                escape_text(l.name)
            )
        })
        .collect();
    let stats = format!(
        "{} nodes · {} edges · {} flows",
        graph.nodes.len().saturating_sub(1),
        graph.edges.len(),
        graph.flows.len()
    );
    let tests = panels
        .tests
        .as_ref()
        .map(|(on, off, title)| {
            format!(
                r#"<button class="btn" id="btn-tests" data-test="tests-toggle" title="{}"><span class="tests-on">{on}</span><span class="tests-off" hidden>{off}</span></button>"#,
                escape_text(title)
            )
        })
        .unwrap_or_default();
    let css = format!("{}\n{}\n{}\n{}", crate::CSS, svg::STYLE, PAGE_CSS, CHAT_CSS);
    fill(
        TEMPLATE,
        &[
            ("generator", &escape_text(&graph.generator)),
            ("title", &escape_text(&format!("docair · {}", graph.system))),
            ("css", &css),
            ("system", &escape_text(&graph.system)),
            ("stats", &escape_text(&stats)),
            ("layers", &links),
            ("tests", &tests),
            ("buttons", &panels.buttons),
            ("ask", crate::page_ai::BUTTON),
            ("chat", &crate::page_ai::panel()),
            ("walks", &walk_bar),
            ("tours", &tours),
            ("tree", &panels.tree),
            ("legend", &panels.legend),
            ("drawer", &panels.drawer),
            ("diag-class", &panels.diag.0),
            ("diag", &escape_text(&panels.diag.1)),
            ("diag-title", &escape_text(&panels.diag.2)),
            ("index", &script_json(&index.to_string())),
            ("packed", &script_json(&Value::from(packed).to_string())),
            ("app", &script_js(SCRIPT)),
        ],
    )
}

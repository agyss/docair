//! The documentation as one page (docs/SLICES.md §4, step 5): every picture a reader can reach,
//! laid out and drawn by the CLI and packed into one HTML file, and a thin script that unpacks
//! and shows each by its `#` address. Until step 5.5 writes it by default, `DOCAIR_PAGE=rust`
//! asks for it.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use base64::Engine as _;
use docair_core::layer_view::{LayerInfo, LayerKey, LayerPicture, layers, picture};
use docair_core::model::Graph;
use docair_core::slice::{SeedKind, SliceSeed};
use docair_core::timing::Sum;
use serde_json::{Map, Value, json};

use crate::{
    escape_text, fill, layout_input, layout_store, page_changes, page_panels, page_state,
    page_walks, script_js, script_json, svg,
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
    /// For each picture, what it is across saves (docs/REGENERATE.md §2): its address, with `|other`
    /// for the address's second picture.
    idents: Vec<String>,
    /// For each picture, whether it was taken over from the last page unchanged.
    taken: Vec<bool>,
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
        self.add_taken(addr, title, layer, crumbs, (on, off), false);
    }

    /// As `add`, the pictures taken over from the last page when `taken`.
    fn add_taken(
        &mut self,
        addr: String,
        title: String,
        layer: Option<LayerKey>,
        crumbs: Vec<(String, String)>,
        (on, off): (LayerPicture, Option<LayerPicture>),
        taken: bool,
    ) {
        let first = self.pictures.len();
        self.walks.resize_with(first, || None);
        self.idents.push(addr.clone());
        self.taken.push(taken);
        self.pictures.push(on);
        let shown = match off {
            Some(off) if off != self.pictures[first] => {
                self.idents.push(format!("{addr}|other"));
                self.taken.push(taken);
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
fn spots(graph: &Graph, prev: Option<&Prev>) -> Spots {
    let mut out = Spots {
        spots: Vec::new(),
        pictures: Vec::new(),
        walks: Vec::new(),
        idents: Vec::new(),
        taken: Vec::new(),
    };
    // A section's or a slice's pictures from the last page, when none of their parts was touched
    // (docs/REGENERATE.md §2.3).
    let take = |addr: &str, id: &str| -> Option<(LayerPicture, Option<LayerPicture>)> {
        let p = prev?;
        if p.touched.parts.contains(id) {
            return None;
        }
        let from = |k: &page_state::PictureEntry| {
            page_state::untouched(&k.pic, p.touched)
                .then(|| page_state::taken_over(&k.pic, p.graph, graph))
                .flatten()
        };
        let on = from(p.kept.get(addr)?)?;
        let off = match p.kept.get(&format!("{addr}|other")) {
            Some(k) => Some(from(k)?),
            None => None,
        };
        Some((on, off))
    };
    let infos = layers(graph);
    let both = |f: &dyn Fn(Option<bool>) -> Option<LayerPicture>| -> Option<(LayerPicture, Option<LayerPicture>)> {
        let shown = f(None)?;
        let other = f(Some(!graph.view.tests));
        Some((shown, other))
    };
    let t_layers = docair_core::timing::stopwatch("page: layers worked out");
    // The layers side by side, each with its tests and without; added in their order.
    let made_layers = side_by_side(&infos, |l| both(&|t| picture(graph, l.key, &[], t)));
    for (l, found) in infos.iter().zip(made_layers) {
        let Some((on, off)) = found else {
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
    drop(t_layers);
    // The graph looked up once for all of them (docs/REGENERATE.md §3).
    let made = docair_core::layer_view::PictureSet::new(graph);
    let slicer = docair_core::slice::Slicer::new(graph);
    let t_sections = docair_core::timing::stopwatch("page: sections worked out");
    let sections = side_by_side(&boxes, |n| {
        if let Some((on, off)) = take(&format!("box/{}", n.id), &n.id)
            && let Some(key) = on.layer
        {
            return Some((key, on, off, true));
        }
        let (key, on) = made.section_picture(&n.id, None)?;
        let off = made
            .section_picture(&n.id, Some(!graph.view.tests))
            .map(|(_, p)| p);
        Some((key, on, off, false))
    });
    drop(t_sections);
    docair_core::layer_view::T_SETS.report("page: sections' sets (all threads)");
    docair_core::layer_view::T_DRAW.report("page: sections' draw (all threads)");
    for (n, found) in boxes.iter().zip(sections) {
        let Some((key, on, off, taken)) = found else {
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
        out.add_taken(
            format!("box/{}", n.id),
            n.name.clone(),
            Some(key),
            crumbs,
            (on, off),
            taken,
        );
    }
    let seeds = docair_core::slice::seeds(graph);
    let t_slices = docair_core::timing::stopwatch("page: slices worked out");
    let (t_slice, t_pic) = (Sum::default(), Sum::default());
    let slices = side_by_side(&seeds, |seed| {
        // A walk is worked out each time: its steps need its slice.
        if !page_walks::is_walk(seed)
            && let Some((pic, _)) = take(&slice_addr(seed), &seed.id)
        {
            return Some((pic, None, true));
        }
        let s = t_slice.time(|| slicer.slice(seed))?;
        let pic = t_pic.time(|| made.slice_picture(&s));
        Some((pic, page_walks::is_walk(seed).then_some(s), false))
    });
    drop(t_slices);
    t_slice.report("page: slices (all threads)");
    t_pic.report("page: slices as pictures (all threads)");
    for (seed, found) in seeds.iter().zip(slices) {
        let Some((pic, walk, taken)) = found else {
            continue;
        };
        let title = slice_title(seed, &name);
        let addr = slice_addr(seed);
        out.add_taken(
            addr.clone(),
            title.clone(),
            None,
            vec![(title, addr)],
            (pic, None),
            taken,
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

/// How many sections and slices are worked out at once: `DOCAIR_JOBS`, else every core, at most 16.
/// Unlike laying out, working a picture out needs little memory of its own (docs/REGENERATE.md §1).
fn workers_for_pictures() -> usize {
    std::env::var("DOCAIR_JOBS")
        .ok()
        .and_then(|j| j.parse::<usize>().ok())
        .filter(|&j| j > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map_or(4, |n| n.get())
                .min(16)
        })
}

/// `f` of each item, worked out side by side; the results in the items' order.
fn side_by_side<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let out: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    let workers = workers_for_pictures().min(items.len().max(1));
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
    /// For each picture, what moving a part needs (docair-9qv): its layout input and its layout, packed.
    moves: Vec<String>,
    /// For each picture, what the next page keeps of it (docs/REGENERATE.md §2).
    kept: Vec<page_state::PictureEntry>,
    counts: page_state::Counts,
}

/// The last page, for the pictures a change did not touch (docs/REGENERATE.md §2.3).
struct Prev<'a> {
    graph: &'a Graph,
    kept: &'a std::collections::HashMap<String, page_state::PictureEntry>,
    touched: &'a page_state::ChangeReach,
}

/***
 * Lay out, draw and pack each picture (docs/REGENERATE.md §2.4, §2.5): one taken over from the last page as it was; else its layout kept for the same key, updated from the last page's for another key, or from the layouts kept under their keys, laid out when none is kept.
 *
 * @calls docair-layout.incremental.update_layout "updates a picture's last layout"
 * @calls docair-viewer.layout_store.kept_or_made "takes a kept layout, or lays one out"
 * @calls docair-viewer.svg.draw "draws each picture"
 ***/
fn draw_all(
    graph: &Graph,
    all: &Spots,
    keep: Option<&Path>,
    last: Option<&std::collections::HashMap<String, page_state::PictureEntry>>,
) -> DrawnPictures {
    let (pictures, walks) = (&all.pictures, &all.walks);
    let linker = crate::page_glossary::Linker::new(graph);
    let steps: Vec<Mutex<Option<Value>>> = pictures.iter().map(|_| Mutex::new(None)).collect();
    let kept: Vec<Mutex<Option<page_state::PictureEntry>>> =
        pictures.iter().map(|_| Mutex::new(None)).collect();
    let mut order: Vec<usize> = (0..pictures.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(pictures[i].lines.len() + pictures[i].parts.len()));
    let next = AtomicUsize::new(0);
    let workers = workers().min(pictures.len().max(1));
    let _t = docair_core::timing::stopwatch("page: pictures laid out, drawn, packed");
    let (t_lay, t_draw, t_pack, t_moves, t_walks) = (
        Sum::default(),
        Sum::default(),
        Sum::default(),
        Sum::default(),
        Sum::default(),
    );
    let (n_taken, n_kept, n_updated, n_laid) = (
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
        AtomicUsize::new(0),
    );
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&i) = order.get(k) else {
                        break;
                    };
                    let pic = &pictures[i];
                    let old = last.and_then(|m| m.get(&all.idents[i]));
                    // Taken over: its layout, its drawing and its move data as they were.
                    if all.taken[i]
                        && let Some(o) = old
                    {
                        n_taken.fetch_add(1, Ordering::Relaxed);
                        *kept[i].lock().expect("no worker panics") =
                            Some(page_state::PictureEntry {
                                pic: pic.clone(),
                                ..o.clone()
                            });
                        continue;
                    }
                    let walk = walks.get(i).and_then(Option::as_ref);
                    // A walk's lines point ahead in its layout.
                    let ahead = walk
                        .map(|(_, s)| page_walks::walked(graph, pic, s))
                        .unwrap_or_default();
                    let input = layout_input::input_with(
                        graph,
                        pic,
                        &docair_layout::Monospace::default(),
                        &ahead,
                    );
                    let options = layout_input::options_for(pic);
                    let key = docair_layout::key(&input, &options);
                    let laid = t_lay.time(|| {
                        if let Some(o) = old {
                            if o.key == key {
                                n_kept.fetch_add(1, Ordering::Relaxed);
                                return o.layout.clone();
                            }
                            match docair_layout::try_update_layout(
                                &o.input, &o.layout, &input, &options,
                            ) {
                                Ok(l) => {
                                    n_updated.fetch_add(1, Ordering::Relaxed);
                                    return l;
                                }
                                Err(why) if docair_core::timing::on() => eprintln!(
                                    "docair timing: not updated: {}: {why}",
                                    all.idents[i]
                                ),
                                Err(_) => {}
                            }
                        }
                        let (l, was_kept) = match keep {
                            Some(dir) => layout_store::kept_or_made(&input, &options, &key, dir)
                                .unwrap_or_else(|_| {
                                    (docair_layout::lay_out(&input, &options), false)
                                }),
                            None => (docair_layout::lay_out(&input, &options), false),
                        };
                        if was_kept {
                            n_kept.fetch_add(1, Ordering::Relaxed);
                        } else {
                            n_laid.fetch_add(1, Ordering::Relaxed);
                            if docair_core::timing::on() {
                                eprintln!(
                                    "docair timing: laid out anew: {}, {} parts",
                                    all.idents[i],
                                    pic.parts.len()
                                );
                            }
                        }
                        l
                    });
                    let drawn = t_draw.time(|| svg::draw(graph, pic, &laid));
                    let packed = t_pack.time(|| pack(&drawn));
                    let moves = t_moves
                        .time(|| pack(&json!({ "input": input, "layout": laid }).to_string()));
                    if let Some((seed, slice)) = walk {
                        *steps[i].lock().expect("no worker panics") = t_walks.time(|| {
                            Some(page_walks::steps(graph, pic, &laid, seed, slice, &linker))
                        });
                    }
                    // The same key and layout: the same file on disk as before.
                    let file = old
                        .filter(|o| o.key == key && o.layout == laid)
                        .and_then(|o| o.file.clone());
                    *kept[i].lock().expect("no worker panics") = Some(page_state::PictureEntry {
                        pic: pic.clone(),
                        input,
                        key,
                        layout: laid,
                        packed,
                        moves,
                        file,
                    });
                }
            });
        }
    });
    t_lay.report("page: layouts kept, updated or made (all threads)");
    t_draw.report("page: drawn (all threads)");
    t_pack.report("page: drawings packed (all threads)");
    t_moves.report("page: move data packed (all threads)");
    t_walks.report("page: walk steps (all threads)");
    let kept: Vec<page_state::PictureEntry> = kept
        .into_iter()
        .map(|m| {
            m.into_inner()
                .expect("no worker panics")
                .expect("every picture is drawn")
        })
        .collect();
    let taken = n_taken.into_inner();
    DrawnPictures {
        packed: kept.iter().map(|k| k.packed.clone()).collect(),
        walks: steps
            .into_iter()
            .map(|m| m.into_inner().expect("no worker panics"))
            .collect(),
        moves: kept.iter().map(|k| k.moves.clone()).collect(),
        counts: page_state::Counts {
            worked_out: pictures.len() - taken,
            taken_over: taken,
            kept: n_kept.into_inner(),
            updated: n_updated.into_inner(),
            laid_out: n_laid.into_inner(),
        },
        kept,
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
    render_page_keeping(graph, Some(&layouts_dir()))
}

/// Where layouts are kept under their keys: `layouts` in docair's cache folder.
pub fn layouts_dir() -> PathBuf {
    cache_dir().join("layouts")
}

/// A slice's title, as the crumbs and the walks say it: `Scenario checkout`, `Datum order`.
pub(crate) fn slice_title(seed: &SliceSeed, name: &dyn Fn(&str) -> String) -> String {
    match seed.kind {
        SeedKind::Data => format!("Datum {}", name(&seed.id)),
        SeedKind::Route => format!("Route {}", name(&seed.id)),
        SeedKind::Flow => format!("Scenario {}", seed.id),
        SeedKind::Walkthrough => format!("Walkthrough {}", seed.id),
        SeedKind::Part => name(&seed.id),
    }
}

/// As `render_page`, with layouts kept in `keep`, or none kept.
pub fn render_page_keeping(graph: &Graph, keep: Option<&Path>) -> String {
    render_page_with(graph, keep, None)
}

/// What changed between two versions of the project, for the page's changes section
/// (docs/CHANGES.md §7, docair-zrr.7), with what the model explained about it (docs/EXPLAIN.md,
/// docair-8l6.6).
pub struct Changed<'a> {
    /// The graph at A, the version before.
    pub before: &'a Graph,
    /// What `docair diff` found between A and B.
    pub changes: &'a docair_core::changes::Changes,
    /// The stored findings (`docair_ai::explain::ChangeFindings` as JSON), when there are any.
    pub findings: Option<&'a Value>,
    /// Where a part's file opens, at A and at B: the review links each piece's parts there instead of
    /// embedding their code (docair-8l6.82).
    pub links: &'a CodeLinks,
}

/// Where a part's file and line open, at A and at B (docair-8l6.82): a URL with `{path}` and `{line}`,
/// e.g. `https://github.com/o/r/blob/<sha>/{path}#L{line}`, or `vscode://file/<root>/{path}:{line}` for
/// the working tree; none shows the file and line alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeLinks {
    pub at_a: Option<String>,
    pub at_b: Option<String>,
    /// A's commit and B's (none for the working tree), for the places the page in the editor opens
    /// (docs/EDITOR.md §5).
    pub rev_a: Option<String>,
    pub rev_b: Option<String>,
}

/// A place in the code at one version (docs/EDITOR.md §5): its commit, none for the working tree;
/// its file, as the project knows it; its line.
pub(crate) fn place(rev: Option<&str>, path: &str, line: usize) -> Value {
    json!({ "rev": rev, "path": path, "line": line })
}

/// ` data-open="…"`: where a place in the code is at the old and at the new version, null where it is
/// not; the page in the editor opens it there instead of following a link (docs/EDITOR.md §5).
pub(crate) fn data_open(old: Option<Value>, new: Option<Value>) -> String {
    format!(
        r#" data-open="{}""#,
        escape_text(&json!({ "old": old, "new": new }).to_string())
    )
}

impl CodeLinks {
    /// The link to `path` at `line` from a version's template; none without one.
    pub fn to(template: Option<&String>, path: &str, line: usize) -> Option<String> {
        template.map(|t| {
            t.replace("{path}", path)
                .replace("{line}", &line.to_string())
        })
    }
}

/// As `render_page_keeping`, with the changes section when `changed` is given (`docair build
/// --changes <A>`): the changed flows' list, and each flow's slice before and after (docair-zrr.7).
pub fn render_page_with(graph: &Graph, keep: Option<&Path>, changed: Option<&Changed>) -> String {
    render(graph, keep, changed, None)
}

/***
 * As `render_page_with`, keeping the pictures in `state` for the next page (docs/REGENERATE.md §2): `docair serve` after each save. A picture none of whose parts the change touched is taken over; another is laid out from its last layout.
 *
 * @calls docair-viewer.page_state.touched "what the change touched"
 ***/
pub fn render_page_kept(
    graph: &Graph,
    keep: Option<&Path>,
    changed: Option<&Changed>,
    state: &mut crate::page_state::PageState,
) -> String {
    render(graph, keep, changed, Some(state))
}

fn render(
    graph: &Graph,
    keep: Option<&Path>,
    changed: Option<&Changed>,
    state: Option<&mut crate::page_state::PageState>,
) -> String {
    let _t = docair_core::timing::stopwatch("page");
    let last = state.as_deref();
    let touched = last
        .and_then(|s| s.graph.as_ref())
        .and_then(|g| page_state::touched(g, graph));
    let all = {
        let _t = docair_core::timing::stopwatch("page: pictures worked out");
        let prev = match (last, &touched) {
            (Some(s), Some(t)) => s.graph.as_ref().map(|g| Prev {
                graph: g,
                kept: &s.pictures,
                touched: t,
            }),
            _ => None,
        };
        spots(graph, prev.as_ref())
    };
    let mut drawn = draw_all(graph, &all, keep, last.map(|s| &s.pictures));
    if docair_core::timing::on() {
        eprintln!("docair timing: page: {:?}", drawn.counts);
    }
    if let Some(s) = state {
        s.pictures = all
            .idents
            .iter()
            .cloned()
            .zip(std::mem::take(&mut drawn.kept))
            .collect();
        s.graph = Some(graph.clone());
        s.counts = drawn.counts;
    }
    let _rest = docair_core::timing::stopwatch("page: panels, index and the file");
    let mut packed = drawn.packed;
    // The changes section: each changed flow's picture, before and after, packed after the others.
    let view = changed.map(|c| page_changes::change_view(graph, c, keep));
    let changes_at = packed.len();
    for c in view.iter().flat_map(|v| &v.pictures) {
        packed.push(pack(&c.svg));
    }
    let sections: std::collections::HashSet<String> = all
        .spots
        .iter()
        .filter_map(|s| s.addr.strip_prefix("box/").map(str::to_string))
        .collect();
    // The version the page shows: B's commit, or the working tree (docs/EDITOR.md §5).
    let rev = changed.and_then(|c| c.links.rev_b.as_deref());
    let mut panels = page_panels::panels(graph, &sections, rev);
    if let Some(v) = &view {
        panels.data["changes"] = Value::String(v.list.clone());
        panels.data["flowChanges"] = Value::Object(v.details.clone());
    }
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
    // The walks to start: a button next to Tests and its panel (docair-7h5).
    let walks = page_walks::walk_list(&walk_list);
    panels.data["walks"] = Value::String(walks.panel.clone());
    // The panels and the walks' steps go with the pictures, packed the same way, last.
    packed.push(pack(&panels.data.to_string()));
    let panels_at = packed.len() - 1;
    // A changed flow opens on a walk over its changes, walked like any walk (docair-zrr.22).
    for c in view.iter().flat_map(|v| &v.pictures) {
        if let Some(w) = &c.walk {
            walk_steps.insert(c.addr.clone(), w.clone());
        }
    }
    packed.push(pack(&Value::Object(walk_steps).to_string()));
    let walks_at = packed.len() - 1;
    // Moving a part by hand: each picture's layout input and layout, then docair's router as WebAssembly.
    let moves_at = packed.len();
    packed.extend(drawn.moves);
    // A picture of a change is not moved by hand: it has no layout of its own to move in.
    for _ in view.iter().flat_map(|v| &v.pictures) {
        packed.push(pack("{}"));
    }
    packed.push(base64::engine::general_purpose::STANDARD.encode(
        miniz_oxide::deflate::compress_to_vec(docair_layout_wasm::MODULE, PACK_LEVEL),
    ));
    let router_at = packed.len() - 1;
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
        // Where a part picked by its address went, in words (docair-702.15): in orders-api · Apps.
        let layer_name = s
            .layer
            .and_then(|k| infos.iter().find(|l| l.key == k))
            .map(|l| l.name);
        p["where"] = json!(match (s.addr.starts_with("box/"), layer_name) {
            (true, Some(layer)) => format!("in {} · {layer}", s.title),
            _ => format!("in {}", s.title),
        });
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
    // Each changed flow's picture: still (nothing in it is moved by hand), with its details.
    for (k, c) in view.iter().flat_map(|v| &v.pictures).enumerate() {
        let mut p = json!({
            "title": c.title,
            "crumbs": c.crumbs.iter().map(|(l, a)| json!([l, a])).collect::<Vec<_>>(),
            "where": format!("in {}", c.title),
            "svg": changes_at + k,
            "still": true,
        });
        if let Some(seed) = &c.change {
            p["change"] = json!(seed);
        }
        if c.walk.is_some() {
            p["walk"] = json!(true);
        }
        pictures.insert(c.addr.clone(), p);
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
        // What the page starts with hidden ([view] in docair.toml); the pictures hold everything.
        "view": { "hide": graph.view.hide, "show": graph.view.show },
        "first": first,
        "pictures": pictures,
        "home": homes(graph, &all),
        "panels": panels_at,
        "walks": walks_at,
        // The packed picture i moves with packed[moves + i]; the router is packed[router].
        "moves": moves_at,
        "router": router_at,
        // The words the script puts around what the CLI wrote.
        "words": { "more": "… {n} more", "ask": "✦ Ask about this" },
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
            ("changes", view.as_ref().map_or("", |v| v.button.as_str())),
            ("ask", crate::page_ai::BUTTON),
            ("chat", &crate::page_ai::panel()),
            ("walks", &walks.button),
            ("tours", &walks.tours),
            ("tree", &panels.tree),
            ("legend", &panels.legend),
            ("drawer", &panels.drawer),
            ("diag-class", &panels.diag.0),
            ("diag", &escape_text(&panels.diag.1)),
            ("diag-title", &escape_text(&panels.diag.2)),
            ("tidy", &panels.tidy.0),
            ("tidy-banner", &panels.tidy.1),
            ("index", &script_json(&index.to_string())),
            ("packed", &script_json(&Value::from(packed).to_string())),
            ("app", &script_js(SCRIPT)),
        ],
    )
}

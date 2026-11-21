//! The page kept between saves (docs/REGENERATE.md §2): `docair serve` writes the page again after
//! each change, and keeps for each picture's address what it was made from and what it came to, so
//! the next page works out, lays out, draws and packs only what the change touched.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use docair_core::layer_view::LayerPicture;
use docair_core::model::Graph;
use docair_layout::{Input, Layout};
use serde_json::{Value, json};

/// One picture as the last page made it.
#[derive(Clone)]
pub(crate) struct PictureEntry {
    pub(crate) pic: LayerPicture,
    pub(crate) input: Input,
    pub(crate) key: String,
    pub(crate) layout: Layout,
    /// Its drawing and its move data (layout input and layout), packed.
    pub(crate) packed: String,
    pub(crate) moves: String,
    /// The file its layout input and layout are in on disk, once written (`save`).
    pub(crate) file: Option<String>,
}

/// What the last page did with its pictures, for `DOCAIR_TIMING` and for a build's report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Pictures worked out from the graph, and taken over unchanged from the last page.
    pub worked_out: usize,
    pub taken_over: usize,
    /// Layouts kept for the same key, updated from the last one, laid out from the kept layouts by
    /// key or from scratch.
    pub kept: usize,
    pub updated: usize,
    pub laid_out: usize,
}

/***
 * What `docair serve` keeps of its page between saves (docs/REGENERATE.md §2): the graph its pictures were worked out from and, for each picture by its address, the picture, its layout input, key and layout, and its packed drawing. Empty at first: the first page is made as `docair build` makes it.
 ***/
#[derive(Default)]
pub struct PageState {
    pub(crate) graph: Option<Graph>,
    pub(crate) pictures: HashMap<String, PictureEntry>,
    /// What the last page did.
    pub counts: Counts,
}

impl PageState {
    /// Each kept picture by its address (with `|other` for an address's second picture).
    pub fn pictures(&self) -> impl Iterator<Item = (&str, &LayerPicture)> {
        self.pictures.iter().map(|(k, v)| (k.as_str(), &v.pic))
    }

    /// Forget the kept layouts of one picture, or of all: they are laid out again (docs/REGENERATE.md §7).
    pub fn forget(&mut self, addr: Option<&str>) {
        match addr {
            None => {
                self.pictures.clear();
                self.graph = None;
            }
            Some(a) => {
                let other = format!("{a}|other");
                self.pictures.retain(|k, _| k != a && *k != other);
                // Every picture worked out again, so this one is not taken over with its old layout;
                // the others find their layouts kept under their keys.
                self.graph = None;
            }
        }
    }

    /***
     * Write the kept layouts to `dir` (docs/REGENERATE.md §8): `state.json` names, for each picture's address, its key and the file of its layout input and layout; each file is named by its content, so only a new layout is written. `state.json` is replaced in one step, so a reader gets the old state or the new one; files it no longer names are removed.
     ***/
    pub fn save(&mut self, dir: &Path) -> anyhow::Result<()> {
        let files = dir.join("layouts");
        std::fs::create_dir_all(&files)?;
        let mut named = serde_json::Map::new();
        let mut used: HashSet<String> = HashSet::new();
        let mut ids: Vec<&String> = self.pictures.keys().collect();
        ids.sort();
        let ids: Vec<String> = ids.into_iter().cloned().collect();
        for id in ids {
            let Some(k) = self.pictures.get_mut(&id) else {
                continue;
            };
            let file = match &k.file {
                Some(f) => f.clone(),
                None => {
                    let text =
                        serde_json::to_string(&json!({ "input": k.input, "layout": k.layout }))?;
                    let f = format!("{:016x}.json", fnv(text.as_bytes()));
                    let path = files.join(&f);
                    if !path.exists() {
                        std::fs::write(&path, text)?;
                    }
                    k.file = Some(f.clone());
                    f
                }
            };
            used.insert(file.clone());
            named.insert(id, json!({ "key": k.key, "file": file }));
        }
        let state = json!({
            "schema": SCHEMA,
            "code": docair_layout::code_version().to_string(),
            "pictures": named,
        });
        let part = dir.join(format!("state.json.{}.part", std::process::id()));
        std::fs::write(&part, serde_json::to_string(&state)?)?;
        std::fs::rename(&part, dir.join("state.json"))?;
        for e in std::fs::read_dir(&files)?.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !used.contains(&name) {
                let _ = std::fs::remove_file(e.path());
            }
        }
        Ok(())
    }

    /***
     * The layouts `save` wrote to `dir`, as the base of a page (`docair build --from-serve`, docs/REGENERATE.md §8): each picture's layout input, key and layout, nothing taken over (the build works every picture out from the files). Why there is none: no state, another schema, or another layout code's version.
     ***/
    pub fn load(dir: &Path) -> Result<PageState, String> {
        let text = std::fs::read_to_string(dir.join("state.json"))
            .map_err(|_| format!("no state of docair serve in {}", dir.display()))?;
        let v: Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", dir.display()))?;
        if v["schema"] != SCHEMA {
            return Err(format!("{}: not {SCHEMA}", dir.display()));
        }
        if v["code"].as_str() != Some(docair_layout::code_version().to_string().as_str()) {
            return Err("docair serve's layouts were made by other layout code".into());
        }
        let mut pictures = HashMap::new();
        for (id, p) in v["pictures"].as_object().into_iter().flatten() {
            let (Some(key), Some(file)) = (p["key"].as_str(), p["file"].as_str()) else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(dir.join("layouts").join(file)) else {
                continue;
            };
            let Ok(mut v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            let (Ok(input), Ok(layout)) = (
                serde_json::from_value::<Input>(v["input"].take()),
                serde_json::from_value::<Layout>(v["layout"].take()),
            ) else {
                continue;
            };
            let saved = Saved { input, layout };
            pictures.insert(
                id.clone(),
                PictureEntry {
                    pic: LayerPicture {
                        schema: String::new(),
                        layer: None,
                        slice: None,
                        opened: Vec::new(),
                        parts: Vec::new(),
                        lines: Vec::new(),
                    },
                    input: saved.input,
                    key: key.to_string(),
                    layout: saved.layout,
                    packed: String::new(),
                    moves: String::new(),
                    file: Some(file.to_string()),
                },
            );
        }
        Ok(PageState {
            graph: None,
            pictures,
            counts: Counts::default(),
        })
    }
}

/// What `state.json` is (docs/REGENERATE.md §8).
const SCHEMA: &str = "docair.serve-state/0";

/// A layout file as `save` writes it.
struct Saved {
    input: Input,
    layout: Layout,
}

/// FNV-1a, 64 bits: a layout file's name.
fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// What a change touched (docs/REGENERATE.md §2.3).
pub(crate) struct ChangeReach {
    /// The parts a picture that shows them must be worked out again for: a changed node, the ends of
    /// a changed line, the ends of a line that carries a changed datum, and the box a part came into
    /// or left (a box's count of the parts a slice leaves out counts its own parts).
    pub(crate) parts: HashSet<String>,
    /// The boxes around a part that came, went or moved, or around an end of a changed line: a
    /// picture that shows one of them closed draws it for what it holds.
    pub(crate) within: HashSet<String>,
}

/***
 * What a change touched (docs/REGENERATE.md §2.3), between the graph the last page was made from and the new one; a picture it did not touch is the same picture. None when something every picture depends on changed: the system, the view, the flows or the walkthroughs.
 ***/
pub(crate) fn touched(old: &Graph, new: &Graph) -> Option<ChangeReach> {
    let json = |v: &dyn Fn() -> serde_json::Result<serde_json::Value>| v().ok();
    if old.system != new.system
        || old.view != new.view
        || json(&|| serde_json::to_value(&old.flows)) != json(&|| serde_json::to_value(&new.flows))
        || json(&|| serde_json::to_value(&old.walkthroughs))
            != json(&|| serde_json::to_value(&new.walkthroughs))
    {
        return None;
    }
    fn nodes(g: &Graph) -> HashMap<String, &docair_core::model::Node> {
        g.nodes.iter().map(|n| (n.id.clone(), n)).collect()
    }
    fn edges(g: &Graph) -> HashMap<String, &docair_core::model::Edge> {
        g.edges.iter().map(|e| (e.id.clone(), e)).collect()
    }
    let (on, nn) = (nodes(old), nodes(new));
    let mut t = ChangeReach {
        parts: HashSet::new(),
        within: HashSet::new(),
    };
    // Every box around a part, in both graphs.
    let around = |id: &str, within: &mut HashSet<String>| {
        for g in [&on, &nn] {
            let mut cur = g.get(id).and_then(|n| n.part_of.clone());
            let mut steps = 0;
            while let Some(c) = cur {
                if !within.insert(c.clone()) || steps > 64 {
                    break;
                }
                cur = g.get(&c).and_then(|n| n.part_of.clone());
                steps += 1;
            }
        }
    };
    let mut data: Vec<String> = Vec::new();
    for id in on.keys().chain(nn.keys().filter(|k| !on.contains_key(*k))) {
        let (a, b) = (on.get(id), nn.get(id));
        if a == b {
            continue;
        }
        t.parts.insert(id.clone());
        if a.zip(b)
            .is_some_and(|(x, y)| x.kind == "data" || y.kind == "data")
        {
            data.push(id.clone());
        }
        // A part that came, went, moved or became another kind: its box's count and every closed box
        // around it change.
        let moved = match (a, b) {
            (Some(x), Some(y)) => x.part_of != y.part_of || x.kind != y.kind,
            _ => true,
        };
        if moved {
            for n in [a, b].into_iter().flatten() {
                if let Some(p) = &n.part_of {
                    t.parts.insert(p.clone());
                }
            }
            around(id, &mut t.within);
        }
    }
    let (oe, ne) = (edges(old), edges(new));
    for id in oe.keys().chain(ne.keys().filter(|k| !oe.contains_key(*k))) {
        let (a, b) = (oe.get(id), ne.get(id));
        if a == b {
            continue;
        }
        for e in [a, b].into_iter().flatten() {
            for end in [&e.from, &e.to] {
                t.parts.insert(end.clone());
                around(end, &mut t.within);
            }
        }
    }
    // A changed datum's name is in the words of every line that carries it.
    if !data.is_empty() {
        for g in [old, new] {
            for e in &g.edges {
                if e.carries
                    .iter()
                    .chain(&e.carries_back)
                    .any(|d| data.contains(d))
                {
                    t.parts.insert(e.from.clone());
                    t.parts.insert(e.to.clone());
                }
            }
        }
    }
    Some(t)
}

/// A picture of the old graph as a picture of the new one: its lines' graph indices mapped by the
/// lines' ids. None when one of them is not in the new graph.
pub(crate) fn taken_over(pic: &LayerPicture, old: &Graph, new: &Graph) -> Option<LayerPicture> {
    let at: HashMap<&str, usize> = new
        .edges
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.as_str(), i))
        .collect();
    let map = |i: &usize| {
        old.edges
            .get(*i)
            .and_then(|e| at.get(e.id.as_str()).copied())
    };
    let mut out = pic.clone();
    for l in &mut out.lines {
        l.members = l.members.iter().map(map).collect::<Option<Vec<_>>>()?;
        if let Some(s) = &l.says {
            l.says = Some(s.iter().map(map).collect::<Option<Vec<_>>>()?);
        }
    }
    Some(out)
}

/// Whether the change did not touch a picture: none of its parts is touched, and none of its closed
/// boxes holds a part that is.
pub(crate) fn untouched(pic: &LayerPicture, t: &ChangeReach) -> bool {
    pic.parts
        .iter()
        .all(|p| !t.parts.contains(&p.id) && !(p.closed && t.within.contains(&p.id)))
}

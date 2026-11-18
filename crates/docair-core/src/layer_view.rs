//! What each layer draws (SPEC §3.4, §3.5): the parts it is about, the boxes
//! around them, what a line joins to them as context, which boxes stand closed,
//! and lines through left-out parts drawn once, between the parts they join.
//!
//! This is the view model the viewer worked out in the browser (viewer.js:
//! `classify`, `RULES`, `computeView`, `finishView`, `bridgesFrom`,
//! `visibleElements`), now worked out by the CLI (docs/SLICES.md §9, step 2).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::model::{Edge, EdgeKind, Graph, Node, Origin};

/// The schema of `docair layer`'s JSON.
pub const SCHEMA: &str = "docair.layer/0";

const PLATFORM: &[&str] = &[
    "host",
    "network",
    "cluster",
    "namespace",
    "release",
    "workload",
    "container",
];
const CODE_KINDS: &[&str] = &["module", "api", "component", "function"];
/// What the layers take as a module: one of code, and an API description, which holds its endpoints.
const MODULE_KINDS: &[&str] = &["module", "api"];
/// Parts a request through left-out parts may end at, pulled into the picture.
const PULLABLE: &[&str] = &["external", "store", "topic"];
/// How far a line is followed through parts a layer leaves out.
const BRIDGE_HOPS: usize = 8;

/// One of the six layers (SPEC §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayerKey {
    Overview,
    Infra,
    Network,
    K8s,
    Apps,
    Code,
}

/// A layer as the header offers it: its key, its name and the question it answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LayerInfo {
    pub key: LayerKey,
    pub name: &'static str,
    pub ask: &'static str,
}

const LAYERS: [LayerInfo; 6] = [
    LayerInfo {
        key: LayerKey::Overview,
        name: "Overview",
        ask: "What are the big pieces, and how do they talk?",
    },
    LayerInfo {
        key: LayerKey::Infra,
        name: "Infrastructure",
        ask: "Which machines, networks and clusters are there, and what provisions them?",
    },
    LayerInfo {
        key: LayerKey::Network,
        name: "Network",
        ask: "How does a request get in, and where does it go?",
    },
    LayerInfo {
        key: LayerKey::K8s,
        name: "Kubernetes",
        ask: "What is installed in the cluster, and how is it configured?",
    },
    LayerInfo {
        key: LayerKey::Apps,
        name: "Apps",
        ask: "Which programs are there, and how do they talk to each other?",
    },
    LayerInfo {
        key: LayerKey::Code,
        name: "Code",
        ask: "How does a program work inside?",
    },
];

/// A part a layer draws.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerPart {
    pub id: String,
    /// The drawn box it stands in; none at the top of the picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// A box drawn open: parts are drawn inside it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub open: bool,
    /// A box drawn closed: it stands for what is in it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closed: bool,
    /// A closed box: how many parts it holds, all the way down.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub holds: usize,
    /// An open box of code: how many parts docair only found in the code it leaves out (`· 13 more`).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub more: usize,
}

/// A line a layer draws: one or more lines of the graph between two drawn parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerLine {
    pub from: String,
    pub to: String,
    /// The kind of the first line it stands for.
    pub kind: EdgeKind,
    /// Indices into `Graph::edges`: the lines it stands for.
    pub members: Vec<usize>,
    /// Parts the layer leaves out that its lines pass, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub via: Vec<String>,
    /// For each member, the line whose words say what it carries: a request through left-out
    /// parts says the hop that names a route; absent when every member says it itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub says: Option<Vec<usize>>,
    /// It stands for lines whose ends are inside closed boxes, or that pass left-out parts.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bundled: bool,
    /// One of its lines ends at a ref docair could not resolve.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dangling: bool,
}

/// What one layer draws, with some boxes opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerPicture {
    pub schema: String,
    /// The layer it is a picture of; none for a slice's picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<LayerKey>,
    /// The seed of the slice it is a picture of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slice: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opened: Vec<String>,
    /// In the order of the graph's nodes.
    pub parts: Vec<LayerPart>,
    pub lines: Vec<LayerLine>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// What a node is, for the layers: what it is, and what it lies in.
#[derive(Debug, Clone, Copy, Default)]
struct PartClass {
    top: bool,
    leaf: bool,
    iac: bool,
    program: bool,
    library: bool,
    quiet: bool,
    gate: bool,
    in_cluster: bool,
    in_namespace: bool,
    in_program: bool,
    in_library: bool,
    in_iac: bool,
    in_store: bool,
    in_test: bool,
    in_leaf: bool,
    in_quiet: bool,
}

/// The family of a line, for lines through left-out parts: one request, or one piece of data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Request,
    Data,
}

fn family(kind: EdgeKind) -> Option<Family> {
    match kind {
        EdgeKind::Call | EdgeKind::Route | EdgeKind::Select => Some(Family::Request),
        EdgeKind::Data => Some(Family::Data),
        _ => None,
    }
}

/// The graph, looked up, with each node's class and what the view leaves out.
struct Index<'g> {
    graph: &'g Graph,
    sys: &'g str,
    nodes: HashMap<&'g str, &'g Node>,
    children: HashMap<&'g str, Vec<&'g str>>,
    out_of: HashMap<&'g str, Vec<usize>>,
    cls: HashMap<&'g str, PartClass>,
    hidden: HashSet<&'g str>,
    tests: bool,
}

impl<'g> Index<'g> {
    fn new(graph: &'g Graph, tests: Option<bool>) -> Self {
        let sys = graph.system.as_str();
        let nodes: HashMap<&str, &Node> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
        for n in &graph.nodes {
            if n.id == sys {
                continue;
            }
            if let Some(p) = n.part_of.as_deref() {
                children.entry(p).or_default().push(n.id.as_str());
            }
        }
        let mut out_of: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, e) in graph.edges.iter().enumerate() {
            out_of.entry(e.from.as_str()).or_default().push(i);
        }
        let mut ix = Index {
            graph,
            sys,
            nodes,
            children,
            out_of,
            cls: HashMap::new(),
            hidden: HashSet::new(),
            tests: tests.unwrap_or(graph.view.tests),
        };
        ix.classify();
        ix.load_view();
        ix
    }

    fn kind(&self, id: &str) -> &'g str {
        self.nodes.get(id).map_or("", |n| n.kind.as_str())
    }

    fn is_data(&self, id: &str) -> bool {
        self.kind(id) == "data"
    }

    fn parent(&self, id: &str) -> Option<&'g str> {
        self.nodes.get(id).and_then(|n| n.part_of.as_deref())
    }

    fn kids(&self, id: &str) -> &[&'g str] {
        self.children.get(id).map_or(&[], Vec::as_slice)
    }

    /// The boxes around a part, from the inside out, without the system.
    fn ancestors(&self, id: &str) -> Vec<&'g str> {
        let mut out = Vec::new();
        let mut cur = self.parent(id);
        while let Some(c) = cur {
            if c == self.sys || out.contains(&c) {
                break;
            }
            out.push(c);
            cur = self.parent(c);
        }
        out
    }

    fn inside_of(&self, id: &str, outer: &str) -> bool {
        id == outer || self.ancestors(id).contains(&outer)
    }

    /// `inner` lies somewhere inside the group `outer`.
    fn is_inside(&self, outer: &str, inner: &str) -> bool {
        self.ancestors(inner).contains(&outer)
    }

    fn descendants(&self, id: &str) -> usize {
        self.kids(id).iter().map(|k| 1 + self.descendants(k)).sum()
    }

    fn class(&self, id: &str) -> PartClass {
        self.cls.get(id).copied().unwrap_or_default()
    }

    /// What each node is, for the layers (viewer.js `classify`).
    fn classify(&mut self) {
        let sys = self.sys;
        // What speaks at the code layer: a part a doc block is about, a part with a line, and every box around one.
        let mut loud: HashSet<&str> = HashSet::new();
        let raise = |ix: &Index<'g>, loud: &mut HashSet<&'g str>, id: &'g str| {
            let mut cur = Some(id);
            while let Some(c) = cur {
                if c == sys || loud.contains(c) {
                    break;
                }
                loud.insert(c);
                cur = ix.parent(c);
            }
        };
        for n in &self.graph.nodes {
            if n.origin != Origin::Discovered && !self.is_data(&n.id) {
                raise(self, &mut loud, n.id.as_str());
            }
        }
        // A part a doc block is about, and every box around one.
        let told = loud.clone();
        for e in &self.graph.edges {
            if !self.is_data(&e.from) && !self.is_data(&e.to) {
                raise(self, &mut loud, e.from.as_str());
                raise(self, &mut loud, e.to.as_str());
            }
        }
        let roots: Vec<&str> = self
            .graph
            .nodes
            .iter()
            .filter(|n| {
                n.id != sys
                    && match n.part_of.as_deref() {
                        None => true,
                        Some(p) => p == sys || !self.nodes.contains_key(p),
                    }
            })
            .map(|n| n.id.as_str())
            .collect();
        let mut cls: HashMap<&str, PartClass> = HashMap::new();
        let mut todo: Vec<(&str, PartClass)> = roots
            .into_iter()
            .rev()
            .map(|r| (r, PartClass::default()))
            .collect();
        while let Some((id, up)) = todo.pop() {
            if cls.contains_key(id) {
                continue;
            }
            let n = self.nodes[id];
            let kind = n.kind.as_str();
            let file = n.source.as_ref().map_or("", |s| s.file.as_str());
            // Terraform's own parts: its resources and the run itself, not the machines it declares.
            let iac =
                (file.ends_with(".tf") || file.ends_with(".tf.json") || file.ends_with(".hcl"))
                    && !PLATFORM.contains(&kind)
                    && !["gateway", "endpoint", "store", "topic", "external"].contains(&kind);
            let program = !iac && !up.in_program && (kind == "service" || kind == "job");
            let kids = self.kids(id);
            let parts: Vec<&str> = kids.iter().copied().filter(|k| !self.is_data(k)).collect();
            let code_box = |p: Option<&str>| {
                p.is_some_and(|p| {
                    p != sys
                        && (CODE_KINDS.contains(&self.kind(p))
                            || cls.get(p).is_some_and(|c: &PartClass| c.program))
                })
            };
            let c = PartClass {
                top: n.part_of.as_deref() == Some(sys),
                // At the code layer these stand closed until zoomed into or walked through: a module of functions
                // only, and a box of code whose parts docair only found in the code (SPEC §3.4).
                leaf: ((MODULE_KINDS.contains(&kind) || kind == "component")
                    && !kids.is_empty()
                    && kids.iter().all(|k| self.kids(k).is_empty()))
                    || ((CODE_KINDS.contains(&kind) || program)
                        && !parts.is_empty()
                        && parts.iter().all(|k| !told.contains(k))),
                iac,
                program,
                library: MODULE_KINDS.contains(&kind)
                    && !up.in_program
                    && !up.in_library
                    && !up.in_iac,
                // A part docair only found in the code, with no line, in a box of code.
                quiet: !loud.contains(id) && !self.is_data(id) && code_box(n.part_of.as_deref()),
                gate: !up.in_program
                    && (kind == "gateway" || (kind == "endpoint" && up.in_cluster)),
                ..up
            };
            cls.insert(id, c);
            let down = PartClass {
                in_cluster: up.in_cluster || kind == "cluster",
                in_namespace: up.in_namespace || kind == "namespace",
                in_program: up.in_program || program,
                in_library: up.in_library || c.library,
                in_iac: up.in_iac || iac,
                in_store: up.in_store || kind == "store",
                in_test: up.in_test || kind == "test",
                in_leaf: up.in_leaf || c.leaf,
                in_quiet: up.in_quiet || c.quiet,
                ..PartClass::default()
            };
            for k in kids.iter().rev() {
                todo.push((k, down));
            }
        }
        self.cls = cls;
    }

    /// What the page hides when it opens: `[view] hide`, with `[view] show` brought back inside (SPEC §11).
    fn load_view(&mut self) {
        let view = &self.graph.view;
        self.hidden = view
            .hide
            .iter()
            .filter_map(|id| self.nodes.get(id.as_str()).map(|n| n.id.as_str()))
            .collect();
        for id in &view.show {
            if let Some(n) = self.nodes.get(id.as_str()) {
                self.reveal_within(n.id.as_str());
            }
        }
    }

    /// Show a part whose section is hidden: each group from the highest hidden one down comes back, its other children are hidden.
    fn reveal_within(&mut self, id: &'g str) {
        let mut path: Vec<&str> = self.ancestors(id);
        path.reverse();
        path.push(id);
        let Some(first) = path.iter().position(|x| self.hidden.contains(x)) else {
            return;
        };
        for j in first..path.len() {
            self.hidden.remove(path[j]);
            if j == path.len() - 1 {
                break;
            }
            let sibs: Vec<&str> = self.kids(path[j]).to_vec();
            for sib in sibs {
                if sib != path[j + 1] && !self.is_data(sib) {
                    self.hidden.insert(sib);
                }
            }
        }
    }

    /// A node is left out: it or a group around it is hidden, or it is a test while tests are off.
    fn hidden_node(&self, id: &str) -> bool {
        let mut cur = Some(id);
        let mut steps = 0;
        while let Some(c) = cur {
            if c == self.sys || steps > self.nodes.len() {
                break;
            }
            if self.hidden.contains(c) || (!self.tests && self.kind(c) == "test") {
                return true;
            }
            cur = self.parent(c);
            steps += 1;
        }
        false
    }

    fn real(&self, id: &str) -> bool {
        id != self.sys && self.nodes.contains_key(id) && !self.is_data(id)
    }

    /// A part of the Apps picture: a program, a library, a store, a client or service outside, a test suite.
    fn app_part(&self, n: &Node, c: PartClass) -> bool {
        if c.in_program || c.in_library || c.in_iac || c.in_store || c.in_test {
            return false;
        }
        if c.program || c.library || c.iac {
            return true;
        }
        if ["store", "topic", "external", "test"].contains(&n.kind.as_str()) {
            return true;
        }
        // A top-level part of plain code that is no program.
        c.top && !PLATFORM.contains(&n.kind.as_str()) && !c.gate
    }

    /// The parts a layer is about.
    fn focus(&self, key: LayerKey, n: &Node, c: PartClass) -> bool {
        let kind = n.kind.as_str();
        match key {
            LayerKey::Overview => c.top,
            LayerKey::Infra => {
                !c.in_program
                    && !c.in_test
                    && (c.iac
                        || ((PLATFORM.contains(&kind) || c.gate)
                            && !c.in_namespace
                            && !["release", "workload", "container"].contains(&kind)))
            }
            LayerKey::Network => {
                !c.in_program
                    && !c.in_iac
                    && !c.in_test
                    && (c.gate
                        || kind == "network"
                        || kind == "workload"
                        || (kind == "host" && c.in_cluster))
            }
            LayerKey::K8s => c.in_cluster && !c.in_program && !c.in_test,
            LayerKey::Apps => self.app_part(n, c),
            // Inside the programs, down to their modules; a module of functions only stands closed.
            LayerKey::Code => {
                self.app_part(n, c)
                    || ((c.in_program || c.in_library || c.in_iac || c.in_store || c.in_test)
                        && !c.in_leaf
                        && !c.quiet
                        && !c.in_quiet)
            }
        }
    }

    /// Lines of these kinds pull other parts in as context; `Some(None)` for all, `None` for none.
    fn context(key: LayerKey) -> Option<Option<&'static [EdgeKind]>> {
        match key {
            LayerKey::Infra | LayerKey::K8s => Some(None),
            LayerKey::Network => Some(Some(&[
                EdgeKind::Call,
                EdgeKind::Route,
                EdgeKind::Select,
                EdgeKind::Data,
            ])),
            _ => None,
        }
    }

    /// A layer is offered when the graph has what it is about.
    fn needs(&self, key: LayerKey, n: &Node, c: PartClass) -> bool {
        let kind = n.kind.as_str();
        match key {
            LayerKey::Overview => true,
            LayerKey::Infra => ["host", "network", "cluster"].contains(&kind) || c.iac,
            LayerKey::Network => c.gate,
            LayerKey::K8s => ["namespace", "release", "workload", "container"].contains(&kind),
            LayerKey::Apps => c.program || c.library || (c.top && CODE_KINDS.contains(&kind)),
            LayerKey::Code => CODE_KINDS.contains(&kind) && !c.in_iac,
        }
    }

    fn offered(&self) -> Vec<LayerInfo> {
        LAYERS
            .iter()
            .filter(|l| {
                l.key == LayerKey::Overview
                    || self.graph.nodes.iter().any(|n| {
                        self.cls.contains_key(n.id.as_str())
                            && self.needs(l.key, n, self.class(&n.id))
                    })
            })
            .cloned()
            .collect()
    }
}

/// The sets a layer's picture is made of (viewer.js `computeView`).
struct LayerSets<'g> {
    /// The parts the picture is about.
    focus: HashSet<&'g str>,
    flat: HashSet<&'g str>,
    drawn: HashSet<&'g str>,
    parent: HashMap<&'g str, Option<&'g str>>,
    kids: HashMap<&'g str, Vec<&'g str>>,
    closed: HashSet<&'g str>,
}

/// Where a line through left-out parts arrives.
struct BridgeEnd<'g> {
    to: &'g str,
    rep: &'g str,
    via: Vec<&'g str>,
    /// The lines followed, the first one included.
    hops: Vec<usize>,
    pulled: bool,
}

impl<'g> Index<'g> {
    fn compute(&self, key: LayerKey, opened: &[&'g str]) -> LayerSets<'g> {
        let focus = self.layer_focus(key, opened);
        self.sets(key, focus, Self::context(key))
    }

    /// The parts a layer is about, with the given boxes opened.
    fn layer_focus(&self, key: LayerKey, opened: &[&'g str]) -> HashSet<&'g str> {
        let mut focus: HashSet<&str> = HashSet::new();
        for n in &self.graph.nodes {
            if self.real(&n.id) && self.focus(key, n, self.class(&n.id)) {
                focus.insert(n.id.as_str());
            }
        }
        if key == LayerKey::Apps {
            // A library of several crates shows them; one crate is one box.
            let open: Vec<&str> = focus
                .iter()
                .copied()
                .filter(|g| {
                    self.class(g).library
                        && self
                            .kids(g)
                            .iter()
                            .all(|k| MODULE_KINDS.contains(&self.kind(k)))
                })
                .collect();
            for g in open {
                for k in self.kids(g) {
                    if self.real(k) {
                        focus.insert(k);
                    }
                }
            }
        }
        for &g in opened {
            if !self.real(g) {
                continue;
            }
            focus.insert(g);
            for k in self.kids(g) {
                if self.real(k) {
                    focus.insert(k);
                }
            }
        }
        focus
    }

    /// A picture's sets from what it is about: the dissolved boxes, the context (lines of these
    /// kinds; `Some(None)` all, `None` none), the frames and closed boxes, and at Network what a
    /// request pulls in.
    fn sets(
        &self,
        key: LayerKey,
        focus: HashSet<&'g str>,
        context_kinds: Option<Option<&'static [EdgeKind]>>,
    ) -> LayerSets<'g> {
        let mut flat: HashSet<&str> = HashSet::new();
        if matches!(key, LayerKey::Apps | LayerKey::Code) {
            for n in &self.graph.nodes {
                if PLATFORM.contains(&n.kind.as_str()) && !focus.contains(n.id.as_str()) {
                    flat.insert(n.id.as_str());
                }
            }
        }
        let mut holds: HashSet<&str> = HashSet::new();
        for id in &focus {
            holds.extend(self.ancestors(id));
        }
        // Context: what a line joins to the parts of this layer, folded into the outermost box that holds none of them.
        let mut context: HashSet<&str> = HashSet::new();
        if let Some(kinds) = context_kinds {
            let stands_for = |id: &str| {
                (focus.contains(id) && !holds.contains(id))
                    || self
                        .ancestors(id)
                        .iter()
                        .any(|a| focus.contains(a) && !holds.contains(a))
            };
            for e in &self.graph.edges {
                if kinds.is_some_and(|k| !k.contains(&e.kind)) {
                    continue;
                }
                for (x, y) in [
                    (e.from.as_str(), e.to.as_str()),
                    (e.to.as_str(), e.from.as_str()),
                ] {
                    if !focus.contains(x) || !self.real(y) || focus.contains(y) || stands_for(y) {
                        continue;
                    }
                    context.insert(self.context_box(y, &focus, &holds, &flat));
                }
            }
        }
        let mut drawn: HashSet<&str> = focus.union(&context).copied().collect();
        let mut v = self.finish(flat.clone(), &mut drawn);
        if key != LayerKey::Network {
            v.focus = focus;
            return v;
        }
        // A request that leaves through parts this layer leaves out may end at a service outside: draw it.
        let mut pulled: HashSet<&str> = HashSet::new();
        for (i, e) in self.graph.edges.iter().enumerate() {
            if family(e.kind).is_none()
                || self.drawn_rep_in(&e.from, &v, &v.closed).is_none()
                || self.drawn_rep_in(&e.to, &v, &v.closed).is_some()
            {
                continue;
            }
            for end in self.bridges_from(i, &v, &v.closed, true) {
                if end.pulled {
                    pulled.insert(self.context_box(end.to, &focus, &holds, &flat));
                }
            }
        }
        if pulled.is_empty() {
            v.focus = focus;
            return v;
        }
        let mut drawn = v.drawn;
        drawn.extend(pulled);
        let mut v = self.finish(flat, &mut drawn);
        v.focus = focus;
        v
    }

    /// The first layer whose focus holds the part, else the first that draws it (viewer.js `layerFor`).
    fn layer_for(&self, id: &'g str) -> Option<LayerKey> {
        let views: Vec<(LayerKey, LayerSets)> = self
            .offered()
            .iter()
            .map(|l| (l.key, self.compute(l.key, &[])))
            .collect();
        views
            .iter()
            .find(|(_, v)| v.focus.contains(id))
            .or_else(|| {
                views
                    .iter()
                    .find(|(_, v)| self.drawn_rep_in(id, v, &v.closed).is_some())
            })
            .map(|(k, _)| *k)
    }

    /// A box's section at a layer: the box and everything in it one level down, the boxes around it,
    /// and what lines join to it folded into context boxes; nothing else of the layer.
    fn section(&self, key: LayerKey, bx: &'g str) -> LayerPicture {
        let mut focus: HashSet<&str> = HashSet::from([bx]);
        for k in self.kids(bx) {
            if self.real(k) {
                focus.insert(k);
            }
        }
        // Context from lines of every kind, except at Network, which keeps its own.
        let kinds = Self::context(key).unwrap_or(None);
        let v = self.sets(key, focus, Some(kinds));
        self.draw(Some(key), &[bx], v)
    }

    /// The outermost box around `id` that holds no part of the layer and is not dissolved.
    fn context_box(
        &self,
        id: &'g str,
        focus: &HashSet<&str>,
        holds: &HashSet<&str>,
        flat: &HashSet<&str>,
    ) -> &'g str {
        let mut r = id;
        let mut p = self.parent(id);
        while let Some(c) = p {
            if c == self.sys || focus.contains(c) || holds.contains(c) || flat.contains(c) || r == c
            {
                break;
            }
            r = c;
            p = self.parent(c);
        }
        r
    }

    /// Frames, parents and closed boxes of a layer's picture (viewer.js `finishView`).
    fn finish(&self, flat: HashSet<&'g str>, drawn: &mut HashSet<&'g str>) -> LayerSets<'g> {
        // Frames: every box that holds a drawn part, unless the layer dissolves it.
        let before: Vec<&str> = drawn.iter().copied().collect();
        for id in before {
            for a in self.ancestors(id) {
                if !flat.contains(a) {
                    drawn.insert(a);
                }
            }
        }
        let mut parent: HashMap<&str, Option<&str>> = HashMap::new();
        let mut kids: HashMap<&str, Vec<&str>> = HashMap::new();
        // In the order of the graph's nodes, so the picture is the same every time.
        let order: Vec<&str> = self
            .graph
            .nodes
            .iter()
            .map(|n| n.id.as_str())
            .filter(|id| drawn.contains(id))
            .chain(
                drawn
                    .iter()
                    .copied()
                    .filter(|id| !self.nodes.contains_key(id)),
            )
            .collect();
        for id in &order {
            let mut p = self.parent(id);
            while let Some(c) = p {
                if c == self.sys || drawn.contains(c) {
                    break;
                }
                p = self.parent(c);
            }
            let up = p.filter(|c| *c != self.sys);
            parent.insert(id, up);
            if let Some(up) = up {
                kids.entry(up).or_default().push(id);
            }
        }
        // A drawn box with nothing drawn inside stands closed for what is in it.
        let closed = drawn
            .iter()
            .copied()
            .filter(|id| !kids.contains_key(id) && self.kids(id).iter().any(|k| !self.is_data(k)))
            .collect();
        LayerSets {
            focus: HashSet::new(),
            flat,
            drawn: drawn.clone(),
            parent,
            kids,
            closed,
        }
    }

    /// The drawn node that stands for `id`: itself, or the outermost closed box around it; none when the layer leaves it out.
    fn drawn_rep_in(
        &self,
        id: &'g str,
        v: &LayerSets<'g>,
        closed: &HashSet<&'g str>,
    ) -> Option<&'g str> {
        let mut r = None;
        let mut cur = Some(id);
        let mut steps = 0;
        while let Some(c) = cur {
            if c == self.sys || steps > self.nodes.len() {
                break;
            }
            if closed.contains(c) && v.drawn.contains(c) {
                r = Some(c);
            }
            cur = self.parent(c);
            steps += 1;
        }
        r.or_else(|| v.drawn.contains(id).then_some(id))
    }

    /// What a left-out part goes by on a line: the outermost left-out box it lies in, or itself.
    fn via_part(&self, id: &'g str, v: &LayerSets<'g>) -> &'g str {
        let mut r = id;
        let mut p = self.parent(id);
        while let Some(c) = p {
            if c == self.sys || v.drawn.contains(c) || v.flat.contains(c) || r == c {
                break;
            }
            r = c;
            p = self.parent(c);
        }
        r
    }

    /// Follow line `i` from a drawn part through parts the picture leaves out, along lines of the
    /// same family, to the drawn parts it reaches (viewer.js `bridgesFrom`).
    fn bridges_from(
        &self,
        i: usize,
        v: &LayerSets<'g>,
        closed: &HashSet<&'g str>,
        pull: bool,
    ) -> Vec<BridgeEnd<'g>> {
        let edges: &'g [Edge] = &self.graph.edges;
        let first = &edges[i];
        let Some(fam) = family(first.kind) else {
            return Vec::new();
        };
        let path = route_path(first);
        let mut ends = Vec::new();
        let mut seen: HashSet<&str> = HashSet::from([first.from.as_str()]);
        let mut frontier: Vec<(&str, Vec<&str>, Vec<usize>)> =
            vec![(first.to.as_str(), Vec::new(), vec![i])];
        for _ in 0..BRIDGE_HOPS {
            if frontier.is_empty() {
                break;
            }
            let mut next = Vec::new();
            for (id, via, hops) in frontier {
                if !seen.insert(id) {
                    continue;
                }
                if self.hidden_node(id) || self.is_data(id) {
                    continue;
                }
                if let Some(at) = self.drawn_rep_in(id, v, closed) {
                    ends.push(BridgeEnd {
                        to: id,
                        rep: at,
                        via,
                        hops,
                        pulled: false,
                    });
                    continue;
                }
                if pull && PULLABLE.contains(&self.kind(id)) && self.kids(id).is_empty() {
                    ends.push(BridgeEnd {
                        to: id,
                        rep: id,
                        via,
                        hops,
                        pulled: true,
                    });
                    continue;
                }
                let part = self.via_part(id, v);
                let mut via = via;
                if via.last() != Some(&part) {
                    via.push(part);
                }
                let onward: Vec<usize> = self
                    .out_of
                    .get(id)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|&j| family(edges[j].kind) == Some(fam))
                    .collect();
                // A request that arrives at a box the layer dissolves (a Deployment) ends at the programs drawn inside it.
                if onward.is_empty() && fam == Family::Request {
                    for n in &self.graph.nodes {
                        let d = n.id.as_str();
                        if v.drawn.contains(d) && self.class(d).program && self.inside_of(d, id) {
                            ends.push(BridgeEnd {
                                to: d,
                                rep: self.drawn_rep_in(d, v, closed).unwrap_or(d),
                                via: via.clone(),
                                hops: hops.clone(),
                                pulled: false,
                            });
                        }
                    }
                    continue;
                }
                for j in match_routes(edges, onward, path.as_deref()) {
                    let mut more = hops.clone();
                    more.push(j);
                    next.push((edges[j].to.as_str(), via.clone(), more));
                }
            }
            frontier = next;
        }
        ends
    }

    /// The picture: its parts and its lines (viewer.js `visibleElements`, without the words and sizes).
    fn picture(&self, key: LayerKey, opened: &[&'g str]) -> LayerPicture {
        let v = self.compute(key, opened);
        self.draw(Some(key), opened, v)
    }

    /// The parts and lines of a picture, from its sets.
    fn draw(&self, key: Option<LayerKey>, opened: &[&'g str], v: LayerSets<'g>) -> LayerPicture {
        let closed = &v.closed;
        let rep = |id: &'g str| self.drawn_rep_in(id, &v, closed);
        let mut parts = Vec::new();
        for n in &self.graph.nodes {
            let id = n.id.as_str();
            if id == self.sys
                || self.is_data(id)
                || !v.drawn.contains(id)
                || self.hidden_node(id)
                || rep(id) != Some(id)
            {
                continue;
            }
            let real = self
                .kids(id)
                .iter()
                .filter(|c| !self.hidden_node(c) && !self.is_data(c))
                .count();
            let inside = v
                .kids
                .get(id)
                .map_or(0, |k| k.iter().filter(|c| !self.hidden_node(c)).count());
            let collapsed = closed.contains(id) && real > 0;
            let open = !collapsed && inside > 0;
            let group = real > 0;
            let shut = collapsed || (group && !open);
            let more = if open {
                self.kids(id)
                    .iter()
                    .filter(|k| {
                        self.class(k).quiet && !v.drawn.contains(*k) && !self.hidden_node(k)
                    })
                    .count()
            } else {
                0
            };
            parts.push(LayerPart {
                id: n.id.clone(),
                parent: v.parent.get(id).copied().flatten().map(str::to_string),
                open,
                closed: shut,
                holds: if shut { self.descendants(id) } else { 0 },
                more,
            });
        }
        struct Merged<'g> {
            from: &'g str,
            to: &'g str,
            kind: EdgeKind,
            members: Vec<usize>,
            says: Vec<usize>,
            via: Vec<&'g str>,
            dangling: bool,
        }
        let mut merged: Vec<Merged> = Vec::new();
        let mut at: HashMap<(&str, &str), usize> = HashMap::new();
        let mut add =
            |a: &'g str, b: &'g str, e: &'g Edge, i: usize, via: &[&'g str], said: usize| {
                if a == b
                    || a == self.sys
                    || b == self.sys
                    || self.is_inside(a, b)
                    || self.is_inside(b, a)
                {
                    return;
                }
                let k = *at.entry((a, b)).or_insert_with(|| {
                    merged.push(Merged {
                        from: a,
                        to: b,
                        kind: e.kind,
                        members: Vec::new(),
                        says: Vec::new(),
                        via: Vec::new(),
                        dangling: false,
                    });
                    merged.len() - 1
                });
                let m = &mut merged[k];
                if !m.members.contains(&i) {
                    m.members.push(i);
                    m.says.push(said);
                }
                if [&e.from, &e.to]
                    .iter()
                    .any(|x| self.kind(x) == "unresolved")
                {
                    m.dangling = true;
                }
                for id in via {
                    if !m.via.contains(id) {
                        m.via.push(id);
                    }
                }
            };
        for (i, e) in self.graph.edges.iter().enumerate() {
            if self.hidden_node(&e.from)
                || self.hidden_node(&e.to)
                || self.is_data(&e.from)
                || self.is_data(&e.to)
            {
                continue;
            }
            let Some(a) = rep(e.from.as_str()) else {
                continue;
            };
            if let Some(b) = rep(e.to.as_str()) {
                add(a, b, e, i, &[], i);
                continue;
            }
            // The line goes on through parts this layer leaves out: one line to where it arrives.
            for end in self.bridges_from(i, &v, closed, false) {
                // A joined line says the request it carries: the hop that names a route, else its first.
                let said = end
                    .hops
                    .iter()
                    .copied()
                    .find(|&j| route_path(&self.graph.edges[j]).is_some())
                    .unwrap_or(i);
                add(a, end.rep, e, i, &end.via, said);
            }
        }
        let edges = &self.graph.edges;
        let lines = merged
            .into_iter()
            .map(|m| {
                let says = (m.says != m.members).then(|| m.says.clone());
                let bundled = !m.via.is_empty()
                    || m.members.iter().any(|&i| {
                        let e = &edges[i];
                        rep(e.from.as_str()).unwrap_or(e.from.as_str()) != e.from
                            || rep(e.to.as_str()).unwrap_or(e.to.as_str()) != e.to
                    });
                LayerLine {
                    from: m.from.to_string(),
                    to: m.to.to_string(),
                    kind: m.kind,
                    members: m.members,
                    via: m.via.iter().map(|s| s.to_string()).collect(),
                    says,
                    bundled,
                    dangling: m.dangling,
                }
            })
            .collect();
        LayerPicture {
            schema: SCHEMA.to_string(),
            layer: key,
            slice: None,
            opened: opened.iter().map(|s| s.to_string()).collect(),
            parts,
            lines,
        }
    }
}

/// The path a request names: `POST /api/ask` → `/api/ask`; a route's: `HTTP /api` → `/api`.
pub fn route_path(e: &Edge) -> Option<String> {
    let text = format!(
        "{} {}",
        e.payload.as_deref().unwrap_or(""),
        e.label.as_deref().unwrap_or("")
    );
    const METHODS: &[&str] = &[
        "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "HTTP",
    ];
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    for (p, _) in text.char_indices() {
        if text[..p].chars().next_back().is_some_and(word) {
            continue;
        }
        for m in METHODS {
            let Some(rest) = text[p..].strip_prefix(m) else {
                continue;
            };
            let trimmed = rest.trim_start();
            if trimmed.len() == rest.len() || !trimmed.starts_with('/') {
                continue;
            }
            let end = trimmed
                .find(|c: char| c.is_whitespace() || matches!(c, ':' | ',' | '(' | ')'))
                .unwrap_or(trimmed.len());
            return Some(trimmed[..end].to_string());
        }
    }
    None
}

/// Where a request goes on at a fork: the routes whose path is the longest prefix of its own, and the lines that name no path.
fn match_routes(edges: &[Edge], onward: Vec<usize>, path: Option<&str>) -> Vec<usize> {
    let Some(path) = path else { return onward };
    let paths: Vec<Option<String>> = onward.iter().map(|&j| route_path(&edges[j])).collect();
    if paths.iter().all(Option::is_none) {
        return onward;
    }
    let fits = |p: &str| {
        p == "/"
            || path == p
            || path.starts_with(&if p.ends_with('/') {
                p.to_string()
            } else {
                format!("{p}/")
            })
    };
    let mut best = "";
    for p in paths.iter().flatten() {
        if fits(p) && p.len() > best.len() {
            best = p;
        }
    }
    onward
        .iter()
        .zip(&paths)
        .filter(|(_, p)| p.is_none() || (!best.is_empty() && p.as_deref() == Some(best)))
        .map(|(&j, _)| j)
        .collect()
}

/***
 * The layers a graph offers, in the order of the header, each with the question it answers (SPEC §3.4).
 ***/
pub fn layers(graph: &Graph) -> Vec<LayerInfo> {
    Index::new(graph, None).offered()
}

/***
 * What one layer draws, with the given boxes opened (SPEC §3.4, §3.5).
 *
 * The parts are those the layer is about, the boxes around them, and what a
 * line joins to them as context; a box with nothing drawn inside stands
 * closed; a line through parts the layer leaves out is drawn once, between
 * the drawn parts it joins, and keeps what it passes (`via`). `[view]` in
 * `docair.toml` says what is hidden and whether tests are shown; `tests`
 * overrides the latter. None when the graph does not offer the layer.
 ***/
pub fn picture(
    graph: &Graph,
    key: LayerKey,
    opened: &[String],
    tests: Option<bool>,
) -> Option<LayerPicture> {
    let ix = Index::new(graph, tests);
    if !ix.offered().iter().any(|l| l.key == key) {
        return None;
    }
    let opened: Vec<&str> = opened
        .iter()
        .filter_map(|id| ix.nodes.get(id.as_str()).map(|n| n.id.as_str()))
        .collect();
    Some(ix.picture(key, &opened))
}

/***
 * A box's section: the picture of one box at its layer, one level down, with what its lines join to as context.
 *
 * The box stands at the first layer whose focus holds it, else the first
 * that draws it (viewer.js `layerFor`). Its parts are the box and everything
 * directly in it (parts found only in the code too); the boxes around it are
 * frames; a part outside that a line joins to it is folded into the
 * outermost box around that part which holds nothing of the section, drawn
 * closed. Not the whole layer with the box opened: a section of a box at Code
 * stays small. None for a part that holds nothing, or that no layer draws.
 ***/
pub fn section_picture(
    graph: &Graph,
    bx: &str,
    tests: Option<bool>,
) -> Option<(LayerKey, LayerPicture)> {
    let ix = Index::new(graph, tests);
    let n = ix.nodes.get(bx)?;
    let id = n.id.as_str();
    if id == ix.sys || ix.kids(id).iter().all(|k| ix.is_data(k)) {
        return None;
    }
    let key = ix.layer_for(id)?;
    Some((key, ix.section(key, id)))
}

/***
 * A slice as a picture: its parts in the boxes around them, each box open, each of its lines on its own.
 *
 * A part of the slice that holds parts the slice leaves out stands closed for them.
 ***/
pub fn slice_picture(graph: &Graph, slice: &crate::slice::Slice) -> LayerPicture {
    let ix = Index::new(graph, Some(true));
    let mut held: HashSet<&str> = HashSet::new();
    for p in &slice.parts {
        if let Some(n) = ix.nodes.get(p.as_str()) {
            held.insert(n.id.as_str());
            held.extend(ix.ancestors(&n.id));
        }
    }
    let more: HashMap<&str, usize> = slice
        .boxes
        .iter()
        .map(|b| (b.id.as_str(), b.more))
        .collect();
    let mut parts = Vec::new();
    for n in &graph.nodes {
        let id = n.id.as_str();
        if !held.contains(id) || id == ix.sys || ix.is_data(id) {
            continue;
        }
        let parent = ix.ancestors(id).into_iter().find(|a| held.contains(a));
        let open = ix.kids(id).iter().any(|k| held.contains(k));
        let closed = !open && ix.kids(id).iter().any(|k| !ix.is_data(k));
        parts.push(LayerPart {
            id: n.id.clone(),
            parent: parent.map(str::to_string),
            open,
            closed,
            holds: if closed { ix.descendants(id) } else { 0 },
            more: if open {
                more.get(id).copied().unwrap_or(0)
            } else {
                0
            },
        });
    }
    let lines = slice
        .lines
        .iter()
        .filter(|l| {
            held.contains(l.from.as_str())
                && held.contains(l.to.as_str())
                && l.from != l.to
                && !ix.is_inside(&l.from, &l.to)
                && !ix.is_inside(&l.to, &l.from)
        })
        .map(|l| LayerLine {
            from: l.from.clone(),
            to: l.to.clone(),
            kind: l.kind,
            members: vec![l.edge],
            via: Vec::new(),
            says: None,
            bundled: false,
            dangling: [&l.from, &l.to].iter().any(|x| ix.kind(x) == "unresolved"),
        })
        .collect();
    LayerPicture {
        schema: SCHEMA.to_string(),
        layer: None,
        slice: Some(slice.seed.id.clone()),
        opened: Vec::new(),
        parts,
        lines,
    }
}

/// Pictures as `docair layer` prints them: pretty JSON.
pub fn to_json<T: Serialize + ?Sized>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("a picture is always serializable")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(payload: &str, label: &str) -> Edge {
        Edge {
            from: "a".into(),
            to: "b".into(),
            kind: EdgeKind::Call,
            label: (!label.is_empty()).then(|| label.into()),
            via: None,
            payload: (!payload.is_empty()).then(|| payload.into()),
            reply: None,
            text: None,
            carries: Vec::new(),
            carries_back: Vec::new(),
            from_code: false,
            from_api: None,
            reach: None,
            tag: String::new(),
            source: crate::model::SourceRef {
                file: String::new(),
                line: 0,
            },
            origin: Origin::Authored,
        }
    }

    #[test]
    fn a_request_names_its_path() {
        assert_eq!(
            route_path(&edge("POST /api/ask: question", "")).as_deref(),
            Some("/api/ask")
        );
        assert_eq!(route_path(&edge("", "HTTP /api")).as_deref(), Some("/api"));
        assert_eq!(
            route_path(&edge("GET  /healthz", "")).as_deref(),
            Some("/healthz")
        );
        assert_eq!(route_path(&edge("XGET /a", "")), None);
        assert_eq!(route_path(&edge("GETS /a", "")), None);
        assert_eq!(route_path(&edge("files", "uploads")), None);
    }

    #[test]
    fn a_fork_takes_the_longest_route() {
        let edges = vec![
            edge("", "HTTP /"),
            edge("", "HTTP /api"),
            edge("", "selects"),
        ];
        assert_eq!(
            match_routes(&edges, vec![0, 1, 2], Some("/api/ask")),
            vec![1, 2]
        );
        assert_eq!(
            match_routes(&edges, vec![0, 1, 2], Some("/index.html")),
            vec![0, 2]
        );
        assert_eq!(match_routes(&edges, vec![0, 1, 2], None), vec![0, 1, 2]);
    }
}

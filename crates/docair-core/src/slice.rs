//! Slices: one part of the system, from a seed to its ends (docs/SLICES.md §3, SPEC §3.7).
//!
//! A slice is worked out from the graph alone, the same way every time: no
//! clock, no chance, nothing from the viewer. The CLI prints it
//! (`docair slice`), and later steps lay it out and draw it.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::model::{Edge, EdgeKind, Graph, Node, Reach};

/// The schema of `docair slice`'s JSON.
pub const SCHEMA: &str = "docair.slice/0";

/// How far a datum travels inside other lines from a part that has it (SPEC §3.6).
const INSIDE_HOPS: usize = 6;

/// How far a request is followed back from the route it reaches.
const WAY_IN_HOPS: usize = 8;

/// What a slice starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SeedKind {
    /// A datum (`@kind data`): its way from where it is made.
    Data,
    /// A part that serves a route (it has `http`): the way in, and its own lines.
    Route,
    /// A scenario (`@flow`): its steps.
    Flow,
    /// A recorded walkthrough: its steps.
    Walkthrough,
    /// Any other part: what it calls and what calls it.
    Part,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceSeed {
    pub kind: SeedKind,
    /// A node id, a flow's name, or a walkthrough's id.
    pub id: String,
}

/// Which way a datum travels along a line: with what the line sends, or back in a call's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CarryWay {
    There,
    Back,
}

/// How docair knows that a line carries a datum (docs/SLICES.md §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CarryHow {
    /// The line's words name the datum, as an author wrote them.
    Named,
    /// The words come from the code: the callee's signature or an API description.
    Typed,
}

/// One step of a slice: a datum made or moved, a scenario's step, a walkthrough's step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceStep {
    /// The parts the step is about, most important first.
    pub nodes: Vec<String>,
    /// Indices into `Graph::edges`: the lines the step goes along.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<usize>,
    /// Data slices: where the datum is made.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub made: bool,
    /// Data slices: which way the datum travels along the step's line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub way: Option<CarryWay>,
    /// Data slices: the datum travels inside a line that does not name it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inside: bool,
    /// Data slices: the step starts a stretch the steps before do not reach.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub apart: bool,
    /// Scenarios and walkthroughs: the step's words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A line of the graph that a slice holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceLine {
    /// Index into `Graph::edges`.
    pub edge: usize,
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    /// How sure the code is where the line goes; absent means certain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach: Option<Reach>,
    /// Data slices: how docair knows the line carries the datum; absent when the datum travels inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub how: Option<CarryHow>,
}

/// Where a slice ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceEnd {
    pub id: String,
    /// The end has a name that rarely changes: a route, a store, a program, a service outside.
    pub stable: bool,
    /// When the end's own name changes easily: the nearest part around it whose name rarely changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
}

/// A box that holds parts of the slice, and how many of its own parts the slice leaves out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliceBox {
    pub id: String,
    pub more: usize,
}

/// Data slices: how many of the lines that carry the datum the slice reaches from where it is made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataCovers {
    /// Lines that carry the datum, there or back.
    pub lines: usize,
    /// Of those, the lines reached before the first stretch of its own.
    pub reached: usize,
    /// Stretches of their own: lines not reached from the steps before them.
    pub apart: usize,
}

/// One part of the system: from a seed to its ends (docs/SLICES.md §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slice {
    pub schema: String,
    pub seed: SliceSeed,
    pub ends: Vec<SliceEnd>,
    /// Every part the slice holds, in the order the slice meets them.
    pub parts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<SliceStep>,
    pub lines: Vec<SliceLine>,
    /// The boxes around the parts, by id.
    pub boxes: Vec<SliceBox>,
    /// The data the slice's lines carry, or its parts make.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub covers: Option<DataCovers>,
}

/// The graph, looked up by id.
struct Index<'g> {
    graph: &'g Graph,
    nodes: HashMap<&'g str, &'g Node>,
    children: HashMap<&'g str, Vec<&'g str>>,
}

impl<'g> Index<'g> {
    fn new(graph: &'g Graph) -> Self {
        let nodes: HashMap<&str, &Node> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
        for n in &graph.nodes {
            if let Some(p) = n.part_of.as_deref() {
                children.entry(p).or_default().push(n.id.as_str());
            }
        }
        Index {
            graph,
            nodes,
            children,
        }
    }

    fn is_data(&self, id: &str) -> bool {
        self.nodes.get(id).is_some_and(|n| n.kind == "data")
    }

    fn edge(&self, i: usize) -> &'g Edge {
        &self.graph.edges[i]
    }

    /// The boxes around a part, from the inside out, without the system.
    fn ancestors(&self, id: &str) -> Vec<&'g str> {
        let mut out = Vec::new();
        let mut cur = self.nodes.get(id).and_then(|n| n.part_of.as_deref());
        while let Some(p) = cur {
            if p == self.graph.system || out.contains(&p) {
                break;
            }
            let Some(n) = self.nodes.get(p) else { break };
            out.push(n.id.as_str());
            cur = n.part_of.as_deref();
        }
        out
    }

    /// The part and everything inside it, data left out.
    fn within(&self, id: &str) -> HashSet<&'g str> {
        let mut out = HashSet::new();
        let mut todo = vec![id];
        while let Some(cur) = todo.pop() {
            let Some(n) = self.nodes.get(cur) else {
                continue;
            };
            if n.kind == "data" || !out.insert(n.id.as_str()) {
                continue;
            }
            todo.extend(self.children.get(cur).into_iter().flatten().copied());
        }
        out
    }

    /// A name that rarely changes: anything but code inside a program or a library's crate, config and tests.
    fn stable(&self, id: &str) -> bool {
        let Some(n) = self.nodes.get(id) else {
            return false;
        };
        if n.http.is_some() {
            return true;
        }
        match n.kind.as_str() {
            "function" | "component" | "config" | "test" => false,
            // A library and its crates: no program around it, and no more than the library.
            "module" => {
                let around = self.ancestors(id);
                around.len() <= 1
                    && around
                        .iter()
                        .all(|a| !matches!(self.nodes[a].kind.as_str(), "service" | "job"))
            }
            _ => true,
        }
    }

    /// The nearest part around a part whose name rarely changes.
    fn stable_around(&self, id: &str) -> Option<&'g str> {
        self.ancestors(id).into_iter().find(|a| self.stable(a))
    }
}

/// Every seed the graph offers: each datum, each route, each scenario and walkthrough, and each other part
/// with a line that holds no parts of its own (a box is shown by its section; `find` still takes one).
pub fn seeds(graph: &Graph) -> Vec<SliceSeed> {
    let boxes: HashSet<&str> = graph
        .nodes
        .iter()
        .filter_map(|n| n.part_of.as_deref())
        .collect();
    let touched: HashSet<&str> = graph
        .edges
        .iter()
        .flat_map(|e| [e.from.as_str(), e.to.as_str()])
        .collect();
    let mut out = Vec::new();
    for n in &graph.nodes {
        if n.kind == "data" {
            out.push(seed_of(n));
        }
    }
    for n in &graph.nodes {
        if n.kind != "data" && n.http.is_some() {
            out.push(seed_of(n));
        }
    }
    for f in &graph.flows {
        out.push(SliceSeed {
            kind: SeedKind::Flow,
            id: f.name.clone(),
        });
    }
    for w in &graph.walkthroughs {
        out.push(SliceSeed {
            kind: SeedKind::Walkthrough,
            id: w.id.clone(),
        });
    }
    for n in &graph.nodes {
        if n.id != graph.system
            && n.kind != "data"
            && n.http.is_none()
            && !boxes.contains(n.id.as_str())
            && touched.contains(n.id.as_str())
        {
            out.push(seed_of(n));
        }
    }
    out
}

fn seed_of(n: &Node) -> SliceSeed {
    let kind = if n.kind == "data" {
        SeedKind::Data
    } else if n.http.is_some() {
        SeedKind::Route
    } else {
        SeedKind::Part
    };
    SliceSeed {
        kind,
        id: n.id.clone(),
    }
}

/***
 * Find the seed a reader names: `flow:<name>`, `walkthrough:<id>`, or a part's or datum's id.
 ***/
pub fn find(graph: &Graph, name: &str) -> Option<SliceSeed> {
    if let Some(flow) = name.strip_prefix("flow:") {
        return graph
            .flows
            .iter()
            .any(|f| f.name == flow)
            .then(|| SliceSeed {
                kind: SeedKind::Flow,
                id: flow.to_string(),
            });
    }
    if let Some(walk) = name.strip_prefix("walkthrough:") {
        return graph
            .walkthroughs
            .iter()
            .any(|w| w.id == walk)
            .then(|| SliceSeed {
                kind: SeedKind::Walkthrough,
                id: walk.to_string(),
            });
    }
    let n = graph.nodes.iter().find(|n| n.id == name)?;
    (n.id != graph.system).then(|| seed_of(n))
}

/// Slices as `docair slice` prints them: pretty JSON.
pub fn to_json<T: Serialize + ?Sized>(slices: &T) -> String {
    serde_json::to_string_pretty(slices).expect("a slice is always serializable")
}

/***
 * Every slice the graph offers, in the order of `seeds`.
 *
 * @calls docair-core.slice.seeds "lists what slices start from"
 * @calls docair-core.slice.slice "works out each one"
 ***/
pub fn all(graph: &Graph) -> Vec<Slice> {
    let ix = Index::new(graph);
    seeds(graph)
        .iter()
        .filter_map(|s| slice_in(&ix, s))
        .collect()
}

/***
 * Work out one slice: from its seed, by fixed rules, to its ends (docs/SLICES.md §3).
 *
 * A datum follows its way as Follow does (SPEC §3.6); a part takes what it
 * calls and what calls it; a route also takes the way requests come in; a
 * scenario or walkthrough takes its steps. Lines that could reach a part
 * never carry a datum on inside other lines, nor a request in.
 ***/
pub fn slice(graph: &Graph, seed: &SliceSeed) -> Option<Slice> {
    slice_in(&Index::new(graph), seed)
}

fn slice_in(ix: &Index, seed: &SliceSeed) -> Option<Slice> {
    let graph = ix.graph;
    let mut b = match seed.kind {
        SeedKind::Data => data_slice(ix, &seed.id)?,
        SeedKind::Part => part_slice(ix, &seed.id, false)?,
        SeedKind::Route => part_slice(ix, &seed.id, true)?,
        SeedKind::Flow => {
            let f = graph.flows.iter().find(|f| f.name == seed.id)?;
            let steps = f
                .steps
                .iter()
                .map(|s| SliceStep {
                    nodes: s.nodes.clone(),
                    lines: s.path.clone(),
                    made: false,
                    way: None,
                    inside: false,
                    apart: false,
                    label: s.label.clone(),
                })
                .collect();
            walk_slice(ix, steps)
        }
        SeedKind::Walkthrough => {
            let w = graph.walkthroughs.iter().find(|w| w.id == seed.id)?;
            let steps = w
                .steps
                .iter()
                .map(|s| SliceStep {
                    nodes: s.nodes.clone(),
                    lines: s.path.clone(),
                    made: false,
                    way: None,
                    inside: false,
                    apart: false,
                    label: Some(s.title.clone()),
                })
                .collect();
            walk_slice(ix, steps)
        }
    };
    b.finish(ix, seed.clone());
    Some(b.slice)
}

/// A slice being put together.
struct Builder {
    slice: Slice,
    parts: HashSet<String>,
    lines: HashSet<usize>,
    ends: Vec<String>,
}

impl Builder {
    fn new() -> Self {
        Builder {
            slice: Slice {
                schema: SCHEMA.to_string(),
                seed: SliceSeed {
                    kind: SeedKind::Part,
                    id: String::new(),
                },
                ends: Vec::new(),
                parts: Vec::new(),
                steps: Vec::new(),
                lines: Vec::new(),
                boxes: Vec::new(),
                data: Vec::new(),
                covers: None,
            },
            parts: HashSet::new(),
            lines: HashSet::new(),
            ends: Vec::new(),
        }
    }

    fn part(&mut self, ix: &Index, id: &str) {
        if ix.nodes.contains_key(id) && !ix.is_data(id) && self.parts.insert(id.to_string()) {
            self.slice.parts.push(id.to_string());
        }
    }

    fn line(&mut self, ix: &Index, i: usize, how: Option<CarryHow>) {
        let Some(e) = ix.graph.edges.get(i) else {
            return;
        };
        self.part(ix, &e.from);
        self.part(ix, &e.to);
        if !self.lines.insert(i) {
            if how.is_some()
                && let Some(l) = self.slice.lines.iter_mut().find(|l| l.edge == i)
            {
                l.how = l.how.or(how);
            }
            return;
        }
        self.slice.lines.push(SliceLine {
            edge: i,
            from: e.from.clone(),
            to: e.to.clone(),
            kind: e.kind,
            reach: e.reach.filter(|r| *r != Reach::Certain),
            how,
        });
    }

    fn end(&mut self, id: &str) {
        if !self.ends.iter().any(|e| e == id) {
            self.ends.push(id.to_string());
        }
    }

    /// The seed, the ends with their stability, the boxes around the parts, the data on the lines.
    fn finish(&mut self, ix: &Index, seed: SliceSeed) {
        self.slice.seed = seed;
        self.slice.ends = self
            .ends
            .iter()
            .filter(|id| ix.nodes.contains_key(id.as_str()))
            .map(|id| {
                let stable = ix.stable(id);
                SliceEnd {
                    id: id.clone(),
                    stable,
                    within: if stable {
                        None
                    } else {
                        ix.stable_around(id).map(str::to_string)
                    },
                }
            })
            .collect();
        let mut boxes: Vec<&str> = Vec::new();
        for p in &self.slice.parts {
            for a in ix.ancestors(p) {
                if !boxes.contains(&a) {
                    boxes.push(a);
                }
            }
        }
        boxes.sort_unstable();
        let held: HashSet<&str> = boxes.iter().copied().collect();
        self.slice.boxes = boxes
            .iter()
            .map(|b| SliceBox {
                id: b.to_string(),
                more: ix
                    .children
                    .get(b)
                    .into_iter()
                    .flatten()
                    .filter(|k| !ix.is_data(k) && !self.parts.contains(**k) && !held.contains(**k))
                    .count(),
            })
            .collect();
        if self.slice.data.is_empty() {
            let mut data: Vec<String> = Vec::new();
            for l in &self.slice.lines {
                let e = ix.edge(l.edge);
                for d in e.carries.iter().chain(&e.carries_back) {
                    if !data.contains(d) {
                        data.push(d.clone());
                    }
                }
            }
            for e in &ix.graph.edges {
                if e.kind == EdgeKind::Create
                    && ix.is_data(&e.to)
                    && self.parts.contains(&e.from)
                    && !data.contains(&e.to)
                {
                    data.push(e.to.clone());
                }
            }
            self.slice.data = data;
        }
    }
}

/// One message of a step, in words: who does what to whom, why, and what travels (viewer.js `messageParts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepMessage {
    /// The kind of the line (`call`, `read`, `write`, …), or `return` for an answer.
    pub kind: String,
    /// Index into `Graph::edges`.
    pub edge: usize,
    /// Who, what, to whom: `["cart", "calls", "orders.create"]`.
    pub who: Vec<String>,
    /// What happens and why, in the line's words; empty when it says nothing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub why: String,
    /// What travels: `sends CreateOrder · gets Order back`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub carry: String,
    /// The word on the token that moves along the line.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
    /// What the receiving end has once the message arrived: `Order from orders.create`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has: Option<String>,
}

/// A part's name, with its box's when several parts share it: `ConversationService.ask` (viewer.js `nameOf`).
fn name_of(
    graph: &Graph,
    nodes: &HashMap<&str, &Node>,
    same: &HashMap<&str, usize>,
    id: &str,
) -> String {
    let Some(n) = nodes.get(id) else {
        return id.to_string();
    };
    if same.get(n.name.as_str()).copied().unwrap_or(0) < 2 {
        return n.name.clone();
    }
    match n.part_of.as_deref().and_then(|p| nodes.get(p)) {
        Some(parent) if parent.id != graph.system => format!("{}.{}", parent.name, n.name),
        _ => n.name.clone(),
    }
}

/// One message in words (viewer.js `messageParts`).
fn message_parts(
    graph: &Graph,
    nodes: &HashMap<&str, &Node>,
    same: &HashMap<&str, usize>,
    kind: &str,
    i: usize,
) -> StepMessage {
    let e = &graph.edges[i];
    let a = name_of(graph, nodes, same, &e.from);
    let b = name_of(graph, nodes, same, &e.to);
    let text = e.text.clone().unwrap_or_default();
    let data: Option<String> = e.payload.clone().or_else(|| {
        (e.kind != EdgeKind::Call)
            .then(|| e.label.clone())
            .flatten()
    });
    let d = data.clone().unwrap_or_default();
    let msg = |who: [&str; 3], why: String, carry: String, token: String, has: Option<String>| {
        StepMessage {
            kind: kind.to_string(),
            edge: i,
            who: who.iter().map(|s| s.to_string()).collect(),
            why,
            carry,
            token,
            has,
        }
    };
    match kind {
        "call" => {
            let carry = [
                e.payload.as_ref().map(|p| format!("sends {p}")),
                e.reply.as_ref().map(|r| format!("gets {r} back")),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            msg(
                [&a, "calls", &b],
                text,
                carry,
                e.payload.clone().unwrap_or_default(),
                None,
            )
        }
        "return" => msg(
            [&b, "answers", &a],
            e.reply
                .as_ref()
                .map(|r| format!("with {r}"))
                .unwrap_or_default(),
            String::new(),
            e.reply.clone().unwrap_or_default(),
            e.reply.as_ref().map(|r| format!("{r} from {b}")),
        ),
        "read" => msg(
            [&b, "reads", &a],
            text,
            if data.is_some() {
                format!("gets {d}")
            } else {
                String::new()
            },
            d.clone(),
            Some(format!(
                "{} from {a}",
                data.as_deref().unwrap_or("the data")
            )),
        ),
        "write" => msg(
            [&a, "writes to", &b],
            text,
            if data.is_some() {
                format!("stores {d}")
            } else {
                String::new()
            },
            d.clone(),
            Some(format!(
                "{} stored in {b}",
                data.as_deref().unwrap_or("its data")
            )),
        ),
        "emit" => msg(
            [&a, "publishes to", &b],
            text,
            if data.is_some() {
                format!("the event {d}")
            } else {
                String::new()
            },
            d.clone(),
            None,
        ),
        "consume" => msg(
            [&b, "takes from", &a],
            text,
            if data.is_some() {
                format!("the event {d}")
            } else {
                String::new()
            },
            d.clone(),
            Some(format!(
                "{} from {a}",
                data.as_deref().unwrap_or("the event")
            )),
        ),
        "create" => msg(
            [&a, "creates", &b],
            text,
            d.clone(),
            d.clone(),
            Some(format!("{b} exists")),
        ),
        "cover" => msg([&a, "tests", &b], text, String::new(), String::new(), None),
        "data" => msg(
            [&a, "hands over to", &b],
            text,
            d.clone(),
            d.clone(),
            data.as_ref().map(|x| format!("{x} at {b}")),
        ),
        other => {
            let verb = match other {
                "route" => "routes",
                "select" => "selects",
                _ => other,
            };
            msg([&a, verb, &b], text, d.clone(), d.clone(), None)
        }
    }
}

/// The kind of a line, as its messages name it.
fn kind_word(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Data => "data",
        EdgeKind::Call => "call",
        EdgeKind::Read => "read",
        EdgeKind::Write => "write",
        EdgeKind::Emit => "emit",
        EdgeKind::Consume => "consume",
        EdgeKind::Route => "route",
        EdgeKind::Select => "select",
        EdgeKind::Create => "create",
        EdgeKind::Cover => "cover",
        EdgeKind::Use => "use",
    }
}

/***
 * The messages of each step of a slice, in words, for its step cards (viewer.js `walkMessages`).
 *
 * A datum's walk has no call stack: each step is the one line it travels,
 * there or back. A walk of a scenario or walkthrough keeps one call stack
 * across its steps, so an answer comes back when it really does: a call
 * stays open while the part it reached still sends something later; a part
 * that makes another call has had its earlier calls answered; a call named
 * again is its answer on its way back; the end of the walk closes the calls
 * still open, innermost first. Each message says who does what to whom, why,
 * and what travels.
 ***/
pub fn messages(graph: &Graph, slice: &Slice) -> Vec<Vec<StepMessage>> {
    let nodes: HashMap<&str, &Node> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut same: HashMap<&str, usize> = HashMap::new();
    for n in &graph.nodes {
        *same.entry(n.name.as_str()).or_default() += 1;
    }
    let edges = &graph.edges;
    let words = |kind: &str, i: usize| message_parts(graph, &nodes, &same, kind, i);
    if slice.seed.kind == SeedKind::Data {
        return slice
            .steps
            .iter()
            .map(|st| {
                st.lines
                    .iter()
                    .filter(|&&i| i < edges.len())
                    .map(|&i| {
                        let kind = if st.way == Some(CarryWay::Back) {
                            "return"
                        } else {
                            kind_word(edges[i].kind)
                        };
                        words(kind, i)
                    })
                    .collect()
            })
            .collect();
    }
    let seq: Vec<(usize, usize)> = slice
        .steps
        .iter()
        .enumerate()
        .flat_map(|(k, st)| st.lines.iter().map(move |&i| (k, i)))
        .filter(|&(_, i)| i < edges.len())
        .collect();
    let mut per: Vec<Vec<StepMessage>> = vec![Vec::new(); slice.steps.len()];
    let mut stack: Vec<usize> = Vec::new();
    let answer = |per: &mut Vec<Vec<StepMessage>>, k: usize, c: usize| {
        if edges[c].reply.is_some() {
            per[k].push(words("return", c));
        }
    };
    // Whether the part a call reached still sends something later, before the same call comes up again.
    let busy = |c: usize, j: usize| {
        for &(_, x) in &seq[j..] {
            if x == c {
                return false;
            }
            if edges[x].from == edges[c].to {
                return true;
            }
        }
        false
    };
    for (j, &(k, i)) in seq.iter().enumerate() {
        let e = &edges[i];
        // The same call named again: its answer is on its way back.
        if let Some(again) = stack.iter().position(|&c| c == i) {
            while stack.len() > again {
                let c = stack.pop().expect("longer than again");
                answer(&mut per, k, c);
            }
            continue;
        }
        // Inside the callee of an open call: the calls above it have answered.
        let depth = match stack.iter().rposition(|&c| edges[c].to == e.from) {
            Some(callee) => callee + 1,
            // A caller further down makes another call: what it called before has answered.
            None => stack
                .iter()
                .position(|&c| edges[c].from == e.from)
                .unwrap_or(0),
        };
        while stack.len() > depth && !busy(*stack.last().expect("longer than depth"), j) {
            let c = stack.pop().expect("longer than depth");
            answer(&mut per, k, c);
        }
        per[k].push(words(kind_word(e.kind), i));
        if e.kind == EdgeKind::Call {
            stack.push(i);
        }
    }
    if let Some(last) = slice.steps.len().checked_sub(1) {
        while let Some(c) = stack.pop() {
            answer(&mut per, last, c);
        }
    }
    per
}

/// A move along a line: there along what it sends, or back in a call's answer.
#[derive(Debug, Clone, Copy)]
struct DataMove<'g> {
    i: usize,
    back: bool,
    to: &'g str,
}

/// A datum's way, as Follow walks it (SPEC §3.6), from where it is made.
fn data_slice<'g>(ix: &Index<'g>, id: &str) -> Option<Builder> {
    if !ix.is_data(id) {
        return None;
    }
    let edges: &'g [Edge] = &ix.graph.edges;
    let there = |i: usize| edges[i].carries.iter().any(|d| d == id);
    let back = |i: usize| edges[i].carries_back.iter().any(|d| d == id);
    let makers: Vec<usize> = (0..edges.len())
        .filter(|&i| {
            edges[i].to == id && edges[i].kind == EdgeKind::Create && !ix.is_data(&edges[i].from)
        })
        .collect();
    let mut out: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut into: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut lines: Vec<usize> = Vec::new();
    for (i, e) in edges.iter().enumerate() {
        if ix.is_data(&e.from)
            || ix.is_data(&e.to)
            || matches!(e.kind, EdgeKind::Cover | EdgeKind::Create)
            || !e.kind.drawn()
        {
            continue;
        }
        out.entry(e.from.as_str()).or_default().push(i);
        into.entry(e.to.as_str()).or_default().push(i);
        if there(i) || back(i) {
            lines.push(i);
        }
    }
    let on = |i: usize| DataMove {
        i,
        back: false,
        to: edges[i].to.as_str(),
    };
    let answer = |i: usize| DataMove {
        i,
        back: true,
        to: edges[i].from.as_str(),
    };
    // From a part: on along a line that names it, back along a call whose answer names it.
    let named = |n: &str| -> Vec<DataMove<'g>> {
        let mut v: Vec<DataMove<'g>> = out
            .get(n)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&i| there(i))
            .map(on)
            .collect();
        v.extend(
            into.get(n)
                .into_iter()
                .flatten()
                .copied()
                .filter(|&i| back(i))
                .map(answer),
        );
        v
    };
    // Inside other lines: on along any line, back along any call; never along a line that only could go there.
    let sure = |i: usize| edges[i].reach != Some(Reach::Could);
    let hidden = |n: &str| -> Vec<DataMove<'g>> {
        let mut v: Vec<DataMove<'g>> = out
            .get(n)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&i| sure(i))
            .map(on)
            .collect();
        v.extend(
            into.get(n)
                .into_iter()
                .flatten()
                .copied()
                .filter(|&i| edges[i].kind == EdgeKind::Call && sure(i))
                .map(answer),
        );
        v
    };

    #[derive(Clone, Copy, PartialEq)]
    enum Got {
        Made,
        Back,
        There,
    }
    struct Taken {
        m: usize,
        back: bool,
        inside: bool,
        apart: bool,
    }
    let mut order: Vec<Taken> = Vec::new();
    // A call that names it both ways goes there and later back, never back and then there.
    let mut went: HashSet<usize> = HashSet::new();
    let mut came: HashSet<usize> = HashSet::new();
    let mut held: Vec<&str> = Vec::new();
    let mut held_set: HashSet<&str> = HashSet::new();
    let mut got: HashMap<&str, Got> = HashMap::new();
    for &i in &makers {
        let f = edges[i].from.as_str();
        if held_set.insert(f) {
            held.push(f);
        }
        got.insert(f, Got::Made);
    }
    let free = |m: &DataMove, went: &HashSet<usize>, came: &HashSet<usize>| {
        !came.contains(&m.i) && (m.back || !went.contains(&m.i))
    };
    let seen = |i: usize, went: &HashSet<usize>, came: &HashSet<usize>| {
        went.contains(&i) || came.contains(&i)
    };

    macro_rules! take {
        ($m:expr, $inside:expr, $apart:expr) => {{
            let m: DataMove = $m;
            if m.back {
                came.insert(m.i);
            } else {
                went.insert(m.i);
            }
            order.push(Taken {
                m: m.i,
                back: m.back,
                inside: $inside,
                apart: $apart,
            });
            if held_set.contains(m.to) {
                false
            } else {
                held_set.insert(m.to);
                held.push(m.to);
                got.insert(m.to, if m.back { Got::Back } else { Got::There });
                true
            }
        }};
    }

    let mut frontier: Vec<&str> = held.clone();
    for _guard in 0..=lines.len() {
        // Along the lines that name it, breadth first.
        while !frontier.is_empty() {
            let mut next = Vec::new();
            for f in std::mem::take(&mut frontier) {
                for m in named(f) {
                    if free(&m, &went, &came) && take!(m, false, false) {
                        next.push(m.to);
                    }
                }
            }
            frontier = next;
        }
        // Inside other lines, the shortest way to a part that sends it on to one that does not have it yet.
        let mut prev: HashMap<&str, DataMove> = HashMap::new();
        let mut end: Option<&str> = None;
        for shape in ["on", "back", "back-on"] {
            prev.clear();
            let mut queue: VecDeque<(&str, usize, bool)> = held
                .iter()
                .copied()
                .filter(|n| shape == "on" || got.get(n) != Some(&Got::There))
                .map(|n| (n, 0, false))
                .collect();
            let mut visited: HashSet<&str> = held_set.clone();
            while end.is_none() {
                let Some((n, depth, turned)) = queue.pop_front() else {
                    break;
                };
                if depth >= INSIDE_HOPS {
                    continue;
                }
                for m in hidden(n) {
                    let fits = match shape {
                        "on" => !m.back,
                        "back" => m.back,
                        _ => {
                            if m.back {
                                !turned
                            } else {
                                depth > 0
                            }
                        }
                    };
                    if !fits || seen(m.i, &went, &came) || visited.contains(m.to) {
                        continue;
                    }
                    visited.insert(m.to);
                    prev.insert(m.to, m);
                    if (shape != "back-on" || !m.back)
                        && named(m.to)
                            .iter()
                            .any(|x| x.i != m.i && free(x, &went, &came) && !visited.contains(x.to))
                    {
                        end = Some(m.to);
                        break;
                    }
                    queue.push_back((m.to, depth + 1, turned || !m.back));
                }
            }
            if end.is_some() {
                break;
            }
        }
        if let Some(end) = end {
            let mut hops: Vec<DataMove> = Vec::new();
            let mut n = end;
            while let Some(&m) = prev.get(n) {
                hops.insert(0, m);
                n = if m.back {
                    edges[m.i].to.as_str()
                } else {
                    edges[m.i].from.as_str()
                };
            }
            for m in hops {
                take!(m, true, false);
            }
            frontier = vec![end];
            continue;
        }
        // A line the way cannot reach: a stretch of its own, from a line that no other such line leads to.
        let rest: Vec<usize> = lines
            .iter()
            .copied()
            .filter(|&i| !seen(i, &went, &came))
            .collect();
        if rest.is_empty() {
            break;
        }
        let mv = |i: usize| if there(i) { on(i) } else { answer(i) };
        let ends: HashSet<&str> = rest.iter().map(|&i| mv(i).to).collect();
        let start_of = |i: usize| {
            if there(i) {
                edges[i].from.as_str()
            } else {
                edges[i].to.as_str()
            }
        };
        let i = rest
            .iter()
            .copied()
            .find(|&j| !ends.contains(start_of(j)))
            .unwrap_or(rest[0]);
        let start = start_of(i);
        if !held_set.contains(start) {
            got.insert(start, if there(i) { Got::There } else { Got::Made });
            held_set.insert(start);
            held.push(start);
        }
        let first = order.is_empty() && makers.is_empty();
        take!(mv(i), false, !first);
        frontier = vec![start, mv(i).to];
    }

    let mut b = Builder::new();
    let how = |i: usize| -> Option<CarryHow> {
        (there(i) || back(i)).then(|| {
            if edges[i].evidence.is_some() {
                CarryHow::Typed
            } else {
                CarryHow::Named
            }
        })
    };
    for &i in &makers {
        b.part(ix, &edges[i].from);
        b.end(&edges[i].from);
        b.slice.steps.push(SliceStep {
            nodes: vec![edges[i].from.clone()],
            lines: Vec::new(),
            made: true,
            way: None,
            inside: false,
            apart: false,
            label: None,
        });
    }
    // Where it goes on from: a part that passes it on is no end of its way.
    let mut passes: HashSet<&str> = HashSet::new();
    for t in &order {
        let e = &edges[t.m];
        passes.insert(if t.back {
            e.to.as_str()
        } else {
            e.from.as_str()
        });
    }
    let mut reached = 0;
    let mut apart = 0;
    let mut counted: HashSet<usize> = HashSet::new();
    for t in &order {
        let e = &edges[t.m];
        b.line(ix, t.m, how(t.m));
        let to = if t.back { &e.from } else { &e.to };
        if t.apart {
            apart += 1;
        }
        if apart == 0 && how(t.m).is_some() && counted.insert(t.m) {
            reached += 1;
        }
        if !passes.contains(to.as_str()) {
            b.end(to);
        }
        b.slice.steps.push(SliceStep {
            nodes: vec![to.clone()],
            lines: vec![t.m],
            made: false,
            way: Some(if t.back {
                CarryWay::Back
            } else {
                CarryWay::There
            }),
            inside: t.inside,
            apart: t.apart,
            label: None,
        });
    }
    b.slice.data = vec![id.to_string()];
    b.slice.covers = Some(DataCovers {
        lines: lines.len(),
        reached,
        apart,
    });
    Some(b)
}

/// A part's lines across its border, one hop each way; a route also takes the way its requests come in.
fn part_slice(ix: &Index, id: &str, route: bool) -> Option<Builder> {
    let n = ix.nodes.get(id)?;
    if n.kind == "data" || n.id == ix.graph.system {
        return None;
    }
    let inside = ix.within(id);
    let mut b = Builder::new();
    b.part(ix, id);
    let edges = &ix.graph.edges;
    for (i, e) in edges.iter().enumerate() {
        if ix.is_data(&e.from) || ix.is_data(&e.to) || !e.kind.drawn() {
            continue;
        }
        let (a, z) = (
            inside.contains(e.from.as_str()),
            inside.contains(e.to.as_str()),
        );
        if a != z {
            b.line(ix, i, None);
            // A test that covers the part is in the slice, but no end of it.
            let outside = if a { &e.to } else { &e.from };
            if ix
                .nodes
                .get(outside.as_str())
                .is_some_and(|n| n.kind != "test")
            {
                b.end(outside);
            }
        }
    }
    if route {
        // The way in: back along calls, routes and selections, as far as a request comes from.
        let request = |e: &Edge| {
            matches!(e.kind, EdgeKind::Call | EdgeKind::Route | EdgeKind::Select)
                && e.reach != Some(Reach::Could)
        };
        let mut seen: HashSet<&str> = HashSet::from([id]);
        let mut queue: VecDeque<(&str, usize)> = VecDeque::from([(id, 0)]);
        let mut starts: Vec<&str> = Vec::new();
        while let Some((cur, depth)) = queue.pop_front() {
            let callers: Vec<usize> = (0..edges.len())
                .filter(|&i| {
                    edges[i].to == cur && request(&edges[i]) && !ix.is_data(&edges[i].from)
                })
                .collect();
            if callers.is_empty() && cur != id {
                starts.push(cur);
            }
            if depth >= WAY_IN_HOPS {
                continue;
            }
            for i in callers {
                b.line(ix, i, None);
                let from = edges[i].from.as_str();
                if seen.insert(from) {
                    queue.push_back((from, depth + 1));
                }
            }
        }
        // The ends of a route are where its requests start and what it reaches; its callers on the way are not.
        b.ends.retain(|e| !seen.contains(e.as_str()));
        for s in starts.into_iter().rev() {
            b.ends.insert(0, s.to_string());
        }
    }
    Some(b)
}

/// A scenario's or a walkthrough's steps; its ends are its first and last step.
fn walk_slice(ix: &Index, steps: Vec<SliceStep>) -> Builder {
    let mut b = Builder::new();
    for s in &steps {
        for n in &s.nodes {
            b.part(ix, n);
        }
        for &i in &s.lines {
            b.line(ix, i, None);
        }
    }
    if let Some(first) = steps.iter().find_map(|s| s.nodes.first()) {
        b.end(first);
    }
    if let Some(last) = steps.iter().rev().find_map(|s| s.nodes.first()) {
        b.end(last);
    }
    b.slice.steps = steps;
    b
}

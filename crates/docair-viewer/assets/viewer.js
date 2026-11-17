'use strict';

// Shape and glyph per kind (SPEC §3.2). Colours come from CSS variables.
const KINDS = {
  system: { glyph: '◎', shape: 'round-rectangle' },
  service: { glyph: '◆', shape: 'round-rectangle' },
  module: { glyph: '▤', shape: 'round-rectangle' },
  // An OpenAPI or AsyncAPI description: a page with a folded corner.
  api: { glyph: '⎘', shape: 'polygon', points: '-1 -1 0.78 -1 1 -0.45 1 1 -1 1' },
  component: { glyph: '▣', shape: 'rectangle' },
  function: { glyph: 'ƒ', shape: 'round-rectangle' },
  endpoint: { glyph: '⊙', shape: 'round-tag' },
  store: { glyph: '◯', shape: 'barrel' },
  topic: { glyph: '▱', shape: 'rhomboid' },
  external: { glyph: '☁', shape: 'rectangle' },
  job: { glyph: '⏱', shape: 'octagon' },
  workload: { glyph: '⬡', shape: 'hexagon' },
  config: { glyph: '⚙', shape: 'tag' },
  host: { glyph: '◫', shape: 'rectangle' },
  cluster: { glyph: '⎈', shape: 'round-rectangle' },
  namespace: { glyph: '⬚', shape: 'round-rectangle' },
  network: { glyph: '⇌', shape: 'ellipse' },
  gateway: { glyph: '⇶', shape: 'concave-hexagon' },
  container: { glyph: '▦', shape: 'round-rectangle' },
  release: { glyph: '⎔', shape: 'round-rectangle' },
  test: { glyph: '✓', shape: 'round-rectangle' },
  data: { glyph: '◇', shape: 'round-rectangle' },
  unresolved: { glyph: '?', shape: 'rectangle' },
  unknown: { glyph: '·', shape: 'rectangle' },
};
const EDGE_KINDS = ['data', 'call', 'read', 'write', 'emit', 'consume', 'route', 'select', 'create', 'cover'];
const EDGE_LINE = { emit: 'dotted', consume: 'dotted', route: 'dashed', select: 'dashed', create: 'dashed', cover: 'dotted' };
// What a line does when nothing else says it (SPEC §4.2): every line has words.
const VERB = { call: 'calls', read: 'reads', write: 'writes', emit: 'emits', consume: 'consumes', data: 'passes data', route: 'routes', select: 'selects', create: 'creates', cover: 'tests' };
// Test results (SPEC §12.3): icon and class per status.
const TEST_ICON = { passed: '✓', failed: '✗', flaky: '~', skipped: '–' };

const READABLE_ZOOM = 0.65;
/** The most characters a line's label has on the chart. */
const LABEL_MAX = 48;
// A walkthrough never shows a step smaller than this: labels stay readable (SPEC §10.4).
const TOUR_MIN_ZOOM = 0.8;
const TOUR_MAX_ZOOM = 1.3;
const MAX_FIT_ZOOM = 1.25;
const LABEL_FONT = 11;
const DIRECTIONS = ['auto', 'RIGHT', 'DOWN'];
/** ELK's options for what is inside a box (top-down gives it TOPDOWN.spacing times this room). */
const ELK_OPTIONS = {
  'elk.layered.spacing.nodeNodeBetweenLayers': '30',
  'elk.layered.spacing.edgeNodeBetweenLayers': '12',
  'elk.spacing.nodeNode': '14',
  'elk.spacing.edgeNode': '10',
  'elk.spacing.edgeEdge': '6',
  'elk.spacing.componentComponent': '30',
  'elk.layered.nodePlacement.strategy': 'NETWORK_SIMPLEX',
  'elk.layered.considerModelOrder.strategy': 'NODES_AND_EDGES',
};

/** The kinds of lines that say who depends on whom: the way things happen. */
const AHEAD = new Set(['call', 'route', 'select', 'write', 'create', 'emit', 'consume']);

const S = {
  graph: null,
  html: {},
  byId: new Map(),
  children: new Map(),
  collapsed: new Set(),
  edgeOf: new Map(),
  selected: null,
  cy: null,
  flow: null,
  tours: [],
  step: -1,
  beforeTour: null,
  tourGroups: null,
  tourLevel: null,
  layer: 'overview',
  layers: [],
  opened: new Set(),
  folded: new Set(),
  view: null,
  viewCache: new Map(),
  cls: new Map(),
  outOf: new Map(),
  bridgeOf: new Map(),
  hidden: new Set(),
  showTests: true,
  terms: [],
  termById: new Map(),
  termWords: new Map(),
  termCase: new Map(),
  termRe: null,
  previewTerms: [],
  following: null,
  dataWords: null,
  needed: new Set(),
  playing: false,
  layoutRun: 0,
  tokenRun: 0,
  direction: 'auto',
  dirUsed: 'RIGHT',
  hover: null,
  focus: null,
  font: '',
};

const $ = (sel) => document.querySelector(sel);
const kindOf = (k) => (KINDS[k] ? k : 'unknown');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const cssVar = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

function el(tag, attrs, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k === 'class') e.className = v;
    else if (k === 'html') e.innerHTML = v; // only Markdown rendered and escaped by docair-viewer
    else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else e.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid == null || kid === false) continue;
    e.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
  return e;
}

/***
 * Start the viewer: read the graph embedded in the page and build every panel.
 *
 * @id      docair-viewer.boot
 * @part-of docair-viewer.page
 * @flow    scan 15 "the viewer reads the embedded graph"
 * @flow    ask 9 "the next page reads the graph with the walkthrough"
 * @flow    view 1 "read the embedded graph"
 * @reads   html-file "provides the embedded graph"
 * @calls   docair-viewer.render "draws the first graph"
 * @calls   docair-viewer.play-step "shows the first step"
 ***/
function boot() {
  S.graph = JSON.parse($('#docair-graph').textContent);
  S.html = JSON.parse($('#docair-html').textContent);
  S.font = cssVar('--font');
  for (const n of S.graph.nodes) S.byId.set(n.id, n);
  for (const n of S.graph.nodes) {
    if (n.id === S.graph.system || !n.part_of) continue;
    if (!S.children.has(n.part_of)) S.children.set(n.part_of, []);
    S.children.get(n.part_of).push(n.id);
  }

  S.graph.edges.forEach((e, i) => {
    if (!S.outOf.has(e.from)) S.outOf.set(e.from, []);
    S.outOf.get(e.from).push(i);
  });
  classify();

  $('#system-name').textContent = S.graph.system;
  const shown = S.graph.nodes.length - 1;
  S.graph.walkthroughs = S.graph.walkthroughs || [];
  loadView();
  buildTerms();
  buildTours();
  $('#stats').textContent = `${shown} nodes · ${S.graph.edges.length} edges · ${S.graph.flows.length} flows`;
  document.title = `docair · ${S.graph.system}`;

  S.cy = cytoscape({
    container: $('#cy'),
    style: makeStyle(),
    minZoom: 0.1,
    maxZoom: 3,
    boxSelectionEnabled: false,
  });
  wireGraph();
  wireHeader();
  wireSemanticZoom();
  renderTree();
  renderDiagButton();
  renderTestsButton();
  renderGlossaryButton();
  renderDataButton();
  renderHiddenBar();
  renderFlowBar();
  renderLegend();
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => S.cy.style(makeStyle()));

  wireMessages();
  wireTerms();
  return firstLayer().then(render).then(() => {
    const id = decodeURIComponent(location.hash.slice(1));
    if (id && S.byId.has(id)) select(id, true);
    else renderDetails(null);
    document.body.dataset.ready = 'true';
  });
}

function makeStyle() {
  const v = cssVar;
  return [
    {
      selector: 'node',
      style: {
        label: 'data(label)',
        'font-family': S.font,
        'font-size': 12,
        color: v('--text'),
        'text-valign': 'center',
        'text-halign': 'center',
        width: 'data(w)',
        height: 'data(h)',
        'border-width': 1.5,
      },
    },
    ...Object.keys(KINDS).map((k) => ({
      selector: `node[kind = "${k}"]`,
      style: { shape: KINDS[k].shape, ...(KINDS[k].points ? { 'shape-polygon-points': KINDS[k].points } : {}), 'background-color': v(`--k-${k}-bg`), 'border-color': v(`--k-${k}-line`) },
    })),
    { selector: 'node[kind = "external"], node[kind = "unresolved"]', style: { 'border-style': 'dashed' } },
    { selector: 'node.t-passed', style: { 'border-color': v('--t-passed'), 'background-color': v('--t-passed-bg'), 'border-width': 2 } },
    { selector: 'node.t-failed', style: { 'border-color': v('--t-failed'), 'background-color': v('--t-failed-bg'), 'border-width': 2.5 } },
    { selector: 'node.t-flaky', style: { 'border-color': v('--t-flaky'), 'background-color': v('--t-flaky-bg'), 'border-width': 2 } },
    { selector: 'node.t-none, node.t-skipped', style: { 'border-color': v('--t-none'), 'border-style': 'dashed' } },
    { selector: 'node.collapsed', style: { 'border-width': 3, 'border-style': 'double' } },
    {
      selector: ':parent',
      style: {
        shape: 'round-rectangle',
        'text-valign': 'top',
        'text-halign': 'center',
        // The box is the layout's (fitBoxes): its name sits in the band kept for it at the top.
        'text-margin-y': 18,
        'font-size': 11.5,
        'font-weight': 600,
        color: v('--muted'),
        padding: 0,
        'background-opacity': 0.6,
      },
    },
    { selector: ':parent[kind = "service"]', style: { 'border-width': 2 } },
    { selector: ':parent[kind = "cluster"], :parent[kind = "host"]', style: { 'border-width': 2.5 } },
    { selector: ':parent[kind = "namespace"], :parent[kind = "network"]', style: { 'border-style': 'dashed', 'border-width': 1.5 } },
    { selector: ':parent[kind = "container"]', style: { 'border-width': 2, 'border-style': 'double' } },
    { selector: 'node:selected', style: { 'border-width': 3, 'border-color': v('--accent'), 'overlay-opacity': 0 } },
    {
      selector: 'edge',
      style: {
        width: 1.4,
        'curve-style': 'bezier',
        'target-arrow-shape': 'triangle',
        'arrow-scale': 0.9,
        label: 'data(label)',
        'font-family': S.font,
        'font-size': 10.5,
        color: v('--muted'),
        'text-background-color': v('--canvas'),
        'text-background-opacity': 0.9,
        'text-background-padding': 2,
      },
    },
    ...EDGE_KINDS.map((k) => ({
      selector: `edge[kind = "${k}"]`,
      style: { 'line-color': v(`--e-${k}`), 'target-arrow-color': v(`--e-${k}`), 'line-style': EDGE_LINE[k] || 'solid' },
    })),
    {
      selector: 'edge[?routed]',
      style: {
        'curve-style': 'round-segments',
        'segment-radius': 6,
        'segment-weights': 'data(segW)',
        'segment-distances': 'data(segD)',
        'edge-distances': 'node-position',
        'source-endpoint': 'data(srcEnd)',
        'target-endpoint': 'data(tgtEnd)',
      },
    },
    { selector: 'edge[kind = "data"]', style: { width: 2 } },
    {
      selector: 'edge.bundle',
      style: { width: 'mapData(count, 1, 8, 1.6, 4.5)' },
    },
    { selector: 'edge[?dangling]', style: { 'line-style': 'dashed', 'line-color': v('--error'), 'target-arrow-color': v('--error') } },
    { selector: 'edge.hl', style: { width: 3, 'z-index': 9 } },
    { selector: '.dim, .flow-dim', style: { opacity: 0.16 } },
    { selector: 'node.hit', style: { 'border-color': v('--accent'), 'border-width': 3 } },
    { selector: 'node.flow-done', style: { 'border-color': v('--flow'), 'border-width': 2.5 } },
    { selector: 'node.flow-current', style: { 'border-color': v('--flow'), 'border-width': 4, 'background-color': v('--flow-soft') } },
    { selector: 'edge.flow-trail', style: { 'line-color': v('--flow'), 'target-arrow-color': v('--flow'), width: 2.2, opacity: 0.6 } },
    {
      selector: 'edge.flow-active',
      style: { 'line-color': v('--flow'), 'target-arrow-color': v('--flow'), width: 3.4, opacity: 1, 'z-index': 10, 'line-style': 'dashed', 'line-dash-pattern': [8, 5] },
    },
    {
      selector: 'node.elabel',
      style: {
        shape: 'round-rectangle',
        'background-color': v('--canvas'),
        'background-opacity': 0.92,
        'border-width': 0,
        'font-size': LABEL_FONT,
        color: v('--muted'),
        'text-wrap': 'wrap',
        'text-max-width': 1000,
        'line-height': 1.2,
        'z-index': 20,
        events: 'no',
      },
    },
    { selector: 'node.bundle-label', style: { color: v('--text'), 'font-weight': 600 } },
    // The words of a line that follows a part being moved: back once the part is put down.
    { selector: 'node.elabel.away', style: { display: 'none' } },
    // Top-down: the words inside a scaled box are scaled with it.
    { selector: 'node[fs]', style: { 'font-size': 'data(fs)' } },
    { selector: ':parent[tm]', style: { 'text-margin-y': 'data(tm)' } },
    {
      selector: 'node.token',
      style: {
        width: 13, height: 13, shape: 'ellipse', 'background-color': v('--flow'), 'border-width': 2, 'border-color': v('--canvas'), 'z-index': 99, events: 'no',
        // The token says what it carries, in a small box above it.
        label: 'data(label)', 'font-size': 10, 'font-weight': 600, color: v('--text'), 'text-valign': 'top', 'text-margin-y': -5,
        'text-background-color': v('--panel'), 'text-background-opacity': 0.95, 'text-background-padding': 2, 'text-background-shape': 'round-rectangle',
        'text-border-width': 1, 'text-border-color': v('--flow'), 'text-border-opacity': 1, 'text-max-width': 180, 'text-wrap': 'ellipsis',
      },
    },
    // Following a datum (SPEC §3.6): the lines that carry it.
    { selector: 'edge.carry', style: { width: 3.5, 'line-color': v('--data'), 'target-arrow-color': v('--data'), 'z-index': 60 } },
    { selector: '.carry-dim', style: { opacity: 0.18 } },
    // An answer goes back along its call: a hollow token.
    { selector: 'node.token.reply', style: { 'background-color': v('--canvas'), 'border-color': v('--flow'), 'border-width': 3 } },
  ];
}

/** The node that stands for `id` on screen: itself, or its outermost closed box; `id` itself when the layer leaves it out. */
function rep(id) {
  return drawnRep(id) || id;
}

/** Whether `inner` lies somewhere inside the group `outer`. */
function isInside(outer, inner) {
  for (let cur = S.byId.get(inner)?.part_of; cur && cur !== S.graph.system; cur = S.byId.get(cur)?.part_of) {
    if (cur === outer) return true;
  }
  return false;
}

function descendants(id) {
  return (S.children.get(id) || []).reduce((n, c) => n + 1 + descendants(c), 0);
}

const measure = (() => {
  const ctx = document.createElement('canvas').getContext('2d');
  return (text, size = 12) => {
    ctx.font = `${size}px ${S.font}`;
    return ctx.measureText(text).width;
  };
})();

function nodeSize(kind, label) {
  const extra = { topic: 26, endpoint: 18, workload: 22, config: 20, job: 14, store: 8, gateway: 30, network: 22, api: 14 }[kind] || 0;
  return { w: Math.max(56, Math.ceil(measure(label)) + 26 + extra), h: kind === 'store' ? 40 : 30 };
}

/***
 * Rebuild the visible graph from the collapse state, then lay it out.
 *
 * A collapsed group hides its children. Their edges are re-attached to the
 * group and bundled, so no connection disappears from the picture.
 *
 * @id      docair-viewer.render
 * @part-of docair-viewer.page
 * @flow    view 2 "collapse state becomes visible elements"
 * @calls   docair-viewer.top-down "places the parts box by box"
 * @calls   docair-viewer.grid-lines "draws the lines"
 * @calls   docair-viewer.show-details "highlights a node and shows its details"
 ***/
function render() {
  S.lastRender = renderNow();
  return S.lastRender;
}

/** One layout run; a newer run makes an older one stop before it draws. */
async function renderNow() {
  const run = ++S.layoutRun;
  const { nodes, edges } = visibleElements();
  let pos = new Map();
  let routes = new Map();
  let labels = new Map();
  try {
    const best = await layoutFor(nodes, edges);
    ({ pos, routes, labels } = best);
    S.placed = { pos, scale: best.scale };
    S.dirUsed = best.direction;
    updateDirButton();
    // Top-down: what is inside a box is drawn smaller, its words too.
    if (best.scale) {
      for (const n of nodes) {
        const sc = best.scale.get(n.data.id) ?? 1;
        const p = pos.get(n.data.id);
        if (p && !n.data.compound) Object.assign(n.data, { w: p.w, h: p.h });
        Object.assign(n.data, { sc, fs: (n.data.compound ? 11.5 : 12) * sc, tm: 18 * sc });
      }
      S.cy.maxZoom(Math.min(20, 3 / Math.min(1, ...best.scale.values())));
    } else S.cy.maxZoom(3);
  } catch (err) {
    console.error('docair: layout failed', err);
  }
  if (run !== S.layoutRun) return;
  for (const n of nodes) {
    const p = pos.get(n.data.id);
    if (p && !n.data.compound) n.position = { x: p.x, y: p.y };
  }
  for (const e of edges) {
    const route = routes.get(e.data.id);
    const a = pos.get(e.data.source);
    const b = pos.get(e.data.target);
    if (!route || !a || !b) continue;
    const pts = route.map((p) => ({ x: p.x, y: p.y }));
    Object.assign(e.data, { routed: true, route: pts }, routeStyle(a, b, pts));
  }
  const routedEdges = edges.filter((e) => e.data.routed);
  // Labels are drawn as small nodes where placeLabels put them, not by Cytoscape at the edge's middle.
  const labelNodes = [];
  for (const e of edges) {
    const l = labels.get(e.data.id);
    if (!l || !e.data.label) continue;
    e.data.labelText = wrapLines(e.data.label).join('\n');
    e.data.label = '';
    labelNodes.push({
      group: 'nodes',
      data: { id: `${e.data.id}::label`, label: e.data.labelText, w: l.w, h: l.h, edge: e.data.id, ...(e.data.fs ? { fs: e.data.fs } : {}) },
      classes: `elabel${e.classes === 'bundle' ? ' bundle-label' : ''}`,
      position: { x: l.x, y: l.y },
    });
  }
  const cy = S.cy;
  S.tokenRun++;
  // A part being moved: the picture it was moved in is gone.
  if (S.drag) cancelAnimationFrame(S.drag.frame);
  S.drag = null;
  cy.batch(() => {
    cy.elements().remove();
    cy.add(nodes);
    cy.add(edges);
    cy.add(labelNodes);
  });
  fitBoxes(pos);
  // Segments are relative to the endpoints' centres. A group's centre in
  // Cytoscape (its children plus padding) is not the layout's, so recompute from
  // the positions Cytoscape actually uses.
  const centre = (n) => {
    if (!n.isParent()) return n.position();
    const bb = n.boundingBox({ includeLabels: false, includeOverlays: false });
    return { x: (bb.x1 + bb.x2) / 2, y: (bb.y1 + bb.y2) / 2 };
  };
  // Read the centres outside a batch: inside one, Cytoscape's bounds are stale.
  const updates = routedEdges
    .map((e) => cy.getElementById(e.data.id))
    .filter((edge) => edge.source().isParent() || edge.target().isParent())
    .map((edge) => [edge, routeStyle(centre(edge.source()), centre(edge.target()), edge.data('route'))]);
  cy.batch(() => updates.forEach(([edge, seg]) => edge.data(seg)));
  if (!pos.size) cy.layout({ name: 'breadthfirst', directed: true }).run();
  // A walkthrough moves the camera itself, from where the reader was looking.
  if (!S.keepView) fitView();
  if (S.selected) highlight(S.selected);
  applyFlowClasses();
  applyFollow();
  applySearch();
  renderLayers();
  renderCrumbs();
}

function visibleElements() {
  const sys = S.graph.system;
  const v = S.view;
  const nodes = [];
  for (const n of S.graph.nodes) {
    if (n.id === sys || isData(n.id) || !v.drawn.has(n.id) || hiddenNode(n.id) || rep(n.id) !== n.id) continue;
    const real = (S.children.get(n.id) || []).filter((c) => !hiddenNode(c) && !isData(c));
    const inside = (v.kids.get(n.id) || []).filter((c) => !hiddenNode(c));
    const collapsed = S.collapsed.has(n.id) && real.length > 0;
    const compound = !collapsed && inside.length > 0;
    // Groups say what they are on the canvas too: `docair release`, `kind cluster`.
    const isGroup = real.length > 0;
    let name = isGroup ? groupLabel(n.id) : n.name;
    if (n.kind === 'test' && !isGroup) name = `${TEST_ICON[n.test?.status] || '○'} ${name}`;
    const label = collapsed || (isGroup && !compound) ? `${name} (${descendants(n.id)})` : name;
    const size = compound ? { w: 10, h: 10 } : nodeSize(kindOf(n.kind), label);
    nodes.push({
      group: 'nodes',
      data: {
        id: n.id,
        label,
        kind: kindOf(n.kind),
        parent: v.parent.get(n.id) || undefined,
        compound,
        minW: Math.ceil(measure(n.name)) + 34,
        w: size.w,
        h: size.h,
      },
      classes: [collapsed || (isGroup && !compound) ? 'collapsed' : '', n.kind === 'test' ? `t-${n.test?.status || 'none'}` : ''].join(' ').trim(),
    });
  }

  const merged = new Map();
  S.edgeOf = new Map();
  S.bridgeOf = new Map();
  const add = (a, b, e, i, end) => {
    if (a === b || a === sys || b === sys || isInside(a, b) || isInside(b, a)) return;
    const key = `${a}\u0000${b}`;
    let m = merged.get(key);
    if (!m) {
      m = { id: `e${merged.size}`, source: a, target: b, kind: e.kind, labels: [], members: [], via: [], says: new Map(), dangling: false };
      merged.set(key, m);
    }
    if (!m.members.includes(i)) m.members.push(i);
    // A joined line says the request it carries: the hop that names a route, else its first.
    const said = (end && end.hops.map((j) => S.graph.edges[j]).find(routePath)) || e;
    if (!m.says.has(i)) m.says.set(i, said);
    const text = edgeText(said);
    if (text && !m.labels.includes(text)) m.labels.push(text);
    if ([e.from, e.to].some((x) => S.byId.get(x)?.kind === 'unresolved')) m.dangling = true;
    if (end) {
      end.via.forEach((id) => m.via.includes(id) || m.via.push(id));
      if (!S.bridgeOf.has(i)) S.bridgeOf.set(i, end);
    }
    if (!S.edgeOf.has(i)) S.edgeOf.set(i, m.id);
  };
  S.graph.edges.forEach((e, i) => {
    if (hiddenNode(e.from) || hiddenNode(e.to) || isData(e.from) || isData(e.to)) return;
    const a = drawnRep(e.from);
    if (!a) return;
    const b = drawnRep(e.to);
    if (b) return add(a, b, e, i, null);
    // The line goes on through parts this layer leaves out: one line to where it arrives.
    for (const end of bridgesFrom(i, v, S.collapsed)) add(a, end.rep, e, i, end);
  });
  const edges = [...merged.values()].map((m) => {
    // An edge between collapsed groups stands for several edges: say what they carry.
    const bundled = m.via.length > 0 || m.members.some((i) => {
      const e = S.graph.edges[i];
      return rep(e.from) !== e.from || rep(e.to) !== e.to;
    });
    // A bundle of tests says how many; other bundles say what they carry, and what they pass.
    const tests = m.members.every((i) => S.graph.edges[i].kind === 'cover') ? new Set(m.members.map((i) => S.graph.edges[i].from)).size : 0;
    // On the chart a label stays short; the tip and the details have every word.
    const what = clip(tests ? `${tests} ${tests === 1 ? 'test' : 'tests'}` : bundled ? summarize(m.members.map((i) => shortWords(m.says.get(i))), 2) : m.labels.join(', '), LABEL_MAX);
    const label = m.via.length ? `${what} · via ${viaWords(m.via)}` : what;
    return {
      group: 'edges',
      data: { id: m.id, source: m.source, target: m.target, kind: m.kind, label, count: m.members.length, members: m.members, via: m.via, dangling: m.dangling },
      classes: bundled ? 'bundle' : '',
    };
  });
  return { nodes, edges };
}

/**
 * How much a line should point ahead in the layout (SPEC §3.5): the walk's
 * lines and a client's own requests most, so the picture reads from where
 * things start; lines back into a client (the app it is served) least.
 */
function aheadPriority(e, clients, walked) {
  if (walked.has(e.data.id)) return walked.get(e.data.id);
  if (clients.has(e.data.source)) return 10;
  if (clients.has(e.data.target)) return 0;
  return 1;
}

/**
 * The way things happen, for a layout (SPEC §3.5): which lines are laid out against their arrow, and how
 * much each line should point ahead.
 */
function flowHints(nodes, edges) {
  // An answer that goes back to the part that called (data, or a read of a store the reader also
  // writes) is laid out along the call: the arrow still points back, but the parts keep their order.
  // The call may come from anywhere in the box the answer goes to, and go to anywhere in the box it
  // comes from (the CLI calls docair-core; docair-core's check hands its diagnostics to the CLI's printer).
  const parentOf = new Map(nodes.map((n) => [n.data.id, n.data.parent]));
  const up = (id) => {
    const out = [];
    for (let cur = id; cur; cur = parentOf.get(cur)) out.push(cur);
    return out;
  };
  const ahead = new Set();
  for (const e of edges) {
    if (!AHEAD.has(e.data.kind)) continue;
    const [as, bs] = [up(e.data.source), up(e.data.target)];
    for (const a of as) for (const b of bs) if (!as.includes(b) && !bs.includes(a)) ahead.add(`${a}\u0000${b}`);
  }
  const flips = (e) => {
    if (e.data.kind !== 'data' && e.data.kind !== 'read') return false;
    const [ts, ss] = [up(e.data.target), up(e.data.source)];
    return ts.some((t) => ss.some((x) => ahead.has(`${t}\u0000${x}`)));
  };
  // Clients: people and programs outside that start things (lines go out of them).
  const clients = new Set(nodes.filter((n) => S.byId.get(n.data.id)?.kind === 'external' && edges.some((e) => e.data.source === n.data.id && AHEAD.has(e.data.kind))).map((n) => n.data.id));
  // A walk's lines point ahead when they reach a part the walk has not been to yet; a line back to an earlier part gives way.
  const walked = new Map();
  if (S.flow) {
    const seen = new Map();
    const at = (id) => drawnRep(id);
    S.flow.steps.forEach((st, k) => {
      for (const i of st.path) {
        const e = S.graph.edges[i];
        const [a, b] = [at(e.from), at(e.to)];
        if (a && !seen.has(a)) seen.set(a, k);
        const back = b && seen.has(b) && seen.get(b) < (seen.get(a) ?? k);
        if (b && !seen.has(b)) seen.set(b, k);
        if (S.edgeOf.has(i) && !walked.has(S.edgeOf.get(i))) walked.set(S.edgeOf.get(i), back ? 0 : 20);
      }
      st.nodes.forEach((id) => at(id) && !seen.has(at(id)) && seen.set(at(id), k));
    });
  }
  return { flips, priority: (e) => aheadPriority(e, clients, walked) };
}

/** Zoom at which a layout of this size fits the canvas. */
function fitZoom(width, height) {
  const pad = 72;
  return Math.min((S.cy.width() - pad) / width, (S.cy.height() - pad) / height);
}

/** Positions, lines and their words: top-down (SPEC §3.5). */
async function layoutFor(nodes, edges) {
  const best = await topdownPlace(nodes, edges);
  if (best.scale) for (const e of edges) e.data.fs = LABEL_FONT * Math.min(best.scale.get(e.data.source) ?? 1, best.scale.get(e.data.target) ?? 1);
  // A small picture was routed while its placement was chosen.
  const { routes, none } = best.lines || gridLines(nodes, edges, best);
  S.noWay = none;
  return { ...best, routes, labels: placeLabels(nodes, edges, best.pos, routes, best.scale) };
}

/** The side of box `a` that faces box `b`. */
const facing = (a, b) => {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  return Math.abs(dx) / Math.max(a.w + b.w, 1) > Math.abs(dy) / Math.max(a.h + b.h, 1) ? (dx > 0 ? 'right' : 'left') : dy > 0 ? 'bottom' : 'top';
};

function routeLength(r) {
  let len = 0;
  for (let i = 1; i < r.length; i++) len += Math.abs(r[i].x - r[i - 1].x) + Math.abs(r[i].y - r[i - 1].y);
  return len;
}

/** A right-angled line with one corner, from the side of `a` that faces `b` to the side of `b` that faces `a`. */
function corner(a, b) {
  const at = (p, side) => ({ top: { x: p.x, y: p.y - p.h / 2 }, bottom: { x: p.x, y: p.y + p.h / 2 }, left: { x: p.x - p.w / 2, y: p.y }, right: { x: p.x + p.w / 2, y: p.y } })[side];
  const s = at(a, facing(a, b));
  const t = at(b, facing(b, a));
  const vertical = facing(a, b) === 'top' || facing(a, b) === 'bottom';
  return [s, vertical ? { x: s.x, y: t.y } : { x: t.x, y: s.y }, t];
}

/**
 * Where each line's words go: on the line, where they cover no part, no other words, no box's name and no
 * other line; of such spots, the one nearest the arrow. Where every spot on and beside the line covers
 * something, the one that covers least (a part counts most, then words and names, then lines). `scale`: how
 * much smaller each part is drawn, for the names of boxes. `keep`: words that stay where they are (in the
 * way of the others).
 */
function placeLabels(nodes, edges, pos, routes, scale = null, keep = new Map()) {
  const parts = [];
  const words = [...keep.values()].map((l) => ({ x1: l.x - l.w / 2, x2: l.x + l.w / 2, y1: l.y - l.h / 2, y2: l.y + l.h / 2 }));
  for (const n of nodes) {
    const p = pos.get(n.data.id);
    if (!p) continue;
    if (!n.data.compound) parts.push({ x1: p.x - p.w / 2 - 2, x2: p.x + p.w / 2 + 2, y1: p.y - p.h / 2 - 2, y2: p.y + p.h / 2 + 2 });
    else if (n.data.label) {
      // A box's name stands in the band at its top, in the middle.
      const sc = scale?.get(n.data.id) ?? 1;
      const half = (measure(n.data.label, 11.5) * 1.08 * sc) / 2 + 2;
      words.push({ x1: p.x - half, x2: p.x + half, y1: p.y - p.h / 2 + 3 * sc, y2: p.y - p.h / 2 + 21 * sc });
    }
  }
  // Every line's pieces, as thin boxes: words placed on another line's would hide it.
  const pieces = [];
  for (const [id, r] of routes) for (let i = 1; i < r.length; i++) pieces.push({ id, x1: Math.min(r[i].x, r[i - 1].x) - 1, x2: Math.max(r[i].x, r[i - 1].x) + 1, y1: Math.min(r[i].y, r[i - 1].y) - 1, y2: Math.max(r[i].y, r[i - 1].y) + 1 });
  const area = (b, t) => Math.max(0, Math.min(b.x2, t.x2) - Math.max(b.x1, t.x1)) * Math.max(0, Math.min(b.y2, t.y2) - Math.max(b.y1, t.y1));
  const labels = new Map(keep);
  for (const e of edges) {
    const r = routes.get(e.data.id);
    if (!r || !e.data.label || keep.has(e.data.id)) continue;
    const lines = wrapLines(e.data.label);
    const k = e.data.fs ? e.data.fs / LABEL_FONT : 1;
    const w = (Math.max(...lines.map((l) => Math.ceil(measure(l, LABEL_FONT)))) + 22) * k;
    const h = (14 * lines.length + 3) * k;
    // Spots along every piece of the line, from its arrow back and from each piece's middle outwards: on the
    // line first, then beside it.
    const on = [];
    const beside = [];
    for (let i = r.length - 1; i > 0; i--) {
      const flat = Math.abs(r[i].y - r[i - 1].y) < 0.5;
      const len = Math.abs(r[i].x - r[i - 1].x) + Math.abs(r[i].y - r[i - 1].y);
      const half = (flat ? w : h) / 2 + 4;
      // the words stay on their piece where it is long enough for them
      const reach = Math.max(0, len / 2 - half);
      for (let d = 0, n = 0; d <= reach && n < 15; d += 20, n++) {
        for (const t of d ? [len / 2 + d, len / 2 - d] : [len / 2]) {
          const c = { x: r[i - 1].x + ((r[i].x - r[i - 1].x) * t) / Math.max(len, 1), y: r[i - 1].y + ((r[i].y - r[i - 1].y) * t) / Math.max(len, 1) };
          on.push(c);
          if (flat) beside.push({ x: c.x, y: c.y - h / 2 - 3 }, { x: c.x, y: c.y + h / 2 + 3 });
          else beside.push({ x: c.x - w / 2 - 3, y: c.y }, { x: c.x + w / 2 + 3, y: c.y });
        }
      }
    }
    // Only what lies near the line can be covered.
    const reachBox = { x1: Math.min(...r.map((q) => q.x)) - w - 6, x2: Math.max(...r.map((q) => q.x)) + w + 6, y1: Math.min(...r.map((q) => q.y)) - h - 6, y2: Math.max(...r.map((q) => q.y)) + h + 6 };
    const close = (t) => t.x1 < reachBox.x2 && t.x2 > reachBox.x1 && t.y1 < reachBox.y2 && t.y2 > reachBox.y1;
    const [nearParts, nearWords, nearPieces] = [parts.filter(close), words.filter(close), pieces.filter((t) => t.id !== e.data.id && close(t))];
    const box = (c) => ({ x1: c.x - w / 2, x2: c.x + w / 2, y1: c.y - h / 2, y2: c.y + h / 2 });
    const cost = (c) => {
      const bx = box(c);
      let sum = 0;
      for (const t of nearParts) sum += 3 * area(bx, t);
      for (const t of nearWords) sum += 2 * area(bx, t);
      for (const t of nearPieces) sum += area(bx, t);
      return sum;
    };
    let at = null;
    let least = Infinity;
    for (const c of [...on, ...beside]) {
      const price = cost(c);
      if (price < least) [at, least] = [c, price];
      if (price === 0) break;
    }
    words.push(box(at));
    labels.set(e.data.id, { x: at.x, y: at.y, w, h });
  }
  return labels;
}

/**
 * Top-down (after Kasperowski & von Hanxleden): what is inside a box is laid
 * out on its own, then scaled to the box's size budget, so a box with a lot
 * inside does not blow the picture up; the reader zooms in to read it. elkjs
 * cannot size a box by its content (its size approximators are missing from
 * the JavaScript build, and every box then gets one size: six boxes deep, a
 * part is drawn at 0.5 %), so docair does the sizing: inside out, each box
 * shrinks what it holds to at most its budget, but never below its floor.
 * Lines between boxes are given to each layout as lines between the parts
 * that hold their ends, so the order things happen in is kept.
 *
 * Laid out one by one, a box does not know where its lines go outside it,
 * and most crossings come from that (on docair's code layer the lines, drawn
 * straight from part to part, cross 2 123 times; laid out across all boxes
 * by ELK, 1 259 times). So the picture is laid out once as it is, then again
 * in each of TOPDOWN_WAYS, which tell every box about its surroundings; the
 * placement whose lines cost least is kept.
 */
const TOPDOWN = { budget: 800, floor: 0.7, spacing: 1.6 };
/**
 * The ways a box learns where its lines go, from the placement before:
 *   ports    'line': a port on the box's border for every line that leaves it, on the side that faces the
 *            line's other end; 'group': one port for all lines of one part that leave by one side
 *   ordered  the ports of a side stand in the order of where their lines go
 *   linked   only boxes whose parts have lines among them get ports (the others are packed, and ports
 *            would string them up in a row)
 *   sorted   the parts of every box are given to ELK in the order of where their lines go: ELK keeps that
 *            order where its own crossing count does not say otherwise, and packs unconnected parts in it
 */
const TOPDOWN_WAYS = [
  { key: 'a port per line', ports: 'line' },
  { key: 'ports in order', ports: 'group', ordered: true },
  { key: 'a port per line, parts in order', ports: 'line', linked: true, sorted: true },
  { key: 'ports and parts in order', ports: 'group', ordered: true, linked: true, sorted: true },
];
/** Room inside a box: its name at the top. */
const BOX_PAD = { top: 28, left: 10, right: 10, bottom: 10 };

/***
 * Place a picture top-down: every box laid out on its own, knowing where its lines go.
 *
 * The picture is laid out as it is, in the direction that fits the canvas
 * best; then again in each of TOPDOWN_WAYS, which tell every box where its
 * lines go outside it. The cheapest placement wins: a small picture is routed
 * for each (that takes 20 ms) and the price of its lines decides; a big one
 * is judged by its lines drawn straight from part to part.
 *
 * @id      docair-viewer.top-down
 * @part-of docair-viewer.page
 * @flow    view 3 "the parts are placed box by box"
 * @calls   docair-viewer.grid-lines "routes a small picture to price a placement"
 ***/
async function topdownPlace(nodes, edges) {
  const hints = flowHints(nodes, edges);
  const cache = { elk: new ELK(), laid: new Map() };
  const first = await topdownFirst(nodes, edges, hints, cache);
  const small = edges.length <= ROUTER.exactUpTo;
  const priceOf = (r) => {
    if (!small) return straightPrice(edges, r.pos);
    r.lines = gridLines(nodes, edges, r);
    return r.lines.price;
  };
  let best = first;
  let least = priceOf(first);
  first.way = 'as it is';
  for (const way of TOPDOWN_WAYS) {
    let r;
    try {
      r = await topdownLayout(nodes, edges, first.direction, hints, cache, way, first);
    } catch (err) {
      console.warn(`docair: top-down, ${way.key}: ELK failed`, err);
      continue;
    }
    r.way = way.key;
    const c = priceOf(r);
    if (c < least) [best, least] = [r, c];
  }
  return best;
}

/** The picture placed as it is, in the direction that fits the canvas best (the other has to fit 1.1 times larger). */
async function topdownFirst(nodes, edges, hints = flowHints(nodes, edges), cache = { elk: new ELK(), laid: new Map() }) {
  const dirs = S.direction === 'auto' ? ['DOWN', 'RIGHT'] : [S.direction];
  let first = null;
  for (const d of dirs) {
    const r = await topdownLayout(nodes, edges, d, hints, cache);
    r.zoom = fitZoom(r.width, r.height);
    if (!first || r.zoom > first.zoom * 1.1) first = r;
  }
  return first;
}

/** What a placement's lines cost when drawn straight from part to part: their length, and 150 px for every crossing. */
function straightPrice(edges, pos) {
  const segs = edges.filter((e) => pos.get(e.data.source) && pos.get(e.data.target)).map((e) => [pos.get(e.data.source), pos.get(e.data.target), e.data]);
  const ccw = (a, b, c) => (c.y - a.y) * (b.x - a.x) > (b.y - a.y) * (c.x - a.x);
  let price = 0;
  for (let i = 0; i < segs.length; i++) {
    const [a, b, e] = segs[i];
    price += Math.abs(a.x - b.x) + Math.abs(a.y - b.y);
    for (let j = i + 1; j < segs.length; j++) {
      const [c, d, f] = segs[j];
      if (e.source === f.source || e.source === f.target || e.target === f.source || e.target === f.target) continue;
      if (ccw(a, c, d) !== ccw(b, c, d) && ccw(a, b, c) !== ccw(a, b, d)) price += ROUTER.cross;
    }
  }
  return price;
}

/**
 * One top-down placement: every box laid out inside out, then everything placed. With `way` and the
 * placement `prev` it starts from, the boxes know where their lines go (TOPDOWN_WAYS).
 */
async function topdownLayout(nodes, edges, direction, hints, cache, way = null, prev = null) {
  const sys = '__root';
  const byId = new Map(nodes.map((n) => [n.data.id, n.data]));
  const parentOf = (id) => byId.get(id)?.parent || sys;
  const kids = new Map([[sys, []]]);
  for (const n of nodes) {
    if (!kids.has(parentOf(n.data.id))) kids.set(parentOf(n.data.id), []);
    kids.get(parentOf(n.data.id)).push(n.data.id);
  }
  const chain = (id) => {
    const out = [];
    for (let cur = id; cur && cur !== sys; cur = parentOf(cur)) out.unshift(cur);
    return out;
  };
  // Each line, lifted to the two parts side by side under the box that holds both ends; laid out the way
  // things happen (an answer along its call: flowHints).
  const lifted = new Map();
  for (const e of edges) {
    const flip = hints.flips(e);
    const [a, b] = flip ? [chain(e.data.target), chain(e.data.source)] : [chain(e.data.source), chain(e.data.target)];
    let k = 0;
    while (k < a.length && k < b.length && a[k] === b[k]) k++;
    if (k >= a.length || k >= b.length) continue;
    const holder = k === 0 ? sys : a[k - 1];
    if (!lifted.has(holder)) lifted.set(holder, new Map());
    // A line between two parts side by side is that line itself: its words get room beside its arrow.
    const own = !flip && a[k] === e.data.source && b[k] === e.data.target;
    const key = `${a[k]}\u0000${b[k]}`;
    const had = lifted.get(holder).get(key);
    const ahead = hints.priority(e);
    if (!had || own) lifted.get(holder).set(key, { a: a[k], b: b[k], id: own ? e.data.id : null, ahead: Math.max(ahead, had ? had.ahead : 0) });
    else had.ahead = Math.max(had.ahead, ahead);
  }
  // Room for the lines: ELK's spacing, times 1.6.
  const spacing = Object.fromEntries(Object.entries(ELK_OPTIONS).map(([key, v]) => [key, /spacing/.test(key) ? String(Math.round(Number(v) * TOPDOWN.spacing)) : v]));
  const labelOf = new Map(edges.map((e) => [e.data.id, e.data.label]));
  const ELK_SIDE = { top: 'NORTH', bottom: 'SOUTH', left: 'WEST', right: 'EAST' };
  // Where each box's lines go outside it, in the placement before: ports on the sides that face their other ends.
  const ports = new Map();
  if (way?.ports) {
    const seen = new Set();
    for (const e of edges) {
      const [a, b] = [chain(e.data.source), chain(e.data.target)];
      for (const [mine, theirs, other, out] of [[a, b, e.data.target, true], [b, a, e.data.source, false]]) {
        for (let k = 0; k < mine.length - 1; k++) {
          const box = mine[k];
          const at = prev.pos.get(other);
          if (theirs.includes(box) || !prev.pos.get(box) || !at || (way.linked && !lifted.get(box)?.size)) continue;
          const q = { child: mine[k + 1], side: facing(prev.pos.get(box), at), out, at };
          if (way.ports === 'group') {
            const key = `${box}|${q.child}|${q.side}|${out}`;
            if (seen.has(key)) continue;
            seen.add(key);
          }
          if (!ports.has(box)) ports.set(box, []);
          ports.get(box).push(q);
        }
      }
    }
    // ELK counts a box's ports clockwise from its top left corner.
    if (way.ordered) {
      const rank = { top: 0, right: 1, bottom: 2, left: 3 };
      const along = (q) => (q.side === 'top' ? q.at.x : q.side === 'right' ? q.at.y : q.side === 'bottom' ? -q.at.x : -q.at.y);
      for (const list of ports.values()) list.sort((u, v) => rank[u.side] - rank[v.side] || along(u) - along(v));
    }
  }
  // The parts of each box in the order of where their lines go (across the direction of the layout).
  const order = new Map();
  if (way?.sorted) {
    const sum = new Map();
    const axis = (p) => (direction === 'RIGHT' ? p.y : p.x);
    const pull = (id, v) => {
      const s = sum.get(id) || [0, 0];
      sum.set(id, [s[0] + v, s[1] + 1]);
    };
    for (const e of edges) {
      const [a, b] = [chain(e.data.source), chain(e.data.target)];
      const [pa, pb] = [prev.pos.get(e.data.source), prev.pos.get(e.data.target)];
      if (!pa || !pb) continue;
      let k = 0;
      while (k < a.length && k < b.length && a[k] === b[k]) k++;
      for (let i = k; i < a.length; i++) pull(a[i], axis(pb));
      for (let i = k; i < b.length; i++) pull(b[i], axis(pa));
    }
    const key = (id) => (sum.has(id) ? sum.get(id)[0] / sum.get(id)[1] : axis(prev.pos.get(id)));
    for (const [box, ids] of kids) order.set(box, [...ids].sort((u, v) => key(u) - key(v)));
  }
  const size = new Map(); // a part's size as drawn in its box (a box: its scaled size)
  const local = new Map(); // a part's top-left inside its box's content
  const inner = new Map(); // a box: { k, minX, minY, w, h } of its content
  const graphOf = (box) => {
    const ids = order.get(box) || kids.get(box) || [];
    for (const id of ids) if (!kids.has(id)) size.set(id, { w: byId.get(id).w, h: byId.get(id).h });
    const out_ = ports.get(box) || [];
    return {
      id: box,
      layoutOptions: { ...spacing, 'elk.algorithm': 'layered', 'elk.direction': direction, 'elk.edgeRouting': 'ORTHOGONAL', ...(out_.length ? { 'elk.portConstraints': way.ordered ? 'FIXED_ORDER' : 'FIXED_SIDE' } : {}) },
      ports: out_.map((q, i) => ({ id: `${box}@${i}`, width: 1, height: 1, layoutOptions: { 'elk.port.side': ELK_SIDE[q.side], ...(way.ordered ? { 'elk.port.index': String(i) } : {}) } })),
      children: ids.map((id) => ({ id, width: size.get(id).w, height: size.get(id).h })),
      // A box's own lines carry their words: ELK keeps room for them beside the arrow.
      edges: [...(lifted.get(box) || new Map()).values()].map((l, i) => {
        const edge = { id: l.id || `${box}#${i}`, sources: [l.a], targets: [l.b], layoutOptions: { 'elk.layered.priority.direction': String(l.ahead) } };
        const text = l.id && labelOf.get(l.id);
        if (text) {
          const lines = wrapLines(text);
          edge.labels = [{ id: `${l.id}::label`, text, width: Math.max(...lines.map((t) => Math.ceil(measure(t, LABEL_FONT)))) + 22, height: 14 * lines.length + 3, layoutOptions: { 'elk.edgeLabels.inline': 'false', 'elk.edgeLabels.placement': 'HEAD' } }];
        }
        return edge;
      }).concat(out_.map((q, i) => ({ id: `${box}#port${i}`, sources: [q.out ? q.child : `${box}@${i}`], targets: [q.out ? `${box}@${i}` : q.child] }))),
    };
  };
  const take = (box, out) => {
    let [x1, y1, x2, y2] = [Infinity, Infinity, -Infinity, -Infinity];
    for (const c of out.children) {
      local.set(c.id, { x: c.x, y: c.y });
      [x1, y1, x2, y2] = [Math.min(x1, c.x), Math.min(y1, c.y), Math.max(x2, c.x + c.width), Math.max(y2, c.y + c.height)];
    }
    const w = Math.max(x2 - x1, 1);
    const h = Math.max(y2 - y1, 1);
    // The root is not scaled; a box shrinks what it holds to its budget, but not below its floor.
    const k = box === sys ? 1 : Math.max(TOPDOWN.floor, Math.min(1, TOPDOWN.budget / Math.max(w, h)));
    inner.set(box, { k, minX: x1, minY: y1, w, h });
    if (box !== sys) size.set(box, { w: Math.max(byId.get(box).minW || 0, w * k + BOX_PAD.left + BOX_PAD.right), h: h * k + BOX_PAD.top + BOX_PAD.bottom });
  };
  // Inside out: the boxes that hold no boxes first, then the boxes around them. All boxes of one height go to
  // ELK in one call, as the children of a frame that lays each out on its own (a call costs 3 ms on top of the
  // layout; a box with ports has to be such a child anyway: elkjs fails on ports of the graph it lays out).
  // The same box with the same parts, ports and lines is laid out once: the ways share most boxes.
  const heights = new Map();
  const height = (box) => {
    if (!heights.has(box)) heights.set(box, 1 + Math.max(0, ...(kids.get(box) || []).filter((id) => kids.has(id)).map(height)));
    return heights.get(box);
  };
  height(sys);
  const levels = [];
  for (const [box, h] of heights) (levels[h] ||= []).push(box);
  for (const level of levels) {
    if (!level) continue;
    const graphs = level.map(graphOf);
    const keys = graphs.map((gr) => JSON.stringify(gr));
    const fresh = graphs.filter((_, i) => !cache.laid.has(keys[i]));
    if (fresh.length) {
      const out = await cache.elk.layout({ id: '^', layoutOptions: { 'elk.algorithm': 'box', 'elk.hierarchyHandling': 'SEPARATE_CHILDREN' }, children: fresh });
      for (const c of out.children) cache.laid.set(keys[level.indexOf(c.id)], c);
    }
    level.forEach((box, i) => take(box, cache.laid.get(keys[i])));
  }
  // Place everything: inside a box, its content scaled by the box's k, below its name, centred.
  const pos = new Map();
  const scale = new Map();
  // `s` is the scale the box itself is drawn at; what it holds is drawn at s times its own k.
  const place = (box, ox, oy, s) => {
    const { k, minX, minY } = inner.get(box);
    const cs = s * k;
    for (const id of kids.get(box) || []) {
      const l = local.get(id);
      const sz = size.get(id);
      const x = ox + (l.x - minX) * cs;
      const y = oy + (l.y - minY) * cs;
      pos.set(id, { x: x + (sz.w * cs) / 2, y: y + (sz.h * cs) / 2, w: sz.w * cs, h: sz.h * cs });
      scale.set(id, cs);
      if (kids.has(id)) {
        const c = inner.get(id);
        const free = sz.w - BOX_PAD.left - BOX_PAD.right - c.w * c.k;
        place(id, x + (BOX_PAD.left + Math.max(0, free) / 2) * cs, y + BOX_PAD.top * cs, cs);
      }
    }
  };
  place(sys, 0, 0, 1);
  const root = inner.get(sys);
  const scaled = [...scale.values()].some((x) => x < 1);
  return { pos, routes: new Map(), labels: new Map(), scale: scaled ? scale : null, width: root.w, height: root.h, direction };
}

/**
 * What a line's way costs in docair's own router (top-down), in px of line. Tuned on docair's own layers
 * (SPEC §3.5). A crossing costs as much as 400 px of line and nearly seven bends: at 150 px and a bend at
 * 100 the four small layers had 24 crossings and 0.6 bends per line, now 17 and 0.9. Running on a line
 * costs three crossings: cheaper, a line would rather run a short piece on another than cross it.
 */
const ROUTER = {
  pitch: 8, // between two tracks in open space: lines run 8 px apart
  clear: 8, // between a line and a part (times the scale the part is drawn at)
  bend: 60,
  cross: 400, // crossing another line
  over: 1200, // a piece of line on top of another line (or on a track less than 3 px beside it): per piece …
  overPx: 6, // … and per px
  near: 1.5, // per px closer than `apart` beside another line
  apart: 6,
  thru: 150, // the border of a box that holds neither end
  hugBox: 1, // per px along a box's border
  hugPart: 0.25, // per px on the track right beside a part
  name: 3, // per px across a box's name
  shareEnd: 800, // ending where another line ends
  offCentre: 0.08, // per px an end is off the middle of its side
  detour: [200, 0.3], // a line keeps to the points on a way at most this much longer than the direct one: px, plus a share of the direct way
  rounds: 5, // every line is routed again, given the others, at most this often
  cell: 32, // big pictures: a line is first routed roughly, from cell to cell of this size, then exactly in the cells around that way
  around: 2, // how many cells around the rough way the exact way may use
  cellsFrom: 60000, // "big": from this many grid nodes on
  exactUpTo: 60, // up to this many lines, every top-down placement is routed and the cheapest picture wins
  most: 1500000, // more grid nodes than this: wider tracks
  merge: 3, // tracks closer than this are one
  side: 150, // a line that follows a part being moved: leaving or entering a part by another side than a moment ago
};

/***
 * Draw the lines of a top-down picture: each line the cheapest way, given all the others.
 *
 * docair's own router (SPEC §3.5). Lines run on tracks: beside every part (at
 * its clearance), through the middle of every line's end, and on a lattice
 * 8 px apart in between. A line may leave a part anywhere on any side. Each
 * line takes the cheapest way given all the others (ROUTER says what costs
 * what: length, bends, crossings, running on or close beside another line,
 * the borders of boxes that hold neither end, a box's name); then every line
 * is routed again, given the others as they now are, until no line finds a
 * cheaper way (after McMurchie & Ebeling's PathFinder, which routes the wires
 * of chips this way). Counting a crossing takes one look-up, so crossings
 * cost in every picture, however many lines it has. In a big picture a line
 * is first routed roughly, from cell to cell, and then exactly within the
 * cells around that way (global and detailed routing): 1.0 s instead of 2.9 s
 * for the 227 lines of docair's code layer.
 *
 * Returns the routes, the picture's price (to compare placements), and how
 * many lines found no way. While a part is moved by hand (rerouteHere):
 * `keep`, the lines that keep their way: in the way of the others as the
 * others are in each other's, but not routed again; `sides`, the sides the
 * other lines left and entered their parts by a moment ago, which they keep
 * unless another is clearly better (so they do not flip from frame to frame);
 * `rank`, the order they are routed in (else the shortest first); `rounds`,
 * how often they are routed at most.
 *
 * @id      docair-viewer.grid-lines
 * @part-of docair-viewer.page
 ***/
function gridLines(nodes, edges, placed, { keep = null, sides = null, rank = null, rounds = ROUTER.rounds } = {}) {
  const R = ROUTER;
  const { pos } = placed;
  const scaleOf = (id) => placed.scale?.get(id) ?? 1;
  const byId = new Map(nodes.map((n) => [n.data.id, n.data]));
  const rect = (id) => {
    const p = pos.get(id);
    return { l: p.x - p.w / 2, r: p.x + p.w / 2, t: p.y - p.h / 2, b: p.y + p.h / 2, x: p.x, y: p.y };
  };
  const lines = edges.filter((e) => pos.get(e.data.source) && pos.get(e.data.target));
  const ends = new Set(lines.flatMap((e) => [e.data.source, e.data.target]));
  const parts = nodes.filter((n) => !n.data.compound && pos.get(n.data.id)).map((n) => ({ id: n.data.id, ...rect(n.data.id), c: R.clear * scaleOf(n.data.id) }));
  const boxes = nodes.filter((n) => n.data.compound && pos.get(n.data.id)).map((n, i) => ({ id: n.data.id, i, ...rect(n.data.id), sc: scaleOf(n.data.id), nameW: n.data.label ? measure(n.data.label, 11.5) * 1.08 : 0 }));
  const boxIndex = new Map(boxes.map((b) => [b.id, b.i]));
  const routes = new Map();
  for (const e of lines) if (keep?.has(e.data.id)) routes.set(e.data.id, keep.get(e.data.id));
  const fresh = keep ? lines.filter((e) => !keep.has(e.data.id)) : lines;
  if (!fresh.length) return { routes, price: 0, none: 0 };
  // ── Tracks: beside every part (the edge of its room), through the middle of every end, a lattice between ──
  let [minX, minY, maxX, maxY] = [Infinity, Infinity, -Infinity, -Infinity];
  for (const p of pos.values()) [minX, minY, maxX, maxY] = [Math.min(minX, p.x - p.w / 2), Math.min(minY, p.y - p.h / 2), Math.max(maxX, p.x + p.w / 2), Math.max(maxY, p.y + p.h / 2)];
  const M = 40;
  const hard = { x: [], y: [] };
  const soft = { x: [], y: [] };
  for (const p of parts) {
    hard.x.push(p.l - p.c, p.r + p.c);
    hard.y.push(p.t - p.c, p.b + p.c);
    if (ends.has(p.id)) {
      soft.x.push(p.x);
      soft.y.push(p.y);
    }
  }
  for (const b of boxes) {
    if (!ends.has(b.id)) continue;
    const c = R.clear * b.sc;
    hard.x.push(b.l - c, b.r + c);
    hard.y.push(b.t - c, b.b + c);
    soft.x.push(b.x);
    soft.y.push(b.y);
  }
  // A kept line runs on tracks of its own: its rows where it runs along, its columns where it runs down.
  const flat = (a, b) => Math.abs(a.y - b.y) < 0.5;
  for (const e of lines) {
    const r = keep?.get(e.data.id);
    if (!r) continue;
    for (let k = 0; k < r.length; k++) {
      const [before, after] = [k > 0 ? flat(r[k - 1], r[k]) : null, k + 1 < r.length ? flat(r[k], r[k + 1]) : null];
      if (before === true || after === true) hard.y.push(r[k].y);
      if (before === false || after === false) hard.x.push(r[k].x);
    }
  }
  const tracks = (hardV, softV, lo, hi, pitch) => {
    const acc = [];
    const add = (vals, d) => {
      for (const v of vals) if (!acc.some((u) => Math.abs(u - v) < d)) acc.push(v);
    };
    add(hardV, 0.5);
    add(softV, R.merge);
    acc.sort((a, b) => a - b);
    const all = [...acc];
    let k = 0;
    for (let v = Math.floor((lo - M) / pitch) * pitch; v <= hi + M; v += pitch) {
      while (k < acc.length && acc[k] < v - R.merge) k++;
      if ((k < acc.length && Math.abs(acc[k] - v) < R.merge) || (k > 0 && Math.abs(acc[k - 1] - v) < R.merge)) continue;
      all.push(v);
    }
    return Float64Array.from(all.sort((a, b) => a - b));
  };
  let pitch = R.pitch;
  let X = tracks(hard.x, soft.x, minX, maxX, pitch);
  let Y = tracks(hard.y, soft.y, minY, maxY, pitch);
  // A picture too big for 8 px tracks gets wider ones.
  while (X.length * Y.length > R.most) {
    pitch *= 1.5;
    X = tracks(hard.x, soft.x, minX, maxX, pitch);
    Y = tracks(hard.y, soft.y, minY, maxY, pitch);
  }
  const [Nx, Ny] = [X.length, Y.length];
  const N = Nx * Ny;
  const lower = (arr, v) => {
    let lo = 0;
    let hi = arr.length;
    while (lo < hi) {
      const m = (lo + hi) >> 1;
      if (arr[m] < v) lo = m + 1;
      else hi = m;
    }
    return lo; // the first track at or after v
  };
  const at = (arr, v) => {
    const i = lower(arr, v - 0.25);
    return i < arr.length && Math.abs(arr[i] - v) < 0.6 ? i : -1;
  };
  // ── An edge's own price: its length, more where it runs along a border or a part or across a name;
  //    -1: no way (through a part's room). hC[n]: from node n to the right; vC[n]: from node n down ──
  const hC = new Float32Array(N);
  const vC = new Float32Array(N);
  for (let j = 0; j < Ny; j++) {
    for (let i = 0; i < Nx; i++) {
      hC[j * Nx + i] = i + 1 < Nx ? X[i + 1] - X[i] : -1;
      vC[j * Nx + i] = j + 1 < Ny ? Y[j + 1] - Y[j] : -1;
    }
  }
  const hSoft = new Float32Array(N);
  const vSoft = new Float32Array(N);
  const blocked = new Uint8Array(N);
  const rows = (i1, i2) => [Math.max(0, i1 - 1), Math.min(Nx - 2, i2)];
  const cols = (j1, j2) => [Math.max(0, j1 - 1), Math.min(Ny - 2, j2)];
  for (const p of parts) {
    const [i1, i2, j1, j2] = [lower(X, p.l - p.c + 0.5), lower(X, p.r + p.c - 0.5) - 1, lower(Y, p.t - p.c + 0.5), lower(Y, p.b + p.c - 0.5) - 1];
    const [ia, ib] = rows(i1, i2);
    const [ja, jb] = cols(j1, j2);
    for (let j = j1; j <= j2; j++) {
      for (let i = i1; i <= i2; i++) blocked[j * Nx + i] = 1;
      for (let i = ia; i <= ib; i++) hC[j * Nx + i] = -1;
    }
    for (let i = i1; i <= i2; i++) for (let j = ja; j <= jb; j++) vC[j * Nx + i] = -1;
    // The tracks right beside the part cost a little more: a line that only passes keeps its distance.
    for (const j of [at(Y, p.t - p.c), at(Y, p.b + p.c)]) if (j >= 0) for (let i = ia; i <= ib; i++) hSoft[j * Nx + i] = Math.max(hSoft[j * Nx + i], R.hugPart);
    for (const i of [at(X, p.l - p.c), at(X, p.r + p.c)]) if (i >= 0) for (let j = ja; j <= jb; j++) vSoft[j * Nx + i] = Math.max(vSoft[j * Nx + i], R.hugPart);
  }
  // ── Boxes: whose border an edge crosses; the tracks along a border; a box's name ──
  const hBox = new Int16Array(N).fill(-1);
  const vBox = new Int16Array(N).fill(-1);
  for (const b of boxes) {
    const [i1, i2, j1, j2] = [lower(X, b.l), lower(X, b.r) - 1, lower(Y, b.t), lower(Y, b.b) - 1];
    const [ia, ib] = rows(i1, i2);
    const [ja, jb] = cols(j1, j2);
    for (let j = j1; j <= j2; j++) {
      if (i1 > 0) hBox[j * Nx + i1 - 1] = b.i;
      if (i2 >= 0 && i2 < Nx - 1) hBox[j * Nx + i2] = b.i;
    }
    for (let i = i1; i <= i2; i++) {
      if (j1 > 0) vBox[(j1 - 1) * Nx + i] = b.i;
      if (j2 >= 0 && j2 < Ny - 1) vBox[j2 * Nx + i] = b.i;
    }
    for (const bx of [b.l, b.r]) for (let i = lower(X, bx - 3); i < Nx && X[i] <= bx + 3; i++) for (let j = ja; j <= jb; j++) vSoft[j * Nx + i] = Math.max(vSoft[j * Nx + i], R.hugBox);
    for (const by of [b.t, b.b]) for (let j = lower(Y, by - 3); j < Ny && Y[j] <= by + 3; j++) for (let i = ia; i <= ib; i++) hSoft[j * Nx + i] = Math.max(hSoft[j * Nx + i], R.hugBox);
    if (b.nameW) {
      // The name stands in the band at the box's top, in the middle.
      const [a1, a2, c1, c2] = [lower(X, b.x - (b.nameW * b.sc) / 2 - 2), lower(X, b.x + (b.nameW * b.sc) / 2 + 2) - 1, lower(Y, b.t + 3 * b.sc), lower(Y, b.t + 21 * b.sc) - 1];
      for (let j = c1; j <= c2; j++) for (let i = Math.max(0, a1 - 1); i <= Math.min(Nx - 2, a2); i++) hSoft[j * Nx + i] += R.name;
      for (let i = a1; i <= a2; i++) for (let j = Math.max(0, c1 - 1); j <= Math.min(Ny - 2, c2); j++) vSoft[j * Nx + i] += R.name;
    }
  }
  for (let n = 0; n < N; n++) {
    if (hC[n] > 0) hC[n] *= 1 + hSoft[n];
    if (vC[n] > 0) vC[n] *= 1 + vSoft[n];
  }
  // ── Tracks closer than `apart` beside each other (few: the lattice keeps its pitch) ──
  const nearOf = (arr) => {
    const start = new Int32Array(arr.length + 1);
    const list = [];
    const tight = []; // less than 3 px apart: a line there is as good as on the other
    for (let i = 0; i < arr.length; i++) {
      start[i] = list.length;
      for (let k = i - 1; k >= 0 && arr[i] - arr[k] < R.apart; k--) {
        list.push(k);
        tight.push(arr[i] - arr[k] < R.merge ? 1 : 0);
      }
      for (let k = i + 1; k < arr.length && arr[k] - arr[i] < R.apart; k++) {
        list.push(k);
        tight.push(arr[k] - arr[i] < R.merge ? 1 : 0);
      }
    }
    start[arr.length] = list.length;
    return [start, Int32Array.from(list), Uint8Array.from(tight)];
  };
  const [nearXs, nearXl, nearXt] = nearOf(X);
  const [nearYs, nearYl, nearYt] = nearOf(Y);
  // ── Where a line may leave a part or a box: the nodes on the track beside each of its sides ──
  const termCache = new Map();
  const terminals = (id) => {
    if (termCache.has(id)) return termCache.get(id);
    const r = rect(id);
    const c = R.clear * scaleOf(id);
    const out = []; // n: the node; o: 0 the line leaves along a row, 1 along a column; bx, by: where it touches the part
    const m = Math.min(4 * scaleOf(id), (r.r - r.l) / 4, (r.b - r.t) / 4);
    const [jt, jb, il, ir] = [at(Y, r.t - c), at(Y, r.b + c), at(X, r.l - c), at(X, r.r + c)];
    for (let i = lower(X, r.l + m); i < Nx && X[i] <= r.r - m; i++) {
      if (jt >= 0 && !blocked[jt * Nx + i]) out.push({ n: jt * Nx + i, o: 1, stub: c, off: Math.abs(X[i] - r.x), bx: X[i], by: r.t, side: 'top' });
      if (jb >= 0 && !blocked[jb * Nx + i]) out.push({ n: jb * Nx + i, o: 1, stub: c, off: Math.abs(X[i] - r.x), bx: X[i], by: r.b, side: 'bottom' });
    }
    for (let j = lower(Y, r.t + m); j < Ny && Y[j] <= r.b - m; j++) {
      if (il >= 0 && !blocked[j * Nx + il]) out.push({ n: j * Nx + il, o: 0, stub: c, off: Math.abs(Y[j] - r.y), bx: r.l, by: Y[j], side: 'left' });
      if (ir >= 0 && !blocked[j * Nx + ir]) out.push({ n: j * Nx + ir, o: 0, stub: c, off: Math.abs(Y[j] - r.y), bx: r.r, by: Y[j], side: 'right' });
    }
    termCache.set(id, out);
    return out;
  };
  // ── The rough picture: cells of `cell` px. How many tracks lead from a cell to the next and how many of
  //    them are taken; how much line runs through a cell along its rows and along its columns ──
  const CS = R.cell;
  const rough = N >= R.cellsFrom;
  const cx = new Int32Array(Nx);
  const cy = new Int32Array(Ny);
  for (let i = 0; i < Nx; i++) cx[i] = Math.floor((X[i] - X[0]) / CS);
  for (let j = 0; j < Ny; j++) cy[j] = Math.floor((Y[j] - Y[0]) / CS);
  const [Cx, Cy] = [cx[Nx - 1] + 1, cy[Ny - 1] + 1];
  const C = Cx * Cy;
  const capR = new Uint16Array(C);
  const capD = new Uint16Array(C);
  const useR = new Uint16Array(C);
  const useD = new Uint16Array(C);
  const flowH = new Float32Array(C);
  const flowV = new Float32Array(C);
  const cellBox = new Int16Array(C).fill(-1); // the innermost box around a cell's middle
  if (rough) {
    for (let j = 0; j < Ny; j++) {
      for (let i = 0; i < Nx; i++) {
        const n = j * Nx + i;
        const c = cy[j] * Cx + cx[i];
        if (i + 1 < Nx && cx[i + 1] !== cx[i] && hC[n] >= 0) capR[c]++;
        if (j + 1 < Ny && cy[j + 1] !== cy[j] && vC[n] >= 0) capD[c]++;
      }
    }
    // big boxes first: the smaller ones inside them overwrite
    for (const b of [...boxes].sort((u, v) => (v.r - v.l) * (v.b - v.t) - (u.r - u.l) * (u.b - u.t))) {
      const [a1, a2, c1, c2] = [Math.ceil((b.l - X[0]) / CS - 0.5), Math.floor((b.r - X[0]) / CS - 0.5), Math.ceil((b.t - Y[0]) / CS - 0.5), Math.floor((b.b - Y[0]) / CS - 0.5)];
      for (let q = Math.max(0, c1); q <= Math.min(Cy - 1, c2); q++) for (let p = Math.max(0, a1); p <= Math.min(Cx - 1, a2); p++) cellBox[q * Cx + p] = b.i;
    }
  }
  const boxParent = boxes.map((b) => (boxIndex.has(byId.get(b.id)?.parent) ? boxIndex.get(byId.get(b.id).parent) : -1));
  const boxDepth = boxes.map((b, i) => {
    let d = 0;
    for (let c = boxParent[i]; c >= 0; c = boxParent[c]) d++;
    return d;
  });
  /** What it costs a line to go between two cells with different boxes around them: the borders in between. */
  const borders = (a, b, allowed) => {
    let cost = 0;
    while (a !== b) {
      if (a >= 0 && (b < 0 || boxDepth[a] >= boxDepth[b])) {
        if (!allowed[a]) cost += R.thru;
        a = boxParent[a];
      } else {
        if (!allowed[b]) cost += R.thru;
        b = boxParent[b];
      }
    }
    return cost;
  };
  // ── Who uses what: edges, and how lines pass each node (along its row, along its column, with a bend) ──
  const hUse = new Uint16Array(N);
  const vUse = new Uint16Array(N);
  const hThru = new Uint16Array(N);
  const vThru = new Uint16Array(N);
  const corner_ = new Uint16Array(N);
  const termUse = new Uint16Array(N);
  const B = R.bend;
  let none = 0;
  // ── The rough search, from cell to cell (a binary heap) ──
  const cG = new Float32Array(C * 2);
  const cStamp = new Uint32Array(C * 2);
  const cDone = new Uint32Array(C * 2);
  const cFrom = new Int32Array(C * 2);
  const inWay = new Uint32Array(C); // the cells the exact search may use, by search
  let cGen = 0;
  let hpF = new Float64Array(1 << 12);
  let hpS = new Int32Array(1 << 12);
  let hpN = 0;
  const hpush = (f, s) => {
    if (hpN === hpF.length) {
      const a = new Float64Array(hpN * 2);
      a.set(hpF);
      hpF = a;
      const b = new Int32Array(hpN * 2);
      b.set(hpS);
      hpS = b;
    }
    let i = hpN++;
    while (i > 0) {
      const p = (i - 1) >> 1;
      if (hpF[p] <= f) break;
      hpF[i] = hpF[p];
      hpS[i] = hpS[p];
      i = p;
    }
    hpF[i] = f;
    hpS[i] = s;
  };
  const hpop = () => {
    const top = hpS[0];
    const f = hpF[--hpN];
    const s = hpS[hpN];
    let i = 0;
    for (;;) {
      let c = 2 * i + 1;
      if (c >= hpN) break;
      if (c + 1 < hpN && hpF[c + 1] < hpF[c]) c++;
      if (hpF[c] >= f) break;
      hpF[i] = hpF[c];
      hpS[i] = hpS[c];
      i = c;
    }
    hpF[i] = f;
    hpS[i] = s;
    return top;
  };
  /** A line's rough way, cell by cell: marks the cells around it in `inWay`. False: none found. */
  const roughWay = (L) => {
    const tr = rect(L.e.data.target);
    const sr = rect(L.e.data.source);
    const tc = R.clear * scaleOf(L.e.data.target);
    const sc = R.clear * scaleOf(L.e.data.source);
    const cells = (r, c) => [Math.max(0, Math.floor((r.l - c - X[0]) / CS)), Math.min(Cx - 1, Math.floor((r.r + c - X[0]) / CS)), Math.max(0, Math.floor((r.t - c - Y[0]) / CS)), Math.min(Cy - 1, Math.floor((r.b + c - Y[0]) / CS))];
    const [sp1, sp2, sq1, sq2] = cells(sr, sc);
    const [tp1, tp2, tq1, tq2] = cells(tr, tc);
    const direct = (Math.max(0, tp1 - sp2, sp1 - tp2) + Math.max(0, tq1 - sq2, sq1 - tq2)) * CS;
    const budget = direct * (1 + R.detour[1]) + R.detour[0] + 2 * CS;
    cGen++;
    hpN = 0;
    const left = (p, q, o) => {
      const a = p < tp1 ? tp1 - p : p > tp2 ? p - tp2 : 0;
      const b = q < tq1 ? tq1 - q : q > tq2 ? q - tq2 : 0;
      return (a + b) * CS + (a > 0 && b > 0 ? B : a > 0 ? (o ? B : 0) : b > 0 ? (o ? 0 : B) : 0);
    };
    for (let q = sq1; q <= sq2; q++) {
      for (let p = sp1; p <= sp2; p++) {
        for (let o = 0; o < 2; o++) {
          const s = (q * Cx + p) * 2 + o;
          cStamp[s] = cGen;
          cG[s] = 0;
          cFrom[s] = -1;
          hpush(left(p, q, o), s);
        }
      }
    }
    let end = -1;
    while (hpN) {
      const s = hpop();
      if (cDone[s] === cGen) continue;
      cDone[s] = cGen;
      const c = s >> 1;
      const o = s & 1;
      const p = c % Cx;
      const q = (c - p) / Cx;
      if (p >= tp1 && p <= tp2 && q >= tq1 && q <= tq2) {
        end = s;
        break;
      }
      for (let d = 0; d < 4; d++) {
        // the border between the two cells is kept at the left or upper one
        const c2 = c + (d === 0 ? 1 : d === 1 ? -1 : d === 2 ? Cx : -Cx);
        const p2 = d === 0 ? p + 1 : d === 1 ? p - 1 : p;
        const q2 = d === 2 ? q + 1 : d === 3 ? q - 1 : q;
        if (p2 < 0 || p2 >= Cx || q2 < 0 || q2 >= Cy) continue;
        const brd = Math.min(c, c2);
        const cap = d < 2 ? capR[brd] : capD[brd];
        const use = d < 2 ? useR[brd] : useD[brd];
        if (!cap) continue;
        const o2 = d < 2 ? 0 : 1;
        const far = ((p2 < sp1 ? sp1 - p2 : p2 > sp2 ? p2 - sp2 : 0) + (q2 < sq1 ? sq1 - q2 : q2 > sq2 ? q2 - sq2 : 0) + (p2 < tp1 ? tp1 - p2 : p2 > tp2 ? p2 - tp2 : 0) + (q2 < tq1 ? tq1 - q2 : q2 > tq2 ? q2 - tq2 : 0)) * CS;
        if (far > budget) continue;
        // The lines that run the other way through the two cells are crossed; a full border costs like a line run on.
        let cost = cG[s] + CS + R.cross * 0.5 * (o2 ? flowH[c] + flowH[c2] : flowV[c] + flowV[c2]);
        if (use >= cap) cost += R.over + R.overPx * CS;
        else if (use) cost += R.near * CS * (use / cap);
        if (o2 !== o) cost += B;
        if (cellBox[c] !== cellBox[c2]) cost += borders(cellBox[c], cellBox[c2], L.allowed);
        const s2 = c2 * 2 + o2;
        if (cStamp[s2] === cGen && cG[s2] <= cost) continue;
        cStamp[s2] = cGen;
        cG[s2] = cost;
        cFrom[s2] = s;
        hpush(cost + left(p2, q2, o2), s2);
      }
    }
    if (end < 0) return false;
    const mark = (p, q, a) => {
      for (let v = Math.max(0, q - a); v <= Math.min(Cy - 1, q + a); v++) for (let u = Math.max(0, p - a); u <= Math.min(Cx - 1, p + a); u++) inWay[v * Cx + u] = cGen;
    };
    for (let s = end; s >= 0; s = cFrom[s]) mark((s >> 1) % Cx, ((s >> 1) / Cx) | 0, R.around);
    for (let q = sq1; q <= sq2; q++) for (let p = sp1; p <= sp2; p++) mark(p, q, 1);
    for (let q = tq1; q <= tq2; q++) for (let p = tp1; p <= tp2; p++) mark(p, q, 1);
    return true;
  };
  // ── The exact search: a state is a node and how the line lies in it (along its row or its column);
  //    states wait in buckets, one per px of price ──
  const gCost = new Float32Array(N * 2);
  const stamp = new Uint32Array(N * 2);
  const done = new Uint32Array(N * 2);
  const from = new Int32Array(N * 2);
  const goalAt = new Int32Array(N);
  const goalGen = new Uint32Array(N);
  const hx = new Float32Array(Nx);
  const hy = new Float32Array(Ny);
  const sx = new Float32Array(Nx);
  const sy = new Float32Array(Ny);
  const NB = Math.ceil((maxX - minX + maxY - minY) * 4 + 40000);
  const bHead = new Int32Array(NB).fill(-1);
  let qNext = new Int32Array(1 << 16);
  let qState = new Int32Array(1 << 16);
  let qN = 0;
  let qCur = 0;
  let qMax = -1;
  const qpush = (f, s) => {
    const b = Math.min(NB - 1, f | 0);
    if (qN === qNext.length) {
      const a = new Int32Array(qN * 2);
      a.set(qNext);
      qNext = a;
      const c = new Int32Array(qN * 2);
      c.set(qState);
      qState = c;
    }
    qState[qN] = s;
    qNext[qN] = bHead[b];
    bHead[b] = qN++;
    if (b < qCur) qCur = b;
    if (b > qMax) qMax = b;
  };
  let gen = 0;
  /**
   * The cheapest way for a line, given the others. `within`: 'cells' (the cells its rough way marked),
   * 'budget' (no further around than ROUTER.detour allows), or 'all'.
   */
  const search = (L, within) => {
    const src = terminals(L.e.data.source);
    const dst = terminals(L.e.data.target);
    if (!src.length || !dst.length) return null;
    const tr = rect(L.e.data.target);
    const sr = rect(L.e.data.source);
    const tc = R.clear * scaleOf(L.e.data.target);
    const sc = R.clear * scaleOf(L.e.data.source);
    const [gx1, gx2, gy1, gy2] = [tr.l - tc, tr.r + tc, tr.t - tc, tr.b + tc];
    const [ax1, ax2, ay1, ay2] = [sr.l - sc, sr.r + sc, sr.t - sc, sr.b + sc];
    // How far a column and a row are from the target's room (what is left at least) and from the source's.
    for (let i = 0; i < Nx; i++) {
      const x = X[i];
      hx[i] = x < gx1 ? gx1 - x : x > gx2 ? x - gx2 : 0;
      sx[i] = x < ax1 ? ax1 - x : x > ax2 ? x - ax2 : 0;
    }
    for (let j = 0; j < Ny; j++) {
      const y = Y[j];
      hy[j] = y < gy1 ? gy1 - y : y > gy2 ? y - gy2 : 0;
      sy[j] = y < ay1 ? ay1 - y : y > ay2 ? y - ay2 : 0;
    }
    const direct = Math.max(0, gx1 - ax2, ax1 - gx2) + Math.max(0, gy1 - ay2, ay1 - gy2);
    const budget = within === 'budget' ? direct * (1 + R.detour[1]) + R.detour[0] : Infinity;
    const cells = within === 'cells';
    gen++;
    for (let b = qCur; b <= qMax; b++) bHead[b] = -1;
    [qN, qCur, qMax] = [0, NB, -1];
    dst.forEach((t, k) => {
      goalAt[t.n] = k;
      goalGen[t.n] = gen;
    });
    const allowed = L.allowed;
    // What is left at least from a node: the way to the target's room, and the bend that takes.
    const left = (i, j, o) => {
      const a = hx[i];
      const b = hy[j];
      return a + b + (a > 0 && b > 0 ? B : a > 0 ? (o ? B : 0) : b > 0 ? (o ? 0 : B) : 0);
    };
    const was = L.sides;
    for (const t of src) {
      const s = t.n * 2 + t.o;
      const c = t.stub + R.offCentre * t.off + R.shareEnd * termUse[t.n] + (was && was.s !== t.side ? R.side : 0);
      if (stamp[s] === gen && c >= gCost[s]) continue;
      stamp[s] = gen;
      gCost[s] = c;
      from[s] = -1;
      qpush(c + left(t.n % Nx, (t.n / Nx) | 0, t.o), s);
    }
    let best = Infinity;
    let bestS = -1;
    let bestT = null;
    for (;;) {
      while (qCur <= qMax && bHead[qCur] < 0) qCur++;
      if (qCur > qMax || qCur >= best) break;
      const q = bHead[qCur];
      bHead[qCur] = qNext[q];
      const s = qState[q];
      if (done[s] === gen) continue;
      done[s] = gen;
      const n = s >> 1;
      const o = s & 1;
      const gHere = gCost[s];
      if (goalGen[n] === gen) {
        // Into the part: straight in if the line already lies that way.
        const t = dst[goalAt[n]];
        const c = gHere + (o === t.o ? 0 : B) + t.stub + R.offCentre * t.off + R.shareEnd * termUse[n] + (was && was.t !== t.side ? R.side : 0);
        if (c < best) [best, bestS, bestT] = [c, s, t];
      }
      const i = n % Nx;
      const j = (n - i) / Nx;
      // Four ways on: right, left (along the row), down, up (along the column).
      for (let d = 0; d < 4; d++) {
        let e = n; // the edge, kept at its left or upper node
        let n2 = n + 1;
        let i2 = i;
        let j2 = j;
        if (d === 0) i2 = i + 1;
        else if (d === 1) {
          if (i === 0) continue;
          e = n2 = n - 1;
          i2 = i - 1;
        } else if (d === 2) {
          n2 = n + Nx;
          j2 = j + 1;
        } else {
          if (j === 0) continue;
          e = n2 = n - Nx;
          j2 = j - 1;
        }
        const o2 = d < 2 ? 0 : 1;
        let c = o2 ? vC[e] : hC[e];
        if (c < 0) continue;
        if (cells) {
          if (inWay[cy[j2] * Cx + cx[i2]] !== cGen) continue;
        } else if (sx[i2] + sy[j2] + hx[i2] + hy[j2] > budget) continue;
        const len = o2 ? Math.abs(Y[j2] - Y[j]) : Math.abs(X[i2] - X[i]);
        c += gHere;
        const box = o2 ? vBox[e] : hBox[e];
        if (box >= 0 && !allowed[box]) c += R.thru;
        const use = o2 ? vUse[e] : hUse[e];
        if (use) c += use * (R.over + R.overPx * len);
        if (o2) {
          for (let k = nearXs[i]; k < nearXs[i + 1]; k++) {
            const beside = vUse[e - i + nearXl[k]];
            if (beside) c += beside * (nearXt[k] ? R.over + R.overPx * len : R.near * len);
          }
        } else {
          for (let k = nearYs[j]; k < nearYs[j + 1]; k++) {
            const beside = hUse[nearYl[k] * Nx + (e % Nx)];
            if (beside) c += beside * (nearYt[k] ? R.over + R.overPx * len : R.near * len);
          }
        }
        // A bend where another line bends would touch it; straight on crosses the lines that go the other way here.
        if (o2 !== o) c += B + R.cross * corner_[n];
        else c += R.cross * (o2 ? hThru[n] : vThru[n]);
        const s2 = n2 * 2 + o2;
        if (stamp[s2] === gen && gCost[s2] <= c) continue;
        stamp[s2] = gen;
        gCost[s2] = c;
        from[s2] = s;
        qpush(c + left(i2, j2, o2), s2);
      }
    }
    if (bestS < 0) return null;
    const way = [];
    let s0 = bestS;
    for (let s = bestS; s >= 0; s = from[s]) {
      // a node reached along its row and left along its column is listed twice: once is enough
      if (way[way.length - 1] !== s >> 1) way.push(s >> 1);
      s0 = s;
    }
    way.reverse();
    return { nodes: way, st: src.find((t) => t.n === s0 >> 1 && t.o === (s0 & 1)), tt: bestT };
  };
  const occupy = (L, sign) => {
    const nd = L.way.nodes;
    termUse[nd[0]] += sign;
    termUse[nd[nd.length - 1]] += sign;
    for (let k = 0; k < nd.length; k++) {
      const n = nd[k];
      if (k + 1 < nd.length) {
        const flat = Math.abs(nd[k + 1] - n) === 1;
        const e = Math.min(n, nd[k + 1]); // an edge is kept at its left or upper node
        if (flat) hUse[e] += sign;
        else vUse[e] += sign;
        if (rough) {
          const i = e % Nx;
          const j = (e - i) / Nx;
          const c = cy[j] * Cx + cx[i];
          if (flat) {
            flowH[c] += (sign * (X[i + 1] - X[i])) / CS;
            if (cx[i + 1] !== cx[i]) useR[c] += sign;
          } else {
            flowV[c] += (sign * (Y[j + 1] - Y[j])) / CS;
            if (cy[j + 1] !== cy[j]) useD[c] += sign;
          }
        }
      }
      const prevFlat = k > 0 ? Math.abs(nd[k - 1] - n) === 1 : L.way.st.o === 0;
      const nextFlat = k + 1 < nd.length ? Math.abs(nd[k + 1] - n) === 1 : L.way.tt.o === 0;
      if (prevFlat && nextFlat) hThru[n] += sign;
      else if (!prevFlat && !nextFlat) vThru[n] += sign;
      else corner_[n] += sign;
    }
  };
  /** A kept line on the grid: the nodes it passes, from the track beside its source to the one beside its target. */
  const gridWay = (e, r) => {
    const nd = [];
    for (let k = 1; k < r.length; k++) {
      const [a, b] = [r[k - 1], r[k]];
      const along = flat(a, b);
      const fix = along ? at(Y, a.y) : at(X, a.x);
      if (fix < 0) return null;
      const arr = along ? X : Y;
      const [u, v] = along ? [a.x, b.x] : [a.y, b.y];
      const run = [];
      for (let i = lower(arr, Math.min(u, v) - 0.5); i < arr.length && arr[i] <= Math.max(u, v) + 0.5; i++) run.push(along ? fix * Nx + i : i * Nx + fix);
      if (u > v) run.reverse();
      for (const n of run) if (nd[nd.length - 1] !== n) nd.push(n);
    }
    // In the rooms at its ends the line only reaches its parts: it takes the grid from the tracks beside them on.
    const inRoom = (n, q, id) => Math.abs(X[n % Nx] - q.x) + Math.abs(Y[(n / Nx) | 0] - q.y) < R.clear * scaleOf(id) - 0.6;
    while (nd.length && inRoom(nd[0], r[0], e.data.source)) nd.shift();
    while (nd.length && inRoom(nd[nd.length - 1], r[r.length - 1], e.data.target)) nd.pop();
    return nd.length ? { nodes: nd, st: { o: flat(r[0], r[1]) ? 0 : 1 }, tt: { o: flat(r[r.length - 2], r[r.length - 1]) ? 0 : 1 } } : null;
  };
  for (const e of lines) {
    const r = keep?.get(e.data.id);
    const way = r && r.length > 1 && gridWay(e, r);
    if (way) occupy({ way }, 1);
  }
  const Ls = fresh.map((e) => {
    // The boxes around either end: their borders are the line's to cross.
    const allowed = new Uint8Array(boxes.length);
    for (const id of [e.data.source, e.data.target]) for (let c = id; c; c = byId.get(c)?.parent) if (boxIndex.has(c)) allowed[boxIndex.get(c)] = 1;
    return { e, allowed, way: null, sides: sides?.get(e.data.id) || null };
  });
  // Short lines first: they have the fewest ways to choose from.
  const span = (L) => rank?.get(L.e.data.id) ?? Math.abs(pos.get(L.e.data.source).x - pos.get(L.e.data.target).x) + Math.abs(pos.get(L.e.data.source).y - pos.get(L.e.data.target).y);
  const order = [...Ls].sort((u, v) => span(u) - span(v));
  /** Every line again, given the others as they now are; how many found another way. */
  const round = (list) => {
    let changed = 0;
    for (const L of list) {
      const old = L.way;
      if (old) occupy(L, -1);
      const way = (rough && roughWay(L) && search(L, 'cells')) || search(L, 'budget') || search(L, 'all');
      if (!way) {
        L.way = old;
        if (old) occupy(L, 1);
        continue;
      }
      if (!old || way.nodes.length !== old.nodes.length || way.nodes.some((n, k) => n !== old.nodes[k])) changed++;
      L.way = way;
      occupy(L, 1);
    }
    return changed;
  };
  for (let r = 0; r < rounds; r++) if (!round(order)) break;
  /** A line's points: from where it touches its source, along its way, to where it touches its target. */
  const points = (L) => {
    const pts = [{ x: L.way.st.bx, y: L.way.st.by }];
    const add = (q) => {
      const k = pts.length;
      // points on a straight run are left out
      if (k >= 2 && ((Math.abs(pts[k - 2].y - pts[k - 1].y) < 0.01 && Math.abs(pts[k - 1].y - q.y) < 0.01) || (Math.abs(pts[k - 2].x - pts[k - 1].x) < 0.01 && Math.abs(pts[k - 1].x - q.x) < 0.01))) pts[k - 1] = q;
      else pts.push(q);
    };
    for (const n of L.way.nodes) add({ x: X[n % Nx], y: Y[(n / Nx) | 0] });
    add({ x: L.way.tt.bx, y: L.way.tt.by });
    return pts;
  };
  // The routes, and the picture's price: length, bends, crossings, lines on lines, borders of other boxes.
  let price = 0;
  for (const L of Ls) {
    const [a, b] = [pos.get(L.e.data.source), pos.get(L.e.data.target)];
    if (!L.way) {
      none++;
      routes.set(L.e.data.id, corner(a, b));
      price += 5000;
      continue;
    }
    const pts = points(L);
    routes.set(L.e.data.id, pts);
    price += routeLength(pts) + B * (pts.length - 2);
    const nd = L.way.nodes;
    for (let i = 1; i < nd.length; i++) {
      const e = Math.min(nd[i - 1], nd[i]);
      const box = Math.abs(nd[i] - nd[i - 1]) === 1 ? hBox[e] : vBox[e];
      if (box >= 0 && !L.allowed[box]) price += R.thru;
    }
  }
  for (let n = 0; n < N; n++) {
    price += R.cross * hThru[n] * vThru[n];
    if (hUse[n] > 1) price += (hUse[n] - 1) * (R.over + R.overPx * Math.max(0, hC[n]));
    if (vUse[n] > 1) price += (vUse[n] - 1) * (R.over + R.overPx * Math.max(0, vC[n]));
  }
  return { routes, price, none };
}

function groupDepth(id) {
  let d = 0;
  for (let cur = id; cur && cur !== S.graph.system; cur = S.byId.get(cur)?.part_of) d++;
  return d;
}

/** Every node that has children, except the system. */
function groups() {
  return [...S.children.keys()].filter((id) => id !== S.graph.system && S.byId.has(id) && !isData(id));
}

/** Data (SPEC §3.6) is what the parts make and carry: it has its own list, not a box among the parts. */
function isData(id) {
  return S.byId.get(id)?.kind === 'data';
}

/**
 * Layers (SPEC §3.5): each one answers one question about the system and
 * draws the parts that question is about. Everything else is left out, or
 * stands as context where a line joins it to them. A line that passes parts a
 * layer leaves out is drawn once, between the parts it joins, and says what it
 * passes: `POST /api/ask via host-port › traefik › Ingress docair`.
 */
const LAYERS = [
  { key: 'overview', name: 'Overview', ask: 'What are the big pieces, and how do they talk?' },
  { key: 'infra', name: 'Infrastructure', ask: 'Which machines, networks and clusters are there, and what provisions them?' },
  { key: 'network', name: 'Network', ask: 'How does a request get in, and where does it go?' },
  { key: 'k8s', name: 'Kubernetes', ask: 'What is installed in the cluster, and how is it configured?' },
  { key: 'apps', name: 'Apps', ask: 'Which programs are there, and how do they talk to each other?' },
  { key: 'code', name: 'Code', ask: 'How does a program work inside?' },
];
const PLATFORM = new Set(['host', 'network', 'cluster', 'namespace', 'release', 'workload', 'container']);
const CODE_KINDS = new Set(['module', 'api', 'component', 'function']);
/** What the layers take as a module: one of code, and an API description, which holds the endpoints it declares as a module holds its functions. */
const MODULE_KINDS = new Set(['module', 'api']);
/** Lines that carry one request (a Service hands it to the pods it selects), or one piece of data, on through the parts a layer leaves out. */
const FAMILY = { call: 'request', route: 'request', select: 'request', data: 'data' };
/** Parts a request through left-out parts may end at, pulled into the picture: a service outside, a store. */
const PULLABLE = new Set(['external', 'store', 'topic']);

/** What each node is, for the layers: what it is, and what it lies in. */
function classify() {
  S.cls = new Map();
  const sys = S.graph.system;
  const visit = (id, up) => {
    const n = S.byId.get(id);
    // Terraform's own parts: its resources and the run itself, not the machines it declares.
    const iac = /\.(tf|tf\.json|hcl)$/.test(n.source?.file || '') && !PLATFORM.has(n.kind) && !['gateway', 'endpoint', 'store', 'topic', 'external'].includes(n.kind);
    const program = !iac && !up.inProgram && (n.kind === 'service' || n.kind === 'job');
    const kids = S.children.get(id) || [];
    const parts = kids.filter((k) => !isData(k));
    const c = {
      ...up,
      top: n.part_of === sys,
      // At the code layer these stand closed until they are zoomed into or walked through: a module of functions
      // only, and a box of code (a module, a type, a program) whose parts docair only found in the code, with no
      // doc block about any of them (SPEC §3.4).
      leaf:
        ((MODULE_KINDS.has(n.kind) || n.kind === 'component') && kids.length > 0 && kids.every((k) => !(S.children.get(k) || []).length)) ||
        ((CODE_KINDS.has(n.kind) || program) && parts.length > 0 && parts.every((k) => S.byId.get(k)?.origin === 'discovered')),
      iac,
      program,
      library: MODULE_KINDS.has(n.kind) && !up.inProgram && !up.inLibrary && !up.inIac,
      gate: !up.inProgram && (n.kind === 'gateway' || (n.kind === 'endpoint' && up.inCluster)),
    };
    S.cls.set(id, c);
    const down = {
      inCluster: up.inCluster || n.kind === 'cluster',
      inNamespace: up.inNamespace || n.kind === 'namespace',
      inProgram: up.inProgram || program,
      inLibrary: up.inLibrary || c.library,
      inIac: up.inIac || iac,
      inStore: up.inStore || n.kind === 'store',
      inTest: up.inTest || n.kind === 'test',
      inLeaf: up.inLeaf || c.leaf,
    };
    for (const k of kids) visit(k, down);
  };
  const none = { inCluster: false, inNamespace: false, inProgram: false, inLibrary: false, inIac: false, inStore: false, inTest: false, inLeaf: false };
  for (const n of S.graph.nodes) if (n.id !== sys && (!n.part_of || n.part_of === sys || !S.byId.has(n.part_of))) visit(n.id, none);
  S.viewCache = new Map();
  S.layers = LAYERS.filter((L) => L.key === 'overview' || S.graph.nodes.some((n) => S.cls.has(n.id) && LAYER_NEEDS[L.key](n, S.cls.get(n.id))));
}

/** What an Apps picture is about: the programs, the libraries, the stores, the clients and services outside, the test suites. */
function appPart(n, c) {
  if (c.inProgram || c.inLibrary || c.inIac || c.inStore || c.inTest) return false;
  if (c.program || c.library || c.iac) return true;
  if (['store', 'topic', 'external', 'test'].includes(n.kind)) return true;
  // A top-level part of plain code that is no program.
  return c.top && !PLATFORM.has(n.kind) && !c.gate;
}

/**
 * The parts each layer is about (`focus`), the lines that pull other parts in
 * as context (`context`: kinds of lines, null for all, absent for none),
 * whether the layer dissolves the boxes of the machines and the cluster
 * (`flat`), which of its boxes show what is in them (`open`), and whether a
 * request through left-out parts may pull its end in (`pull`).
 */
const RULES = {
  overview: { focus: (n, c) => c.top },
  infra: {
    focus: (n, c) => !c.inProgram && !c.inTest && (c.iac || ((PLATFORM.has(n.kind) || c.gate) && !c.inNamespace && !['release', 'workload', 'container'].includes(n.kind))),
    context: null,
  },
  network: {
    focus: (n, c) => !c.inProgram && !c.inIac && !c.inTest && (c.gate || n.kind === 'network' || n.kind === 'workload' || (n.kind === 'host' && c.inCluster)),
    context: ['call', 'route', 'select', 'data'],
    pull: true,
  },
  k8s: { focus: (n, c) => c.inCluster && !c.inProgram && !c.inTest, context: null },
  apps: {
    focus: appPart,
    flat: true,
    // A library of several crates shows them; one crate is one box.
    open: (n, c) => c.library && (S.children.get(n.id) || []).every((k) => MODULE_KINDS.has(S.byId.get(k)?.kind)),
  },
  // Inside the programs, down to their modules; a module of functions only stands closed (zoom in, or a walk opens it).
  code: { focus: (n, c) => appPart(n, c) || ((c.inProgram || c.inLibrary || c.inIac || c.inStore || c.inTest) && !c.inLeaf), flat: true },
};

/** A layer is offered when the graph has what it is about. */
const LAYER_NEEDS = {
  infra: (n, c) => ['host', 'network', 'cluster'].includes(n.kind) || c.iac,
  network: (n, c) => c.gate,
  k8s: (n) => ['namespace', 'release', 'workload', 'container'].includes(n.kind),
  apps: (n, c) => c.program || c.library || (c.top && CODE_KINDS.has(n.kind)),
  code: (n, c) => CODE_KINDS.has(n.kind) && !c.inIac,
};

const layerName = (key) => LAYERS.find((L) => L.key === key)?.name || key;

/***
 * The picture of one layer: which parts are drawn, which boxes are dissolved
 * (their parts move up into the next box drawn), and which boxes stand closed
 * for what is in them. `opened` boxes show every part inside them.
 *
 * @id      docair-viewer.layer-view
 * @part-of docair-viewer.page
 ***/
function computeView(key, opened = new Set()) {
  const cacheKey = `${key}|${[...opened].sort().join(',')}`;
  if (S.viewCache.has(cacheKey)) return S.viewCache.get(cacheKey);
  const rule = RULES[key] || RULES.overview;
  const sys = S.graph.system;
  const real = (id) => id && id !== sys && S.byId.has(id) && !isData(id);
  const focus = new Set();
  for (const n of S.graph.nodes) if (real(n.id) && rule.focus(n, S.cls.get(n.id))) focus.add(n.id);
  if (rule.open) for (const g of [...focus]) if (rule.open(S.byId.get(g), S.cls.get(g))) (S.children.get(g) || []).forEach((k) => real(k) && focus.add(k));
  for (const g of opened) {
    if (!real(g)) continue;
    focus.add(g);
    (S.children.get(g) || []).forEach((k) => real(k) && focus.add(k));
  }
  const flat = new Set();
  if (rule.flat) for (const n of S.graph.nodes) if (PLATFORM.has(n.kind) && !focus.has(n.id)) flat.add(n.id);
  const holds = new Set();
  for (const id of focus) ancestorsOf(id).forEach((a) => holds.add(a));
  // Context: what a line joins to the parts of this layer, folded into the outermost box that holds none of them.
  const context = new Set();
  if (rule.context !== undefined) {
    const standsFor = (id) => [id, ...ancestorsOf(id)].some((a) => focus.has(a) && !holds.has(a));
    S.graph.edges.forEach((e) => {
      if (rule.context && !rule.context.includes(e.kind)) return;
      for (const [x, y] of [[e.from, e.to], [e.to, e.from]]) {
        if (!focus.has(x) || !real(y) || focus.has(y) || standsFor(y)) continue;
        context.add(contextBox(y, focus, holds, flat));
      }
    });
  }
  const drawn = new Set([...focus, ...context]);
  const v = finishView({ key, opened, focus, flat, drawn });
  // A request that leaves through parts this layer leaves out may end at a service outside: draw it.
  if (rule.pull) {
    const pulled = new Set();
    S.graph.edges.forEach((e, i) => {
      if (!FAMILY[e.kind] || !drawnRepIn(e.from, v) || drawnRepIn(e.to, v)) return;
      for (const end of bridgesFrom(i, v, v.closed, true)) if (end.pulled) pulled.add(contextBox(end.to, focus, holds, flat));
    });
    if (pulled.size) {
      pulled.forEach((id) => drawn.add(id));
      const w = finishView({ key, opened, focus, flat, drawn });
      S.viewCache.set(cacheKey, w);
      return w;
    }
  }
  S.viewCache.set(cacheKey, v);
  return v;
}

/** The outermost box around `id` that holds no part of the layer and is not dissolved: what stands for `id` as context. */
function contextBox(id, focus, holds, flat) {
  let r = id;
  for (let p = S.byId.get(id)?.part_of; p && p !== S.graph.system && !focus.has(p) && !holds.has(p) && !flat.has(p); p = S.byId.get(p)?.part_of) r = p;
  return r;
}

/** Frames, parents and closed boxes of a layer's picture. */
function finishView(v) {
  const sys = S.graph.system;
  // Frames: every box that holds a drawn part, unless the layer dissolves it.
  for (const id of [...v.drawn]) for (const a of ancestorsOf(id)) if (!v.flat.has(a)) v.drawn.add(a);
  v.parent = new Map();
  v.kids = new Map();
  for (const id of v.drawn) {
    let p = S.byId.get(id)?.part_of;
    while (p && p !== sys && !v.drawn.has(p)) p = S.byId.get(p)?.part_of;
    const parent = p && p !== sys ? p : null;
    v.parent.set(id, parent);
    if (parent) {
      if (!v.kids.has(parent)) v.kids.set(parent, []);
      v.kids.get(parent).push(id);
    }
  }
  // A drawn box with nothing drawn inside stands closed for what is in it.
  v.closed = new Set([...v.drawn].filter((id) => !v.kids.has(id) && (S.children.get(id) || []).some((k) => !isData(k))));
  return v;
}

/** The drawn node that stands for `id` in a picture: itself, or the outermost closed box around it; null when the layer leaves it out. */
function drawnRepIn(id, v, closed = v.closed) {
  let r = null;
  for (let cur = id; cur && cur !== S.graph.system; cur = S.byId.get(cur)?.part_of) if (closed.has(cur) && v.drawn.has(cur)) r = cur;
  return r || (v.drawn.has(id) ? id : null);
}

/** The path a request names: `POST /api/ask` → `/api/ask`; a route's: `HTTP /api` → `/api`. */
function routePath(e) {
  const m = /\b(?:GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS|HTTP)\s+(\/[^\s:,()]*)/.exec(`${e.payload || ''} ${e.label || ''}`);
  return m ? m[1] : null;
}

/** Where a request goes on at a fork: to the routes whose path is the longest prefix of its own, and to the lines that name no path. */
function matchRoutes(onward, path) {
  if (!path) return onward;
  const paths = onward.map((j) => routePath(S.graph.edges[j]));
  if (!paths.some(Boolean)) return onward;
  const fits = (p) => p === '/' || path === p || path.startsWith(p.endsWith('/') ? p : `${p}/`);
  const best = paths.filter((p) => p && fits(p)).reduce((a, p) => (p.length > a.length ? p : a), '');
  return onward.filter((j, k) => !paths[k] || (best && paths[k] === best));
}

/** What a left-out part goes by on a line: the outermost left-out box it lies in, or itself. */
function viaPart(id, v) {
  let r = id;
  for (let p = S.byId.get(id)?.part_of; p && p !== S.graph.system && !v.drawn.has(p) && !v.flat.has(p); p = S.byId.get(p)?.part_of) r = p;
  return r;
}

/** The parts a line passes, in a few words: its gateways when it has any (`host-port › traefik › Ingress docair`), else every part. */
function viaWords(ids) {
  const gates = ids.filter((id) => S.byId.get(id)?.kind === 'gateway');
  const names = (gates.length ? gates : ids).map((id) => S.byId.get(id)?.name || id);
  return names.length > 4 ? [names[0], '…', ...names.slice(-2)].join(' › ') : names.join(' › ');
}

/** A line's words in a bundle: a request's method and path (`POST /api/ask`), else its words, kept short. */
function shortWords(e) {
  const path = routePath(e);
  if (path) {
    const said = `${e.payload || ''} ${e.label || ''}`;
    const m = /\b(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\b/.exec(said) || /\b(HTTP)\b/.exec(said);
    return `${m ? m[1] : ''} ${path}`.trim();
  }
  return clip(describeEdge(e), 30);
}

/** A label in lines of about 28 characters, broken between words, so a long label makes the picture taller, not wider. */
function wrapLines(text, width = 28) {
  const lines = [];
  for (const word of text.split(' ')) {
    const last = lines[lines.length - 1];
    if (last !== undefined && last.length + 1 + word.length <= width) lines[lines.length - 1] = `${last} ${word}`;
    else lines.push(word);
  }
  return lines;
}

const clip = (text, max) => (text.length > max ? `${text.slice(0, max - 1)}…` : text);

/**
 * Follow line `i` from a drawn part through parts the picture leaves out, along
 * lines of the same family, to the drawn parts it reaches (at most 8 hops; at a
 * route fork, the route that matches its path). With `pull`, a service outside
 * or a store it reaches counts as reached.
 */
function bridgesFrom(i, v, closed, pull = false) {
  const first = S.graph.edges[i];
  const fam = FAMILY[first.kind];
  if (!fam) return [];
  const path = routePath(first);
  const ends = [];
  const seen = new Set([first.from]);
  let frontier = [{ id: first.to, via: [], hops: [i] }];
  for (let depth = 0; depth < 8 && frontier.length; depth++) {
    const next = [];
    for (const f of frontier) {
      if (seen.has(f.id)) continue;
      seen.add(f.id);
      if (hiddenNode(f.id) || isData(f.id)) continue;
      const at = drawnRepIn(f.id, v, closed);
      if (at) {
        ends.push({ to: f.id, rep: at, via: f.via, hops: f.hops });
        continue;
      }
      if (pull && PULLABLE.has(S.byId.get(f.id)?.kind) && !(S.children.get(f.id) || []).length) {
        ends.push({ to: f.id, rep: f.id, via: f.via, hops: f.hops, pulled: true });
        continue;
      }
      const part = viaPart(f.id, v);
      const via = f.via[f.via.length - 1] === part ? f.via : [...f.via, part];
      const onward = (S.outOf.get(f.id) || []).filter((j) => FAMILY[S.graph.edges[j].kind] === fam);
      // A request that arrives at a box the layer dissolves (a Deployment) ends at the programs drawn inside it.
      if (!onward.length && fam === 'request') {
        for (const id of v.drawn) if (S.cls.get(id)?.program && insideOf(id, f.id)) ends.push({ to: id, rep: drawnRepIn(id, v, closed), via, hops: f.hops });
        continue;
      }
      for (const j of matchRoutes(onward, path)) next.push({ id: S.graph.edges[j].to, via, hops: [...f.hops, j] });
    }
    frontier = next;
  }
  return ends;
}

/** Make the current layer (a walk's, while one plays), with what the reader opened or folded, the picture. */
function applyView() {
  const key = S.flow ? S.flow.layer : S.layer;
  const opened = S.flow ? new Set([...S.flow.opened, ...S.opened]) : S.opened;
  const folded = S.flow ? new Set([...S.flow.folded, ...S.folded]) : S.folded;
  S.view = computeView(key, opened);
  S.collapsed = new Set([...S.view.closed, ...[...folded].filter((g) => S.view.drawn.has(g) && !S.opened.has(g) && (S.children.get(g) || []).length)]);
}

/** The drawn node that stands for `id` now, or null when this layer leaves it out. */
function drawnRep(id) {
  return S.view ? drawnRepIn(id, S.view, S.collapsed) : id;
}

/** A group as text: `kind cluster`, `docair namespace`, `docair-core`. */
function groupLabel(id) {
  const n = S.byId.get(id);
  const plain = n.name.includes(' ') || !['namespace', 'cluster', 'network', 'workload', 'container', 'host', 'release'].includes(n.kind);
  return plain ? n.name : `${n.name} ${n.kind}`;
}

/** The first layer that is about `id` (draws it as one of its own parts), else the first that draws it at all. */
function layerFor(id) {
  const views = S.layers.map((L) => [L.key, computeView(L.key)]);
  return (views.find(([, v]) => v.focus.has(id)) || views.find(([, v]) => drawnRepIn(id, v)) || [null])[0];
}

/***
 * Show the system at one layer: the overview, the infrastructure, the network, Kubernetes, the apps, or the code.
 *
 * Each layer draws what its question is about. Zooming out past the whole
 * picture goes back up (code to apps, any other layer to the overview).
 * Zooming into a closed box opens it.
 *
 * @id      docair-viewer.show-layer
 * @part-of docair-viewer.page
 * @calls   docair-viewer.layer-view "works out what this layer draws"
 * @calls   docair-viewer.render "redraws at the chosen layer"
 ***/
function showLayer(key) {
  if (!S.layers.some((L) => L.key === key)) key = S.layers.some((L) => L.key === 'apps') ? 'apps' : S.layers[0].key;
  S.layer = key;
  S.opened = new Set();
  S.folded = new Set();
  S.focus = null;
  S.tourLevel = null;
  applyView();
  return render();
}

/** The layer to go back up to when zooming out past the whole picture. */
const layerUp = (key) => (key === 'code' && S.layers.some((L) => L.key === 'apps') ? 'apps' : key === 'overview' ? null : 'overview');

function ancestorsOf(id) {
  const out = [];
  for (let cur = S.byId.get(id)?.part_of; cur && cur !== S.graph.system; cur = S.byId.get(cur)?.part_of) out.push(cur);
  return out;
}

/***
 * Zoom into one box: it opens where it is, and the rest of the layer stays as it was.
 *
 * Scrolling out past the picture closes it again, one box at a time.
 *
 * @id      docair-viewer.drill
 * @part-of docair-viewer.page
 * @calls   docair-viewer.render "redraws with the box opened"
 ***/
async function drill(id) {
  if (!(S.children.get(id) || []).some((k) => !isData(k))) return;
  S.focus = id;
  S.tourLevel = null;
  S.opened.add(id);
  S.folded.delete(id);
  // A box this layer leaves out comes in with the boxes around it.
  for (const a of ancestorsOf(id)) {
    if (S.view.drawn.has(a) || S.view.flat.has(a)) continue;
    S.opened.add(a);
    S.folded.delete(a);
  }
  applyView();
  await render();
  const n = S.cy.getElementById(id);
  if (n.nonempty()) S.cy.animate({ fit: { eles: n, padding: 40 } }, { duration: 300 });
  toast(`inside ${groupLabel(id)} · scroll out past the picture to close it`);
}

function drillUp() {
  const at = S.focus;
  if (!at) return Promise.resolve();
  S.opened.delete(at);
  const parent = ancestorsOf(at).find((a) => S.opened.has(a));
  S.focus = parent || null;
  applyView();
  return render().then(() => toast(parent ? `inside ${groupLabel(parent)}` : layerName(S.layer)));
}

/** Where a zoom-in is: Apps › docair-lib › docair-core. */
function renderCrumbs() {
  const box = $('#crumbs');
  if (!box) return;
  box.hidden = !S.focus || !!S.flow;
  if (box.hidden) return;
  const path = [...ancestorsOf(S.focus).filter((a) => S.opened.has(a)).reverse(), S.focus];
  box.replaceChildren(
    el('a', { class: 'link', onclick: () => showLayer(S.layer) }, layerName(S.layer)),
    ...path.flatMap((id) => [' › ', id === S.focus ? el('b', {}, groupLabel(id)) : el('a', { class: 'link', onclick: () => closeInside(id) }, groupLabel(id))]),
  );
}

/** Back to a box on the zoom-in path: the boxes opened inside it close. */
function closeInside(id) {
  for (const g of [...S.opened]) if (g !== id && insideOf(g, id)) S.opened.delete(g);
  S.focus = id;
  applyView();
  return render();
}

/** The layer switch in the header: one button per layer, each says the question it answers. */
function renderLayers() {
  const box = $('#layers');
  if (!box) return;
  box.hidden = S.layers.length < 2;
  const active = S.flow ? S.flow.layer : S.layer;
  box.replaceChildren(
    el('span', { class: 'seg-label' }, 'Layer'),
    ...S.layers.map((L) => {
      // While a walk plays, a layer that draws none of its parts cannot show it.
      const off = !!S.flow && !walkLayers(S.flow).has(L.key);
      return el('button', { class: `seg${active === L.key ? ' on' : ''}`, 'data-layer': L.key, disabled: off, title: off ? `${L.name}: draws none of this walk's parts` : `${L.name}: ${L.ask}`, onclick: () => (S.flow ? setLayer(L.key) : showLayer(L.key)) }, L.name);
    }),
  );
}

/**
 * First view: the apps, when they are readable with the whole picture on the
 * screen; else the first other layer that is; else the overview.
 */
async function firstLayer() {
  const order = ['apps', ...S.layers.map((L) => L.key).filter((k) => k !== 'apps' && k !== 'overview' && k !== 'code')];
  for (const key of order) {
    if (!S.layers.some((L) => L.key === key)) continue;
    S.layer = key;
    applyView();
    const { nodes, edges } = visibleElements();
    // Readable: the smallest words, with the whole picture on the screen (what is inside a box is drawn smaller).
    const first = await topdownFirst(nodes, edges);
    if (first.zoom * Math.min(1, ...(first.scale ? first.scale.values() : [])) >= READABLE_ZOOM) return;
  }
  S.layer = 'overview';
  applyView();
}

/** Zoom at which the drawn graph fits the canvas. */
function wholeFitZoom() {
  const bb = S.cy.elements().not('.token').boundingBox();
  if (!bb.w || !bb.h) return S.cy.zoom();
  return Math.min((S.cy.width() - 72) / bb.w, (S.cy.height() - 72) / bb.h);
}

/** Zoom out: close the box zoomed into, else go up a layer. */
function zoomOut() {
  if (S.focus) return drillUp();
  const up = layerUp(S.layer);
  if (!up) return Promise.resolve();
  return showLayer(up).then(() => toast(`${layerName(up)} · scroll into a box to open it`));
}

/**
 * Semantic zoom: scrolling out past the whole picture closes the box zoomed
 * into, or goes up a layer; scrolling into a closed box opens it.
 */
function wireSemanticZoom() {
  let lock = 0;
  $('#cy').addEventListener(
    'wheel',
    (e) => {
      const now = Date.now();
      if (now < lock || !S.cy || S.flow) return;
      const zoom = S.cy.zoom();
      if (e.deltaY > 0 && zoom < wholeFitZoom() * 0.75 && (S.focus || layerUp(S.layer))) {
        lock = now + 900;
        zoomOut();
      } else if (e.deltaY < 0 && zoom > 1.6 && S.hover && S.collapsed.has(S.hover)) {
        lock = now + 900;
        drill(S.hover);
      }
    },
    { passive: true },
  );
}


let toastTimer = 0;
function toast(text) {
  const t = $('#toast');
  if (!t) return;
  t.textContent = text;
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), 2600);
}

/** What one edge carries, in a few words: a route, a call, a payload, a file. */
function describeEdge(e) {
  if (e.text) return wordsOf(e);
  if (e.kind === 'cover') return `tests ${S.byId.get(e.to)?.name ?? e.to}`;
  const reply = e.reply ? ` → ${e.reply}` : '';
  if (e.kind === 'call') {
    const callee = S.byId.get(e.to);
    if (e.via) return `${e.via.replace(/^http:/, '')}${e.payload ? ` (${e.payload})` : ''}${reply}`;
    if (callee?.kind === 'function') return `${callee.name}(${e.payload ?? ''})${reply}`;
    return `${e.payload ?? callee?.name ?? e.to}${reply}`;
  }
  if (e.label) return e.label;
  if (e.kind === 'emit') return S.byId.get(e.to)?.name || 'emits';
  if (e.kind === 'consume') return S.byId.get(e.from)?.name || 'consumes';
  if (e.kind === 'create') return `creates ${S.byId.get(e.to)?.name ?? e.to}`;
  return VERB[e.kind] || e.kind;
}

/** A line's own words; ✦ marks words the model proposed that are not in the code yet. */
function wordsOf(e) {
  return e.preview ? `✦ ${e.text}` : e.text;
}

/** The label of one edge on screen: calls to functions read `(args) → result`, other hops just say what travels. */
function edgeText(e) {
  if (e.text) return wordsOf(e);
  if (e.kind === 'call' && S.byId.get(e.to)?.kind !== 'function' && (e.payload || e.reply)) {
    return `${e.payload ?? ''}${e.reply ? ` → ${e.reply}` : ''}`.trim();
  }
  return e.label || VERB[e.kind] || e.kind;
}

function summarize(items, max = 3) {
  const unique = [...new Set(items)];
  return unique.length > max ? `${unique.slice(0, max).join(' · ')} +${unique.length - max}` : unique.join(' · ');
}

/** The ancestor of `id` at `depth`, or `id` itself if it is not that deep. */
function ancestorAt(id, depth) {
  let cur = id;
  while (groupDepth(cur) > depth) cur = S.byId.get(cur).part_of;
  return cur;
}

function updateDirButton() {
  const b = $('#btn-dir');
  const arrow = (d) => (d === 'DOWN' ? '↓' : '→');
  if (b) b.textContent = S.direction === 'auto' ? `Layout: auto ${arrow(S.dirUsed)}` : `Layout: ${arrow(S.direction)}`;
}

/**
 * Make every drawn box the box the layout made. Cytoscape sizes a group to
 * its children; the layout's box is larger (the band for its name, room for
 * the lines inside), and its lines end on its border. So each box gets the
 * layout's size as its minimum, with the extra space on the sides where the
 * layout has it, innermost boxes first.
 */
function fitBoxes(pos) {
  const boxes = S.cy.nodes(':parent').sort((a, b) => b.ancestors().length - a.ancestors().length);
  boxes.forEach((n) => {
    const e = pos.get(n.id());
    if (!e) return;
    const kids = n.children().boundingBox({ includeLabels: false, includeOverlays: false });
    const gap = {
      l: Math.max(0, kids.x1 - (e.x - e.w / 2)),
      r: Math.max(0, e.x + e.w / 2 - kids.x2),
      t: Math.max(0, kids.y1 - (e.y - e.h / 2)),
      b: Math.max(0, e.y + e.h / 2 - kids.y2),
    };
    const share = (a, b) => (a + b ? (100 * a) / (a + b) : 50);
    n.style({
      'min-width': kids.w + gap.l + gap.r,
      'min-width-bias-left': `${share(gap.l, gap.r)}%`,
      'min-width-bias-right': `${share(gap.r, gap.l)}%`,
      'min-height': kids.h + gap.t + gap.b,
      'min-height-bias-top': `${share(gap.t, gap.b)}%`,
      'min-height-bias-bottom': `${share(gap.b, gap.t)}%`,
    });
  });
}

/**
 * A route as Cytoscape draws it: both ends pinned where the router put them (left to
 * itself, Cytoscape aims from the node's centre at the first bend, which
 * slants the first and the last piece and tilts the arrow), and the bends in
 * between. A straight route gets its own middle as its one bend.
 */
function routeStyle(src, tgt, pts) {
  const first = pts[0];
  const last = pts[pts.length - 1];
  const inner = pts.length > 2 ? pts.slice(1, -1) : [{ x: (first.x + last.x) / 2, y: (first.y + last.y) / 2 }];
  return {
    srcEnd: `${first.x - src.x}px ${first.y - src.y}px`,
    tgtEnd: `${last.x - tgt.x}px ${last.y - tgt.y}px`,
    ...relativeSegments(src, tgt, inner),
  };
}

/**
 * Express route points the way Cytoscape's `segments` edges want them: a
 * weight along the line from source centre to target centre, and a signed
 * distance perpendicular to it.
 */
function relativeSegments(src, tgt, pts) {
  const dx = tgt.x - src.x;
  const dy = tgt.y - src.y;
  const len2 = dx * dx + dy * dy || 1;
  const len = Math.sqrt(len2);
  const segW = [];
  const segD = [];
  for (const p of pts) {
    const vx = p.x - src.x;
    const vy = p.y - src.y;
    segW.push((vx * dx + vy * dy) / len2);
    segD.push((dx * vy - dy * vx) / len);
  }
  return { segW, segD };
}

/** Fit the graph, but don't blow small graphs up beyond a comfortable size. */
function fitView() {
  S.cy.fit(undefined, 36);
  if (S.cy.zoom() > MAX_FIT_ZOOM) {
    S.cy.zoom(MAX_FIT_ZOOM);
    S.cy.center();
  }
}

function wireGraph() {
  const cy = S.cy;
  cy.on('tap', 'node', (ev) => {
    if (!ev.target.hasClass('token')) select(ev.target.id());
  });
  cy.on('tap', (ev) => {
    if (ev.target === cy) select(null);
  });
  cy.on('dbltap', 'node', (ev) => toggleCollapse(ev.target.id()));
  // A part moved by hand: its lines follow it, routed again as it moves; the other lines keep their ways.
  cy.on('drag', 'node', dragLines);
  cy.on('dragfree', 'node', dropLines);
  cy.on('mouseover', 'edge', (ev) => showTip(ev, edgeTip(ev.target)));
  cy.on('mouseover', 'node', (ev) => {
    if (ev.target.hasClass('token')) return;
    S.hover = ev.target.id();
    showTip(ev, nodeTip(ev.target.id()));
  });
  cy.on('mouseout', 'node', () => (S.hover = null));
  cy.on('mouseout', 'node, edge', hideTip);
  cy.on('pan zoom', hideTip);
}

/** While a part is held: its lines are routed again, at most once a frame, and never without the last move. */
function dragLines() {
  if (!S.drag) S.drag = dragStart();
  if (S.drag.frame) {
    S.drag.late = true;
    return;
  }
  rerouteHere(true);
  S.drag.frame = requestAnimationFrame(() => {
    if (!S.drag) return;
    S.drag.frame = 0;
    if (S.drag.late) {
      S.drag.late = false;
      dragLines();
    }
  });
}

/** The part is put down: its lines take their final ways and their words come back. */
function dropLines() {
  if (!S.drag) return;
  cancelAnimationFrame(S.drag.frame);
  rerouteHere(false);
  S.placed = { pos: S.drag.now, scale: S.placed?.scale ?? null };
  S.drag = null;
}

/** The side of a part a line leaves or enters by: `end` on the part's border, `next` the line's next point. */
function sideOf(end, next) {
  if (Math.abs(next.x - end.x) < 0.5) return next.y < end.y ? 'top' : 'bottom';
  return next.x < end.x ? 'left' : 'right';
}

/** The picture as a move finds it: where everything stands, every line's way, the place of its words. */
function dragStart() {
  const words = new Map();
  S.cy.nodes('.elabel').forEach((l) => words.set(l.data('edge'), { x: l.position('x'), y: l.position('y'), w: l.data('w'), h: l.data('h') }));
  const pos = S.placed?.pos || new Map();
  // The order the lines are routed in while the part moves: shortest first, as the move found them (an
  // order that changed as the part moves would make the lines change their ways with it).
  const rank = new Map();
  S.cy.edges().forEach((e) => {
    const [a, b] = [pos.get(e.source().id()), pos.get(e.target().id())];
    if (a && b) rank.set(e.id(), Math.abs(a.x - b.x) + Math.abs(a.y - b.y));
  });
  return { pos, routes: new Map(S.cy.edges().map((e) => [e.id(), e.data('route')])), words, rank, now: null, frame: 0, late: false, slow: false, key: null, partial: false, fresh: null };
}

/**
 * The lines again while a part is moved by hand, from the picture as the move found it (SPEC §3.5). Only
 * the lines the move touches find a new way: those of what moved (a box that grew or shrank with it
 * counts) and those a moved part now stands on; a line whose two ends moved alike moves along with
 * them. Every other line keeps its way and its words: the picture stays the one the reader knows, and a
 * line pushed aside by the part comes back once the part has passed (incremental routing, as in
 * libavoid, after Wybrow, Marriott & Stuckey). What moved is routed as if it had moved by whole tracks
 * (ROUTER.pitch): it then stands to the tracks around it as before, so its lines keep their shape as it
 * moves instead of flickering between near-equal ways, and they need routing only once it has moved a
 * whole track; their ends are then stretched onto where it stands. `held`: the part is still held, and
 * the moved lines' words wait; then they, and the words a moved part now covers, find their place.
 */
function rerouteHere(held) {
  const cy = S.cy;
  const base = S.drag;
  const snap = (v) => Math.round(v / ROUTER.pitch) * ROUTER.pitch;
  const pos = new Map(); // where everything stands
  const grid = new Map(); // where the router takes it to stand: what moved as a whole, by whole tracks
  const scale = new Map();
  const nodes = [];
  const moved = new Map(); // id → how far it moved (and by whole tracks), whether it changed its size
  cy.nodes().forEach((n) => {
    if (n.hasClass('elabel') || n.hasClass('token')) return;
    const id = n.id();
    const p = { x: n.position('x'), y: n.position('y'), w: n.isParent() ? n.width() : n.data('w'), h: n.isParent() ? n.height() : n.data('h') };
    const q = base.pos.get(id);
    if (q && Math.abs(q.x - p.x) < 0.5 && Math.abs(q.y - p.y) < 0.5 && Math.abs(q.w - p.w) < 0.5 && Math.abs(q.h - p.h) < 0.5) {
      pos.set(id, q);
      grid.set(id, q);
    } else {
      const sized = !q || Math.abs(q.w - p.w) >= 0.5 || Math.abs(q.h - p.h) >= 0.5;
      const m = q ? { dx: p.x - q.x, dy: p.y - q.y, sx: snap(p.x - q.x), sy: snap(p.y - q.y), sized } : null;
      pos.set(id, p);
      grid.set(id, m && !sized ? { ...p, x: q.x + m.sx, y: q.y + m.sy } : p);
      moved.set(id, m);
    }
    scale.set(id, n.data('sc') ?? 1);
    nodes.push({ data: { id, compound: n.isParent(), parent: n.parent().id(), label: n.data('label') } });
  });
  base.now = pos;
  // The room around each moved part: a line that stays may not run through it.
  const rooms = [...moved.keys()].filter((id) => !cy.getElementById(id).isParent()).map((id) => {
    const p = pos.get(id);
    const c = ROUTER.clear * scale.get(id) - 1;
    return { l: p.x - p.w / 2 - c, r: p.x + p.w / 2 + c, t: p.y - p.h / 2 - c, b: p.y + p.h / 2 + c };
  });
  const inRoom = (x1, x2, y1, y2) => rooms.some((m) => x2 > m.l && x1 < m.r && y2 > m.t && y1 < m.b);
  const through = (r) => r.some((q, i) => i > 0 && inRoom(Math.min(q.x, r[i - 1].x), Math.max(q.x, r[i - 1].x), Math.min(q.y, r[i - 1].y), Math.max(q.y, r[i - 1].y)));
  const shift = (r, dx, dy) => r.map((q) => ({ x: q.x + dx, y: q.y + dy }));
  const edges = [];
  const keep = new Map(); // what the router sees of the lines that stay
  const along = new Map(); // lines that move with both their ends: how far
  const redo = new Set();
  cy.edges().forEach((e) => {
    const [s, t] = [e.source().id(), e.target().id()];
    edges.push({ data: { id: e.id(), source: s, target: t, label: (e.data('labelText') || '').replace(/\n/g, ' '), fs: e.data('fs') } });
    const r = base.routes.get(e.id());
    const [ms, mt] = [moved.get(s), moved.get(t)];
    if (r && ms && mt && !ms.sized && !mt.sized && Math.abs(ms.dx - mt.dx) < 0.5 && Math.abs(ms.dy - mt.dy) < 0.5) {
      keep.set(e.id(), shift(r, ms.sx, ms.sy));
      along.set(e.id(), ms);
    } else if (!r || moved.has(s) || moved.has(t) || through(r)) redo.add(e.id());
    else keep.set(e.id(), r);
  });
  // The lines that find a new way, routed only when what moved has moved a whole track, or more lines need
  // it. Where that takes longer than a frame (docair's code layer), they are routed once while the part is
  // held, and to the end once it is put down. They keep the sides they left and entered their parts by,
  // unless another is clearly better.
  const key = [...redo].join(' ') + [...moved.keys()].map((id) => {
    const g = grid.get(id);
    return ` ${id}@${g.x},${g.y},${g.w},${g.h}`;
  }).join('');
  if (redo.size && (base.key !== key || (!held && base.partial))) {
    const sides = new Map();
    for (const id of redo) {
      const r = cy.getElementById(id).data('route');
      if (r?.length > 1) sides.set(id, { s: sideOf(r[0], r[1]), t: sideOf(r[r.length - 1], r[r.length - 2]) });
    }
    const partial = held && base.slow;
    const t0 = performance.now();
    base.fresh = gridLines(nodes, edges, { pos: grid, scale }, { keep, sides, rank: base.rank, rounds: partial ? 1 : ROUTER.rounds }).routes;
    if (held) base.slow = performance.now() - t0 > 16;
    [base.key, base.partial] = [key, partial];
  }
  // A line's end on a moved part: stretched onto where the part stands (by less than half a track).
  const onto = (r, k, next, id) => {
    const [p, g] = [pos.get(id), grid.get(id)];
    if (p === g) return;
    if (Math.abs(r[next].x - r[k].x) < 0.5) r[k] = { x: r[k].x, y: r[k].y + p.y - g.y };
    else r[k] = { x: r[k].x + p.x - g.x, y: r[k].y };
  };
  const routes = new Map();
  cy.edges().forEach((e) => {
    const id = e.id();
    const d = along.get(id);
    if (d) return routes.set(id, shift(base.routes.get(id), d.dx, d.dy));
    if (!redo.has(id)) return routes.set(id, keep.get(id));
    const r = base.fresh?.get(id);
    if (!r) return;
    const out = [...r];
    onto(out, 0, 1, e.source().id());
    onto(out, out.length - 1, out.length - 2, e.target().id());
    routes.set(id, out);
  });
  // Words: those of the lines that stay (moved along with them), but not where a moved part now stands.
  const words = new Map();
  for (const [id, w] of base.words) {
    const d = along.get(id);
    const at = d ? { ...w, x: w.x + d.dx, y: w.y + d.dy } : w;
    if (!redo.has(id) && (d || !inRoom(at.x - at.w / 2, at.x + at.w / 2, at.y - at.h / 2, at.y + at.h / 2))) words.set(id, at);
  }
  const placed = held ? words : placeLabels(nodes, edges.filter((e) => !words.has(e.data.id)), pos, routes, scale, words);
  cy.batch(() => {
    cy.edges().forEach((e) => {
      const id = e.id();
      const r = routes.get(id);
      // drawn from its ends' middles: again where an end moved, even if the way did not change
      const [s, t] = [e.source().id(), e.target().id()];
      if (r && (moved.has(s) || moved.has(t) || (r !== e.data('route') && JSON.stringify(r) !== JSON.stringify(e.data('route'))))) e.data({ routed: true, route: r, ...routeStyle(pos.get(s), pos.get(t), r) });
      const text = e.data('labelText');
      if (!text) return;
      let l = cy.getElementById(`${id}::label`);
      const w = placed.get(id);
      if (!w) return l.addClass('away');
      if (l.empty()) l = cy.add({ group: 'nodes', data: { id: `${id}::label`, label: text, w: w.w, h: w.h, edge: id, ...(e.data('fs') ? { fs: e.data('fs') } : {}) }, classes: `elabel${e.hasClass('bundle') ? ' bundle-label' : ''}` });
      l.removeClass('away');
      l.position({ x: w.x, y: w.y });
    });
  });
  applyFlowClasses();
}

function wireHeader() {
  $('#btn-fit').onclick = fitView;
  $('#btn-dir').onclick = () => {
    S.direction = DIRECTIONS[(DIRECTIONS.indexOf(S.direction) + 1) % DIRECTIONS.length];
    render();
  };
  $('#btn-legend').onclick = () => {
    $('#legend').hidden = !$('#legend').hidden;
    $('#btn-legend').classList.toggle('on', !$('#legend').hidden);
  };
  $('#search').addEventListener('input', applySearch);
  document.addEventListener('keydown', (e) => {
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;
    if (e.key === 'f') fitView();
    if (e.key === '-' && !S.flow) zoomOut();
    if ((e.key === '+' || e.key === '=') && !S.flow) {
      const k = S.layers.findIndex((L) => L.key === S.layer);
      if (k >= 0 && k < S.layers.length - 1) showLayer(S.layers[k + 1].key);
    }
    if (e.key === 'ArrowRight' && S.flow) stepBy(1);
    if (e.key === 'ArrowLeft' && S.flow) stepBy(-1);
    if (e.key === 'Escape') {
      $('#drawer').hidden = true;
      select(null);
    }
  });
}

function toggleCollapse(id) {
  if (!(S.children.get(id) || []).length) return;
  if (S.collapsed.has(id)) {
    S.folded.delete(id);
    S.opened.add(id);
  } else {
    S.opened.delete(id);
    S.folded.add(id);
  }
  applyView();
  render();
}

/***
 * Select a node: highlight its edges and show its summary, docs and tags.
 *
 * @id      docair-viewer.show-details
 * @part-of docair-viewer.page
 * @flow    view 4 "the user selects a node"
 ***/
function select(id, center = false) {
  if (id && !S.flow && S.view && !drawnRep(id) && S.byId.has(id) && !isData(id)) {
    const key = layerFor(id);
    if (key && key !== S.layer) {
      S.selected = id;
      return showLayer(key).then(() => {
        toast(`${layerName(key)} · it shows ${S.byId.get(id).name}`);
        select(id, center);
      });
    }
  }
  S.selected = id;
  const url = id ? `#${encodeURIComponent(id)}` : location.pathname + location.search;
  try {
    history.replaceState(null, '', url);
  } catch (_) {
    /* some file:// contexts forbid it */
  }
  S.cy.$(':selected').unselect();
  highlight(id);
  if (id) {
    const node = S.cy.getElementById(rep(id));
    node.select();
    if (center && node.nonempty()) S.cy.animate({ center: { eles: node }, zoom: Math.max(S.cy.zoom(), 1) }, { duration: 300 });
  }
  document.querySelectorAll('#tree-list .row.sel').forEach((r) => r.classList.remove('sel'));
  const li = id && document.querySelector(`#tree-list li[data-id="${CSS.escape(id)}"]`);
  if (li) {
    for (let p = li.parentElement; p && p.id !== 'tree-list'; p = p.parentElement) p.classList.remove('closed');
    li.querySelector(':scope > .row').classList.add('sel');
    li.scrollIntoView({ block: 'nearest' });
  }
  renderDetails(id);
  const n = id && S.byId.get(id);
  tellParent({ type: 'docair:selected', id: id || null, name: n?.name || null, kind: n?.kind || null });
}

function highlight(id) {
  S.cy.edges().removeClass('hl');
  if (id) S.cy.getElementById(rep(id)).connectedEdges().addClass('hl');
}

function renderDetails(id) {
  if (id && isData(id)) return showData(id);
  renderDetailsNow(id);
  linkTerms($('#details'));
  linkData($('#details'));
}

function renderDetailsNow(id) {
  const box = $('#details');
  box.replaceChildren();
  if (S.flow) box.append(stepCard());
  if (!id) {
    // The walkthroughs to start, if there are any (none: nothing, not an empty list).
    const tours = !S.flow && tourList();
    if (tours) box.append(tours);
    box.append(el('p', { class: 'empty' }, 'Select a node to see its documentation.'));
    return;
  }
  const n = S.byId.get(id);
  const h = S.html[id] || {};
  const k = kindOf(n.kind);
  const docs = h.docs || [];
  const tags = n.tags || [];
  box.append(
    el('h2', {}, el('span', { class: 'glyph' }, KINDS[k].glyph), n.name),
    el(
      'div',
      { class: 'meta' },
      `${n.kind} · `,
      el('span', { class: 'id' }, n.id),
      n.origin !== 'authored' ? [' ', el('span', { class: 'badge' }, n.origin)] : null,
      ' ',
      S.hidden.has(id) || ancestorsOf(id).some((a) => S.hidden.has(a))
        ? el('a', { class: 'link hide-link', onclick: () => setHidden([], [id]), 'data-test': 'show-section', title: 'Bring this part back; if its section is hidden, the section comes back with only this in it' }, 'Show this section')
        : [
            el('a', { class: 'link hide-link', onclick: () => setHidden([id], []), 'data-test': 'hide-section', title: 'Leave this part out of the picture; show it again from the chips above the picture' }, 'Hide this section'),
            ' · ',
            el('a', { class: 'link hide-link', onclick: () => showOnly(id), 'data-test': 'show-only', title: 'Hide everything beside this part and the groups around it' }, 'Show only this'),
          ],
    ),
  );
  const tabs = [
    ['Overview', () => overview(n, h)],
    [`Docs (${docs.length})`, () => docsTab(docs)],
    [`Tags (${tags.length})`, () => tagsTab(tags)],
  ];
  const body = el('div');
  const bar = el('div', { class: 'tabs' });
  tabs.forEach(([label, fill], i) => {
    bar.append(
      el('button', {
        class: i === 0 ? 'on' : '',
        onclick: (e) => {
          bar.querySelectorAll('button').forEach((b) => b.classList.remove('on'));
          e.currentTarget.classList.add('on');
          body.replaceChildren(fill());
        },
      }, label),
    );
  });
  body.append(tabs[0][1]());
  box.append(bar, body);
}

function nodeLink(id) {
  const n = S.byId.get(id);
  return el('a', { class: 'link', onclick: () => select(id, true), title: id }, n ? n.name : id);
}

function overview(n, h) {
  const frag = el('div');
  if (n.summary) frag.append(el('p', { class: 'summary' }, n.summary));
  if (h.description) frag.append(el('div', { class: 'md', html: h.description }));
  const kv = el('div', { class: 'kv' });
  const row = (k, v) => v && kv.append(el('div', { class: 'k' }, k), el('div', {}, v));
  row('symbol', n.symbol && el('code', {}, n.symbol));
  row('signature', n.signature && el('code', {}, `(${n.signature.params.join(', ')})${n.signature.returns ? ` → ${n.signature.returns}` : ''}`));
  row('source', n.source && el('code', {}, `${n.source.file}:${n.source.line}`));
  row('http', n.http && el('code', {}, `${n.http.method} ${n.http.path}`));
  row('part of', n.part_of && nodeLink(n.part_of));
  row('deployed as', n.deployed_as && el('code', {}, n.deployed_as));
  row('runs in', n.runs_in && nodeLink(n.runs_in));
  row('code from', n.code_from && nodeLink(n.code_from));
  const makes = S.graph.edges.filter((e) => e.from === n.id && e.kind === 'create' && isData(e.to));
  row('makes', makes.length && el('span', {}, makes.map((e, i) => [i ? ' ' : '', dataChip(e.to), e.text ? el('span', { class: 'muted' }, ` ${e.text}`) : null])));
  const kids = S.children.get(n.id) || [];
  if (kids.length) {
    row('contains', el('span', {}, `${descendants(n.id)} nodes · `, el('a', { class: 'link', onclick: () => drill(n.id), 'data-test': 'zoom-in' }, 'zoom in'), ' · ', el('a', { class: 'link', onclick: () => toggleCollapse(n.id) }, S.collapsed.has(n.id) ? 'expand' : 'collapse')));
  }
  if (n.test) row('result', el('span', { class: `t-badge t-${n.test.status}` }, `${TEST_ICON[n.test.status] || ''} ${n.test.status} · ${(n.test.duration_ms / 1000).toFixed(1)} s${n.test.at ? ` · ${n.test.at.slice(0, 16).replace('T', ' ')}` : ''}`));
  else if (n.kind === 'test' && !kids.length) row('result', el('span', { class: 't-badge t-none' }, 'no result yet'));
  frag.append(kv);
  if (kids.length) frag.append(talksTo(n.id));
  frag.append(testedBy(n.id));

  const ins = S.graph.edges.filter((e) => e.to === n.id && e.kind !== 'cover');
  const outs = S.graph.edges.filter((e) => e.from === n.id && (e.kind !== 'cover' || n.kind === 'test'));
  const list = (title, es, other) => {
    if (!es.length) return;
    frag.append(el('h3', { class: 'sec' }, `${title} (${es.length})`));
    frag.append(el('ul', { class: 'edges' }, es.map((e) => el('li', { title: `${e.tag} · ${e.source.file}:${e.source.line}` }, el('span', { class: 'ek' }, e.kind), nodeLink(other(e)), el('span', { class: 'badge' }, edgeText(e)), e.text && e.label ? el('span', { class: 'badge code' }, e.label) : null))));
  };
  list('Incoming', ins, (e) => e.from);
  list('Outgoing', outs, (e) => e.to);

  const steps = [];
  S.graph.flows.forEach((f) => f.steps.forEach((s, k) => s.nodes.includes(n.id) && steps.push([f, s, k])));
  if (steps.length) {
    frag.append(el('h3', { class: 'sec' }, 'Flows'));
    frag.append(el('ul', { class: 'edges' }, steps.map(([f, s, k]) => el('li', {}, el('span', { class: 'ek' }, `${f.name} ${s.step}`), el('a', { class: 'link', onclick: () => setFlow(f.name, k) }, s.label || 'play from here')))));
  }
  return frag;
}

/** For a group: every part outside it that something inside it talks to, and what they exchange. */
function talksTo(id) {
  const inside = new Set([id]);
  const add = (x) => (S.children.get(x) || []).forEach((c) => (inside.add(c), add(c)));
  add(id);
  const depth = groupDepth(id);
  const peers = new Map();
  for (const e of S.graph.edges) {
    // Tests are listed under "Tested by", not as communication.
    if (e.kind === 'cover') continue;
    const fromInside = inside.has(e.from);
    if (fromInside === inside.has(e.to)) continue;
    const peer = ancestorAt(fromInside ? e.to : e.from, depth);
    const key = `${fromInside ? 'out' : 'in'}${peer}`;
    if (!peers.has(key)) peers.set(key, { out: fromInside, peer, items: [], sync: false });
    const p = peers.get(key);
    p.items.push(describeEdge(e));
    if (e.kind !== 'emit' && e.kind !== 'consume') p.sync = true;
  }
  // events are asynchronous: ⇢ / ⇠ instead of → / ←
  for (const p of peers.values()) p.arrow = p.out ? (p.sync ? '→' : '⇢') : p.sync ? '←' : '⇠';
  const frag = el('div');
  if (!peers.size) return frag;
  frag.append(el('h3', { class: 'sec' }, `Talks to (${peers.size})`));
  frag.append(
    el('ul', { class: 'edges' }, [...peers.values()].map((p) =>
      el('li', { title: p.out ? 'from inside to this part' : 'from this part to inside' }, el('span', { class: 'ek' }, p.arrow), nodeLink(p.peer), el('span', { class: 'badge' }, summarize(p.items, 4))))),
  );
  return frag;
}

function docsTab(docs) {
  if (!docs.length) return el('p', { class: 'empty' }, 'No Markdown is attached to this node.');
  return el('div', {}, docs.map((d) => [el('div', { class: 'docfile' }, d.anchor ? `${d.file}#${d.anchor}` : d.file), el('div', { class: 'md', html: d.html })]));
}

function tagsTab(tags) {
  if (!tags.length) return el('p', { class: 'empty' }, 'No tags.');
  const known = new Set(['id', 'kind', 'part-of', 'doc', 'deployed-as', 'in', 'out', 'calls', 'reads', 'writes', 'emits', 'consumes', 'http', 'flow', 'creates']);
  return el('table', { class: 'tags' }, tags.map((t) => el('tr', {}, el('td', {}, `@${t.name}`), el('td', {}, t.value, ' ', t.origin === 'inferred' ? el('span', { class: 'badge inferred', title: t.reason || '' }, `ⓘ ${t.reason || 'inferred'}`) : null, !known.has(t.name) ? el('span', { class: 'badge warn' }, '⚠ unknown tag') : null))));
}

function edgeTip(edge) {
  const members = edge.data('members') || [];
  const via = edge.data('via') || [];
  return el('div', {}, via.length ? el('div', { class: 'tip-words' }, `passes ${via.map((id) => S.byId.get(id)?.name || id).join(' › ')}`) : null, members.slice(0, 8).map((i) => {
    const e = S.graph.edges[i];
    const code = e.payload || e.reply ? `${e.payload ? `(${e.payload})` : ''}${e.reply ? ` → ${e.reply}` : ''}` : '';
    return el(
      'div',
      { class: 'tip-edge' },
      el('div', { class: 'tip-words' }, `${S.byId.get(e.from)?.name ?? e.from} ${e.text ? wordsOf(e) : describeEdge(e)}`),
      code ? el('div', { class: 'mono' }, code, e.from_code ? el('span', { class: 'where' }, ' · from the code') : null, e.from_api ? el('span', { class: 'where' }, ` · from ${e.from_api}`) : null) : null,
      e.carries?.length ? el('div', {}, 'carries ', e.carries.map((d, i) => [i ? ', ' : '', `◇ ${S.byId.get(d)?.name ?? d}`])) : null,
      el('code', {}, e.tag),
      el('div', { class: 'where mono' }, `${e.source.file}:${e.source.line} · ${e.origin}${e.via ? ` · via ${e.via}` : ''}`),
    );
  }), members.length > 8 ? el('div', {}, `… ${members.length - 8} more`) : null);
}

function nodeTip(id) {
  const n = S.byId.get(id);
  return el('div', {}, el('b', {}, n.name), ` · ${n.kind}`, n.summary ? el('div', {}, n.summary) : null);
}

function showTip(ev, content) {
  const tip = $('#tip');
  tip.replaceChildren(content);
  const p = ev.renderedPosition || ev.target.renderedPosition();
  tip.style.left = `${Math.min(p.x + 14, $('#graph').clientWidth - 300)}px`;
  tip.style.top = `${p.y + 14}px`;
  tip.hidden = false;
}

function hideTip() {
  $('#tip').hidden = true;
}

function sortedChildren(id) {
  return (S.children.get(id) || []).filter((c) => !isData(c)).sort((a, b) => {
    const ga = (S.children.get(a) || []).length > 0;
    const gb = (S.children.get(b) || []).length > 0;
    if (ga !== gb) return ga ? -1 : 1;
    return S.byId.get(a).name.localeCompare(S.byId.get(b).name);
  });
}

function renderTree() {
  const build = (ids, depth) =>
    el('ul', {}, ids.map((id) => {
      const n = S.byId.get(id);
      const kids = sortedChildren(id);
      const li = el('li', { 'data-id': id, class: kids.length && depth > 0 ? 'closed' : '' });
      const caret = el('span', { class: 'caret' }, kids.length ? (depth > 0 ? '▸' : '▾') : '');
      const eye = el('button', { class: 'eye', title: 'Hide or show this section', 'aria-label': `Hide or show ${n.name}`, 'data-test': 'eye' }, eyeIcon());
      eye.addEventListener('click', (e) => {
        e.stopPropagation();
        const hidden = S.hidden.has(id) || ancestorsOf(id).some((a) => S.hidden.has(a));
        setHidden(hidden ? [] : [id], hidden ? [id] : []);
      });
      const row = el('div', { class: 'row', title: n.summary || n.id }, caret, el('span', { class: 'glyph' }, KINDS[kindOf(n.kind)].glyph), el('span', { class: 'name' }, n.name), eye);
      caret.addEventListener('click', (e) => {
        e.stopPropagation();
        li.classList.toggle('closed');
        caret.textContent = li.classList.contains('closed') ? '▸' : '▾';
      });
      row.addEventListener('click', () => select(id, true));
      li.append(row);
      if (kids.length) li.append(build(kids, depth + 1));
      return li;
    }));
  $('#tree-list').replaceChildren(build(sortedChildren(S.graph.system), 0));
  markHiddenRows();
}

function applySearch() {
  const q = $('#search').value.trim().toLowerCase();
  const match = (id) => {
    const n = S.byId.get(id);
    return [n.id, n.name, n.summary].some((s) => (s || '').toLowerCase().includes(q));
  };
  const visit = (li) => {
    const id = li.dataset.id;
    let any = !q || match(id);
    for (const child of li.querySelectorAll(':scope > ul > li')) any = visit(child) || any;
    li.classList.toggle('miss', !any);
    if (q && any && li.querySelector(':scope > ul')) li.classList.remove('closed');
    return any;
  };
  document.querySelectorAll('#tree-list > ul > li').forEach(visit);
  S.cy.nodes().removeClass('hit dim');
  if (!q) return;
  S.cy.nodes().forEach((n) => {
    if (n.hasClass('token') || n.hasClass('elabel') || n.isParent()) return;
    n.addClass(match(n.id()) ? 'hit' : 'dim');
  });
}

function renderDiagButton() {
  const d = S.graph.diagnostics;
  const errors = d.filter((x) => x.severity === 'error').length;
  const warnings = d.length - errors;
  const b = $('#btn-diag');
  b.textContent = d.length ? [errors ? `✖ ${errors}` : '', warnings ? `⚠ ${warnings}` : ''].join(' ').trim() : '✓ 0 issues';
  b.title = d.length ? `${errors} errors, ${warnings} warnings from docair check` : 'docair check found nothing to report';
  b.className = `btn ${errors ? 'diag-error' : warnings ? 'diag-warning' : 'diag-ok'}`;
  b.onclick = () => {
    const drawer = $('#drawer');
    drawer.hidden = !drawer.hidden;
    if (drawer.hidden) return;
    drawer.replaceChildren(
      ...(d.length
        ? d.map((x) => el('div', { class: 'row', onclick: () => { if (x.node && S.byId.has(x.node)) select(x.node, true); drawer.hidden = true; } },
            el('span', { class: x.severity }, x.severity === 'error' ? '✖' : '⚠'),
            el('span', { class: `${x.severity} mono` }, x.code),
            el('span', {}, x.message),
            el('span', { class: 'where' }, x.file ? `${x.file}:${x.line}` : '')))
        : [el('div', { class: 'none' }, 'docair check found nothing to report.')]),
    );
  };
}

function renderLegend() {
  const kinds = [...new Set(S.graph.nodes.filter((n) => n.id !== S.graph.system).map((n) => kindOf(n.kind)))];
  const edges = [...new Set(S.graph.edges.map((e) => e.kind))];
  $('#legend').replaceChildren(
    el('h4', {}, 'Nodes'),
    el('div', { class: 'grid' }, kinds.map((k) => [el('span', { class: 'sw', style: `background: var(--k-${k}-bg); border-color: var(--k-${k}-line)` }), el('span', {}, `${KINDS[k].glyph} ${k}`)])),
    el('h4', {}, 'Edges'),
    el('div', { class: 'grid' }, edges.map((k) => [el('span', { class: 'ln', style: `border-top-color: var(--e-${k}); border-top-style: ${EDGE_LINE[k] || 'solid'}` }), el('span', {}, k)])),
  );
}

/** Scenarios (@flow) and recorded walkthroughs, as one list of tours with the same step shape. */
function buildTours() {
  const nameOf = (id) => S.byId.get(id)?.name || id;
  const walks = S.graph.walkthroughs.map((w) => ({
    key: `walk:${w.id}`,
    name: w.id,
    source: 'walk',
    title: w.title || w.id,
    question: w.question,
    summary: w.summary,
    recorded: w.recorded,
    file: w.file,
    steps: (w.steps || [])
      .map((st, k) => ({
        step: k + 1,
        title: st.title || '',
        text: st.text || '',
        nodes: (st.nodes || []).filter((id) => S.byId.has(id)),
        // Bind by the edges' ends (as the backend does), so a walkthrough fits a regenerated graph too.
        path: Array.isArray(st.edges) && st.edges.length ? bindEdges(st.edges) : (st.path || []).filter((i) => Number.isInteger(i) && i >= 0 && i < S.graph.edges.length),
      }))
      .filter((st) => st.nodes.length || st.path.length),
  }));
  const flows = S.graph.flows.map((f) => ({
    key: `flow:${f.name}`,
    name: f.name,
    source: 'flow',
    title: f.title || f.name,
    summary: f.description || '',
    steps: f.steps.map((st) => ({ step: st.step, title: st.label || st.nodes.map(nameOf).join(', '), text: '', nodes: st.nodes, path: st.path })),
  }));
  S.tours = [...flows, ...walks.filter((t) => t.steps.length)];
}

/** Edge indices for `{from, to}` refs: every edge from → to, else every edge to → from (SPEC §10.3). */
function bindEdges(refs) {
  const out = [];
  for (const r of refs) {
    let found = [];
    S.graph.edges.forEach((e, i) => e.from === r.from && e.to === r.to && found.push(i));
    if (!found.length) S.graph.edges.forEach((e, i) => e.from === r.to && e.to === r.from && found.push(i));
    found.forEach((i) => !out.includes(i) && out.push(i));
  }
  return out;
}

function findTour(key) {
  return S.tours.find((t) => t.key === key) || S.tours.find((t) => t.source === 'flow' && t.name === key) || null;
}

const tourGlyph = (t) => (t.source === 'walk' ? '✦' : t.source === 'data' ? '◇' : '▶');

function renderFlowBar() {
  const bar = $('#flow');
  bar.replaceChildren();
  if (!S.tours.length) {
    bar.append(el('span', { class: 'stats' }, 'No scenarios or walkthroughs yet. Tag steps with @flow, or ask: docair ask "How does … work?"'));
    return;
  }
  const flows = S.tours.filter((t) => t.source === 'flow');
  const walks = S.tours.filter((t) => t.source === 'walk');
  const data = S.tours.filter((t) => t.source === 'data');
  const option = (t) => el('option', { value: t.key }, `${tourGlyph(t)} ${t.title} (${t.steps.length} steps)`);
  bar.append(
    el(
      'select',
      { id: 'flow-select', 'aria-label': 'Walk me through', onchange: (e) => setFlow(e.target.value) },
      el('option', { value: '' }, 'walk: none'),
      flows.length ? el('optgroup', { label: 'Scenarios (@flow)' }, flows.map(option)) : null,
      walks.length ? el('optgroup', { label: 'Walkthroughs (asked)' }, walks.map(option)) : null,
      data.length ? el('optgroup', { label: 'Following data' }, data.map(option)) : null,
    ),
    el('button', { class: 'btn', id: 'flow-prev', title: 'Previous step (←)', onclick: () => stepBy(-1) }, '◀'),
    el('button', { class: 'btn', id: 'flow-play', onclick: togglePlay }, 'Play'),
    el('button', { class: 'btn', id: 'flow-next', title: 'Next step (→)', onclick: () => stepBy(1) }, '▶'),
    el('div', { id: 'steps' }),
    el('div', { id: 'step-label' }),
  );
  if (S.flow) $('#flow-select').value = S.flow.key;
}

/***
 * Start or stop a walkthrough: a @flow scenario or a recorded one. Closing it restores the level from before.
 *
 * @id      docair-viewer.walk
 * @part-of docair-viewer.page
 * @calls   docair-viewer.play-step "steps through the walkthrough"
 ***/
async function setFlow(key, step = 0) {
  const base = key ? findTour(key) : null;
  if (base && !S.flow) {
    S.beforeTour = { layer: S.layer, opened: new Set(S.opened), folded: new Set(S.folded), focus: S.focus };
    S.opened = new Set();
    S.folded = new Set();
  }
  // A walk plays at one layer, with a focus (SPEC §10.4): the least detailed layer where it still has steps.
  S.walkStack = [];
  const tour = base ? projectTour(base, defaultLayer(base, null), null) : null;
  S.flow = tour;
  S.playing = false;
  if ($('#flow-select')) $('#flow-select').value = S.flow ? S.flow.key : '';
  updatePlay();
  if (!S.flow) {
    S.step = -1;
    S.tokenRun++;
    S.tourGroups = null;
    S.tourLevel = null;
    S.needed = new Set();
    S.cy.$('node.token').remove();
    renderSteps();
    const before = S.beforeTour;
    S.beforeTour = null;
    if (before) {
      Object.assign(S, { layer: before.layer, opened: before.opened, folded: before.folded, focus: before.focus });
      applyView();
      await render();
    } else applyFlowClasses();
    renderDetails(S.selected);
    return;
  }
  S.tourGroups = tourGroups(S.flow.base);
  S.lastFramed = null;
  if (S.following && S.flow.source !== 'data') {
    S.following = null;
    renderFollowBar();
  }
  const needed = new Set([...S.tourGroups]);
  S.flow.base.steps.forEach((st) => stepIds(st).forEach((id) => needed.add(id)));
  const shown = [...needed].filter((id) => S.hidden.has(id) || (!S.showTests && S.byId.get(id)?.kind === 'test'));
  S.needed = needed;
  if (shown.length) toast(`showing ${shown.map((id) => S.byId.get(id)?.name || id).join(', ')} for this walkthrough`);
  await goStep(step);
}

/** A walk's picture at one layer: the layer, its focus box opened down to the parts the walk passes, and every box the walk never touches folded, as context. */
function walkView(base, key, focus) {
  const opened = new Set();
  // At the code layer, a walk opens the modules it passes through, down to its functions.
  if (key === 'code') for (const st of base.steps) for (const id of stepIds(st)) ancestorsOf(id).forEach((a) => S.cls.get(a)?.leaf && opened.add(a));
  if (focus) {
    opened.add(focus);
    for (const st of base.steps) {
      for (const id of stepIds(st)) {
        if (id === focus || !insideOf(id, focus)) continue;
        for (const a of ancestorsOf(id)) {
          if (a === focus) break;
          opened.add(a);
        }
      }
    }
  }
  const view = computeView(key, opened);
  const touched = tourGroups(base);
  const folded = new Set([...view.drawn].filter((g) => !touched.has(g) && (S.children.get(g) || []).length));
  return { view, opened, folded, collapsed: new Set([...view.closed, ...folded]) };
}

const insideOf = (id, box) => id === box || ancestorsOf(id).includes(box);

/***
 * A walk at one layer (SPEC §10.4): each step's parts are shown as the boxes they lie in at that
 * layer, and consecutive steps in the same boxes become one step, so the picture stays put and
 * the walk moves from box to box. Steps only in parts the layer leaves out (the hops of a request
 * through the network, in the apps) belong to the step before them. A step that hides inner steps
 * can be opened.
 *
 * @id      docair-viewer.layer
 * @part-of docair-viewer.page
 * @calls   docair-viewer.layer-view "works out what the layer draws"
 ***/
function projectTour(base, key, focus) {
  const w = walkView(base, key, focus);
  const steps = [];
  let before = [];
  base.steps.forEach((st, k) => {
    const own = stepIds(st).filter((id) => !isData(id));
    const reps = [...new Set(own.map((id) => drawnRepIn(id, w.view, w.collapsed)).filter(Boolean))];
    const last = steps[steps.length - 1];
    if (!reps.length) {
      // Nothing of this step is drawn at this layer: it happens on the way, in the step before.
      if (last) {
        last.inner.push(k);
        last.path = [...new Set([...last.path, ...st.path])];
        last.passes = true;
      } else before.push(k);
      return;
    }
    const key2 = reps.slice().sort().join('|');
    if (last && last.key === key2) {
      last.inner.push(k);
      last.path = [...new Set([...last.path, ...st.path])];
      return;
    }
    const inner = [...before, k];
    steps.push({ key: key2, nodes: reps, path: [...new Set(inner.flatMap((j) => base.steps[j].path))], inner, passes: before.length > 0 });
    before = [];
  });
  steps.forEach((s, j) => {
    const first = base.steps[s.inner[0]];
    // The box a step happens in, when its parts are hidden in one: what Open walks into.
    const hiding = s.nodes.filter((r) => s.inner.some((k) => stepIds(base.steps[k]).some((id) => id !== r && insideOf(id, r))));
    s.box = hiding.length === 1 ? hiding[0] : null;
    s.deeper = hiding.length > 0 || s.passes;
    s.title = s.inner.length > 1 && s.box && !s.passes ? `Inside ${groupLabel(s.box)}: ${first.title}` : first.title;
    s.text = s.inner.length > 1 ? '' : first.text;
    s.step = j + 1;
  });
  return { ...base, steps, layer: key, focus, view: w.view, opened: w.opened, folded: w.folded, collapsed: w.collapsed, base };
}

/**
 * The layer that suits a walk: the one that shows most of its steps as its
 * own parts, drawn as themselves (the apps also own the code inside them),
 * and has to leave out, or show as mere context, the fewest; on a tie, the
 * one with fewer steps. It needs a few steps, and the code layer is only for
 * walks the apps fold into too few. The overview only when nothing else can
 * show the walk.
 */
function defaultLayer(base, focus, skip = null) {
  const want = Math.min(3, base.steps.length);
  let best = null;
  for (const L of S.layers) {
    if (L.key === 'overview' || L.key === 'code' || L.key === skip) continue;
    const t = projectTour(base, L.key, focus);
    if (t.steps.length < want) continue;
    let about = 0;
    for (const st of base.steps) {
      const ids = stepIds(st).filter((id) => !isData(id));
      if (!ids.length) continue;
      const reps = ids.map((id) => [id, drawnRepIn(id, t.view, t.collapsed)]);
      const own = reps.some(([id, r]) => r && t.view.focus.has(r) && (r === id || (L.key === 'apps' && (S.cls.get(id).inProgram || S.cls.get(id).inLibrary))));
      if (own) about++;
      else if (!reps.some(([, r]) => r && t.view.focus.has(r))) about--;
    }
    const score = [about, -t.steps.length];
    if (!best || score[0] > best.score[0] || (score[0] === best.score[0] && score[1] > best.score[1])) best = { key: L.key, score };
  }
  if (best) return best.key;
  // No layer has a few steps: the one that has the most, the code first on a tie.
  const counts = S.layers
    .filter((L) => L.key !== skip)
    .map((L) => [L.key, projectTour(base, L.key, focus).steps.length + (L.key === 'code' ? 0.5 : 0)])
    .sort((a, b) => b[1] - a[1]);
  return counts.length && counts[0][1] >= 1 ? counts[0][0] : 'overview';
}

/** The layers at which a walk has steps: at the others, none of its parts is drawn. */
function walkLayers(t) {
  if (!t.layersWith) t.layersWith = new Set(S.layers.filter((L) => L.key === t.layer || projectTour(t.base, L.key, t.focus).steps.length).map((L) => L.key));
  return t.layersWith;
}

/** Show the walk at another layer, at the step that holds the base step shown now. */
function setLayer(key) {
  if (!S.flow) return;
  if (!walkLayers(S.flow).has(key)) {
    toast(`${layerName(key)} draws none of this walk's parts`);
    renderLayers();
    return Promise.resolve();
  }
  const at = S.flow.steps[S.step]?.inner[0] ?? 0;
  S.flow = projectTour(S.flow.base, key, S.flow.focus);
  const k = S.flow.steps.findIndex((st) => st.inner.includes(at));
  return goStep(Math.max(0, k));
}

/** Open a step: walk its inner steps inside its box, or, for steps on the way, at the layer that is about them. */
function openStep() {
  const st = S.flow?.steps[S.step];
  if (!st || !st.deeper) return;
  S.walkStack.push({ tour: S.flow, step: S.step });
  const outer = S.flow.base;
  if (!outer.messages) outer.messages = walkMessages(outer.steps);
  // The inner steps keep the outer walk's messages: answers come back where they do in the whole walk.
  const base = { ...outer, steps: st.inner.map((k) => outer.steps[k]), messages: st.inner.map((k) => outer.messages[k]), title: st.title };
  // One box: the same layer, with the box opened down to the parts the inner steps pass.
  // Steps on the way: the layer that is about them.
  const inner = st.box && !st.passes ? projectTour(base, S.flow.layer, st.box) : projectTour(base, defaultLayer(base, null, S.flow.layer), null);
  if (!inner.steps.length) {
    S.walkStack.pop();
    toast('no layer draws these steps');
    return Promise.resolve();
  }
  S.flow = inner;
  S.flow.key = S.walkStack[0].tour.key;
  return goStep(0);
}

/** Back to the walk the opened step belongs to. */
function closeStep() {
  const back = S.walkStack.pop();
  if (!back) return;
  S.flow = back.tour;
  return goStep(back.step);
}

/** The groups a tour touches: every ancestor of every node it names or passes. */
function tourGroups(tour) {
  const out = new Set();
  for (const st of tour.steps) {
    const ids = [...st.nodes, ...st.path.flatMap((i) => [S.graph.edges[i].from, S.graph.edges[i].to])];
    for (const id of ids) {
      if (S.children.has(id)) out.add(id);
      ancestorsOf(id).forEach((a) => out.add(a));
    }
  }
  return out;
}

function stepIds(st) {
  const ids = st.nodes.filter((id) => S.byId.has(id));
  if (ids.length) return ids;
  return [...new Set(st.path.flatMap((i) => [S.graph.edges[i].from, S.graph.edges[i].to]))];
}

/**
 * Go to the level of the walk so far: the least detailed one that shows the
 * nodes of this step and of every step before it as themselves. The level
 * only goes deeper as the walk goes on, so the picture stays put while the
 * camera moves; going back comes back up. Parts of the system the walk never
 * touches stay folded, as context. Then the camera fits the step.
 */
async function followStep() {
  const st = S.flow.steps[S.step];
  const ids = stepIds(st);
  const level = S.flow.layer;
  const want = new Set(S.flow.collapsed);
  const changed = S.view !== S.flow.view || want.size !== S.collapsed.size || [...want].some((g) => !S.collapsed.has(g));
  const levelChanged = level !== S.tourLevel;
  S.tourLevel = level;
  if (changed) {
    S.view = S.flow.view;
    S.collapsed = want;
    S.opened = new Set();
    S.folded = new Set();
    S.focus = null;
    // Re-laid out, the picture keeps the reader's place: it opens on the
    // previous step's parts in their new positions, then glides on.
    S.keepView = !!S.lastFramed;
    try {
      await render();
    } finally {
      S.keepView = false;
    }
    const before = S.lastFramed && cameraFor(stepElements(S.lastFramed, []));
    if (before) S.cy.viewport({ zoom: Math.max(before.zoom, TOUR_MIN_ZOOM), pan: before.zoom >= TOUR_MIN_ZOOM ? before.pan : cameraAt(before.center, TOUR_MIN_ZOOM).pan });
  } else {
    // A step before this one may still be laying out the same picture.
    await S.lastRender;
    renderLayers();
  }
  if (levelChanged && S.layers.length > 1) toast(`${layerName(level)} · ${LAYERS.find((L) => L.key === level)?.ask || ''}`);
  frameStep(ids, st.path);
  S.lastFramed = ids;
}

/** The drawn elements of a step: its nodes, and the edges that lead to them with their ends. */
function stepElements(ids, path) {
  const cy = S.cy;
  let eles = cy.collection();
  for (const id of ids) eles = eles.union(cy.getElementById(rep(id)));
  for (const i of path) {
    const e = cy.getElementById(S.edgeOf.get(i) || '');
    if (e.nonempty()) eles = eles.union(e).union(e.connectedNodes());
  }
  return eles;
}

/** The camera that fits these elements (not capped below), or null. */
function cameraFor(eles, pad = 70) {
  if (!eles || eles.empty()) return null;
  const cy = S.cy;
  const bb = eles.boundingBox({ includeLabels: true });
  // Top-down: parts drawn smaller need the camera closer.
  const sc = Math.min(1, ...eles.nodes().map((n) => n.data('sc') ?? 1));
  const zoom = Math.min(TOUR_MAX_ZOOM / sc, (cy.width() - 2 * pad) / Math.max(bb.w, 1), (cy.height() - 2 * pad) / Math.max(bb.h, 1));
  const center = { x: bb.x1 + bb.w / 2, y: bb.y1 + bb.h / 2 };
  return { zoom, center, pan: { x: cy.width() / 2 - zoom * center.x, y: cy.height() / 2 - zoom * center.y } };
}

function cameraAt(point, zoom) {
  const cy = S.cy;
  return { zoom, center: point, pan: { x: cy.width() / 2 - zoom * point.x, y: cy.height() / 2 - zoom * point.y } };
}

function moveCamera(cam, duration = 450) {
  S.cy.stop();
  S.cy.animate({ zoom: cam.zoom, pan: cam.pan }, { duration, easing: 'ease-in-out-cubic' });
}

/**
 * Bring the step into view at a zoom the reader can read.
 *
 * If the step and the edges leading to it fit at a readable zoom, the camera
 * shows them all. If not, it never zooms out further: it starts where the
 * step's first edge starts, follows the token along the edges (playMessages),
 * and comes to rest on the step's own parts, or on its first part when even
 * those do not fit. "Whole step" in the card shows everything at once.
 */
function frameStep(ids, path) {
  S.follow = null;
  const eles = stepElements(ids, path);
  const all = cameraFor(eles);
  if (!all) return;
  // Top-down: parts drawn smaller are readable only closer.
  const readable = TOUR_MIN_ZOOM / Math.min(1, ...eles.nodes().map((n) => n.data('sc') ?? 1));
  if (all.zoom >= readable) return moveCamera(all);
  const own = cameraFor(stepElements(ids, []));
  const first = S.cy.getElementById(rep(ids[0] || ''));
  const rest = own && own.zoom >= readable ? own : cameraAt(first.nonempty() ? first.position() : all.center, readable);
  const edge = path.length ? S.cy.getElementById(S.edgeOf.get(path[0]) || '') : S.cy.collection();
  const start = edge.nonempty() ? edge.sourceEndpoint() : null;
  if (start && Number.isFinite(start.x)) {
    S.follow = { zoom: readable, rest };
    moveCamera(cameraAt(start, readable));
  } else moveCamera(rest);
}

/** "Whole step": everything the step touches, however small. */
function showWholeStep() {
  const st = S.flow?.steps[S.step];
  const cam = st && cameraFor(stepElements(stepIds(st), st.path));
  if (cam) moveCamera({ ...cam, zoom: Math.max(S.cy.minZoom(), cam.zoom), pan: cameraAt(cam.center, Math.max(S.cy.minZoom(), cam.zoom)).pan });
}

function renderSteps() {
  const box = $('#steps');
  const label = $('#step-label');
  if (!box || !label) return;
  box.replaceChildren();
  label.replaceChildren();
  if (!S.flow) return;
  S.flow.steps.forEach((st, k) => {
    if (k) box.append(el('span', { class: `bar ${k <= S.step ? 'done' : ''}` }));
    box.append(el('button', { class: `dot ${k < S.step ? 'done' : k === S.step ? 'cur' : ''}`, title: `${k + 1}: ${st.title}`, onclick: () => goStep(k) }));
  });
  const st = S.flow.steps[S.step];
  if (st) label.append(el('b', {}, `${S.step + 1}/${S.flow.steps.length}`), ` ${st.title} `, el('span', { class: 'stats' }, st.nodes.map((id) => S.byId.get(id)?.name || id).join(', ')));
}

function stepBy(d) {
  if (!S.flow) return S.tours.length ? setFlow(S.tours[0].key) : undefined;
  return goStep(Math.max(0, Math.min(S.flow.steps.length - 1, S.step + d)));
}

/***
 * Show one step: go to its level, frame it, mark its nodes, tell it in words, and move a token along the edges that lead to it.
 *
 * @id      docair-viewer.play-step
 * @part-of docair-viewer.page
 * @flow    ask 10 "the viewer replays it on the chart"
 * @flow    view 5 "the user plays a flow"
 ***/
async function goStep(k) {
  if (!S.flow || !S.flow.steps.length) return;
  S.step = Math.max(0, Math.min(S.flow.steps.length - 1, k));
  const tour = S.flow;
  renderSteps();
  await followStep();
  if (S.flow !== tour) return;
  applyFlowClasses();
  applyFollow();
  S.hop = -1;
  renderDetails(S.selected);
  await playMessages(drawnMessages(S.flow.steps[S.step]).shown);
}

/** The messages of a step that cross between boxes at this layer; the rest happen inside a box. */
function drawnMessages(st) {
  const all = stepMessages(st);
  // A message along a line drawn through left-out parts arrives where that line does.
  const shown = all.filter((m) => S.edgeOf.has(m.i)).map((m) => (S.bridgeOf.has(m.i) ? { ...m, end: S.bridgeOf.get(m.i) } : m));
  return { shown, inside: all.length - shown.length };
}

const MSG_ICON = { call: '→', return: '↩', read: '⇠', write: '✎', emit: '⇢', consume: '⇢', data: '→', create: '✚', cover: '✓', route: '⇶', select: '⇶' };

/***
 * The messages of a whole walk, step by step, in the order they happen (SPEC §7.3).
 *
 * One call stack for the whole walk, like a debugger: a call stays open while
 * the walk goes on inside it, in this step or in later ones. Its answer goes
 * back when the walk moves on from the part that was called (the next line
 * starts from a caller further down the stack, or from a part not on it), but
 * never while that part still has work later in the walk: a pipeline whose
 * stages hand data to each other keeps its orchestrator open until its last
 * call. A later step naming the same call again (the reply travelling back)
 * and the end of the walk close calls in any case, innermost first.
 *
 * @id      docair-viewer.messages
 * @part-of docair-viewer.page
 ***/
function walkMessages(steps) {
  const seq = steps.flatMap((st, k) => st.path.map((i) => ({ k, i, e: S.graph.edges[i] }))).filter((m) => m.e);
  const per = steps.map(() => []);
  const stack = [];
  const answer = (k, c) => c.e.reply && per[k].push({ kind: 'return', i: c.i, e: c.e });
  // Whether the part a call reached still sends something later, before the same call comes up again.
  const busy = (c, j) => {
    for (let x = j; x < seq.length; x++) {
      if (seq[x].i === c.i) return false;
      if (seq[x].e.from === c.e.to) return true;
    }
    return false;
  };
  const unwind = (k, depth, j = seq.length) => {
    while (stack.length > depth && !busy(stack[stack.length - 1], j)) answer(k, stack.pop());
  };
  const close = (k, depth) => {
    while (stack.length > depth) answer(k, stack.pop());
  };
  seq.forEach(({ k, i, e }, j) => {
    // The same call named again: its answer is on its way back.
    const again = stack.findIndex((c) => c.i === i);
    if (again >= 0) return close(k, again);
    // Inside the callee of an open call: the calls above it have answered.
    const callee = stack.map((c) => c.e.to).lastIndexOf(e.from);
    if (callee >= 0) unwind(k, callee + 1, j);
    else {
      // A caller further down makes another call: what it called before has answered.
      const caller = stack.findIndex((c) => c.e.from === e.from);
      unwind(k, caller >= 0 ? caller : 0, j);
    }
    per[k].push({ kind: e.kind, i, e });
    if (e.kind === 'call') stack.push({ i, e });
  });
  if (steps.length) close(steps.length - 1, 0);
  return per;
}

/** The messages of a step: from its walk, so answers come back when they really do. */
function stepMessages(st) {
  const base = S.flow?.base;
  if (base && st.inner) {
    if (!base.messages) base.messages = walkMessages(base.steps);
    return st.inner.flatMap((k) => base.messages[k] || []);
  }
  return walkMessages([st])[0];
}

/** A node's name, with its group's when several nodes share it: `ConversationService.ask`. */
function nameOf(id) {
  const n = S.byId.get(id);
  if (!n) return id;
  if (!S.sameName) {
    S.sameName = new Map();
    for (const x of S.graph.nodes) S.sameName.set(x.name, (S.sameName.get(x.name) || 0) + 1);
  }
  if ((S.sameName.get(n.name) || 0) < 2) return n.name;
  const parent = S.byId.get(n.part_of);
  return parent && parent.id !== S.graph.system ? `${parent.name}.${n.name}` : n.name;
}

/** One message in words: who does what to whom, why, and what travels. */
function messageParts(m) {
  const e = m.e;
  const a = nameOf(e.from);
  // A line drawn through parts this layer leaves out: it arrives where the line on the chart does.
  const b = m.end ? `${nameOf(m.end.to)} (via ${viaWords(m.end.via)})` : nameOf(e.to);
  const data = e.payload || (e.kind !== 'call' ? e.label : null);
  switch (m.kind) {
    case 'call':
      return { who: [a, 'calls', b], why: e.text, carry: [e.payload && `sends ${e.payload}`, e.reply && `gets ${e.reply} back`].filter(Boolean).join(' · '), token: e.payload || '', has: null };
    case 'return':
      return { who: [b, 'answers', a], why: `with ${e.reply}`, carry: '', token: e.reply, has: `${e.reply} from ${b}` };
    case 'read':
      return { who: [b, 'reads', a], why: e.text, carry: data ? `gets ${data}` : '', token: data || '', has: `${data || 'the data'} from ${a}` };
    case 'write':
      return { who: [a, 'writes to', b], why: e.text, carry: data ? `stores ${data}` : '', token: data || '', has: `${data || 'its data'} stored in ${b}` };
    case 'emit':
      return { who: [a, 'publishes to', b], why: e.text, carry: data ? `the event ${data}` : '', token: data || '', has: null };
    case 'consume':
      return { who: [b, 'takes from', a], why: e.text, carry: data ? `the event ${data}` : '', token: data || '', has: `${data || 'the event'} from ${a}` };
    case 'create':
      return { who: [a, 'creates', b], why: e.text, carry: data || '', token: data || '', has: `${b} exists` };
    case 'cover':
      return { who: [a, 'tests', b], why: e.text, carry: '', token: '', has: null };
    case 'data':
      return { who: [a, 'hands over to', b], why: e.text, carry: data || '', token: data || '', has: data ? `${data} at ${b}` : null };
    default:
      return { who: [a, VERB[m.kind] || m.kind, b], why: e.text, carry: data || '', token: data || '', has: null };
  }
}

/** Mark the message that is moving now in the step card, without redrawing the card. */
function setHop(k) {
  S.hop = k;
  document.querySelectorAll('#details .hop').forEach((li, j) => {
    li.classList.toggle('cur', j === k);
    li.classList.toggle('done', j < k);
  });
  const cur = document.querySelector('#details .hop.cur');
  if (cur) cur.scrollIntoView({ block: 'nearest' });
}

/** Show one message on the chart: its edge and both ends. */
function showHop(m) {
  const edge = S.cy.getElementById(S.edgeOf.get(m.i) || '');
  if (edge.empty()) return;
  const cam = cameraFor(edge.union(edge.connectedNodes()));
  if (cam) moveCamera(cam.zoom >= TOUR_MIN_ZOOM ? cam : cameraAt(cam.center, TOUR_MIN_ZOOM));
  edge.flashClass?.('flow-active', 900);
}

/** Text with `code` spans, as DOM nodes: never HTML. */
function richText(text) {
  return text.split('`').map((part, i) => (i % 2 ? el('code', {}, part) : part));
}

/** The card on top of the details panel: the tour, the step in words, links to its nodes. */
function stepCard() {
  const t = S.flow;
  const st = t.steps[S.step] || t.steps[0];
  const n = t.steps.length;
  const when = t.recorded?.at ? t.recorded.at.slice(0, 10) : '';
  return el(
    'section',
    { class: `card ${t.source}`, 'data-test': 'step-card' },
    el('div', { class: 'card-head' }, el('span', { class: 'card-glyph' }, tourGlyph(t)), el('b', {}, t.title), el('button', { class: 'card-close', title: 'Close the walkthrough', onclick: () => setFlow('') }, '×')),
    t.question ? el('div', { class: 'card-q' }, `asked: ${t.question}`) : null,
    S.step <= 0 && t.summary ? el('p', { class: 'card-summary' }, richText(t.summary)) : null,
    S.walkStack?.length ? el('div', { class: 'card-back' }, el('a', { class: 'link', onclick: closeStep, 'data-test': 'close-step' }, `◂ Back to ${S.walkStack[S.walkStack.length - 1].tour.title}`)) : null,
    el(
      'div',
      { class: 'card-step' },
      `step ${S.step + 1} of ${n} · `,
      el(
        'select',
        { class: 'layer-select', 'aria-label': 'Layer of this walk', title: 'The layer this walk plays at: boxes stand for what is inside them, and parts the layer leaves out are passed on the way', onchange: (e) => setLayer(e.target.value), 'data-test': 'layer' },
        S.layers.map((L) => el('option', { value: L.key, selected: t.layer === L.key, disabled: !walkLayers(t).has(L.key) }, L.name)),
      ),
    ),
    el('h3', {}, richText(st.title)),
    st.text ? st.text.split(/\n\s*\n/).map((para) => el('p', {}, richText(para))) : null,
    st.inner && st.inner.length > 1
      ? el('ol', { class: 'inner-steps', 'data-test': 'inner-steps' }, st.inner.map((k) => el('li', {}, richText(t.base.steps[k].title))))
      : null,
    st.deeper
      ? el('button', { class: 'btn open-step', onclick: openStep, 'data-test': 'open-step', title: 'Walk what happens inside, one layer deeper' }, `Open ▸ ${st.inner.length > 1 ? `${st.inner.length} steps` : 'this step'}${st.passes ? ' on the way' : st.box ? ` inside ${groupLabel(st.box)}` : ''}`)
      : null,
    el('div', { class: 'chips' }, stepIds(st).map((id) => {
      const node = S.byId.get(id);
      return el('a', { class: 'chip', title: id, onclick: () => select(id, true) }, `${KINDS[kindOf(node?.kind)].glyph} ${node?.name || id}`);
    })),
    hopList(st),
    el(
      'div',
      { class: 'card-nav' },
      el('button', { class: 'btn ghost', title: 'Zoom out to everything this step touches', onclick: showWholeStep, 'data-test': 'whole-step' }, '⤢ Whole step'),
      el('button', { class: 'btn', disabled: S.step <= 0, onclick: () => stepBy(-1) }, '◀ Back'),
      el('button', { class: 'btn', disabled: S.step >= n - 1, onclick: () => stepBy(1) }, 'Next ▶'),
    ),
    t.source === 'walk' ? el('div', { class: 'card-foot' }, [t.recorded?.by === 'human' ? 'written by hand' : `written by ${t.recorded?.model || 'an AI model'}`, when ? ` · ${when}` : '', t.file ? ` · ${t.file}` : ' · not recorded yet'].join('')) : null,
  );
}

/** "What happens": the step's messages, one line each, and what the step has built up at its end. */
function hopList(st) {
  const { shown: msgs, inside } = drawnMessages(st);
  if (!msgs.length && !inside) return null;
  const parts = msgs.map(messageParts);
  const has = parts.map((p) => p.has).filter(Boolean);
  return el(
    'div',
    { class: 'hops-box' },
    el('div', { class: 'card-sub' }, 'What happens'),
    el(
      'ol',
      { class: 'hops', 'data-test': 'hops' },
      msgs.map((m, k) => {
        const p = parts[k];
        return el(
          'li',
          { class: `hop ${m.kind}${k === S.hop ? ' cur' : ''}${k < S.hop ? ' done' : ''}`, title: 'Show this on the chart', onclick: () => showHop(m) },
          el('span', { class: 'hop-icon' }, MSG_ICON[m.kind] || '→'),
          el('span', { class: 'hop-text' }, el('b', {}, p.who[0]), ` ${p.who[1]} `, el('b', {}, p.who[2]), p.why ? [': ', richText(p.why)] : null, p.carry ? el('span', { class: 'hop-carry' }, p.carry) : null),
        );
      }),
    ),
    inside ? el('div', { class: 'hop-inside', 'data-test': 'hops-inside' }, `+${inside} ${inside === 1 ? 'message' : 'messages'} ${st.passes ? 'on the way, in parts this layer leaves out' : `inside ${st.box ? groupLabel(st.box) : 'the boxes'}`}${st.deeper ? ': Open to see them' : ''}`) : null,
    has.length ? el('div', { class: 'hop-has', 'data-test': 'built-up' }, el('span', { class: 'card-sub' }, 'Now it has'), el('ul', {}, has.map((h) => el('li', {}, h)))) : null,
  );
}

/** First view of the details panel: "walk me through", the scenarios and walkthroughs (SPEC §7.4). */
function tourList() {
  if (!S.tours.length) return null;
  return el(
    'section',
    { class: 'tours' },
    el('h4', {}, 'Walk me through'),
    S.tours.map((t) =>
      el(
        'button',
        { class: 'tour', onclick: () => setFlow(t.key) },
        el('span', { class: 'tour-title' }, `${tourGlyph(t)} ${t.title}`),
        el('span', { class: 'stats' }, `${t.steps.length} steps`),
        t.question ? el('span', { class: 'tour-q' }, t.question) : null,
      ),
    ),
  );
}

/***
 * Add a walkthrough sent by the page around this one (the web app's Ask box), and play it.
 *
 * Only the parent window may send; the walkthrough is already bound to this graph by the server.
 *
 * @id      docair-viewer.receive
 * @part-of docair-viewer.page
 * @calls   docair-viewer.walk "starts the received walkthrough"
 ***/
function addWalkthrough(w, play = true) {
  if (!w || typeof w.id !== 'string' || !Array.isArray(w.steps)) return false;
  S.graph.walkthroughs = S.graph.walkthroughs.filter((x) => x.id !== w.id).concat([w]);
  const current = S.flow?.key;
  buildTours();
  renderFlowBar();
  if (play) setFlow(`walk:${w.id}`);
  else if (current) S.flow = findTour(current);
  if (!S.flow) renderDetails(S.selected);
  return true;
}

function wireMessages() {
  if (window.parent === window) return;
  window.addEventListener('message', (e) => {
    if (e.source !== window.parent) return;
    const m = e.data;
    if (m && m.type === 'docair:walkthrough') addWalkthrough(m.walkthrough, m.play !== false);
    if (m && m.type === 'docair:wording' && Array.isArray(m.items)) previewWording(m.items);
    if (m && m.type === 'docair:glossary' && Array.isArray(m.terms)) previewTerms(m.terms);
    if (m && m.type === 'docair:view') {
      if (typeof m.tests === 'boolean') setTests(m.tests);
      setHidden((m.hide || []).filter((id) => S.byId.has(id)), (m.show || []).filter((id) => S.byId.has(id)));
    }
  });
}

/***
 * Show words the model proposed for the lines at once, marked ✦, before they are applied to the code.
 *
 * @id      docair-viewer.wording
 * @part-of docair-viewer.page
 * @calls   docair-viewer.render "shows the proposed words"
 ***/
function previewWording(items) {
  let changed = 0;
  for (const it of items) {
    for (const e of S.graph.edges) {
      if (e.from === it.from && e.to === it.to && !e.text && typeof it.text === 'string') {
        e.text = it.text;
        e.preview = true;
        changed++;
      }
    }
  }
  if (changed) {
    toast(`✦ words for ${changed} lines`);
    render();
    if (S.selected) renderDetails(S.selected);
  }
  return changed;
}

/** Tell the web app around this page what happened (selection, view), if there is one. */
function tellParent(message) {
  if (window.parent !== window) window.parent.postMessage(message, '*');
}

/***
 * Play a step's messages: before each one moves, its line in the card lights up and
 * the token shows what it carries; an answer goes back along its call as a hollow token.
 *
 * @id      docair-viewer.play-messages
 * @part-of docair-viewer.page
 * @calls   docair-viewer.messages
 ***/
async function playMessages(msgs) {
  const cy = S.cy;
  const run = ++S.tokenRun;
  cy.$('node.token').remove();
  const follow = S.follow;
  for (let k = 0; k < msgs.length; k++) {
    if (run !== S.tokenRun) return;
    const m = msgs[k];
    setHop(k);
    const e = cy.getElementById(S.edgeOf.get(m.i) || '');
    if (e.empty()) continue;
    let pts = [e.sourceEndpoint(), ...(e.segmentPoints() || []), e.targetEndpoint()];
    if (!pts.every((p) => p && Number.isFinite(p.x) && Number.isFinite(p.y))) continue;
    if (m.kind === 'return') pts = pts.reverse();
    const words = messageParts(m).token || '';
    const label = words.length > 32 ? `${words.slice(0, 31)}…` : words;
    const tok = cy.add({ group: 'nodes', data: { id: `token-${run}-${k}`, w: 13, h: 13, label }, classes: m.kind === 'return' ? 'token reply' : 'token', position: { ...pts[0] } });
    if (follow) {
      cy.stop();
      cy.animate({ zoom: follow.zoom, pan: cameraAt(pts[0], follow.zoom).pan }, { duration: 300, easing: 'ease-in-out-cubic' });
    }
    // A moment to read the line before the token moves.
    await sleep(msgs.length > 1 ? 450 : 150);
    for (let i = 1; i < pts.length && run === S.tokenRun; i++) {
      const dist = Math.hypot(pts[i].x - pts[i - 1].x, pts[i].y - pts[i - 1].y);
      // Followed by the camera, the token moves slower, so the eye can keep up.
      const duration = follow ? Math.max(150, Math.min(800, dist * 1.5)) : Math.max(120, Math.min(700, dist * 2.4));
      if (follow) {
        cy.stop();
        cy.animate({ zoom: follow.zoom, pan: cameraAt(pts[i], follow.zoom).pan }, { duration, easing: 'linear' });
      }
      // A removed token never completes its animation; never wait longer than planned.
      await Promise.race([tok.animation({ position: pts[i], duration }).play().promise(), sleep(duration + 250)]);
    }
    tok.remove();
  }
  if (run !== S.tokenRun) return;
  setHop(msgs.length);
  if (S.follow) {
    moveCamera(S.follow.rest, 500);
    S.follow = null;
  }
}

function applyFlowClasses() {
  const cy = S.cy;
  if (!cy) return;
  cy.elements().removeClass('flow-done flow-current flow-trail flow-active flow-dim');
  if (!S.flow) return;
  const inFlow = new Set();
  const onPath = new Set();
  for (const st of S.flow.steps) {
    st.nodes.forEach((id) => inFlow.add(rep(id)));
    for (const i of st.path) {
      const e = S.graph.edges[i];
      inFlow.add(rep(e.from));
      inFlow.add(rep(e.to));
      onPath.add(S.edgeOf.get(i));
    }
  }
  cy.nodes().forEach((n) => {
    if (!n.isParent() && !n.hasClass('token') && !n.hasClass('elabel') && !inFlow.has(n.id())) n.addClass('flow-dim');
  });
  cy.edges().forEach((e) => {
    if (onPath.has(e.id())) return;
    e.addClass('flow-dim');
    cy.getElementById(`${e.id()}::label`).addClass('flow-dim');
  });
  S.flow.steps.forEach((st, k) => {
    if (k > S.step) return;
    const current = k === S.step;
    st.nodes.forEach((id) => cy.getElementById(rep(id)).addClass(current ? 'flow-current' : 'flow-done'));
    st.path.forEach((i) => {
      const id = S.edgeOf.get(i);
      if (id) cy.getElementById(id).addClass(current ? 'flow-active' : 'flow-trail');
    });
  });
}

async function togglePlay() {
  if (!S.flow) {
    if (!S.tours.length) return;
    await setFlow(S.tours[0].key);
    await sleep(800);
  }
  S.playing = !S.playing;
  updatePlay();
  if (S.playing && S.step >= S.flow.steps.length - 1) S.step = -1;
  while (S.playing && S.flow && S.step < S.flow.steps.length - 1) {
    await goStep(S.step + 1);
    // Time to read the step: longer for longer texts.
    const st = S.flow?.steps[S.step];
    await sleep(Math.min(9000, 900 + 35 * ((st?.text || '').length + (st?.title || '').length)));
  }
  S.playing = false;
  updatePlay();
}

function updatePlay() {
  const b = $('#flow-play');
  if (b) b.textContent = S.playing ? 'Pause' : 'Play';
}

/* ---------- sections and tests (SPEC §11, §12) ---------- */

const viewKey = () => `docair:view:${S.graph.system}`;

/** What to hide when the page opens: docair.toml [view], then what this browser remembered. */
function loadView() {
  const v = S.graph.view || {};
  S.hidden = new Set((v.hide || []).filter((id) => S.byId.has(id)));
  // [view] show: parts to show inside hidden sections (SPEC §11).
  for (const id of v.show || []) if (S.byId.has(id)) revealWithin(id);
  S.showTests = v.tests !== false;
  try {
    const saved = JSON.parse(localStorage.getItem(viewKey()) || 'null');
    if (saved && Array.isArray(saved.hidden)) S.hidden = new Set(saved.hidden.filter((id) => S.byId.has(id)));
    if (saved && typeof saved.tests === 'boolean') S.showTests = saved.tests;
  } catch (_) {
    /* storage can be blocked; the defaults stand */
  }
}

function saveView() {
  try {
    localStorage.setItem(viewKey(), JSON.stringify({ hidden: [...S.hidden], tests: S.showTests }));
  } catch (_) {
    /* not remembered, still applied */
  }
}

/** Whether a node is left out: it or a group around it is hidden, or it is a test while tests are off. A walkthrough shows what it needs. */
function hiddenNode(id) {
  if (S.needed.has(id)) return false;
  for (let cur = id; cur && cur !== S.graph.system; cur = S.byId.get(cur)?.part_of) {
    if (S.hidden.has(cur)) return true;
    if (!S.showTests && S.byId.get(cur)?.kind === 'test') return true;
  }
  return false;
}

/***
 * Hide and show sections: a hidden node disappears with everything inside it and every edge that touches it.
 *
 * @id      docair-viewer.sections
 * @part-of docair-viewer.page
 * @calls   docair-viewer.render "redraws without the hidden section"
 ***/
function setHidden(hide, show) {
  const before = [...S.hidden].sort().join();
  hide.forEach((id) => id !== S.graph.system && S.hidden.add(id));
  const revealed = [];
  for (const id of show) {
    // Inside a hidden section: the section comes back with only this part in it.
    if (ancestorsOf(id).some((a) => S.hidden.has(a))) revealed.push(...revealWithin(id));
    else S.hidden.delete(id);
  }
  if ([...S.hidden].sort().join() === before) return Promise.resolve();
  S.viewCache.clear();
  applyView();
  saveView();
  markHiddenRows();
  renderHiddenBar();
  if (revealed.length) toast(`showing ${show.map((id) => S.byId.get(id)?.name || id).join(', ')}; hidden around it: ${summarize(revealed.map((id) => S.byId.get(id)?.name || id), 4)}`);
  else if (hide.length) toast(`hidden: ${hide.map((id) => S.byId.get(id)?.name || id).join(', ')}`);
  tellParent({ type: 'docair:view-changed', hidden: [...S.hidden], tests: S.showTests });
  if (S.selected && hiddenNode(S.selected)) select(null);
  return render();
}

/**
 * Show a part whose section is hidden (SPEC §11): from the highest hidden
 * section down to the part, each group comes back, and its other children are
 * hidden instead. Returns the parts that became hidden.
 */
function revealWithin(id) {
  const path = [...ancestorsOf(id).reverse(), id];
  const first = path.findIndex((x) => S.hidden.has(x));
  if (first < 0) return [];
  const hid = [];
  for (let j = first; j < path.length; j++) {
    S.hidden.delete(path[j]);
    if (j === path.length - 1) break;
    for (const sib of S.children.get(path[j]) || []) {
      if (sib !== path[j + 1] && !S.hidden.has(sib) && !isData(sib)) {
        S.hidden.add(sib);
        hid.push(sib);
      }
    }
  }
  return hid;
}

/** "Show only this": every part beside it, and beside each group around it, is hidden. */
function showOnly(id) {
  const path = [...ancestorsOf(id).reverse(), id];
  const hide = [];
  let parent = S.graph.system;
  for (const p of path) {
    for (const sib of S.children.get(parent) || []) if (sib !== p && !isData(sib)) hide.push(sib);
    parent = p;
  }
  return setHidden(hide, path);
}

/***
 * Show or hide every test node and the edges to what the tests cover.
 *
 * @id      docair-viewer.tests-layer
 * @part-of docair-viewer.page
 * @calls   docair-viewer.render "redraws with or without tests"
 ***/
function setTests(on) {
  if (S.showTests === on) return Promise.resolve();
  S.showTests = on;
  S.viewCache.clear();
  applyView();
  saveView();
  renderTestsButton();
  markHiddenRows();
  tellParent({ type: 'docair:view-changed', hidden: [...S.hidden], tests: S.showTests });
  return render();
}

function eyeIcon() {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('width', '14');
  svg.setAttribute('height', '14');
  svg.setAttribute('aria-hidden', 'true');
  const path = document.createElementNS(ns, 'path');
  path.setAttribute('d', 'M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12zm10 4a4 4 0 1 0 0-8 4 4 0 0 0 0 8z');
  const slash = document.createElementNS(ns, 'path');
  slash.setAttribute('class', 'slash');
  slash.setAttribute('d', 'M3 3l18 18');
  svg.append(path, slash);
  return svg;
}

function markHiddenRows() {
  document.querySelectorAll('#tree-list li').forEach((li) => {
    const id = li.dataset.id;
    const own = S.hidden.has(id);
    const inside = !own && ancestorsOf(id).some((a) => S.hidden.has(a));
    li.classList.toggle('hid', own);
    li.classList.toggle('hid-in', !own && hiddenNode(id));
    const eye = li.querySelector(':scope > .row > .eye');
    if (eye) {
      eye.classList.toggle('off', own || inside);
      eye.classList.toggle('inside', inside);
      eye.title = own ? 'Show this section again' : inside ? 'Show this part: its hidden section comes back with only this in it' : 'Hide this section';
    }
  });
}

/** Chips above the picture: what is hidden, each one a way back; several parts of one group are one chip. */
function renderHiddenBar() {
  const bar = $('#hidden-bar');
  if (!bar) return;
  const ids = [...S.hidden].filter((id) => S.byId.has(id));
  bar.hidden = !ids.length;
  const byParent = new Map();
  for (const id of ids) {
    const p = S.byId.get(id).part_of || S.graph.system;
    if (!byParent.has(p)) byParent.set(p, []);
    byParent.get(p).push(id);
  }
  const chips = [];
  for (const [parent, kids] of byParent) {
    if (kids.length === 1) {
      const id = kids[0];
      chips.push(el('button', { class: 'chip-x', title: `Show ${S.byId.get(id).name} again`, onclick: () => setHidden([], [id]), 'data-test': 'hidden-chip' }, `${S.byId.get(id).name} ×`));
    } else {
      const names = kids.map((id) => S.byId.get(id).name);
      const where = parent === S.graph.system ? S.graph.system : S.byId.get(parent)?.name || parent;
      chips.push(el('button', { class: 'chip-x', title: `Show them again: ${names.join(', ')}`, onclick: () => setHidden([], kids), 'data-test': 'hidden-group' }, `${where}: ${kids.length} hidden ×`));
    }
  }
  bar.replaceChildren(
    ...[el('span', { class: 'hb-label' }, 'hidden:'), ...chips, ids.length > 1 ? el('a', { class: 'link', onclick: () => setHidden([], ids), 'data-test': 'show-all' }, 'Show all') : null].filter(Boolean),
  );
}

function testCounts() {
  const tests = S.graph.nodes.filter((n) => n.kind === 'test' && !(S.children.get(n.id) || []).length);
  const by = (st) => tests.filter((n) => n.test?.status === st).length;
  return { total: tests.length, passed: by('passed'), failed: by('failed'), flaky: by('flaky'), none: tests.filter((n) => !n.test).length };
}

function renderTestsButton() {
  const b = $('#btn-tests');
  if (!b) return;
  const c = testCounts();
  b.hidden = !c.total;
  b.replaceChildren(
    ...[
      'Tests ',
      c.passed ? el('span', { class: 't-passed-text' }, `✓${c.passed} `) : null,
      c.failed ? el('span', { class: 't-failed-text' }, `✗${c.failed} `) : null,
      c.flaky ? el('span', {}, `~${c.flaky} `) : null,
      !c.passed && !c.failed && !c.flaky ? `${c.total} ` : null,
      el('span', { class: 'onoff' }, S.showTests ? 'shown' : 'hidden'),
    ].filter(Boolean),
  );
  b.classList.toggle('on', S.showTests);
  b.title = `${c.total} tests: ${c.passed} passed, ${c.failed} failed, ${c.flaky} flaky, ${c.none} without a result. Click to ${S.showTests ? 'hide' : 'show'} them.`;
  b.onclick = () => setTests(!S.showTests);
}

/** "Tested by": the tests that cover this node or anything inside it. */
function testedBy(id) {
  const inside = new Set([id]);
  const add = (x) => (S.children.get(x) || []).forEach((c) => (inside.add(c), add(c)));
  add(id);
  const tests = new Map();
  for (const e of S.graph.edges) {
    if (e.kind === 'cover' && inside.has(e.to) && !inside.has(e.from)) tests.set(e.from, S.byId.get(e.from));
  }
  const frag = el('div');
  if (!tests.size) return frag;
  frag.append(el('h3', { class: 'sec' }, `Tested by (${tests.size})`));
  frag.append(
    el('ul', { class: 'edges', 'data-test': 'tested-by' }, [...tests.values()].map((t) =>
      el('li', {}, el('span', { class: `ek t-${t.test?.status || 'none'}-text` }, TEST_ICON[t.test?.status] || '○'), nodeLink(t.id), t.test ? el('span', { class: 'badge' }, `${t.test.status} · ${(t.test.duration_ms / 1000).toFixed(1)} s`) : el('span', { class: 'badge' }, 'no result')))),
  );
  return frag;
}

/* ---------- glossary: what the words mean (SPEC §13) ---------- */

const escapeRe = (t) => t.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** The terms of the glossary, plus those the model proposed, and the pattern that finds them in text. */
function buildTerms() {
  S.terms = [...(S.graph.glossary || []), ...S.previewTerms.filter((p) => !(S.graph.glossary || []).some((t) => t.id === p.id))];
  S.termById = new Map(S.terms.map((t) => [t.id, t]));
  S.termWords = new Map();
  // A word with a capital letter (Ingress, Secret) counts only as written (SPEC §13.1).
  S.termCase = new Map();
  for (const t of S.terms)
    for (const w of [t.term, ...(t.aliases || [])]) {
      if (!w || w.length < 2) continue;
      S.termWords.set(w.toLowerCase(), t.id);
      if (w !== w.toLowerCase()) S.termCase.set(w.toLowerCase(), w);
    }
  const alts = [...S.termWords.keys()].sort((a, b) => b.length - a.length).map(escapeRe);
  // Not inside an id or a path: `docair-ai.ask.digest`, `docs/graph` (SPEC §13.2).
  S.termRe = alts.length ? new RegExp(`(?<![\\p{L}\\p{N}_./-])(${alts.join('|')})(?:e?s)?(?![\\p{L}\\p{N}_-])`, 'giu') : null;
}

/** A code span that names a part: its id, or its name when only one part has it. */
function nodeNamed(text) {
  if (S.byId.has(text)) return text;
  if (text.length < 3) return null;
  const hits = S.graph.nodes.filter((n) => n.name === text);
  return hits.length === 1 ? hits[0].id : null;
}

const NO_TERMS = new Set(['A', 'CODE', 'PRE', 'BUTTON', 'INPUT', 'TEXTAREA', 'SELECT', 'H1', 'H2', 'H3', 'OPTION', 'SCRIPT', 'STYLE']);

/***
 * Mark the words of the glossary in a panel: the first mention of each term becomes hoverable; code spans that name a part link to it.
 *
 * @id      docair-viewer.terms
 * @part-of docair-viewer.page
 ***/
function linkTerms(root, except) {
  if (!root) return;
  root.querySelectorAll('code').forEach((c) => {
    if (c.closest('pre, a, .no-terms, .node-ref')) return;
    const id = nodeNamed(c.textContent.trim());
    if (id) {
      c.classList.add('node-ref');
      c.dataset.node = id;
      c.tabIndex = 0;
    }
  });
  if (!S.termRe) return;
  const done = new Set(except ? [except] : []);
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode: (n) => {
      for (let p = n.parentElement; p && p !== root.parentElement; p = p.parentElement) {
        if (NO_TERMS.has(p.tagName) || p.classList.contains('term') || p.classList.contains('no-terms') || p.classList.contains('badge') || p.classList.contains('ek') || p.classList.contains('id') || p.classList.contains('where') || p.classList.contains('docfile') || p.classList.contains('hop-carry') || p.classList.contains('empty')) return NodeFilter.FILTER_REJECT;
      }
      return n.nodeValue.trim() ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT;
    },
  });
  const texts = [];
  while (walker.nextNode()) texts.push(walker.currentNode);
  for (const tn of texts) {
    const text = tn.nodeValue;
    const frag = document.createDocumentFragment();
    let last = 0;
    let any = false;
    S.termRe.lastIndex = 0;
    for (let m = S.termRe.exec(text); m; m = S.termRe.exec(text)) {
      const id = S.termWords.get(m[1].toLowerCase());
      const exact = S.termCase.get(m[1].toLowerCase());
      if (!id || done.has(id) || (exact && exact !== m[1])) continue;
      done.add(id);
      any = true;
      frag.append(text.slice(last, m.index), el('span', { class: `term${S.termById.get(id)?.preview ? ' preview' : ''}`, 'data-term': id, tabindex: '0', role: 'button' }, m[0]));
      last = m.index + m[0].length;
    }
    if (any) {
      frag.append(text.slice(last));
      tn.replaceWith(frag);
    }
  }
}

/** A term's short definition as DOM: rendered Markdown, or the plain text the model proposed. */
function termSummary(t, full = false) {
  const h = S.html[`term:${t.id}`];
  if (h) return el('div', { class: 'md', html: full ? h.body : h.summary });
  return el('div', { class: 'md' }, el('p', {}, t.definition || t.summary || ''), full && t.link ? el('p', {}, el('a', { href: t.link }, 'Read more ↗')) : null);
}

/** The first "read more" link of a term: a doc in the page, or a web page. */
function termLink(t) {
  if (t.link) return t.link;
  const body = S.html[`term:${t.id}`]?.body || '';
  const m = body.match(/href="((?:docair-doc:|https?:)[^"]+)"/);
  return m ? m[1].replace(/&amp;/g, '&') : null;
}

function wireTerms() {
  const pop = $('#term-pop');
  let timer = 0;
  const hide = () => {
    clearTimeout(timer);
    timer = setTimeout(() => (pop.hidden = true), 220);
  };
  const show = (target) => {
    const termId = target.dataset.term;
    const nodeId = target.dataset.node;
    const dataId = target.dataset.data;
    if (dataId) {
      const d = S.byId.get(dataId);
      const makers = S.graph.edges.filter((x) => x.to === dataId && x.kind === 'create').map((x) => x.from);
      pop.replaceChildren(
        ...[
          el('div', { class: 'pop-title' }, `◇ ${d?.name ?? dataId}`, el('span', { class: 'badge' }, 'data')),
          d?.summary ? el('p', {}, d.summary) : null,
          makers.length ? el('div', { class: 'pop-see' }, 'made by ', makers.map((m, i) => [i ? ', ' : '', nodeLink(m)])) : null,
          el('div', { class: 'pop-foot' }, el('a', { class: 'link', onclick: () => ((pop.hidden = true), followData(dataId)) }, 'Follow it'), ' · ', el('a', { class: 'link', onclick: () => ((pop.hidden = true), showData(dataId)) }, 'About it')),
        ].filter(Boolean),
      );
    } else if (termId) {
      const t = S.termById.get(termId);
      if (!t) return;
      const link = termLink(t);
      pop.replaceChildren(
        ...[
          el('div', { class: 'pop-title' }, t.term, t.preview ? el('span', { class: 'badge inferred' }, '✦ proposed') : null),
          termSummary(t),
          t.see?.length ? el('div', { class: 'pop-see' }, 'see ', t.see.filter((id) => S.byId.has(id)).slice(0, 4).map((id, i) => [i ? ', ' : '', nodeLink(id)])) : null,
          el(
            'div',
            { class: 'pop-foot' },
            el('a', { class: 'link', onclick: () => ((pop.hidden = true), showTerm(termId)), 'data-test': 'pop-open' }, 'In the glossary'),
            link ? [' · ', el('a', { class: 'link', href: link, 'data-test': 'pop-more' }, link.startsWith('docair-doc:') ? 'Read more' : 'Read more ↗')] : null,
          ),
        ].filter(Boolean),
      );
    } else if (nodeId) {
      const n = S.byId.get(nodeId);
      pop.replaceChildren(
        el('div', { class: 'pop-title' }, `${KINDS[kindOf(n.kind)].glyph} ${n.name}`, el('span', { class: 'badge' }, n.kind)),
        n.summary ? el('p', {}, n.summary) : null,
        el('div', { class: 'pop-foot' }, el('a', { class: 'link', onclick: () => ((pop.hidden = true), select(nodeId, true)) }, 'Show it')),
      );
    }
    const r = target.getBoundingClientRect();
    pop.hidden = false;
    const w = pop.offsetWidth;
    const h = pop.offsetHeight;
    const left = Math.max(8, Math.min(innerWidth - w - 8, r.left));
    const top = r.bottom + 6 + h > innerHeight ? r.top - h - 6 : r.bottom + 6;
    pop.style.left = `${left}px`;
    pop.style.top = `${Math.max(8, top)}px`;
  };
  document.addEventListener('mouseover', (e) => {
    const t = e.target.closest?.('.term, .node-ref, .data-ref');
    if (t) {
      clearTimeout(timer);
      timer = setTimeout(() => show(t), 120);
    } else if (e.target.closest?.('#term-pop')) clearTimeout(timer);
  });
  document.addEventListener('mouseout', (e) => {
    if (e.target.closest?.('.term, .node-ref, .data-ref, #term-pop') && !e.relatedTarget?.closest?.('#term-pop, .term, .node-ref, .data-ref')) hide();
  });
  document.addEventListener('focusin', (e) => {
    const t = e.target.closest?.('.term, .node-ref, .data-ref');
    if (t) show(t);
  });
  document.addEventListener('keydown', (e) => {
    const t = e.target.closest?.('.term, .node-ref, .data-ref');
    if (t && (e.key === 'Enter' || e.key === ' ')) {
      e.preventDefault();
      t.click();
    }
  });
  document.addEventListener('click', (e) => {
    const a = e.target.closest?.('a[href]');
    if (a) {
      const href = a.getAttribute('href');
      if (href.startsWith('docair-doc:')) {
        e.preventDefault();
        pop.hidden = true;
        openDoc(href.slice('docair-doc:'.length));
        return;
      }
      if (/^https?:/.test(href)) {
        a.target = '_blank';
        a.rel = 'noopener';
      }
    }
    const t = e.target.closest?.('.term');
    if (t) {
      pop.hidden = true;
      showTerm(t.dataset.term);
      return;
    }
    const dref = e.target.closest?.('.data-ref');
    if (dref && !dref.closest('#term-pop')) {
      pop.hidden = true;
      showData(dref.dataset.data);
      return;
    }
    const r = e.target.closest?.('.node-ref');
    if (r && !r.closest('#term-pop')) {
      pop.hidden = true;
      select(r.dataset.node, true);
    }
  });
}

function renderGlossaryButton() {
  const b = $('#btn-glossary');
  if (!b) return;
  b.hidden = !S.terms.length;
  b.textContent = `Glossary (${S.terms.length})`;
  b.onclick = () => renderGlossary();
}

/** The parts whose texts use a term. */
function mentionsOf(t) {
  const words = [t.term, ...(t.aliases || [])].map(escapeRe).join('|');
  const re = new RegExp(`(?<![\\p{L}\\p{N}_-])(${words})(?:e?s)?(?![\\p{L}\\p{N}_-])`, 'iu');
  return S.graph.nodes.filter((n) => re.test(`${n.summary} ${n.description}`));
}

/***
 * The glossary in the details panel: every term with its short definition, or one term in full.
 *
 * @id      docair-viewer.glossary
 * @part-of docair-viewer.page
 * @calls   docair-viewer.terms
 ***/
function renderGlossary(filter = '') {
  const box = $('#details');
  const q = filter.trim().toLowerCase();
  const terms = [...S.terms].sort((a, b) => a.term.localeCompare(b.term)).filter((t) => !q || [t.term, ...(t.aliases || [])].some((w) => w.toLowerCase().includes(q)));
  const search = el('input', { type: 'search', class: 'gl-search', placeholder: 'Find a term…', value: filter, 'aria-label': 'Find a term', 'data-test': 'glossary-search' });
  search.addEventListener('input', () => {
    renderGlossary(search.value);
    const again = $('.gl-search');
    again.focus();
    again.setSelectionRange(again.value.length, again.value.length);
  });
  box.replaceChildren(
    ...[
      el('h2', {}, el('span', { class: 'glyph' }, '¶'), 'Glossary'),
      el('div', { class: 'meta' }, `${S.terms.length} terms · what the words of these docs mean`),
      search,
      el(
        'dl',
        { class: 'gl-list', 'data-test': 'glossary' },
        terms.map((t) => [
          el('dt', {}, el('a', { class: 'link', onclick: () => showTerm(t.id) }, t.term), t.preview ? el('span', { class: 'badge inferred' }, '✦') : null),
          el('dd', {}, termSummary(t)),
        ]),
      ),
      S.selected ? el('a', { class: 'link', onclick: () => renderDetails(S.selected) }, `← back to ${S.byId.get(S.selected)?.name}`) : null,
    ].filter(Boolean),
  );
  linkTerms(box.querySelector('.gl-list'));
}

/** One term in full: definition, related parts, where it is used, where it is defined. */
function showTerm(id) {
  const t = S.termById.get(id);
  if (!t) return;
  const box = $('#details');
  const mentions = mentionsOf(t);
  box.replaceChildren(
    ...[
      el('div', { class: 'gl-nav' }, el('a', { class: 'link', onclick: () => renderGlossary() }, '← Glossary'), S.selected ? [' · ', el('a', { class: 'link', onclick: () => renderDetails(S.selected) }, `back to ${S.byId.get(S.selected)?.name}`)] : null),
      el('h2', { 'data-test': 'term-title' }, el('span', { class: 'glyph' }, '¶'), t.term, t.preview ? el('span', { class: 'badge inferred' }, '✦ proposed by the model') : null),
      t.aliases?.length ? el('div', { class: 'meta' }, `also: ${t.aliases.join(', ')}`) : null,
      termSummary(t, true),
      t.see?.length ? [el('h3', { class: 'sec' }, 'See'), el('ul', { class: 'edges' }, t.see.filter((s) => S.byId.has(s)).map((s) => el('li', {}, nodeLink(s), el('span', { class: 'badge' }, S.byId.get(s).kind))))] : null,
      mentions.length ? [el('h3', { class: 'sec' }, `Used by (${mentions.length})`), el('ul', { class: 'edges' }, mentions.slice(0, 20).map((n) => el('li', {}, nodeLink(n.id), el('span', { class: 'badge' }, n.kind))))] : null,
      t.source ? el('div', { class: 'docfile' }, `${t.source.file}:${t.source.line}`) : el('div', { class: 'docfile' }, 'not in the code yet: apply the proposal to keep it'),
    ]
      .flat(Infinity)
      .filter(Boolean),
  );
  linkTerms(box, id);
}

/***
 * Open a doc of the project inside the page, at a heading: the "read more" of a glossary entry.
 *
 * @id      docair-viewer.reader
 * @part-of docair-viewer.page
 * @calls   docair-viewer.terms
 ***/
function openDoc(target) {
  const [file, anchor] = target.split('#');
  const doc = S.html[`doc:${file}`];
  const reader = $('#reader');
  if (!doc) return toast(`${file} is not in this page`);
  reader.replaceChildren(
    el('div', { class: 'reader-head' }, el('b', {}, file), el('span', { class: 'spacer' }), el('button', { class: 'btn', onclick: () => (reader.hidden = true), 'data-test': 'reader-close' }, 'Close')),
    el('div', { class: 'reader-body md', html: doc.body }),
  );
  reader.hidden = false;
  const body = reader.querySelector('.reader-body');
  linkTerms(body);
  const at = anchor && body.querySelector(`[id="${CSS.escape(anchor)}"]`);
  if (at) {
    at.scrollIntoView({ block: 'start' });
    at.classList.add('flash');
  } else body.scrollTop = 0;
}

/***
 * Show terms the model proposed at once, marked ✦, before they are written into the glossary.
 *
 * @id      docair-viewer.glossary-preview
 * @part-of docair-viewer.page
 * @calls   docair-viewer.glossary
 ***/
function previewTerms(items) {
  const slug = (s) => s.toLowerCase().replace(/[^\p{L}\p{N}_-]+/gu, '-').replace(/^-|-$/g, '');
  const added = items
    .filter((t) => t && typeof t.term === 'string' && t.term.trim())
    .map((t) => ({ id: slug(t.term), term: t.term.trim(), aliases: t.aliases || [], definition: t.definition || '', summary: t.definition || '', see: (t.see || []).filter((id) => S.byId.has(id)), link: /^https?:\/\//.test(t.link || '') ? t.link : null, preview: true }));
  S.previewTerms = [...S.previewTerms.filter((p) => !added.some((a) => a.id === p.id)), ...added];
  buildTerms();
  renderGlossaryButton();
  if (added.length) toast(`✦ ${added.length} terms explained`);
  if (S.selected) renderDetails(S.selected);
  else renderGlossary();
  return added.length;
}

/* ---------- data: what the parts make and carry (SPEC §3.6) ---------- */

function dataItems() {
  return S.graph.nodes.filter((n) => n.kind === 'data' && !(S.children.get(n.id) || []).length);
}

/** The words that name each datum in payloads: its name, the end of its id, its `@carried-as` words. */
function dataWords() {
  if (S.dataWords) return S.dataWords;
  const words = new Map();
  for (const d of dataItems()) {
    const own = [d.name, d.id.split('.').pop(), ...(d.tags || []).filter((t) => t.name === 'carried-as').flatMap((t) => t.value.split(','))];
    for (const w of own) if (w && w.trim().length > 1) words.set(w.trim().toLowerCase(), d.id);
  }
  const alts = [...words.keys()].sort((a, b) => b.length - a.length).map(escapeRe);
  S.dataWords = { words, re: alts.length ? new RegExp(`(?<![\\p{L}\\p{N}_-])(${alts.join('|')})(?:e?s)?(?![\\p{L}\\p{N}_-])`, 'giu') : null };
  return S.dataWords;
}

function dataChip(id) {
  const d = S.byId.get(id);
  return el('span', { class: 'data-ref', 'data-data': id, tabindex: '0', role: 'button', title: d?.summary || id }, `◇ ${d?.name ?? id}`);
}

/** Data names in payload text (step cards, edge words) become data chips. */
function linkData(root) {
  const { words, re } = dataWords();
  if (!root || !re) return;
  root.querySelectorAll('.hop-carry, .hop-has li, .edges .badge, .carry').forEach((elm) => {
    const walker = document.createTreeWalker(elm, NodeFilter.SHOW_TEXT);
    const texts = [];
    while (walker.nextNode()) if (!walker.currentNode.parentElement.closest('.data-ref')) texts.push(walker.currentNode);
    for (const tn of texts) {
      const text = tn.nodeValue;
      const frag = document.createDocumentFragment();
      let last = 0;
      let any = false;
      re.lastIndex = 0;
      for (let m = re.exec(text); m; m = re.exec(text)) {
        const id = words.get(m[1].toLowerCase());
        if (!id) continue;
        any = true;
        frag.append(text.slice(last, m.index), el('span', { class: 'data-ref inline', 'data-data': id, tabindex: '0', role: 'button', title: S.byId.get(id)?.summary || id }, m[0]));
        last = m.index + m[0].length;
      }
      if (any) {
        frag.append(text.slice(last));
        tn.replaceWith(frag);
      }
    }
  });
}

function renderDataButton() {
  const b = $('#btn-data');
  if (!b) return;
  const n = dataItems().length;
  b.hidden = !n;
  b.textContent = `◇ Data (${n})`;
  b.onclick = () => renderData();
}

/***
 * The data list: what users and the code make, who makes it, and how many lines carry it.
 *
 * @id      docair-viewer.data
 * @part-of docair-viewer.page
 ***/
function renderData() {
  const box = $('#details');
  const items = dataItems().sort((a, b) => a.name.localeCompare(b.name));
  box.replaceChildren(
    ...[
      el('h2', {}, el('span', { class: 'glyph' }, '◇'), 'Data'),
      el('div', { class: 'meta' }, 'What users and the code make and carry. The boxes of the chart are the working parts; data travels along their lines.'),
      el(
        'ul',
        { class: 'data-list', 'data-test': 'data-list' },
        items.map((d) => {
          const carriers = S.graph.edges.filter((e) => (e.carries || []).includes(d.id)).length;
          const makers = S.graph.edges.filter((e) => e.to === d.id && e.kind === 'create');
          return el(
            'li',
            {},
            el('div', {}, el('a', { class: 'link', onclick: () => showData(d.id) }, `◇ ${d.name}`), el('span', { class: 'badge' }, `${carriers} lines`), carriers ? el('a', { class: 'link follow', onclick: () => followData(d.id), 'data-test': 'follow' }, 'Follow') : null),
            el('div', { class: 'muted' }, d.summary),
            makers.length ? el('div', { class: 'made-by' }, 'made by ', makers.map((e, i) => [i ? ', ' : '', nodeLink(e.from)])) : null,
          );
        }),
      ),
      S.selected && !isData(S.selected) ? el('a', { class: 'link', onclick: () => renderDetails(S.selected) }, `← back to ${S.byId.get(S.selected)?.name}`) : null,
    ].filter(Boolean),
  );
  linkTerms(box);
}

/** One datum: what it is, who makes it, which lines carry it. */
function showData(id) {
  const d = S.byId.get(id);
  const box = $('#details');
  const makers = S.graph.edges.filter((e) => e.to === id && e.kind === 'create');
  const lines = S.graph.edges.filter((e) => (e.carries || []).includes(id));
  box.replaceChildren(
    ...[
      el('div', { class: 'gl-nav' }, el('a', { class: 'link', onclick: () => renderData() }, '← Data')),
      el('h2', { 'data-test': 'data-title' }, el('span', { class: 'glyph' }, '◇'), d.name, el('span', { class: 'badge' }, 'data')),
      d.summary ? el('p', { class: 'summary' }, d.summary) : null,
      lines.length ? el('button', { class: 'btn', onclick: () => followData(id), 'data-test': 'follow-data' }, S.following === id ? 'Following: on the chart' : 'Follow it on the chart') : null,
      makers.length ? [el('h3', { class: 'sec' }, 'Made by'), el('ul', { class: 'edges' }, makers.map((e) => el('li', {}, nodeLink(e.from), e.text ? el('span', { class: 'badge' }, e.text) : null)))] : null,
      lines.length ? [el('h3', { class: 'sec' }, `Carried by (${lines.length})`), el('ul', { class: 'edges' }, lines.map((e) => el('li', {}, el('span', { class: 'ek' }, e.kind), nodeLink(e.from), ' → ', nodeLink(e.to), el('span', { class: 'badge' }, edgeText(e)))))] : null,
    ]
      .flat(Infinity)
      .filter(Boolean),
  );
  linkTerms(box);
}

/***
 * Follow one datum on the chart: the lines that carry it light up, everything else fades.
 *
 * @id      docair-viewer.follow
 * @part-of docair-viewer.page
 ***/
function followData(id) {
  S.following = id || null;
  renderFollowBar();
  if (!id) {
    applyFollow();
    if (S.flow?.source === 'data') setFlow('');
    return;
  }
  const tour = dataTour(id);
  S.tours = [...S.tours.filter((t) => t.source !== 'data'), tour];
  renderFlowBar();
  if (tour.steps.length) setFlow(tour.key);
  else applyFollow();
}

/***
 * The way of one datum as a walkthrough: where it is made, then each line that carries it, in the order it travels from there.
 *
 * @id      docair-viewer.data-tour
 * @part-of docair-viewer.page
 * @calls   docair-viewer.walk
 ***/
function dataTour(id) {
  const d = S.byId.get(id);
  const edges = S.graph.edges;
  const makers = edges.filter((e) => e.to === id && e.kind === 'create' && !isData(e.from));
  const carries = (i) => (edges[i].carries || []).includes(id);
  const usable = (i) => !isData(edges[i].from) && !isData(edges[i].to) && edges[i].kind !== 'cover';
  const out = new Map();
  edges.forEach((e, i) => {
    if (!usable(i)) return;
    if (!out.has(e.from)) out.set(e.from, []);
    out.get(e.from).push(i);
  });
  const order = [];
  const seen = new Set();
  const reached = new Set(makers.map((e) => e.from));
  let frontier = [...reached];
  for (let guard = 0; guard < 200; guard++) {
    // Along the lines that carry it, breadth first.
    while (frontier.length) {
      const next = [];
      for (const f of frontier)
        for (const i of out.get(f) || []) {
          if (seen.has(i) || !carries(i)) continue;
          seen.add(i);
          order.push({ i, inside: false });
          if (!reached.has(edges[i].to)) {
            reached.add(edges[i].to);
            next.push(edges[i].to);
          }
        }
      frontier = next;
    }
    // Lines that do not name it can still carry it inside (a request through the network):
    // the shortest way along the arrows from where it is to the next line that names it.
    const prev = new Map();
    const queue = [...reached].map((n) => [n, 0]);
    const visited = new Set(reached);
    let bridge = null;
    while (queue.length && !bridge) {
      const [n, depth] = queue.shift();
      if (depth >= 6) continue;
      for (const i of out.get(n) || []) {
        const to = edges[i].to;
        if (carries(i) && !seen.has(i) && reached.has(n) === false) {
          bridge = { end: n, via: i };
          break;
        }
        if (visited.has(to)) continue;
        visited.add(to);
        prev.set(to, i);
        if ((out.get(to) || []).some((j) => carries(j) && !seen.has(j))) {
          bridge = { end: to };
          break;
        }
        queue.push([to, depth + 1]);
      }
    }
    if (!bridge) break;
    const hops = [];
    for (let n = bridge.end; prev.has(n); n = edges[prev.get(n)].from) hops.unshift(prev.get(n));
    for (const i of hops) {
      seen.add(i);
      order.push({ i, inside: true });
      reached.add(edges[i].to);
    }
    frontier = [bridge.end];
  }
  const steps = [
    ...makers.map((e) => ({ title: `${nameOf(e.from)} makes ${d.name}`, text: e.text ? `${e.text[0].toUpperCase()}${e.text.slice(1)}.` : '', nodes: [e.from], path: [] })),
    ...order.map(({ i, inside }) => {
      const e = edges[i];
      const p = messageParts({ kind: e.kind, e, i });
      const how = inside ? `carries ${d.name} inside ${edgeText(e)}` : p.carry;
      return { title: p.who.join(' '), text: [e.text, how].filter(Boolean).join(' · '), nodes: [e.to], path: [i] };
    }),
  ].map((st, k) => ({ ...st, step: k + 1 }));
  return { key: `data:${id}`, name: id, source: 'data', title: `Follow ${d.name}`, summary: d.summary, steps };
}

function applyFollow() {
  const cy = S.cy;
  if (!cy) return;
  cy.elements().removeClass('carry carry-dim');
  if (!S.following) return;
  const on = new Set();
  S.graph.edges.forEach((e, i) => {
    if ((e.carries || []).includes(S.following)) on.add(S.edgeOf.get(i));
  });
  const nodes = new Set();
  cy.edges().forEach((e) => {
    if (on.has(e.id())) {
      e.addClass('carry');
      nodes.add(e.source().id());
      nodes.add(e.target().id());
    } else {
      e.addClass('carry-dim');
      cy.getElementById(`${e.id()}::label`).addClass('carry-dim');
    }
  });
  cy.nodes().forEach((n) => {
    if (!n.isParent() && !n.hasClass('elabel') && !n.hasClass('token') && !nodes.has(n.id())) n.addClass('carry-dim');
  });
}

function renderFollowBar() {
  const bar = $('#follow-bar');
  if (!bar) return;
  bar.hidden = !S.following;
  if (!S.following) return bar.replaceChildren();
  const d = S.byId.get(S.following);
  bar.replaceChildren('following ', dataChip(S.following), el('button', { class: 'chip-x', onclick: () => followData(null), title: 'Stop following', 'data-test': 'unfollow' }, '×'));
  bar.title = d?.summary || '';
}

window.docair = { walkMessages, setLayer, openStep, closeStep, projectTour, followData, showData, renderData, showOnly, revealWithin, stepMessages, messageParts, showTerm, renderGlossary, openDoc, linkTerms, previewTerms, previewWording, edgeText, state: S, select, setFlow, goStep, render, rep, drawnRep, showLayer, layerFor, computeView, layerName, drill, drillUp, addWalkthrough, setHidden, setTests, hiddenNode };
boot().catch((err) => {
  console.error('docair: viewer failed to start', err);
  document.body.dataset.ready = 'error';
});

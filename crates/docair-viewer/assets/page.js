// The documentation's script (docs/SLICES.md §7). It unpacks what the CLI drew and wrote, shows
// it by its # address, pans and zooms, follows links, and hides what the reader asks it to hide.
// It decides nothing about what a picture holds, places nothing, measures no text and writes no
// words of its own.
'use strict';

const $ = (sel) => document.querySelector(sel);
const IX = JSON.parse($('#docair-index').textContent);
const PACKED = JSON.parse($('#docair-packed').textContent);
/** What the page shows: its address, the picture's whole extent, the part of it in view, what is selected and hidden. */
const S = { addr: null, svg: null, full: null, view: null, tests: IX.tests, hidden: new Set(), selected: null, run: 0 };
/** The most a picture is enlarged on screen, as the page before the CLI drew it (MAX_FIT_ZOOM). */
const MOST = 1.25;
const unpacked = new Map();
/** The panels the CLI wrote: details, tips, the lines' tips, search words, names, the box around each part. */
let P = null;
/** The walk on screen: its steps, the step shown, the run of tokens moving. */
let W = null;
let WALKS = null;
/** The details panel before anything is selected: the walks to start. */
const EMPTY = $('#details').innerHTML;
/** A walk's camera: never closer than this, never farther (TOUR_MIN_ZOOM, TOUR_MAX_ZOOM). */
const WALK_ZOOM = [0.8, 1.3];

/***
 * Unpack something the CLI packed: base64, then the browser's own inflate.
 *
 * @id      docair-viewer.doc-page.unpack
 * @part-of docair-viewer.doc-page
 ***/
const inflate = (i) => new Response(new Blob([Uint8Array.from(atob(PACKED[i]), (c) => c.charCodeAt(0))]).stream().pipeThrough(new DecompressionStream('deflate-raw')));
async function unpack(i) {
  if (!unpacked.has(i)) unpacked.set(i, await inflate(i).text());
  return unpacked.get(i);
}

const esc = (id) => CSS.escape(id);
/** The address a # names, the first picture when it names none. */
const addressOf = (hash) => decodeURIComponent(hash.replace(/^#/, '')) || IX.first;
const download = (text, name) => Object.assign(document.createElement('a'), { href: URL.createObjectURL(new Blob([text], { type: 'text/x-diff' })), download: name }).click();
const go = (addr) => (location.hash = `#${addr.split('/').map(encodeURIComponent).join('/')}`);

/***
 * Show what an address names: a layer, a box's section, a slice, or a part in its best picture.
 *
 * A part the picture on screen draws stays where it is; any other goes to the picture the CLI
 * named for it.
 *
 * @id      docair-viewer.doc-page.show
 * @part-of docair-viewer.doc-page
 * @calls   docair-viewer.doc-page.unpack "unpacks the picture"
 ***/
async function show(addr, keepView = false) {
  const run = ++S.run;
  let part = null;
  let moved = false;
  if (addr.startsWith('part/')) {
    part = addr.slice(5);
    if (drawnHere(part)) return select(part, true);
    addr = IX.home[part] || IX.first;
    moved = true;
  }
  if (!IX.pictures[addr]) addr = IX.first;
  const p = IX.pictures[addr];
  const text = await unpack(p.svg ?? (S.tests ? p.on : p.off));
  if (run !== S.run) return;
  const view = S.view;
  $('#pic').innerHTML = text;
  [S.svg, S.addr] = [$('#pic > svg'), addr];
  // The legend explains what this picture holds, and no more.
  for (const r of document.querySelectorAll('#legend .row')) r.hidden = !S.svg.querySelector(r.dataset.in);
  for (const g of document.querySelectorAll('#legend .group')) g.hidden = !g.querySelector('.row:not([hidden])');
  const [x, y, w, h] = S.svg.getAttribute('viewBox').split(' ').map(Number);
  S.full = { x, y, w, h };
  if (keepView && view) setView(view);
  else fit();
  crumbs(p);
  for (const a of document.querySelectorAll('#layers a')) a.classList.toggle('on', a.dataset.layer === p.layer);
  document.title = `docair · ${IX.system} · ${p.title}`;
  follow(p);
  applyHidden();
  search();
  for (const a of document.querySelectorAll('.tour[data-walk]')) a.classList.toggle('on', a.dataset.walk === addr);
  const left = !!W;
  if (p.walk) await startWalk(addr);
  else W = null;
  if (part) {
    select(part, true);
    // A part picked by its address that took the reader elsewhere: the page says where.
    if (moved && p.where) toast(p.where);
  } else if (S.selected && drawnHere(S.selected)) {
    mark(S.selected);
    if (left && !W) details(S.selected);
  } else if (W || left) details(null);
  if (p.change && !part) details(null);
  else if (addr.startsWith('box/') && !part && !W) details((S.selected = addr.slice(4)));
}

/** Following a datum: its slice is on screen, and the bar above the picture says so. */
function follow(p) {
  const bar = $('#follow-bar');
  bar.hidden = !p.datum;
  if (!p.datum) {
    if (!p.walk) S.before = S.addr;
    return;
  }
  const chip = document.createElement('span');
  chip.className = 'data-ref';
  chip.dataset.data = p.datum;
  chip.tabIndex = 0;
  chip.textContent = `◇ ${P.names[p.datum] ?? p.datum}`;
  bar.querySelector('.fb-chip').replaceChildren(chip);
}

/** A page of the details panel: a term, the glossary, the data, a datum; a way back to the selected part. */
function panel(html) {
  $('#details').innerHTML = html || '';
  for (const b of document.querySelectorAll('#details [data-back]')) b.hidden = !S.selected;
}

/***
 * Open a doc of the project inside the page, at a heading: the "read more" of a glossary entry.
 *
 * @id      docair-viewer.doc-page.reader
 * @part-of docair-viewer.doc-page
 ***/
function openDoc(target) {
  const [file, anchor] = target.split('#');
  if (!P.docs[file]) return;
  const reader = $('#reader');
  reader.innerHTML = P.docs[file];
  reader.hidden = false;
  const body = reader.querySelector('.reader-body');
  const at = anchor && body.querySelector(`[id="${esc(anchor)}"]`);
  if (at) {
    at.scrollIntoView({ block: 'start' });
    at.classList.add('flash');
  } else body.scrollTop = 0;
}

/***
 * The popups of marked words, of parts named in code and of data, as the CLI wrote them; and what a click on them opens.
 *
 * @id      docair-viewer.doc-page.popups
 * @part-of docair-viewer.doc-page
 * @calls   docair-viewer.doc-page.reader "opens a doc"
 ***/
function wirePopups() {
  const pop = $('#term-pop');
  // Tidy's banner stays closed for this reader until the count changes.
  if ($('#tidy-banner') && stored('localStorage', `docair:tidy:${IX.system}`) === $('#tidy-banner').dataset.count) $('#tidy-banner').hidden = true;
  // The data a picture's line words name are marked too (rect.chip, tspan.d with data-data).
  const marked = '.term, .node-ref, .data-ref, #pic [data-data]';
  let timer = 0;
  const open = (t) => {
    const html = P.pops[t.dataset.term ? `term:${t.dataset.term}` : t.dataset.node ? `node:${t.dataset.node}` : `data:${t.dataset.data}`];
    if (!html) return;
    pop.innerHTML = html;
    pop.hidden = false;
    const r = t.getBoundingClientRect();
    const [w, h] = [pop.offsetWidth, pop.offsetHeight];
    pop.style.left = `${Math.max(8, Math.min(innerWidth - w - 8, r.left))}px`;
    pop.style.top = `${Math.max(8, r.bottom + 6 + h > innerHeight ? r.top - h - 6 : r.bottom + 6)}px`;
  };
  document.addEventListener('mouseover', (e) => {
    const t = e.target.closest?.(marked);
    if (t && !t.closest('#term-pop')) {
      clearTimeout(timer);
      timer = setTimeout(() => open(t), 120);
    } else if (e.target.closest?.('#term-pop')) clearTimeout(timer);
  });
  document.addEventListener('mouseout', (e) => {
    if (e.target.closest?.(`${marked}, #term-pop`) && !e.relatedTarget?.closest?.(`#term-pop, ${marked}`)) {
      clearTimeout(timer);
      timer = setTimeout(() => (pop.hidden = true), 220);
    }
  });
  document.addEventListener('focusin', (e) => {
    const t = e.target.closest?.(marked);
    if (t && !t.closest('#term-pop')) open(t);
  });
  document.addEventListener('click', (ev) => {
    const a = ev.target.closest?.('a[href]');
    if (a) {
      pop.hidden = true;
      const href = a.getAttribute('href');
      if (href.startsWith('docair-doc:')) {
        ev.preventDefault();
        openDoc(href.slice('docair-doc:'.length));
      } else if (href.startsWith('#') && !ev.defaultPrevented && !ev.ctrlKey && !ev.metaKey) {
        // An address in the page: in a frame made from srcdoc, the link itself would load the frame's parent (docair-sra).
        ev.preventDefault(), (location.hash = href);
      } else if (/^https?:/.test(href)) {
        a.target = '_blank';
        a.rel = 'noopener';
      }
      return;
    }
    const t = ev.target.closest?.('.term, [data-term-open]');
    const d = ev.target.closest?.('.data-ref, [data-about], #pic [data-data]');
    const n = ev.target.closest?.('.node-ref');
    if (t || d || n) pop.hidden = true;
    if (t) panel(P.terms[t.dataset.term || t.dataset.termOpen]);
    else if (d) panel(P.details[d.dataset.data || d.dataset.about]);
    else if (n) go(`part/${n.dataset.node}`);
    else if (ev.target.closest?.('.line-ref')) (l => l && pickLine(l))([...(S.svg?.querySelectorAll('.line[data-edges]') || [])].find((x) => x.dataset.edges.split('\n').includes(ev.target.closest('.line-ref').dataset.lineEdge)));
    else if (ev.target.closest?.('[data-glossary], #btn-glossary')) panel(P.glossary);
    else if (ev.target.closest?.('[data-data-list], #btn-data')) panel(P.dataList);
    else if (ev.target.closest?.('#details [data-back]')) select(S.selected, false);
    else if (ev.target.closest?.('[data-test="reader-close"]')) $('#reader').hidden = true;
    else if (ev.target.closest?.('[data-test="unfollow"]')) go(S.before || IX.first);
    else if (ev.target.closest?.('[data-tidy-open], #btn-tidy')) panel(P.tidy);
    else if (ev.target.closest?.('[data-tidy-close]')) ($('#tidy-banner').hidden = true), stored('localStorage', `docair:tidy:${IX.system}`, $('#tidy-banner').dataset.count);
    else if (ev.target.closest?.('[data-test="tidy-copy"]')) navigator.clipboard?.writeText(ev.target.dataset.command).then(() => toast(ev.target.dataset.copied), () => {});
    else if (ev.target.closest?.('[data-changes-list], #btn-changes')) panel(P.changes);
    else if (ev.target.closest?.('#btn-walks')) {
      panel(P.walks);
      for (const a of document.querySelectorAll('.tour[data-walk]')) a.classList.toggle('on', a.dataset.walk === S.addr);
    }
    // ✦ Ask about this: the thing goes along with the next question, as a chip in the panel.
    const about = ev.target.closest?.('[data-ref]');
    if (about) {
      S.refs = [...(S.refs || []).filter(([r]) => r !== about.dataset.ref), [about.dataset.ref, about.dataset.label]];
      if ($('#chat')?.hidden) $('#btn-ask')?.click();
      context();
    }
  });
  $('#details').addEventListener('input', (ev) => {
    if (!ev.target.matches('.gl-search')) return;
    const q = ev.target.value.trim().toLowerCase();
    for (const dt of document.querySelectorAll('#details .gl-list dt')) {
      dt.hidden = !!q && !dt.dataset.words.includes(q);
      dt.nextElementSibling.hidden = dt.hidden;
    }
  });
}

/***
 * The AI (docs/SLICES.md §8): the reader's questions go to docair serve, which answers in HTML. A reply that wrote the documentation again reloads the page where it was; the conversation stays.
 *
 * The server is the page's own address when it serves the page (relative, so an upload served under its own path asks there), else <meta name="docair-serve">; without one the panel says so.
 *
 * @id      docair-viewer.doc-page.chat
 * @part-of docair-viewer.doc-page
 ***/
function wireChat() {
  const chat = $('#chat');
  if (!chat) return;
  const meta = document.querySelector('meta[name="docair-serve"]')?.content;
  const base = location.protocol.startsWith('http') ? '.' : meta ? meta.replace(/\/$/, '') : null;
  const key = `docair:chat:${IX.system}`;
  // The conversation is kept across reloads, for the tab.
  let state = stored('sessionStorage', key) || { log: [], history: [], open: false };
  const save = () => stored('sessionStorage', key, state);
  const log = chat.querySelector('.log');
  const add = (e) => {
    const m = document.createElement('div');
    m.className = `msg ${e.role}`;
    m.dataset.test = `msg-${e.role}`;
    if (e.role === 'ai') m.innerHTML = e.html;
    else m.append(e.text, ...(e.chips || []).map(asked));
    if (e.applied) applied(m);
    if (e.undone) undone(m);
    log.append(m);
    log.scrollTop = log.scrollHeight;
    return m;
  };
  // A question's chips: each opens what it was about, as the chips above the question did; a line's selects it when drawn (docair-rgz).
  const asked = ([ref, label]) => {
    const [kind, id] = [ref.slice(0, ref.indexOf(':')), ref.slice(ref.indexOf(':') + 1).split('\n')[0]];
    const c = Object.assign(document.createElement('span'), { className: `asked ${{ part: 'node-ref', data: 'data-ref', term: 'term', edge: 'line-ref' }[kind] || ''}`, textContent: label, tabIndex: 0 });
    return Object.assign(c.dataset, { node: id, data: id, term: id, lineEdge: id, test: 'asked' }) && c;
  };
  const ids = (list) => (list || '').split(' ').filter(Boolean);
  const undone = (m) => {
    const v = m.querySelector('[data-test="view-change"]');
    if (!v) return;
    v.querySelector('[data-test="undo"]').hidden = true;
    v.querySelector('[data-undone]').hidden = false;
  };
  const applied = (m) => {
    const b = m.querySelector('[data-test="apply"]');
    if (!b) return;
    b.disabled = true;
    b.children[0].hidden = true;
    b.children[1].hidden = false;
  };
  state.log.forEach(add);
  const say = (e) => (state.log.push(e), add(e), save());
  const open = (on) => ((chat.hidden = !on), (state.open = on), save());
  $('#btn-ask')?.addEventListener('click', () => open(chat.hidden));
  chat.querySelector('[data-chat-close]').addEventListener('click', () => open(false));
  // New conversation (docair-702.38): the log, the history and the chips put in go; the next question starts afresh.
  chat.querySelector('[data-chat-new]').addEventListener('click', () => ((state.log = []), (state.history = []), log.querySelectorAll('.msg').forEach((m) => m.remove()), (S.refs = []), context(), save()));
  chat.hidden = !state.open;
  // Off, the panel says why: its own words, or the server's (an upload it no longer keeps).
  const off = (which, said) => {
    Object.assign(chat.querySelector(`[data-test="${which}"]`), { hidden: false }, said ? { textContent: said } : {});
    for (const b of chat.querySelectorAll('.tool, .send, .suggest, textarea')) b.disabled = true;
  };
  if (base === null) return off('no-server');
  fetch(`${base}/api/ai`)
    .then((r) => r.json())
    .then((ai) => {
      if (!ai.error) document.body.dataset.served = 'true';
      chat.querySelector('[data-test="model"]').textContent = ai.model || '';
      chat.querySelector('.model-name').textContent = ai.model || '';
      if (!ai.enabled) off('ai-off', ai.error);
    })
    .catch(() => off('no-server'));
  // Documentation written again from elsewhere (another tab, the project's files): the page reloads.
  let version = null;
  const now = () => fetch(`${base}/api/version`).then((r) => r.json()).then((v) => v.version);
  now().then((v) => (version = v)).catch(() => {});
  setInterval(() => {
    if (document.visibilityState !== 'visible' || version === null) return;
    now().then((v) => v > version && location.reload()).catch(() => {});
  }, 5000);
  const busy = (on) => {
    chat.querySelector('[data-test="typing"]').hidden = !on;
    for (const b of chat.querySelectorAll('.tool, .send, .suggest')) b.disabled = on;
  };
  const post = async (path, body) => {
    const r = await fetch(`${base}${path}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
    const a = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(a.error || `${r.status}`);
    return a;
  };
  const ask = async (path, body, said, chips) => {
    if (said) say({ role: 'user', text: said, ...(chips?.length ? { chips } : {}) });
    busy(true);
    try {
      const a = await post(path, body);
      const e = { role: 'ai', html: a.html || Object.assign(document.createElement('p'), { className: 'answer', textContent: a.reply?.answer || '' }).outerHTML };
      state.log.push(e);
      // A view change hides and shows at once, as the reader's own hiding.
      const view = add(e).querySelector('[data-test="view-change"]');
      if (view && typeof setHidden === 'function') setHidden(ids(view.dataset.hide), ids(view.dataset.show));
      if (said) state.history.push({ question: said, answer: a.reply?.answer || '', ...(body.context ? { context: body.context } : {}) });
      if (a.version) version = a.version;
      save();
      if (a.regenerated) location.reload();
    } catch (err) {
      say({ role: 'error', text: err.message });
    }
    busy(false);
  };
  const question = chat.querySelector('[data-test="question"]');
  const send = () => {
    const q = question.value.trim();
    if (!q) return;
    question.value = '';
    // The chips put in go with the question, into its bubble; the selection's chip stays (docair-rgz).
    const [refs, chips] = [context(), S.chips];
    (S.refs = []), context();
    ask('/api/ask', { question: q, history: state.history, ...(refs.length ? { context: refs } : {}) }, q, chips);
  };
  chat.querySelector('form').addEventListener('submit', (ev) => (ev.preventDefault(), send()));
  question.addEventListener('keydown', (ev) => ev.key === 'Enter' && !ev.shiftKey && (ev.preventDefault(), send()));
  for (const b of chat.querySelectorAll('.suggest')) b.addEventListener('click', () => ((question.value = b.textContent), send()));
  chat.querySelector('[data-test="word-lines"]').addEventListener('click', () => ask('/api/word', S.selected ? { scope: S.selected } : {}, null));
  chat.querySelector('[data-test="explain-terms"]').addEventListener('click', () => ask('/api/glossary', {}, null));
  log.addEventListener('click', async (ev) => {
    const m = ev.target.closest('.msg');
    const review = ev.target.closest('[data-test="review"]');
    if (review) {
      const diff = m.querySelector('[data-test="diff"]');
      diff.hidden = !diff.hidden;
      review.children[0].hidden = !diff.hidden;
      review.children[1].hidden = diff.hidden;
    }
    if (ev.target.closest('[data-test="download-patch"]')) download(m.querySelector('[data-test="diff"]').textContent, 'docair.patch');
    const undo = ev.target.closest('[data-test="undo"]');
    if (undo) {
      const v = undo.closest('[data-test="view-change"]');
      setHidden(ids(v.dataset.show), []);
      setHidden([], ids(v.dataset.hide));
      undone(m);
      const k = [...log.querySelectorAll('.msg')].indexOf(m);
      if (state.log[k]) state.log[k].undone = true;
      save();
    }
    const apply = ev.target.closest('[data-test="apply"]');
    if (apply && !apply.disabled) {
      busy(true);
      try {
        const a = await post('/api/apply', {});
        applied(m);
        const k = [...log.querySelectorAll('.msg')].indexOf(m);
        if (state.log[k]) state.log[k].applied = true;
        if (a.version) version = a.version;
        save();
        if (a.regenerated) location.reload();
      } catch (err) {
        say({ role: 'error', text: err.message });
      }
      busy(false);
    }
  });
  // Tidy (docs/ADAPTERS.md §6.1): the server shows the patch and tidies the ticked tags, or one, then the page reloads.
  document.addEventListener('click', async (ev) => {
    if (ev.target.closest('[data-tidy-open], #btn-tidy')) fetch(`${base}/api/tidy`).then((r) => r.json()).then((t) => Object.assign($('#details [data-test="tidy-diff"]') || {}, { textContent: t.patch || '', hidden: !t.patch })).catch(() => {});
    if (ev.target.closest('[data-test="tidy-patch"]')) download((await (await fetch(`${base}/api/tidy`)).json()).patch || '', 'docair-tidy.patch');
    // Explaining is a job on the server: under the button, a bar and each change as soon as it is made (docair-8l6.11, 8l6.65); done, the page was written again.
    const explain = ev.target.closest('[data-test="explain"]');
    if (explain) {
      const shown = (j) => Object.assign(explain.nextElementSibling, { innerHTML: j.html || j.error || '', hidden: false });
      [explain.disabled, explain.textContent] = [true, '✦ Reviewing …'];
      let job = await post('/api/explain', { a: explain.dataset.a, b: explain.dataset.b || undefined }).catch((err) => ({ error: err.message }));
      for (; job.running; job = await fetch(`${base}/api/explain/${job.job}`).then((r) => r.json()).catch((err) => ({ error: err.message }))) {
        shown(job);
        await new Promise((r) => setTimeout(r, 1000));
      }
      [explain.textContent, explain.disabled] = [job.phase === 'done' ? '✓ Reviewed' : '✦ Review the changes', job.phase === 'done'];
      shown(job);
      if (job.phase === 'done') location.reload();
    }
    const one = ev.target.closest('[data-tidy-one]');
    if (!one && !ev.target.closest('[data-test="tidy-apply"]')) return;
    const tags = (one ? [one] : [...document.querySelectorAll('#details .tidy-tag input:checked')]).map((x) => ({ node: x.dataset.node, tag: x.dataset.tag }));
    const a = tags.length && (await post('/api/tidy', { tags }).catch((err) => toast(err.message)));
    if (a?.regenerated) location.reload();
  });
}

/** What a question is about: the selected part, until the reader clears it. */
function context() {
  const c = $('#chat [data-test="context"]');
  if (!c) return [];
  const sel = S.selected ? [[`part:${S.selected}`, P.names[S.selected] ?? S.selected]] : [];
  const refs = [...sel, ...(S.refs || []).filter(([r]) => r !== sel[0]?.[0])];
  c.querySelector('.refs').replaceChildren(
    ...refs.map(([ref, label]) => Object.assign(document.createElement('button'), { className: 'chip-x', type: 'button', textContent: `${label} ×`, title: ref, onclick: () => ((S.refs = (S.refs || []).filter(([r]) => r !== ref)), ref === sel[0]?.[0] && (S.selected = null), context()) })),
  );
  c.hidden = !(S.chips = refs).length;
  return refs.flatMap(([r]) => r.split('\n'));
}

/** A short note over the picture, for a few seconds. */
function toast(text) {
  const t = $('#toast');
  t.textContent = text;
  t.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (t.hidden = true), 3500);
}

/** The part as itself in the picture on screen. */
function drawnHere(id) {
  return !!S.svg?.querySelector(`.part[data-id="${esc(id)}"], .box[data-id="${esc(id)}"]`);
}

/** The way back up from this picture, as links. */
function crumbs(p) {
  const nav = $('#crumbs');
  nav.replaceChildren();
  p.crumbs.forEach(([label, addr], k) => {
    if (k) nav.append(' › ');
    const a = document.createElement('a');
    a.textContent = label;
    a.href = `#${addr}`;
    if (addr === S.addr) a.className = 'here';
    nav.append(a);
  });
}

/***
 * Select a part: its details, its row in the tree, and in the picture the part and its lines; brought into view when asked.
 *
 * @id      docair-viewer.doc-page.select
 * @part-of docair-viewer.doc-page
 ***/
function select(id, close) {
  S.selected = id;
  details(id);
  context(id);
  document.querySelectorAll('#tree-list .row.sel').forEach((r) => r.classList.remove('sel'));
  const li = document.querySelector(`#tree-list li[data-id="${esc(id)}"]`);
  if (li) {
    for (let up = li.parentElement; up && up.id !== 'tree-list'; up = up.parentElement) up.classList.remove('closed');
    li.querySelector(':scope > .row').classList.add('sel');
    li.scrollIntoView({ block: 'nearest' });
  }
  const g = mark(id);
  if (close && g) {
    const b = g.getBBox();
    look(b.x + b.width / 2, b.y + b.height / 2, Math.max(fitScale(), Math.min(1, MOST)));
  }
}

/** The details panel: a walk's step card on top, then the selected part's details; the walks to start when there is neither. */
function details(id) {
  const card = W ? W.steps[W.k].card : '';
  // A change's picture keeps its flow's changes under the step's card (docair-zrr.22).
  const more = id ? P.details[id] || '' : P.flowChanges?.[IX.pictures[S.addr]?.change] || '';
  $('#details').innerHTML = card || more ? card + more : EMPTY;
  hideLinks();
}

/***
 * Play a walk: each step lights its parts and lines as the CLI listed them, the camera goes to the step's box, and a token runs along each line the step goes along, in the order its messages happen.
 *
 * @id      docair-viewer.doc-page.walk
 * @part-of docair-viewer.doc-page
 * @calls   docair-viewer.doc-page.unpack "unpacks the walks' steps"
 ***/
async function startWalk(addr) {
  if (!WALKS) WALKS = JSON.parse(await unpack(IX.walks));
  const w = WALKS[addr];
  if (!w || S.addr !== addr) return;
  W = { steps: w.steps, k: 0, run: 0 };
  goStep(0);
}

function goStep(k) {
  W.k = Math.max(0, Math.min(W.steps.length - 1, k));
  const run = ++W.run;
  const st = W.steps[W.k];
  const done = new Set();
  const trail = new Set();
  for (const s of W.steps.slice(0, W.k)) {
    s.parts.forEach((id) => done.add(id));
    s.lines.forEach((l) => trail.add(String(l)));
  }
  const cur = new Set(st.parts);
  const active = new Set(st.lines.map(String));
  for (const g of S.svg.querySelectorAll('.part, .box')) {
    const id = g.dataset.id;
    g.classList.toggle('flow-current', cur.has(id));
    g.classList.toggle('flow-done', !cur.has(id) && done.has(id));
    g.classList.toggle('flow-dim', g.classList.contains('part') && !cur.has(id) && !done.has(id));
  }
  for (const l of S.svg.querySelectorAll('.line')) {
    const on = active.has(l.dataset.id);
    const was = !on && trail.has(l.dataset.id);
    l.classList.toggle('flow-active', on);
    l.classList.toggle('flow-trail', was);
    l.classList.toggle('flow-dim', !on && !was);
    const w = S.svg.querySelector(`.words[data-line="${l.dataset.id}"]`);
    w?.classList.toggle('flow-dim', !on && !was), w?.classList.toggle('flow-on', on);
  }
  details(S.selected && cur.has(S.selected) ? S.selected : null);
  if (st.camera) frame(st.camera, true);
  tokens(st, run);
}

/** Bring a box of the picture into view, readable when asked: a walk's camera stays between WALK_ZOOM. */
function frame([x, y, w, h], readable) {
  const r = $('#pic').getBoundingClientRect();
  let scale = Math.min(r.width / w, r.height / h);
  if (readable) scale = Math.max(WALK_ZOOM[0], Math.min(WALK_ZOOM[1], scale));
  const from = S.view;
  const to = { x: x + w / 2 - r.width / scale / 2, y: y + h / 2 - r.height / scale / 2, w: r.width / scale, h: r.height / scale };
  const t0 = performance.now();
  const ease = (t) => (t < 0.5 ? 2 * t * t : 1 - (-2 * t + 2) ** 2 / 2);
  const tick = (now) => {
    const t = Math.min(1, (now - t0) / 300);
    const e = ease(t);
    setView({ x: from.x + (to.x - from.x) * e, y: from.y + (to.y - from.y) * e, w: from.w + (to.w - from.w) * e, h: from.h + (to.h - from.h) * e });
    if (t < 1) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

/** The step's tokens, one after the other along their lines' own paths; the card marks the message moving. */
async function tokens(st, run) {
  S.svg.querySelector('.tokens')?.remove();
  const layer = document.createElementNS('http://www.w3.org/2000/svg', 'g');
  layer.setAttribute('class', 'tokens');
  S.svg.append(layer);
  const hops = [...document.querySelectorAll('#details .hop[data-line]:not([data-line=""])')];
  for (const [k, t] of st.tokens.entries()) {
    if (!W || W.run !== run) return;
    hops.forEach((h, j) => (h.classList.toggle('cur', j === k), h.classList.toggle('done', j < k)));
    const path = S.svg.querySelector(`.line[data-id="${t.line}"] > .ln`);
    if (!path) continue;
    const g = document.createElementNS('http://www.w3.org/2000/svg', 'g');
    g.setAttribute('class', t.back ? 'token reply' : 'token');
    g.innerHTML = '<circle r="6.5"></circle><text y="-12"></text>';
    g.querySelector('text').textContent = t.word;
    layer.replaceChildren(g);
    const length = path.getTotalLength();
    await new Promise((done) => {
      const t0 = performance.now();
      const tick = (now) => {
        if (!W || W.run !== run) return done();
        const f = Math.min(1, (now - t0) / 900);
        const p = path.getPointAtLength((t.back ? 1 - f : f) * length);
        g.setAttribute('transform', `translate(${p.x} ${p.y})`);
        if (f < 1) requestAnimationFrame(tick);
        else done();
      };
      requestAnimationFrame(tick);
    });
  }
  if (W && W.run === run) hops.forEach((h) => (h.classList.remove('cur'), h.classList.add('done')));
}

/***
 * Select a line: it stands out, and the details show its two ends and the lines of the graph it stands for, as the CLI wrote them.
 *
 * @id      docair-viewer.doc-page.pick-line
 * @part-of docair-viewer.doc-page
 ***/
function pickLine(l) {
  mark(null);
  S.selected = null;
  l.classList.add('sel'), S.svg.querySelector(`.words[data-line="${l.dataset.id}"]`)?.classList.add('sel');
  const box = $('#details');
  box.innerHTML = W ? W.steps[W.k].card : '';
  const head = document.createElement('h2');
  head.className = 'line-head';
  [l.dataset.from, l.dataset.to].forEach((id, k) =>
    head.append(...(k ? [' → '] : []), Object.assign(document.createElement('a'), { className: 'link', href: `#part/${encodeURIComponent(id)}`, title: id, textContent: P.names[id] ?? id })),
  );
  const lines = document.createElement('div');
  lines.className = 'line-edges';
  lines.dataset.test = 'line-details';
  lines.innerHTML = l.dataset.edges.split('\n').map((e) => P.lines[e] || '').join('');
  const ask = Object.assign(document.createElement('a'), { className: 'link ask-about', textContent: IX.words.ask });
  Object.assign(ask.dataset, { ref: l.dataset.edges.split('\n').map((e) => `edge:${e}`).join('\n'), label: head.textContent, test: 'ask-about' });
  box.append(head, ask, lines);
}

/***
 * Move a part by hand: the lines that touch it are routed again around everything that stays, by docair's own router built to WebAssembly (docair-layout-wasm). A move lasts until the page reloads.
 *
 * @id      docair-viewer.doc-page.move-part
 * @part-of docair-viewer.doc-page
 ***/
async function movePart(id, dx, dy, held) {
  const w = (S.router ||= (await WebAssembly.instantiate(await inflate(IX.router).arrayBuffer())).instance.exports);
  const p = IX.pictures[S.addr];
  const i = p.svg ?? (S.tests ? p.on : p.off);
  S.moved ||= {};
  const was = (S.moved[i] ||= JSON.parse(await unpack(IX.moves + i)));
  if (held?.done) return;
  was.first ||= was.layout;
  const widths = {};
  for (const l of S.svg.querySelectorAll('.line')) widths[l.dataset.id] = Number(l.querySelector('.ln').getAttribute('stroke-width'));
  const ask = new TextEncoder().encode(JSON.stringify({ input: was.input, layout: was.layout, part: id, dx, dy, widths }));
  const at = w.alloc(ask.length);
  new Uint8Array(w.memory.buffer, at, ask.length).set(ask);
  const res = w.move_part_json(at, ask.length);
  const [ptr, len] = [Number(res >> 32n), Number(res & 0xffffffffn)];
  const answer = JSON.parse(new TextDecoder().decode(new Uint8Array(w.memory.buffer, ptr, len)));
  w.free(ptr, len);
  if (answer.error) return held || toast(answer.error);
  if (!held) was.layout = answer.layout;
  // In place: a part moves; a box takes its new place and size, its name and mark follow its top; lines are drawn anew.
  const shift = (g, x, y) => {
    if (!g) return;
    g.dataset.t ??= g.getAttribute('transform') || '';
    g.setAttribute('transform', `translate(${x} ${y}) ${g.dataset.t}`);
  };
  for (const m of answer.parts) {
    const g = S.svg.querySelector(`[data-id="${esc(m.id)}"]`);
    const o = was.first.placement.parts[m.part];
    if (!g || !o || held?.along.some(([x]) => x === g)) continue;
    if (!g.classList.contains('box')) {
      shift(g, m.at.x - o.x, m.at.y - o.y);
      continue;
    }
    for (const r of g.querySelectorAll(':scope > rect')) for (const [k, v] of Object.entries({ x: m.at.x - m.at.w / 2, y: m.at.y - m.at.h / 2, width: m.at.w, height: m.at.h })) r.setAttribute(k, v);
    shift(g.querySelector('.bn'), m.at.x - o.x, m.at.y - m.at.h / 2 - (o.y - o.h / 2));
    shift(g.querySelector('.tf-mark'), m.at.x - m.at.w / 2 - (o.x - o.w / 2), m.at.y - m.at.h / 2 - (o.y - o.h / 2));
  }
  for (const l of answer.lines) {
    const g = S.svg.querySelector(`.line[data-id="${l.id}"]`);
    if (g && l.drawn) g.querySelector('.ln').setAttribute('d', l.drawn.d), g.querySelector('.ah').setAttribute('d', l.drawn.arrow);
    const o = was.first.words[l.line];
    if (l.words && o && !held) shift(S.svg.querySelector(`.words[data-line="${l.id}"]`), l.words.x - o.x, l.words.y - o.y);
  }
}

/** Mark a part in the picture, and the lines that touch it. */
function mark(id) {
  if (!S.svg) return null;
  S.svg.querySelectorAll('.sel, .hl').forEach((g) => g.classList.remove('sel', 'hl'));
  const g = S.svg.querySelector(`.part[data-id="${esc(id)}"], .box[data-id="${esc(id)}"]`);
  if (!g) return null;
  // The picked part is never dimmed: not by the search, not by a walk.
  g.classList.add('sel');
  g.classList.remove('dim');
  for (const l of S.svg.querySelectorAll('.line')) if (l.dataset.from === id || l.dataset.to === id) l.classList.add('hl'), S.svg.querySelector(`.words[data-line="${l.dataset.id}"]`)?.classList.add('hl');
  return g;
}

/** The scale at which the whole picture fits the canvas, never above MOST. */
function fitScale() {
  const r = $('#pic').getBoundingClientRect();
  return Math.min(MOST, r.width / S.full.w, r.height / S.full.h);
}

/** Look at a point of the picture at a scale: screen pixels per picture unit. */
function look(cx, cy, scale) {
  const r = $('#pic').getBoundingClientRect();
  setView({ x: cx - r.width / scale / 2, y: cy - r.height / scale / 2, w: r.width / scale, h: r.height / scale });
}

/** The whole picture, in the middle of the canvas. */
function fit() {
  look(S.full.x + S.full.w / 2, S.full.y + S.full.h / 2, fitScale());
  S.fitted = true;
}

function setView(v) {
  [S.view, S.fitted] = [v, false];
  S.svg.setAttribute('viewBox', `${v.x} ${v.y} ${v.w} ${v.h}`);
  // Words that sit on other lines show once the zoom makes them readable (svg.rs data-z).
  const px = Math.min(S.svg.clientWidth / v.w, S.svg.clientHeight / v.h);
  for (const w of S.svg.querySelectorAll('.words[data-z]')) w.classList.toggle('far', px < +w.dataset.z);
}

/** A point on the screen in the picture's units. */
function at(ev) {
  const r = S.svg.getBoundingClientRect();
  const u = Math.max(S.view.w / r.width, S.view.h / r.height);
  return { x: S.view.x + S.view.w / 2 + (ev.clientX - r.left - r.width / 2) * u, y: S.view.y + S.view.h / 2 + (ev.clientY - r.top - r.height / 2) * u, u };
}

/** Whether a part is left out by the reader: it, or a box around it, is hidden. */
function hiddenPart(id) {
  for (let cur = id; cur; cur = P.parents[cur]) if (S.hidden.has(cur)) return true;
  return false;
}

/***
 * Hide what the reader hid: in the picture its place stays empty, with its lines and their words; the tree, the details and the chips above the picture say so.
 *
 * @id      docair-viewer.doc-page.hide
 * @part-of docair-viewer.doc-page
 ***/
function applyHidden() {
  if (S.svg) {
    for (const g of S.svg.querySelectorAll('.part, .box')) g.classList.toggle('hidden', hiddenPart(g.dataset.id));
    for (const l of S.svg.querySelectorAll('.line')) {
      const off = hiddenPart(l.dataset.from) || hiddenPart(l.dataset.to);
      l.classList.toggle('hidden', off);
      S.svg.querySelector(`.words[data-line="${l.dataset.id}"]`)?.classList.toggle('hidden', off);
    }
  }
  for (const li of document.querySelectorAll('#tree-list li')) {
    const id = li.dataset.id;
    const own = S.hidden.has(id);
    const inside = !own && hiddenPart(id);
    li.classList.toggle('hid', own);
    li.classList.toggle('hid-in', inside);
    const eye = li.querySelector(':scope > .row > .eye');
    eye?.classList.toggle('off', own || inside);
    eye?.classList.toggle('inside', inside);
  }
  const bar = $('#hidden-bar');
  const chips = bar.querySelector('.hb-chips');
  chips.replaceChildren(
    ...[...S.hidden].map((id) => {
      const b = document.createElement('button');
      b.className = 'chip-x';
      b.dataset.test = 'hidden-chip';
      b.dataset.show = id;
      b.textContent = `${P.names[id] ?? id} ×`;
      return b;
    }),
  );
  bar.hidden = !S.hidden.size;
  bar.querySelector('[data-test="show-all"]').hidden = S.hidden.size < 2;
  hideLinks();
}

/** In the details: "Hide this section", or "Show this section" when it is hidden. */
function hideLinks() {
  const off = S.selected && hiddenPart(S.selected);
  $('#details [data-test="hide-section"]')?.toggleAttribute('hidden', !!off);
  $('#details [data-test="show-section"]')?.toggleAttribute('hidden', !off);
}

/** Hide parts and show others; a part inside a hidden section comes back with only itself in it. */
function setHidden(hide, show) {
  hideShow(hide, show);
  remember();
  applyHidden();
  if (S.selected) mark(hiddenPart(S.selected) ? null : S.selected);
}

/** Hide parts and show others in S.hidden: a part shown inside a hidden section brings that section back with only it. */
function hideShow(hide, show) {
  for (const id of hide) S.hidden.add(id);
  const kids = new Map();
  for (const [id, p] of Object.entries(P.parents)) kids.set(p, [...(kids.get(p) || []), id]);
  for (const id of show) {
    const path = [];
    for (let cur = id; cur; cur = P.parents[cur]) path.unshift(cur);
    const first = path.findIndex((x) => S.hidden.has(x));
    if (first < 0) continue;
    for (let j = first; j < path.length; j++) {
      S.hidden.delete(path[j]);
      if (j === path.length - 1) break;
      for (const sib of kids.get(path[j]) || []) if (sib !== path[j + 1]) S.hidden.add(sib);
    }
  }
}

const viewKey = () => `docair:page:${IX.system}`;
/** JSON in the browser's storage; where that throws (a sandboxed frame), window.name keeps it across reloads. */
function stored(store, key, value) {
  try {
    if (value === undefined) return JSON.parse(window[store].getItem(key) || 'null');
    window[store].setItem(key, JSON.stringify(value));
  } catch (_) {
    let all = {};
    try {
      all = JSON.parse(window.name || '{}');
    } catch (_e) {
      /* a name of its own: replaced */
    }
    if (value === undefined) return all[key] ?? null;
    window.name = JSON.stringify({ ...all, [key]: value });
  }
}
const remember = () => stored('localStorage', viewKey(), { hidden: [...S.hidden], tests: S.tests });
function recall() {
  // What docair.toml [view] hides starts hidden (docair-702.13); the chips show it again.
  hideShow((IX.view?.hide || []).filter((id) => id in P.names), (IX.view?.show || []).filter((id) => id in P.names));
  const saved = stored('localStorage', viewKey());
  if (Array.isArray(saved?.hidden)) S.hidden = new Set(saved.hidden.filter((id) => id in P.names));
  if (typeof saved?.tests === 'boolean') S.tests = saved.tests;
}

/** Show the tests or leave them out: the pictures as the CLI drew them with tests, or without. */
function setTests(on) {
  S.tests = on;
  remember();
  testsButton();
  if (S.addr) show(S.addr, true);
}
function testsButton() {
  const b = $('#btn-tests');
  if (!b) return;
  b.classList.toggle('on', S.tests);
  b.querySelector('.tests-on').hidden = !S.tests;
  b.querySelector('.tests-off').hidden = S.tests;
}

/** The tree and the picture, filtered by the search words the CLI wrote. */
function search() {
  const q = $('#search').value.trim().toLowerCase();
  const match = (id) => !q || (P.search[id] || '').includes(q);
  const visit = (li) => {
    let any = match(li.dataset.id);
    for (const child of li.querySelectorAll(':scope > ul > li')) any = visit(child) || any;
    li.classList.toggle('miss', !any);
    if (q && any && li.querySelector(':scope > ul')) li.classList.remove('closed');
    return any;
  };
  document.querySelectorAll('#tree-list > ul > li').forEach(visit);
  if (!S.svg) return;
  for (const g of S.svg.querySelectorAll('.part')) {
    g.classList.toggle('hit', !!q && match(g.dataset.id));
    g.classList.toggle('dim', !!q && !match(g.dataset.id) && g.dataset.id !== S.selected);
  }
}

/** The tip of a part, or of a line: the lines of the graph it stands for, the first 8 of them. */
function tip(ev) {
  const t = $('#tip');
  // Over a datum in a line's words, the datum's popup shows instead.
  if (ev.target.closest?.('#pic [data-data]')) return (t.hidden = true);
  const words = ev.target.closest?.('#pic .words');
  const g = words ? S.svg.querySelector(`.line[data-id="${words.dataset.line}"]`) : ev.target.closest?.('#pic .part, #pic .box, #pic .line');
  if (!g || !P) return (t.hidden = true);
  if (g.classList.contains('line')) {
    const edges = g.dataset.edges.split('\n');
    t.innerHTML = edges.slice(0, 8).map((i) => P.lines[i]).join('') + (edges.length > 8 ? `<div>${IX.words.more.replace('{n}', edges.length - 8)}</div>` : '');
  } else t.innerHTML = P.tips[g.dataset.id] || '';
  const r = $('#graph').getBoundingClientRect();
  t.style.left = `${Math.min(ev.clientX - r.left + 14, r.width - 300)}px`;
  t.style.top = `${ev.clientY - r.top + 14}px`;
  t.hidden = false;
}

/***
 * Zoom with the wheel around the pointer, pan by dragging, open a box by clicking or zooming into it, go up by zooming out past the picture; and the panels' links and buttons.
 *
 * @id      docair-viewer.doc-page.move
 * @part-of docair-viewer.doc-page
 ***/
function wire() {
  const pic = $('#pic');
  pic.addEventListener('wheel', (ev) => {
    if (!S.svg) return;
    ev.preventDefault();
    const f = Math.exp(Math.max(-60, Math.min(60, ev.deltaY)) * 0.004);
    const w = Math.min(S.full.w * 3, Math.max(40, S.view.w * f));
    const k = w / S.view.w;
    const p = at(ev);
    setView({ x: p.x - (p.x - S.view.x) * k, y: p.y - (p.y - S.view.y) * k, w, h: S.view.h * k });
    // Into a box that fills the canvas: its section. Out past the whole picture: the picture above.
    const box = ev.target.closest?.('.part.closed, .box');
    if (f < 1 && box && IX.pictures[`box/${box.dataset.id}`] && S.addr !== `box/${box.dataset.id}`) {
      const b = box.getBBox();
      if (b.width >= S.view.w * 0.8 || b.height >= S.view.h * 0.8) return go(`box/${box.dataset.id}`);
    }
    const crumbs = IX.pictures[S.addr].crumbs;
    if (f > 1 && S.view.w > S.full.w * 2.2 && S.view.h > S.full.h * 2.2 && crumbs.length > 1) go(crumbs[crumbs.length - 2][1]);
  }, { passive: false });
  let drag = null;
  pic.addEventListener('pointerdown', (ev) => {
    if (!S.svg || ev.button !== 0) return;
    // A part or a box is moved by hand, with what it holds; anywhere else the picture pans.
    const g = IX.pictures[S.addr]?.still ? null : ev.target.closest?.('#pic .part, #pic .box');
    const id = g?.dataset.id;
    const along = g ? [...S.svg.querySelectorAll('.part, .box')].filter((x) => { for (let c = x.dataset.id; c; c = P.parents[c]) if (c === id) return true; return false; }) : [];
    drag = { x: ev.clientX, y: ev.clientY, view: S.view, moved: false, id, along: along.map((x) => [x, x.getAttribute('transform') || '']) };
  });
  window.addEventListener('pointermove', (ev) => {
    if (!drag) return tip(ev);
    const [dx, dy] = [ev.clientX - drag.x, ev.clientY - drag.y];
    if (!drag.moved && Math.abs(dx) + Math.abs(dy) < 4) return;
    drag.moved = true;
    $('#tip').hidden = true;
    S.svg.classList.add('panning');
    const r = S.svg.getBoundingClientRect();
    const u = Math.max(S.view.w / r.width, S.view.h / r.height);
    [drag.dx, drag.dy] = [dx * u, dy * u];
    if (drag.id) for (const [x, t] of drag.along) x.setAttribute('transform', `translate(${drag.dx} ${drag.dy}) ${t}`);
    else setView({ ...drag.view, x: drag.view.x - dx * u, y: drag.view.y - dy * u });
    // While held, its lines are routed again each time it has moved a whole track (8 px); their words wait until it is put down.
    const track = `${Math.round(drag.dx / 8)} ${Math.round(drag.dy / 8)}`;
    if (drag.id && track !== drag.track) (drag.track = track), movePart(drag.id, drag.dx, drag.dy, drag);
  });
  window.addEventListener('pointerup', (ev) => {
    const was = drag;
    drag = null;
    S.svg?.classList.remove('panning');
    if (was?.moved && was.id) {
      was.done = true;
      for (const [x, t] of was.along) x.setAttribute('transform', t);
      return movePart(was.id, was.dx, was.dy);
    }
    if (!was || was.moved) return;
    const g = ev.target.closest?.('#pic .part, #pic .box');
    if (!g) {
      // A line, or its words (not a datum in them): the line is selected.
      const w = ev.target.closest?.('#pic .words');
      const l = w ? S.svg.querySelector(`.line[data-id="${w.dataset.line}"]`) : ev.target.closest?.('#pic .line');
      if (l && !ev.target.closest('#pic [data-data]')) pickLine(l);
      return;
    }
    // A box, open or closed, opens its section: what it holds, and only that; in its own section it is selected.
    const id = g.dataset.id;
    if (IX.pictures[`box/${id}`] && S.addr !== `box/${id}`) go(`box/${id}`);
    else select(id, false);
  });
  $('#tree-list').addEventListener('click', (ev) => {
    const li = ev.target.closest('li');
    if (!li) return;
    if (ev.target.closest('.eye')) return setHidden(hiddenPart(li.dataset.id) ? [] : [li.dataset.id], hiddenPart(li.dataset.id) ? [li.dataset.id] : []);
    if (ev.target.closest('.caret')) return li.classList.toggle('closed');
    go(`part/${li.dataset.id}`);
  });
  $('#details').addEventListener('click', (ev) => {
    const move = ev.target.closest('[data-step]');
    if (move && W) return goStep(W.k + Number(move.dataset.step));
    if (ev.target.closest('[data-test="whole-step"]') && W?.steps[W.k].camera) return frame(W.steps[W.k].camera, false);
    if (ev.target.closest('.card-close')) return go(S.before || IX.first);
    const hop = ev.target.closest('.hop[data-line]');
    if (hop && hop.dataset.line && S.svg) {
      const b = S.svg.querySelector(`.line[data-id="${hop.dataset.line}"]`)?.getBBox();
      if (b) frame([b.x - 40, b.y - 40, b.width + 80, b.height + 80], true);
    }
    const tab = ev.target.closest('.tabs button');
    if (tab) {
      for (const b of $('#details').querySelectorAll('.tabs button')) b.classList.toggle('on', b === tab);
      for (const t of $('#details').querySelectorAll('.tab')) t.hidden = t.dataset.tab !== tab.dataset.tab;
    }
    const hide = ev.target.closest('[data-hide]');
    if (hide) setHidden([hide.dataset.hide], []);
    const showIt = ev.target.closest('[data-show]');
    if (showIt) setHidden([], [showIt.dataset.show]);
  });
  $('#hidden-bar').addEventListener('click', (ev) => {
    const chip = ev.target.closest('[data-show]');
    if (chip) setHidden([], [chip.dataset.show]);
    if (ev.target.closest('[data-test="show-all"]')) setHidden([], [...S.hidden]);
  });
  $('#search').addEventListener('input', search);
  $('#btn-legend').addEventListener('click', () => ($('#legend').hidden = !$('#legend').hidden));
  $('#btn-tests')?.addEventListener('click', () => setTests(!S.tests));
  $('#btn-diag').addEventListener('click', () => ($('#drawer').hidden = !$('#drawer').hidden));
  $('#drawer').addEventListener('click', (ev) => {
    const row = ev.target.closest('[data-part]');
    if (row) go(`part/${row.dataset.part}`);
    $('#drawer').hidden = true;
  });
  $('#btn-fit').addEventListener('click', () => S.svg && fit());
  window.addEventListener('keydown', (ev) => {
    if (ev.target.closest?.('input, textarea')) return;
    if (ev.key === 'f' || ev.key === 'F') S.svg && fit();
    // A walk steps with the arrow keys too.
    if (W && (ev.key === 'ArrowRight' || ev.key === 'ArrowLeft')) goStep(W.k + (ev.key === 'ArrowRight' ? 1 : -1));
  });
  window.addEventListener('hashchange', () => show(addressOf(location.hash)));
  // When the canvas changes size, as when the chat opens, a fitted picture fits again; else it keeps its middle and scale.
  let box = null;
  new ResizeObserver(([e]) => {
    if (S.view && box?.width) S.fitted ? fit() : look(S.view.x + S.view.w / 2, S.view.y + S.view.h / 2, box.width / S.view.w);
    box = e.contentRect;
  }).observe($('#pic'));
}

/***
 * The side panels' widths (docair-jjk): each dragged at its inner edge, between 160 px and half the page,
 * kept per reader; a double-click puts it back.
 *
 * @id      docair-viewer.doc-page.wire-sides
 * @part-of docair-viewer.doc-page
 ***/
function wireSides() {
  const set = (k, w) => (w ? $('#app').style.setProperty(`--side-${k}`, `${w}px`) : $('#app').style.removeProperty(`--side-${k}`));
  for (const h of document.querySelectorAll('.side-handle')) {
    const k = h.dataset.side;
    set(k, +stored('localStorage', `docair:side-${k}`) || 0);
    h.addEventListener('pointerdown', (e) => {
      e.preventDefault(), h.setPointerCapture(e.pointerId), h.classList.add('on');
      const app = $('#app').getBoundingClientRect();
      h.onpointermove = (m) => set(k, Math.round(Math.max(160, Math.min(app.width / 2, k === 'tree' ? m.clientX - app.left : app.right - m.clientX))));
      h.onpointerup = () => ((h.onpointermove = null), h.classList.remove('on'), stored('localStorage', `docair:side-${k}`, parseInt($('#app').style.getPropertyValue(`--side-${k}`)) || null));
    });
    h.addEventListener('dblclick', () => (set(k, 0), stored('localStorage', `docair:side-${k}`, null)));
  }
}

unpack(IX.panels)
  .then((text) => {
    P = JSON.parse(text);
    recall();
    testsButton();
    wire();
    wirePopups();
    wireSides();
    wireChat();
    return show(addressOf(location.hash));
  })
  .then(() => {
    document.body.dataset.ready = 'true';
    // The page around this one (the web app) stops saying the page is on its way.
    if (window.parent !== window) window.parent.postMessage({ type: 'docair:ready' }, '*');
  });

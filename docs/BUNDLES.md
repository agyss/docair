# Bundles: one line for the many lines between two boxes

Design for docair-4wk.22: docair's Code layer takes 12 s to lay out on
docair's own page, and three quarters of that is the router. Not code yet: this says
what would change, what it costs and what it gains, so it can be decided.

## 1. The problem

Since docair reads calls and data from the code (ADAPTERS.md), its own Code
layer draws 907 lines between 433 parts. 701 of them come from the code.

| On docair's page, 1440×960 | Time |
|----------------------------|------|
| Code, first layout (two layout workers) | 12.0 s |
| … the placements (ELK): two directions, then two ways, each pair side by side | 2.5 s |
| … the router (gridLines), one step in one worker | 9.1 s (6.8 s on the page thread) |
| A drag on Code, per reroute | 376 ms (target: 50 ms) |

The router draws every line on its own: on tracks 8 px apart, each line the
cheapest way given all the others, then every line again, five rounds
(SPEC §3.5). That is 907 × 5 = 4 535 path searches on a grid of 1.2 million
points, about 2 ms each in a worker. Every round still makes the picture better
(crossings 10 772 after round one, 8 067 after round five), so stopping
earlier costs quality. Tried, and left out:

| Tried | Router | Picture |
|-------|--------|---------|
| route again only the lines that meet another line | 6.6 s | the same: 800 of 907 lines meet one |
| wider tracks | 25.5 s | worse: more crossings, lines on lines |
| other cell and corridor sizes | 5.0–7.2 s | mixed |

## 2. Bundles

A line between two parts in different boxes is drawn as part of a
**bundle**: one line between the two boxes, for all the lines that run
between them. The boxes are the two side by side in the box that holds both
ends, the same pair ELK places the boxes by, so the bundle runs where the
placement left room for it. Lines between two parts side by side in one box
stay as they are.

```
before                                 after
┌ docair-viewer.page ┐                 ┌ docair-viewer.page ┐
│ ƒ boot ───────────────┐              │ ƒ boot             │
│ ƒ render ──────────┐  │              │ ƒ render           │──29 calls──►┐
│ ƒ select ───────┐  │  │              │ ƒ select           │             │
└─────────────────│──│──│┘             └────────────────────┘             │
┌ docair-viewer ──▼──▼──▼┐             ┌ docair-viewer ─────────────────◄─┘
│ ƒ render_html  …       │             │ ƒ render_html  …                │
```

The viewer already does this for closed boxes (an edge with `members`, its
words summed up: `POST /api/scan · POST /api/ask +4`). Bundles extend it to
open boxes in a big picture.

**Measured** (scratch experiment on docair's page: one line routed for each
pair of boxes, the placement unchanged):

| Code layer | Now | Bundled |
|------------|-----|---------|
| lines routed | 907 | 589 (555 with ELK's pairs) |
| router | 6.9 s | 2.3 s |
| whole layout, page thread | 12.5 s | 7.5 s |
| crossings | 8 180 | 1 379 |
| pieces of line on line | 30 | 0 |
| bends per line | 4.8 | 2.7 |
| lines with a detour of 300 px or more | 122 | 39 |

Where the lines are now: 29 between `docair-viewer.page` and
`docair-viewer`, 27 from the layer tests to the page, 13 from
`docair-lang-rust` to `docair-core`; 84 pairs of boxes have several lines.

## 3. What the reader sees

* **The bundle:** a line a little thicker than one line, with its count and
  what it carries summed up, the way a closed box's lines are now:
  `29 calls · render_html, write_html +27`. Lines both ways are two bundles.
* **Hover:** the tip lists the lines in it (the first 8, then *… 21 more*),
  as for a closed box's lines now.
* **Picking a part:** its own lines light up. A bundle that carries one of
  them lights up too, and the details list each of the part's lines, as now.
* **A walk** (a scenario, a datum followed): a step along a line in a bundle
  moves along the bundle; the step card names the line itself.
* **Smaller pictures** keep every line: bundles only where a picture has
  more than a set number of lines.

## 4. The alternative: route in parallel

Each round, the lines are split between the two layout workers, routed side
by side, and put together. Lines routed side by side do not see each other's
new ways, so a round leaves more lines on lines, and may need a round more.
Estimate: 1.5–2× faster, about the same picture, more work than bundles, and
unsure. It does not make the picture easier to read; bundles do.

More workers do not help as things are: four instead of two leave Code at
12.0 s, since its placements already run side by side (the 40-port rule skips
two of the four ways) and the route is one step. Routing in parallel is the
one way more workers would pay off.

## 5. To decide

1. **Bundles or parallel routing?** Proposed: bundles.
2. **From how many lines on?** Proposed: more than 300 lines in a picture;
   on docair's page only Code bundles, the fixtures and the shop never do.
3. **The bundle's words:** the count and the first two calls, as proposed
   in §3, or the count alone (`29 calls`)?

## 6. Steps, once decided

1. Bundles in a big picture (visibleElements): lines between parts in two
   boxes side by side become one edge with members; the router and the
   walks already handle such edges. The layer script before and after.
2. The bundle's look and words; picking a part lights its bundles.
3. e2e: a fixture with enough lines to bundle; SPEC §3.4 and §3.5.

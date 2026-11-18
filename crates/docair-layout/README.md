# docair-layout

Lays out a picture of nested boxes, the way docair draws a system:

* **Text** is set in one monospace font, so a word's width is its number of
  characters times one advance (`text.rs`).
* **One box at a time, inside out.** The parts of a box are placed in layers
  (`layered.rs`): cycles broken by priority, layers that keep lines short,
  fewer crossings by sweeps, positions that keep lines straight. A box then
  shrinks what it holds to its size budget, never below its floor, and the
  next box out places it (`topdown.rs`).

* **Lines** are routed at right angles on tracks beside the parts, each the
  cheapest way given the others, round after round (`router.rs`, docair's own
  router, the same as viewer.js draws).
* **Words** of a line stand on it where they cover least (`word_spots.rs`).
* **Kept layouts:** `key` names a layout by what it reads and by the layout
  code itself, so a layout kept under that key can stand in for a new one.

The crate knows nothing of docair's graph: the input is parts with a size and
the box they are in, and lines between them (`Input`). docair-viewer maps a
layer's picture or a slice to it. See docs/SLICES.md §5.

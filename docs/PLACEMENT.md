# Placement by the lines

Epic docair-lj0. A picture's boxes can be placed by the lines between their
parts instead of in layers: parts with many lines between them end up side by
side. It is one more candidate placement, and the cheapest one wins as before
(SPEC §5).

## 1. Why

docair places each box on its own, inside out (SLICES.md §5): a box's parts in
layers, callers above what they call, ordered to cross few lines. That reads
top to bottom, but it is not chosen for short lines. On docair's Code layer
(894 parts, 2,173 lines), 94 % of the line length is in lines between boxes,
and 60 % in the 404 lines between the picture's top boxes, 6,946 px each.

Long lines cost twice: they cross more lines, and the router spends most of its
time on them (REGENERATE.md §5). Placing the parts takes 0.05 s; routing the
lines 28 s.

## 2. How a box is placed by its lines

A box's parts get their size first, inside out, as before. Then:

1. **The most connected part** goes first.
2. **The next part** is always the one with the most lines to those placed. It
   goes into the free place beside a placed part (to its right, below, to its
   left or above, at its start, middle or end) where its lines to the placed
   parts are shortest. A line's length is measured between the parts it really
   ends at, deep inside the two boxes, not between the boxes' middles.
3. **Making the box bigger** costs: 1 px for each px its width plus height
   grows, for each line of the part. So a part with no lines goes where the box
   stays smallest.
4. **Each part is taken out and put back** where it is best, three times over
   or until no part goes elsewhere.

Parts keep 96 px apart, so that lines have room between them.

A second placement by the lines starts from the first: each line that leaves a
box pulls its part the way it went there, 3 px of price for each px the part
goes that way (docair-lj0.7). The lines inside the box weigh as before.

## 3. Which placement a picture gets

The two placements by the lines, as it is and pulled, are two more candidates
besides the ones in layers (each direction, each way of SLICES.md §5):

| Picture | The winner |
|---------|-----------|
| up to 60 lines | every candidate routed; the one whose routed lines cost least |
| more than 60 lines | the one whose lines, drawn straight, cost least (length plus 400 px a crossing); only it is routed |

So a picture keeps its layers where they are cheaper, and nothing is routed
twice.

## 4. Measured

Tried on a copy of `docair-layout`, every placement routed by the same router
(the one with the coarse cells put right, docair-live.42).

| | area | line length | crossings | bends | on top of another line | price | routing |
|---|---|---|---|---|---|---|---|
| Code layer, layers | 81 M px² | 5.58 M px | 66,923 | 37,120 | 218,394 px | 91.0 M | 27.7 s |
| Code layer, by the lines | 23 M px² | 2.18 M px | 43,491 | 22,276 | 24,991 px | 32.2 M | 21.6 s |
| 50 other pictures, layers | 113 M px² | 3.12 M px | 23,223 | 17,410 | 4,963 px | 14.5 M | 9.8 s |
| 50 other pictures, by the lines | 65 M px² | 2.17 M px | 20,010 | 14,552 | 2,153 px | 11.5 M | 6.2 s |

The 50 other pictures are docair's sections and slices with more than 60 lines.
By the lines, 19 of them cost less, 4 more, 27 about the same.

Which placement wins, over the 51 pictures (routed price):

| | price |
|---|---|
| layers | 105.5 M |
| by the lines, everywhere | 43.7 M |
| by the lines as a candidate, chosen by its straight lines | 43.2 M |
| the better of the two, each routed | 42.8 M |

As a candidate, the lines win 37 of the 51 pictures, the Code layer among them.

Other ways that were measured:

| | Code layer: line length | price |
|---|---|---|
| by the lines at the picture's top only | 4.01 M px | 75.9 M |
| every box, 48 px apart | 1.76 M px | 145.9 M (lines forced through boxes) |
| every box, 160 px apart | 2.76 M px | 37.9 M |

Then the options for going on, each as one more candidate, over the same 51
pictures (by straight lines as in §3; the tree's placement by the lines before
the pull is the first row):

| | price | Code layer | line length | crossings | placing | routing |
|---|---|---|---|---|---|---|
| by the lines, 96 px apart | 43.6 M | 32.2 M | 4.55 M px | 62,796 | 0.13 s | 29.2 s |
| pulled, 1 px a px | 42.6 M | 31.4 M | 5.43 M px | 62,696 | 0.30 s | 31.2 s |
| **pulled, 3 px a px (built)** | **43.4 M** | 32.2 M | 4.50 M px | 62,464 | 0.36 s | 29.3 s |
| the gap from the lines between two parts, 64 + 12 px per root of their lines | 80.8 M | 68.7 M | 4.27 M px | 72,842 | 0.11 s | 33.5 s |
| the same, 96 + 8 px | 75.7 M | 63.6 M | 4.80 M px | 76,315 | 0.14 s | 36.6 s |
| layers in a box of functions, lines between boxes | 111.1 M | 99.8 M | 4.42 M px | 76,031 | 0.07 s | 30.6 s |
| three candidates, 72, 96 and 128 px apart | 57.3 M | 46.5 M | 4.06 M px | 65,490 | 0.36 s | 27.0 s |

Closer parts and layers inside look cheaper drawn straight, win, and then
route worse: straight lines do not show how crowded the routed ones get.
Routing every candidate and keeping the cheapest would give at most 41.9 M
(the first, both pulls, 72 and 128 px), for 116 s of routing instead of 29 s.

### 4.1 Boxes in boxes, and a placement that fits the canvas

docair-server's section was too cramped (docair-cvz): it sits nine boxes deep,
five of them shrank what they held to the floor 0.7 each, and its parts were
drawn at 0.123. Then the pulled placement by the lines won it at 3 % less than
the others, but 991 × 3309 px, at half their zoom. Two changes, measured over
the same 51 pictures, and as the share of the canvas a part gets on screen
(its scale times the picture's zoom):

| | price | Code layer | line length | Code layer's parts on screen, median / least | docair-server's parts on screen |
|---|---|---|---|---|---|
| before | 43.4 M | 32.2 M | 4.50 M px | 0.115 / 0.081 | 0.137 |
| only a placement that fits the canvas 0.8 times as well as the first competes | 43.6 M | 32.2 M | 4.51 M px | — | — |
| and nothing below the floor, 0.7, over all the boxes a part sits in | 55.0 M | 43.6 M | 5.88 M px | 0.121 / 0.096 | 0.399 |
| **and nothing below the floor squared, 0.49 (built)** | **40.2 M** | 28.9 M | 5.18 M px | 0.123 / 0.086 | 0.304 |

The price counts pixels, so a picture that shrinks less costs more for that
alone. With 0.49, `docair layer --measure` over docair's 133 pictures counts
73,811 crossings instead of 79,684; the Code layer 52,963 instead of 58,980.

## 5. Decisions

docair-lj0.3 to .5 and .7: the lines' placement is a candidate; every box in
it is placed by its lines; 96 px apart, the price on growing, three rounds; a
second one pulled by the lines that leave each box, 3 px a px (.6, not to
pull, is replaced by .7).

## 6. Steps

| Step | Issue | What |
|------|-------|------|
| 1 | docair-lj0.1 | `by_lines.rs`, the two candidates in `place` and `lay_out`, tests, SPEC §5 |
| 2 | docair-lj0.2 | the cluster runs the tree's image again |

Not built yet:

* **A judge that sees crowded lines:** the candidates are chosen by their
  straight lines, which favour parts too close together (§4).

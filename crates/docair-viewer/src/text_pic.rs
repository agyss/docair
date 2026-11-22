//! A picture drawn as text (docs/PULL_REQUESTS.md §3), for a place that shows no drawing: a
//! comment on a pull request, a terminal.
//!
//! It is the layout the SVG is drawn from, compacted into character cells. Every side of a part,
//! of a box's border, of a line's stretch and of a line's words is a value on its axis; values
//! keep their order, and two things that overlap along one axis stay apart along the other: one
//! cell, two beside a part, so an arrow fits in front of it. Then names and words get the cells
//! they need. What does not cross in the SVG does not cross here; sizes are not kept.
//!
//! Only characters every common monospace font has, so the columns line up: light and double box
//! drawing (WGL4), the four WGL4 arrows, and ASCII for what is gone.

use std::collections::HashMap;

use docair_core::change_steps::{FlowMarks, LineMark, PartMark};
use docair_core::layer_view::LayerPicture;
use docair_core::model::Graph;
use docair_layout::text::{WORDS_COLUMNS, columns, wrap};
use docair_layout::{Layout, Monospace, Rect};

use crate::layout_input;

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

/// Values of one axis closer than this, in px, are one column or row.
const SAME: f64 = 0.75;

/// How a border or a line is drawn: dashed in ASCII (gone), light, double (new). Where two meet,
/// the stronger wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
enum Pen {
    Dashed,
    #[default]
    Light,
    Double,
}

#[derive(Debug, Clone, Copy, Default)]
struct GridCell {
    bits: u8,
    pen: Option<Pen>,
    /// Words written over the cell; `'\0'` for the second half of a wide character.
    text: Option<char>,
}

/// One axis: the values the picture uses, and how many cells some of them must be apart.
#[derive(Default)]
struct AxisValues {
    values: Vec<f64>,
    /// `(a, b, k)`: the cell of `b` at least `k` after the cell of `a`.
    apart: Vec<(f64, f64, i64)>,
}

/// An axis solved: its values in groups, and the cell of each group.
struct AxisCells {
    groups: Vec<f64>,
    cell: Vec<i64>,
}

impl AxisValues {
    fn add(&mut self, v: f64) {
        if v.is_finite() {
            self.values.push(v);
        }
    }

    fn need(&mut self, a: f64, b: f64, k: i64) {
        self.add(a);
        self.add(b);
        self.apart.push((a, b, k));
    }

    /// Each group of values in the fewest cells: in order, a group may share the cell before it
    /// unless something needs them apart.
    fn solve(&self) -> AxisCells {
        let mut sorted = self.values.clone();
        sorted.sort_by(f64::total_cmp);
        let mut groups: Vec<f64> = Vec::new();
        let mut last = f64::NEG_INFINITY;
        for v in sorted {
            if v - last > SAME {
                groups.push(v);
            }
            last = v;
        }
        let at = |v: f64| group_of(&groups, v);
        let n = groups.len();
        let mut into: Vec<Vec<(usize, i64)>> = vec![Vec::new(); n];
        for &(a, b, k) in &self.apart {
            let (i, j) = (at(a), at(b));
            if i < j {
                into[j].push((i, k));
            }
        }
        let mut cell = vec![0i64; n];
        for j in 1..n {
            let mut c = cell[j - 1];
            for &(i, k) in &into[j] {
                c = c.max(cell[i] + k);
            }
            cell[j] = c;
        }
        AxisCells { groups, cell }
    }
}

/// The group a value is in: the last group that starts at or before it.
fn group_of(groups: &[f64], v: f64) -> usize {
    groups.partition_point(|&g| g <= v + SAME).saturating_sub(1)
}

impl AxisCells {
    fn at(&self, v: f64) -> i64 {
        if self.groups.is_empty() {
            return 0;
        }
        self.cell[group_of(&self.groups, v)]
    }
}

/// What kind of obstacle a thing in the picture is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObstacleKind {
    Part,
    /// One side of a box's border.
    Border,
    /// A box's name, in its top border.
    Name,
    /// A stretch of the line with this index (into the layout's lines).
    Stretch(usize),
    /// The words of the line with this index.
    Words(usize),
}

/// What takes room in the picture, as a rectangle in px: a part, one side of a box's border, a
/// stretch of a line, a line's words.
#[derive(Debug, Clone, Copy)]
struct Obstacle {
    l: f64,
    r: f64,
    t: f64,
    b: f64,
    what: ObstacleKind,
}

impl Obstacle {
    fn of(r: &Rect, what: ObstacleKind) -> Self {
        let (l, rr, t, b) = edges(r);
        Obstacle {
            l,
            r: rr,
            t,
            b,
            what,
        }
    }

    fn stretch(p: (f64, f64), q: (f64, f64), what: ObstacleKind) -> Self {
        Obstacle {
            l: p.0.min(q.0),
            r: p.0.max(q.0),
            t: p.1.min(q.1),
            b: p.1.max(q.1),
            what,
        }
    }
}

/// How far `a` reaches into `b` along an axis: below zero when they are apart.
fn into(a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    a1.min(b1) - a0.max(b0)
}

/// How far words are looked for a free place, in px, and in which steps.
const WORDS_REACH: f64 = 48.0;
const WORDS_STEP: f64 = 4.0;
/// A note's mark in the picture, `[n]`, in px at the words' size.
const NOTE_W: f64 = 24.0;
const NOTE_H: f64 = 14.0;

/// Whether a line's words, as `w`, lie on nothing they would write over: a part, other words, a
/// line, their own too. A box's border they may lie on, as in the SVG, where words have a background.
fn free_at(w: &Obstacle, i: usize, things: &[Obstacle]) -> bool {
    things.iter().enumerate().all(|(k, o)| {
        k == i
            || matches!(o.what, ObstacleKind::Border)
            || into(w.l, w.r, o.l, o.r) < -SAME
            || into(w.t, w.b, o.t, o.b) < -SAME
    })
}

/// The nearest free place for the words `things[i]`, within reach; none when there is none.
fn nearest_free(things: &[Obstacle], i: usize) -> Option<Obstacle> {
    let w = things[i];
    if free_at(&w, i, things) {
        return Some(w);
    }
    let mut d = WORDS_STEP;
    while d <= WORDS_REACH {
        for (dx, dy) in [(0.0, -d), (0.0, d), (-d, 0.0), (d, 0.0)] {
            let moved = Obstacle {
                l: w.l + dx,
                r: w.r + dx,
                t: w.t + dy,
                b: w.b + dy,
                what: w.what,
            };
            if free_at(&moved, i, things) {
                return Some(moved);
            }
        }
        d += WORDS_STEP;
    }
    None
}

/// Move each line's words to the nearest free place. Words with none in reach become a note: a
/// mark `[n]` where they were (the mark too moved to a free place, if there is one), and the words
/// under the picture. The lines whose words became notes, in order.
fn words_off(things: &mut [Obstacle]) -> Vec<usize> {
    let mut notes = Vec::new();
    for i in 0..things.len() {
        let ObstacleKind::Words(own) = things[i].what else {
            continue;
        };
        if let Some(moved) = nearest_free(things, i) {
            things[i] = moved;
            continue;
        }
        let w = things[i];
        let (x, y) = ((w.l + w.r) / 2.0, (w.t + w.b) / 2.0);
        things[i] = Obstacle {
            l: x - NOTE_W / 2.0,
            r: x + NOTE_W / 2.0,
            t: y - NOTE_H / 2.0,
            b: y + NOTE_H / 2.0,
            what: w.what,
        };
        if let Some(moved) = nearest_free(things, i) {
            things[i] = moved;
        }
        notes.push(own);
    }
    notes
}

/// Things that overlap along one axis stay apart along the other: one cell, two beside a part.
fn keep_apart(things: &[Obstacle], xs: &mut AxisValues, ys: &mut AxisValues) {
    let overlap = |a0: f64, a1: f64, b0: f64, b1: f64| a0 <= b1 + SAME && b0 <= a1 + SAME;
    for (i, a) in things.iter().enumerate() {
        for b in &things[i + 1..] {
            let gap = if a.what == ObstacleKind::Part || b.what == ObstacleKind::Part {
                2
            } else {
                1
            };
            if overlap(a.t, a.b, b.t, b.b) {
                if a.r + SAME < b.l {
                    xs.apart.push((a.r, b.l, gap));
                } else if b.r + SAME < a.l {
                    xs.apart.push((b.r, a.l, gap));
                }
            }
            if overlap(a.l, a.r, b.l, b.r) {
                if a.b + SAME < b.t {
                    ys.apart.push((a.b, b.t, gap));
                } else if b.b + SAME < a.t {
                    ys.apart.push((b.b, a.t, gap));
                }
            }
        }
    }
}

/// A part's or a box's sides in cells: left, right, top, bottom.
#[derive(Debug, Clone, Copy)]
struct CellSides {
    l: i64,
    r: i64,
    t: i64,
    b: i64,
}

impl CellSides {
    fn on_border(&self, (x, y): (i64, i64)) -> bool {
        let inside_x = x >= self.l && x <= self.r;
        let inside_y = y >= self.t && y <= self.b;
        ((x == self.l || x == self.r) && inside_y) || ((y == self.t || y == self.b) && inside_x)
    }
}

struct Grid {
    w: usize,
    h: usize,
    cells: Vec<GridCell>,
}

impl Grid {
    fn new(w: usize, h: usize) -> Self {
        Grid {
            w,
            h,
            cells: vec![GridCell::default(); w * h],
        }
    }

    fn cell(&mut self, x: i64, y: i64) -> Option<&mut GridCell> {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return None;
        }
        self.cells.get_mut(y as usize * self.w + x as usize)
    }

    fn mark(&mut self, x: i64, y: i64, bits: u8, pen: Pen) {
        if let Some(c) = self.cell(x, y) {
            c.bits |= bits;
            c.pen = Some(c.pen.map_or(pen, |p| p.max(pen)));
        }
    }

    /// A straight run from one cell to another, along a row or a column.
    fn run(&mut self, (x0, y0): (i64, i64), (x1, y1): (i64, i64), pen: Pen) {
        if (x0, y0) == (x1, y1) {
            return;
        }
        if y0 == y1 {
            let (a, b) = (x0.min(x1), x0.max(x1));
            for x in a..=b {
                let bits = if x > a { LEFT } else { 0 } | if x < b { RIGHT } else { 0 };
                self.mark(x, y0, bits, pen);
            }
        } else if x0 == x1 {
            let (a, b) = (y0.min(y1), y0.max(y1));
            for y in a..=b {
                let bits = if y > a { UP } else { 0 } | if y < b { DOWN } else { 0 };
                self.mark(x0, y, bits, pen);
            }
        } else {
            // Not at right angles once in cells: along the row first, then the column.
            self.run((x0, y0), (x1, y0), pen);
            self.run((x1, y0), (x1, y1), pen);
        }
    }

    fn outline(&mut self, s: CellSides, pen: Pen) {
        self.run((s.l, s.t), (s.r, s.t), pen);
        self.run((s.l, s.b), (s.r, s.b), pen);
        self.run((s.l, s.t), (s.l, s.b), pen);
        self.run((s.r, s.t), (s.r, s.b), pen);
    }

    /// Words written from a cell on, a wide character taking two cells.
    fn write(&mut self, x: i64, y: i64, words: &str) {
        let mut at = x;
        for ch in words.chars() {
            let w = columns(&ch.to_string()) as i64;
            if w == 0 {
                continue;
            }
            if let Some(c) = self.cell(at, y) {
                c.text = Some(ch);
            }
            if w == 2
                && let Some(c) = self.cell(at + 1, y)
            {
                c.text = Some('\0');
            }
            at += w;
        }
    }

    fn render(&self) -> String {
        let mut rows: Vec<String> = Vec::with_capacity(self.h);
        for y in 0..self.h {
            let mut row = String::new();
            for x in 0..self.w {
                let c = self.cells[y * self.w + x];
                match c.text {
                    Some('\0') => {}
                    Some(ch) => row.push(ch),
                    None => row.push(glyph(c.bits, c.pen.unwrap_or_default())),
                }
            }
            rows.push(row.trim_end().to_string());
        }
        while rows.last().is_some_and(String::is_empty) {
            rows.pop();
        }
        let first = rows
            .iter()
            .position(|r| !r.is_empty())
            .unwrap_or(rows.len());
        let mut out = rows[first..].join("\n");
        out.push('\n');
        out
    }
}

/// The character for a cell's directions, in its pen.
fn glyph(bits: u8, pen: Pen) -> char {
    const LIGHT: [char; 16] = [
        ' ', '│', '│', '│', '─', '┘', '┐', '┤', '─', '└', '┌', '├', '─', '┴', '┬', '┼',
    ];
    const DOUBLE: [char; 16] = [
        ' ', '║', '║', '║', '═', '╝', '╗', '╣', '═', '╚', '╔', '╠', '═', '╩', '╦', '╬',
    ];
    let b = (bits & 15) as usize;
    match pen {
        Pen::Light => LIGHT[b],
        Pen::Double => DOUBLE[b],
        Pen::Dashed => match bits & 15 {
            0 => ' ',
            UP | DOWN | 3 => ':',
            LEFT | RIGHT | 12 => '-',
            _ => '+',
        },
    }
}

/// What a mark puts before a name: `+ ` new, `− ` gone, `~ ` its code changed.
fn prefix(mark: Option<PartMark>) -> &'static str {
    match mark {
        Some(PartMark::Added) => "+ ",
        Some(PartMark::Removed) => "− ",
        Some(PartMark::Code) => "~ ",
        _ => "",
    }
}

fn part_pen(mark: Option<PartMark>) -> Pen {
    match mark {
        Some(PartMark::Added) => Pen::Double,
        Some(PartMark::Removed) => Pen::Dashed,
        _ => Pen::Light,
    }
}

/// Where a run of cells goes: -1, 0 or 1 along each axis.
fn towards(a: (i64, i64), b: (i64, i64)) -> (i64, i64) {
    ((b.0 - a.0).signum(), (b.1 - a.1).signum())
}

fn arrow((dx, dy): (i64, i64)) -> char {
    match (dx, dy) {
        (1, _) => '►',
        (-1, _) => '◄',
        (_, 1) => '▼',
        _ => '▲',
    }
}

fn edges(r: &Rect) -> (f64, f64, f64, f64) {
    (
        r.x - r.w / 2.0,
        r.x + r.w / 2.0,
        r.y - r.h / 2.0,
        r.y + r.h / 2.0,
    )
}

/// The legend under a picture drawn as text, with marks.
pub const LEGEND: &str = "`+` new (double) · `−` gone (dashed) · `~` its code changed";

/***
 * Draw a laid-out picture as text: boxes with their names in their top border, parts with their names inside, lines at right angles with an arrow in front of their target, and their words; with `marks`, what a change added (`+`, double), removed (`−`, dashed) or changed in its code (`~`).
 *
 * `layout` is the picture's layout, the one its SVG is drawn from (`layout_input::lay_out`).
 *
 * @calls docair-viewer.layout_input.input "the lines the layout was given"
 * @calls docair-viewer.layout_input.words "the words of the parts and lines"
 ***/
pub fn draw(
    graph: &Graph,
    pic: &LayerPicture,
    layout: &Layout,
    marks: Option<&FlowMarks>,
) -> String {
    let input = layout_input::input(graph, pic, &Monospace::default());
    let (part_words, line_words) = layout_input::words(graph, pic);
    let n = pic.parts.len().min(layout.placement.parts.len());
    let mut is_box = vec![false; n];
    for p in input.parts.iter().take(n) {
        if let Some(q) = p.parent.filter(|&q| q < n) {
            is_box[q] = true;
        }
    }
    let part_mark = |i: usize| marks.and_then(|m| m.parts.get(&pic.parts[i].id).copied());
    let names: Vec<String> = (0..n)
        .map(|i| {
            format!(
                "{}{}",
                prefix(part_mark(i)),
                part_words[i].replace('\n', " ")
            )
        })
        .collect();
    // The lines the layout was given, each with the index of the picture's line it draws.
    let lines: Vec<(usize, usize)> = input
        .lines
        .iter()
        .enumerate()
        .filter(|(j, _)| *j < layout.routes.lines.len())
        .filter_map(|(j, l)| Some((j, l.id.parse::<usize>().ok()?)))
        .filter(|(_, k)| *k < pic.lines.len())
        .collect();
    let words: HashMap<usize, Vec<String>> = lines
        .iter()
        .filter(|&&(j, _)| layout.words.get(j).is_some_and(Option::is_some))
        .map(|&(j, k)| (j, wrap(&line_words[k], WORDS_COLUMNS)))
        .filter(|(_, w)| w.iter().any(|l| !l.is_empty()))
        .collect();

    let (mut xs, mut ys) = (AxisValues::default(), AxisValues::default());
    let mut things: Vec<Obstacle> = Vec::new();
    for i in 0..n {
        let r = &layout.placement.parts[i];
        let (l, rr, t, b) = edges(r);
        let len = columns(&names[i]) as i64;
        if is_box[i] {
            // ┌─ name ─┐; each side of its border apart from what is inside and outside it.
            xs.need(l, rr, len + 5);
            for (p, q) in [
                ((l, t), (rr, t)),
                ((l, b), (rr, b)),
                ((l, t), (l, b)),
                ((rr, t), (rr, b)),
            ] {
                things.push(Obstacle::stretch(p, q, ObstacleKind::Border));
            }
            // The name's own band, so words keep off it: as wide in px as the SVG writes it,
            // as many cells as it needs from the box's left.
            let name_r = (l + 12.0 + len as f64 * 6.9).min(rr);
            xs.need(l, name_r, len + 3);
            things.push(Obstacle {
                l,
                r: name_r,
                t: t - 1.0,
                b: t + 1.0,
                what: ObstacleKind::Name,
            });
        } else {
            // │ name │
            xs.need(l, rr, len + 3);
            things.push(Obstacle::of(r, ObstacleKind::Part));
        }
        ys.need(t, b, 2);
    }
    for &(j, _) in &lines {
        let route = &layout.routes.lines[j];
        for w in route.windows(2) {
            things.push(Obstacle::stretch(w[0], w[1], ObstacleKind::Stretch(j)));
        }
        for &(x, y) in route {
            xs.add(x);
            ys.add(y);
        }
        // Where a line touches its part, it stays inside that side, off the corners.
        let ends = [
            (route.first(), input.lines[j].from),
            (route.last(), input.lines[j].to),
        ];
        for (p, i) in ends {
            let (Some(&(x, y)), Some(r)) = (p, layout.placement.parts.get(i)) else {
                continue;
            };
            let (l, rr, t, b) = edges(r);
            if (y - t).abs() <= 1.0 || (y - b).abs() <= 1.0 {
                xs.need(l, x, 1);
                xs.need(x, rr, 1);
            } else if (x - l).abs() <= 1.0 || (x - rr).abs() <= 1.0 {
                ys.need(t, y, 1);
                ys.need(y, b, 1);
            }
        }
        if let (Some(Some(at)), true) = (layout.words.get(j), words.contains_key(&j)) {
            things.push(Obstacle::of(at, ObstacleKind::Words(j)));
        }
    }
    let mut words = words;
    let mut notes: Vec<String> = Vec::new();
    for j in words_off(&mut things) {
        if let Some(w) = words.get_mut(&j) {
            notes.push(format!("[{}] {}", notes.len() + 1, w.join(" ")));
            *w = vec![format!("[{}]", notes.len())];
        }
    }
    // The words where they went, and the cells they need.
    let mut spots: HashMap<usize, (f64, f64)> = HashMap::new();
    for t in &things {
        if let ObstacleKind::Words(j) = t.what
            && let Some(w) = words.get(&j)
        {
            let wide = w.iter().map(|s| columns(s)).max().unwrap_or(0) as i64;
            xs.need(t.l, t.r, (wide - 1).max(0));
            ys.need(t.t, t.b, (w.len() as i64 - 1).max(0));
            spots.insert(j, (t.l, t.t));
        }
    }
    keep_apart(&things, &mut xs, &mut ys);
    let (cx, cy) = (xs.solve(), ys.solve());
    let at = |p: (f64, f64)| (cx.at(p.0), cy.at(p.1));
    let sides: Vec<CellSides> = (0..n)
        .map(|i| {
            let (l, r, t, b) = edges(&layout.placement.parts[i]);
            CellSides {
                l: cx.at(l),
                r: cx.at(r),
                t: cy.at(t),
                b: cy.at(b),
            }
        })
        .collect();
    let width = cx.cell.last().map_or(1, |c| *c + 1) as usize;
    let height = cy.cell.last().map_or(1, |c| *c + 1) as usize;
    let mut grid = Grid::new(width, height);

    for (i, s) in sides.iter().enumerate() {
        grid.outline(*s, part_pen(part_mark(i)));
    }
    let mut arrows: Vec<((i64, i64), char)> = Vec::new();
    for &(j, k) in &lines {
        let mut cells: Vec<(i64, i64)> = Vec::new();
        for c in layout.routes.lines[j].iter().map(|&p| at(p)) {
            if cells.last() != Some(&c) {
                cells.push(c);
            }
        }
        if cells.len() < 2 {
            continue;
        }
        let l = &pic.lines[k];
        let pen = match marks.and_then(|m| {
            l.members
                .iter()
                .find_map(|&e| m.lines.get(&crate::svg::edge_key(graph, e)).copied())
        }) {
            Some(LineMark::Added) => Pen::Double,
            Some(LineMark::Removed) => Pen::Dashed,
            _ => Pen::Light,
        };
        // The arrow goes in the cell in front of the target's side; the line stops there.
        let target = input.lines[j].to;
        let last = cells.len() - 1;
        let dir = towards(cells[last - 1], cells[last]);
        if target < n && sides[target].on_border(cells[last]) {
            cells[last] = (cells[last].0 - dir.0, cells[last].1 - dir.1);
        }
        for w in cells.windows(2) {
            grid.run(w[0], w[1], pen);
        }
        arrows.push((cells[last], arrow(dir)));
    }
    for (c, ch) in arrows {
        grid.write(c.0, c.1, &ch.to_string());
    }
    for i in 0..n {
        let s = sides[i];
        if is_box[i] {
            // A box's name in its top border, moved right off the lines that cross it there.
            let name = format!(" {} ", names[i]);
            let len = columns(&name) as i64;
            let crossed = |grid: &mut Grid, x: i64| {
                (x..x + len).any(|c| grid.cell(c, s.t).is_some_and(|c| c.bits & (UP | DOWN) != 0))
            };
            let at = (s.l + 2..=(s.r - len - 1).max(s.l + 2))
                .find(|&x| !crossed(&mut grid, x))
                .unwrap_or(s.l + 2);
            grid.write(at, s.t, &name);
        } else {
            grid.write(s.l + 2, (s.t + s.b) / 2, &names[i]);
        }
    }
    for (&j, w) in &words {
        if let Some(&spot) = spots.get(&j) {
            let (x, y) = at(spot);
            for (row, text) in w.iter().enumerate() {
                grid.write(x, y + row as i64, text);
            }
        }
    }
    let mut out = grid.render();
    if !notes.is_empty() {
        out.push('\n');
        for n in notes {
            out.push_str(&n);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_share_a_cell_unless_something_needs_them_apart() {
        let mut a = AxisValues::default();
        a.need(0.0, 100.0, 10);
        a.add(50.0);
        a.add(50.4);
        a.need(150.0, 200.0, 1);
        let c = a.solve();
        assert_eq!(c.at(0.0), 0);
        assert_eq!(c.at(50.0), c.at(50.4));
        assert_eq!(c.at(50.0), 0, "nothing keeps 50 from 0");
        assert_eq!(c.at(100.0), 10);
        assert_eq!(c.at(150.0), 10, "nothing keeps 150 from 100");
        assert_eq!(c.at(200.0), 11);
    }

    #[test]
    fn things_that_overlap_along_one_axis_stay_apart_along_the_other() {
        let part = Obstacle {
            l: 100.0,
            r: 160.0,
            t: 0.0,
            b: 30.0,
            what: ObstacleKind::Part,
        };
        let track = Obstacle::stretch((92.0, -20.0), (92.0, 50.0), ObstacleKind::Stretch(0));
        let far = Obstacle::stretch((10.0, 200.0), (95.0, 200.0), ObstacleKind::Stretch(1));
        let (mut xs, mut ys) = (AxisValues::default(), AxisValues::default());
        keep_apart(&[part, track, far], &mut xs, &mut ys);
        assert_eq!(
            xs.apart,
            [(92.0, 100.0, 2)],
            "a track beside a part leaves room for an arrow"
        );
        assert_eq!(
            ys.apart,
            [(50.0, 200.0, 1)],
            "the far stretch is below the track"
        );
    }

    #[test]
    fn cells_meet_as_box_drawing_does() {
        assert_eq!(glyph(UP | DOWN | RIGHT, Pen::Light), '├');
        assert_eq!(glyph(LEFT | RIGHT, Pen::Double), '═');
        assert_eq!(glyph(DOWN | RIGHT, Pen::Double), '╔');
        assert_eq!(glyph(UP | DOWN | LEFT | RIGHT, Pen::Light), '┼');
        assert_eq!(glyph(LEFT | RIGHT, Pen::Dashed), '-');
        assert_eq!(glyph(UP | DOWN, Pen::Dashed), ':');
        assert_eq!(glyph(DOWN | RIGHT, Pen::Dashed), '+');
    }

    #[test]
    fn a_grid_draws_a_part_and_a_line_into_it() {
        let mut g = Grid::new(12, 3);
        let part = CellSides {
            l: 5,
            r: 11,
            t: 0,
            b: 2,
        };
        g.outline(part, Pen::Light);
        g.run((0, 1), (3, 1), Pen::Light);
        g.write(4, 1, "►");
        g.write(7, 1, "ab");
        assert_eq!(g.render(), "     ┌─────┐\n────►│ ab  │\n     └─────┘\n");
    }
}

//! docair-layout: lays out a picture of nested boxes.
//!
//! The input is parts, each in a box or at the top, and lines between them
//! (`Input`). Each box is laid out on its own, inside out: its parts in
//! layers (`layered`), then shrunk to the box's size budget and placed in
//! the box around it (`topdown`). The layout depends on nothing but its
//! input and options: no clock, no chance, so the same input gives the same
//! picture every time.

pub mod draw;
pub mod text;

mod hand_move;
mod incremental;
mod layered;
mod router;
mod topdown;
mod word_spots;

pub use hand_move::{MoveOutcome, PartMove, ReroutedLine, ShiftedPart, move_part};
pub use incremental::{try_update_layout, update_layout};
pub use layered::Direction;
pub use router::Routes;
pub use text::Monospace;
pub use word_spots::words_size;

/// A part of the picture. A part that other parts are in is a box: it gets the size of what it holds.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Part {
    pub id: String,
    /// The box it is in, by index into `Input::parts`; none at the top of the picture.
    pub parent: Option<usize>,
    /// A part's size; a box's is worked out.
    pub width: f64,
    pub height: f64,
    /// A box is at least this wide, so its name fits.
    pub min_width: f64,
    /// A box: how wide its name is in the band at its top, which lines rather not cross.
    #[cfg_attr(feature = "serde", serde(default))]
    pub name_width: f64,
}

/// A line between two parts.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Line {
    pub id: String,
    /// Its ends, by index into `Input::parts`.
    pub from: usize,
    pub to: usize,
    /// Laid out against its arrow: an answer, along the call it answers.
    pub flip: bool,
    /// How much it should point ahead: higher, more (a client's own requests most).
    pub ahead: u32,
    /// The size of its words, kept beside its arrow; none for a line without words.
    pub words: Option<(f64, f64)>,
}

/// What to lay out.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Input {
    pub parts: Vec<Part>,
    pub lines: Vec<Line>,
}

/// The room inside a box around what it holds: its name at the top.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Padding {
    pub top: f64,
    pub left: f64,
    pub right: f64,
    pub bottom: f64,
}

/// How to lay out.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Options {
    /// The directions to try; the one whose picture fits the canvas best wins (another one has to
    /// fit 1.1 times larger to win over the first).
    pub directions: Vec<Direction>,
    /// The canvas the picture should fit, in px.
    pub canvas: (f64, f64),
    /// A box shrinks what it holds to at most this size, in px …
    pub budget: f64,
    /// … but never below this scale.
    pub floor: f64,
    pub padding: Padding,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            directions: vec![Direction::Down, Direction::Right],
            canvas: (1440.0, 960.0),
            budget: 800.0,
            floor: 0.7,
            padding: Padding {
                top: 28.0,
                left: 10.0,
                right: 10.0,
                bottom: 10.0,
            },
        }
    }
}

/// A part's place: its centre and its size, as drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Where every part goes.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Placement {
    /// Each part's place, in the order of `Input::parts`.
    pub parts: Vec<Rect>,
    /// The scale each part is drawn at: below 1 inside a box that shrank what it holds.
    pub scale: Vec<f64>,
    pub width: f64,
    pub height: f64,
    pub direction: Direction,
    /// The zoom at which the whole picture fits the canvas.
    pub zoom: f64,
    /// The way that won: `as it is`, or how the boxes learned where their lines go.
    pub way: String,
    /// What its lines cost: drawn straight, their length and 400 px for every crossing.
    pub price: f64,
}

/// A picture laid out: where its parts go, and its lines.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Layout {
    pub placement: Placement,
    pub routes: Routes,
    /// Where each line's words go, in the order of `Input::lines`; none for a line without words.
    pub words: Vec<Option<Rect>>,
}

/// The layout code itself, so a kept layout made by other code is not taken for a new one.
const SOURCES: &[&str] = &[
    include_str!("lib.rs"),
    include_str!("layered.rs"),
    include_str!("topdown.rs"),
    include_str!("router.rs"),
    include_str!("word_spots.rs"),
    include_str!("text.rs"),
    include_str!("draw.rs"),
    include_str!("hand_move.rs"),
    include_str!("incremental.rs"),
];

fn fnv(h: &mut u64, bytes: &[u8]) {
    for &b in bytes {
        *h ^= b as u64;
        *h = h.wrapping_mul(0x0100_0000_01b3);
    }
}

/// A fingerprint of the layout code (FNV-1a of its sources).
pub fn code_version() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for s in SOURCES {
        fnv(&mut h, s.as_bytes());
    }
    h
}

/***
 * The key a layout is kept under: what the layout reads, and the layout code's version.
 *
 * Only what changes the layout is in it: each part's box and size, each
 * line's ends, hints and words' size, the options. Ids and names are not, so
 * a renamed part or a test that changes its result keeps the layout
 * (docs/REGENERATE.md §2). The same key, the same layout.
 ***/
pub fn key(input: &Input, options: &Options) -> String {
    let mut h = code_version();
    let num = |h: &mut u64, v: f64| fnv(h, &v.to_bits().to_le_bytes());
    for p in &input.parts {
        fnv(
            &mut h,
            &(p.parent.map_or(u64::MAX, |q| q as u64)).to_le_bytes(),
        );
        num(&mut h, p.width);
        num(&mut h, p.height);
        num(&mut h, p.min_width);
        num(&mut h, p.name_width);
    }
    fnv(&mut h, b"|");
    for l in &input.lines {
        fnv(&mut h, &(l.from as u64).to_le_bytes());
        fnv(&mut h, &(l.to as u64).to_le_bytes());
        fnv(&mut h, &[l.flip as u8]);
        fnv(&mut h, &l.ahead.to_le_bytes());
        let (w, hh) = l.words.unwrap_or((-1.0, -1.0));
        num(&mut h, w);
        num(&mut h, hh);
    }
    fnv(&mut h, b"|");
    for d in &options.directions {
        fnv(&mut h, &[*d as u8]);
    }
    for v in [
        options.canvas.0,
        options.canvas.1,
        options.budget,
        options.floor,
        options.padding.top,
        options.padding.left,
        options.padding.right,
        options.padding.bottom,
    ] {
        num(&mut h, v);
    }
    format!("{h:016x}")
}

/***
 * Draw the lines of a placed picture: each line the cheapest way at right angles, given all the others.
 ***/
pub fn route(input: &Input, placed: &Placement) -> Routes {
    router::grid_lines(input, placed, router::PRICES)
}

/// How often routed lines cross: a piece along a row through a piece along a column of another line.
pub fn crossings(routes: &Routes) -> usize {
    let mut flat: Vec<(usize, f64, f64, f64)> = Vec::new();
    let mut tall: Vec<(usize, f64, f64, f64)> = Vec::new();
    for (k, pts) in routes.lines.iter().enumerate() {
        for s in pts.windows(2) {
            let (a, b) = (s[0], s[1]);
            if (a.1 - b.1).abs() < 0.01 {
                flat.push((k, a.1, a.0.min(b.0), a.0.max(b.0)));
            } else if (a.0 - b.0).abs() < 0.01 {
                tall.push((k, a.0, a.1.min(b.1), a.1.max(b.1)));
            }
        }
    }
    let mut n = 0;
    for &(k, y, x1, x2) in &flat {
        for &(m, x, y1, y2) in &tall {
            if k != m && x > x1 + 0.01 && x < x2 - 0.01 && y > y1 + 0.01 && y < y2 - 0.01 {
                n += 1;
            }
        }
    }
    n
}

/***
 * Lay out a picture: place its parts and route its lines.
 *
 * As `place`, but a small picture (up to 60 lines) is routed in each of its
 * placements, and the placement whose routed lines cost least wins; a big
 * one is judged by its lines drawn straight, and only the winner is routed.
 *
 * @calls docair-layout.route "routes the lines"
 ***/
pub fn lay_out(input: &Input, options: &Options) -> Layout {
    if input.lines.len() > router::PRICES.exact_up_to {
        let placement = place(input, options);
        let routes = route(input, &placement);
        let words = word_spots::place_words(input, &placement, &routes);
        return Layout {
            placement,
            routes,
            words,
        };
    }
    let mut first: Option<Placement> = None;
    for &d in &options.directions {
        let Some(r) = topdown::top_down(input, d, options, None, None) else {
            continue;
        };
        if first.as_ref().is_none_or(|f| r.zoom > f.zoom * 1.1) {
            first = Some(r);
        }
    }
    let first = first
        .or_else(|| topdown::top_down(input, Direction::Down, options, None, None))
        .expect("a placement as it is always exists");
    // The placement as it is and in each way, each routed; side by side, the same picture as one after another.
    let mut candidates: Vec<Placement> = vec![first.clone()];
    candidates.extend(topdown::WAYS.iter().filter_map(|way| {
        topdown::top_down(input, first.direction, options, Some(way), Some(&first))
    }));
    {
        // The other direction too, as long as it fits the canvas nearly as well: its lines may cost less.
        for &d in &options.directions {
            if d == first.direction {
                continue;
            }
            let Some(other) = topdown::top_down(input, d, options, None, None) else {
                continue;
            };
            if other.zoom < first.zoom * 0.8 {
                continue;
            }
            candidates.push(other.clone());
            candidates.extend(
                topdown::WAYS.iter().filter_map(|way| {
                    topdown::top_down(input, d, options, Some(way), Some(&other))
                }),
            );
        }
    }
    let routed: Vec<Routes> = std::thread::scope(|s| {
        let jobs: Vec<_> = candidates
            .iter()
            .map(|c| s.spawn(move || route(input, c)))
            .collect();
        jobs.into_iter()
            .map(|j| j.join().expect("routing does not panic"))
            .collect()
    });
    let mut best = 0;
    for (k, r) in routed.iter().enumerate() {
        if r.price < routed[best].price {
            best = k;
        }
    }
    let mut placement = candidates.swap_remove(best);
    let routes = routed.into_iter().nth(best).expect("the best was routed");
    placement.price = routes.price;
    let words = word_spots::place_words(input, &placement, &routes);
    Layout {
        placement,
        routes,
        words,
    }
}

/***
 * Place every part of a picture: each box laid out on its own, inside out.
 *
 * The picture is placed as it is, in the direction that fits the canvas
 * best; then again in each of four ways, which tell every box where its
 * lines go outside it in that first placement. The placement whose lines,
 * drawn straight, cost least wins.
 ***/
pub fn place(input: &Input, options: &Options) -> Placement {
    let mut first: Option<Placement> = None;
    for &d in &options.directions {
        let Some(r) = topdown::top_down(input, d, options, None, None) else {
            continue;
        };
        if first.as_ref().is_none_or(|f| r.zoom > f.zoom * 1.1) {
            first = Some(r);
        }
    }
    let first = first
        .or_else(|| topdown::top_down(input, Direction::Down, options, None, None))
        .expect("a placement as it is always exists");
    let mut best: Option<Placement> = None;
    for way in &topdown::WAYS {
        if let Some(r) = topdown::top_down(input, first.direction, options, Some(way), Some(&first))
            && r.price < best.as_ref().map_or(first.price, |b| b.price)
        {
            best = Some(r);
        }
    }
    best.unwrap_or(first)
}

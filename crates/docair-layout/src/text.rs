//! Text in one monospace font: a word's width is its number of characters times one advance.
//!
//! The pictures set every word in one monospace font, embedded in the page,
//! so the width a word takes is known without shaping and without a font
//! library (docs/SLICES.md §5). A wide East Asian character takes two
//! columns; a combining mark takes none.

/// A monospace font, by the advance of one character, in em.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Monospace {
    /// The advance of one column, in em (0.6 for most monospace fonts).
    pub advance: f64,
}

impl Default for Monospace {
    fn default() -> Self {
        Monospace { advance: 0.6 }
    }
}

impl Monospace {
    /// The width of `text` set at `size` px.
    pub fn width(&self, text: &str, size: f64) -> f64 {
        columns(text) as f64 * self.advance * size
    }
}

/// How many columns `text` takes: two for a wide East Asian character, none for a combining mark.
pub fn columns(text: &str) -> usize {
    text.chars().map(char_columns).sum()
}

fn char_columns(c: char) -> usize {
    let u = c as u32;
    if is_combining(u) || u < 0x20 {
        0
    } else if is_wide(u) {
        2
    } else {
        1
    }
}

fn is_combining(u: u32) -> bool {
    matches!(u, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F | 0x200B..=0x200F)
}

fn is_wide(u: u32) -> bool {
    matches!(
        u,
        0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD
    )
}

/// A text in lines of about `width` columns, broken between words, so a long text makes a picture taller, not wider.
/// How wide a line's words wrap in a picture, in columns.
pub const WORDS_COLUMNS: usize = 28;

/// The words wrapped at `width` columns and cut to `lines` lines: the last one ends in `…` when
/// words were left out (docair-702.34). Wrapped again, the result keeps the same lines.
pub fn cut(text: &str, width: usize, lines: usize) -> String {
    let wrapped = wrap(text, width);
    if wrapped.len() <= lines || lines == 0 {
        return text.to_string();
    }
    let mut kept: Vec<String> = wrapped[..lines].to_vec();
    if let Some(last) = kept.last_mut() {
        while !last.is_empty() && columns(last) + 1 > width {
            last.pop();
        }
        *last = format!("{}…", last.trim_end());
    }
    kept.join(" ")
}

pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(last) if columns(last) + 1 + columns(word) <= width => {
                last.push(' ');
                last.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cut to two lines, words keep two lines and end in …; wrapped again, they stay two lines.
    #[test]
    fn words_cut_to_two_lines_end_in_an_ellipsis() {
        let long = "config: Config · graph : Graph +1 · via docair-ai › docair-viewer › docair-cli";
        let cut2 = cut(long, 28, 2);
        let lines = wrap(&cut2, 28);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            cut2.ends_with('…') && lines.iter().all(|l| columns(l) <= 28),
            "{cut2}"
        );
        assert_eq!(cut("calls", 28, 2), "calls", "short words stay as they are");
    }

    #[test]
    fn a_word_is_its_columns_times_the_advance() {
        let m = Monospace::default();
        assert_eq!(m.width("docair", 10.0), 36.0);
        assert_eq!(columns("日本"), 4);
        assert_eq!(columns("e\u{301}"), 1);
    }

    #[test]
    fn a_long_text_breaks_between_words() {
        assert_eq!(
            wrap("POST /api/ask · POST /api/word +3", 16),
            ["POST /api/ask ·", "POST /api/word", "+3"]
        );
        assert_eq!(wrap("short", 28), ["short"]);
    }
}

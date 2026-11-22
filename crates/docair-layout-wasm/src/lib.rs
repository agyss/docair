//! docair-layout's move of a part by hand, for the page (docs/SLICES.md §7).
//!
//! The page loads the module, writes a request into its memory as JSON, calls `move_part`, and
//! reads the answer as JSON from where the call says. Built for wasm32 it exports `memory`,
//! `alloc`, `free` and `move_part`; on other targets it is a library that holds the built module
//! (`MODULE`) and answers requests the same way (`answer`), so the answer can be tested here.

use std::collections::HashMap;

use docair_layout::{Input, Layout, PartMove, move_part};
use serde::Deserialize;

/// The module built for the page, by this crate's build script from the code as it is (docair-702.42).
#[cfg(not(target_arch = "wasm32"))]
pub const MODULE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/layout.wasm"));

/// A part put down elsewhere, as the page asks.
#[derive(Deserialize)]
struct MoveRequest {
    input: Input,
    layout: Layout,
    /// The moved part's id.
    part: String,
    dx: f64,
    dy: f64,
    /// How wide each line is drawn (its `stroke-width`), by line id; 1.4 when not said.
    #[serde(default)]
    widths: HashMap<String, f64>,
}

/***
 * Answer one request: a part moved by hand, as JSON in, the parts and boxes now elsewhere and the lines routed again, ready to draw, as JSON out (`{ "error": … }` for a request it cannot read).
 *
 * @calls docair-layout.hand_move.move_part "moves the part and routes its lines again"
 ***/
pub fn answer(request: &str) -> String {
    let req: MoveRequest = match serde_json::from_str(request) {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": e.to_string() }).to_string(),
    };
    let Some(part) = req.input.parts.iter().position(|p| p.id == req.part) else {
        return serde_json::json!({ "error": format!("no part {}", req.part) }).to_string();
    };
    let width = |j: usize| {
        req.input
            .lines
            .get(j)
            .and_then(|l| req.widths.get(&l.id).copied())
            .unwrap_or(1.4)
    };
    let moved = move_part(
        &req.input,
        &req.layout,
        PartMove {
            part,
            dx: req.dx,
            dy: req.dy,
        },
        &width,
    );
    serde_json::to_string(&moved)
        .unwrap_or_else(|e| serde_json::json!({ "error": e.to_string() }).to_string())
}

/// Room for `len` bytes in the module's memory, for the page to write a request into.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Give back what `alloc` or `move_part` handed out.
///
/// # Safety
/// `ptr` and `len` must come from `alloc(len)` or from `move_part`'s answer.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free(ptr: *mut u8, len: usize) {
    drop(unsafe { Vec::from_raw_parts(ptr, 0, len) });
}

/// Move a part: the request is `len` bytes of JSON at `ptr`; the answer's place and length come
/// back as one number, the place in the high 32 bits.
///
/// # Safety
/// `ptr` must come from `alloc(len)` and hold `len` bytes the page wrote.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn move_part_json(ptr: *mut u8, len: usize) -> u64 {
    let bytes = unsafe { Vec::from_raw_parts(ptr, len, len) };
    let out = answer(std::str::from_utf8(&bytes).unwrap_or(""))
        .into_bytes()
        .into_boxed_slice();
    let (at, n) = (out.as_ptr() as u64, out.len() as u64);
    std::mem::forget(out);
    (at << 32) | n
}

#[cfg(test)]
mod tests {
    use super::*;
    use docair_layout::{Line, Options, Part, lay_out};

    /// The module is built for the page from this code (docair-702.42).
    #[test]
    fn the_module_is_webassembly() {
        assert!(MODULE.starts_with(b"\0asm"), "a WebAssembly module");
    }

    #[test]
    fn a_request_is_answered_with_lines_ready_to_draw() {
        let part = |id: &str| Part {
            id: id.to_string(),
            parent: None,
            width: 80.0,
            height: 30.0,
            min_width: 0.0,
            name_width: 0.0,
        };
        let input = Input {
            parts: vec![part("a"), part("b")],
            lines: vec![Line {
                id: "0".to_string(),
                from: 0,
                to: 1,
                flip: false,
                ahead: 0,
                words: None,
            }],
        };
        let layout = lay_out(&input, &Options::default());
        let request = serde_json::json!({
            "input": input, "layout": layout, "part": "b", "dx": 200.0, "dy": 0.0, "widths": { "0": 2.0 }
        });
        let out: serde_json::Value = serde_json::from_str(&answer(&request.to_string())).unwrap();
        assert_eq!(out["lines"][0]["id"], "0");
        assert!(
            out["lines"][0]["drawn"]["d"]
                .as_str()
                .unwrap()
                .starts_with('M')
        );
        assert!(
            out["lines"][0]["drawn"]["arrow"]
                .as_str()
                .unwrap()
                .ends_with('Z')
        );
        assert_eq!(out["parts"][0]["id"], "b");
        let bad: serde_json::Value = serde_json::from_str(&answer("{")).unwrap();
        assert!(bad["error"].is_string());
        let none: serde_json::Value = serde_json::from_str(&answer(
            &serde_json::json!({"input": input, "layout": layout, "part": "z", "dx": 0, "dy": 0})
                .to_string(),
        ))
        .unwrap();
        assert_eq!(none["error"], "no part z");
    }
}

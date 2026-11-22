//! Layouts kept on disk, each under its key (docair_layout::key): a picture that did not change is
//! not laid out again (docs/REGENERATE.md §2, docs/SLICES.md §5).

use std::path::Path;

use anyhow::Result;
use docair_core::layer_view::LayerPicture;
use docair_core::model::Graph;
use docair_layout::{Layout, Monospace};

/***
 * Lay out a picture, or take its layout from `dir` when one is kept there under the same key; a new layout is kept.
 *
 * @calls docair-viewer.layout_input.input "turns the picture into layout input"
 * @calls docair-layout.key "works out the key"
 * @calls docair-layout.lay_out "lays out a picture not kept yet"
 ***/
pub fn lay_out_kept(graph: &Graph, pic: &LayerPicture, dir: &Path) -> Result<(Layout, bool)> {
    lay_out_kept_with(graph, pic, &std::collections::HashMap::new(), dir)
}

/// As `lay_out_kept`, with how much some of the picture's lines point ahead given: a walk's lines.
pub fn lay_out_kept_with(
    graph: &Graph,
    pic: &LayerPicture,
    ahead: &std::collections::HashMap<usize, u32>,
    dir: &Path,
) -> Result<(Layout, bool)> {
    let font = Monospace::default();
    let input = crate::layout_input::input_with(graph, pic, &font, ahead);
    let options = crate::layout_input::options_for(pic);
    let key = docair_layout::key(&input, &options);
    kept_or_made(&input, &options, &key, dir)
}

/***
 * A layout input's layout kept in `dir` under its `key`, or laid out and kept there; whether it was kept.
 *
 * @calls docair-layout.lay_out "lays out a picture not kept yet"
 ***/
pub fn kept_or_made(
    input: &docair_layout::Input,
    options: &docair_layout::Options,
    key: &str,
    dir: &Path,
) -> Result<(Layout, bool)> {
    let path = dir.join(format!("{key}.json"));
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(laid) = serde_json::from_str::<Layout>(&text)
    {
        return Ok((laid, true));
    }
    let laid = docair_layout::lay_out(input, options);
    std::fs::create_dir_all(dir)?;
    std::fs::write(&path, serde_json::to_string(&laid)?)?;
    Ok((laid, false))
}

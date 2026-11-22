//! Stage 7: the graph file (SPEC §9).

use anyhow::{Context, Result};
use std::path::Path;

use crate::model::Graph;

pub fn to_json(graph: &Graph) -> String {
    serde_json::to_string_pretty(graph).expect("the graph is always serializable")
}

/***
 * Write `docair.graph.json`, the integration API that the viewer and other tools read.
 *
 * @flow   scan 12 "write the outputs"
 * @in     graph: Graph from docair-core.check.check
 * @writes graph-json "writes the graph as JSON"
 ***/
pub fn write_json(graph: &Graph, path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, to_json(graph)).with_context(|| format!("writing {}", path.display()))
}

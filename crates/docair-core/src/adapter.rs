//! Stage 3: adapters read the code and report facts (docs/ADAPTERS.md §5).

use crate::config::Config;
use crate::facts::Facts;
use crate::model::{RawBlock, SourceFile};

/// What adapters may read besides one file: the whole project, with the doc blocks found in it.
pub struct Project<'a> {
    /// The system node: where things without a parent go.
    pub system: &'a str,
    /// `docair.toml`, e.g. `[[import.api]]` for the API descriptions.
    pub config: &'a Config,
    pub files: &'a [SourceFile],
    /// The doc blocks of each file, in the order of `files`. Adapters read them
    /// to interpret code (a block may name what a Terraform resource creates);
    /// they never change them.
    pub blocks: &'a [Vec<RawBlock>],
}

/***
 * The extension point for one language: read the code and report what is in it, as facts.
 *
 * An adapter never touches the graph. It says which symbols exist, what
 * they serve and what they use; the core merges that with the doc blocks.
 ***/
pub trait Adapter: Sync {
    fn name(&self) -> &'static str;
    /// Whether this adapter reads `file`.
    fn handles(&self, file: &SourceFile) -> bool;
    /// The facts of one file it handles; `blocks` are that file's doc blocks.
    fn file(&self, _file: &SourceFile, _blocks: &[RawBlock]) -> Facts {
        Facts::default()
    }
    /// Facts that need the whole project, such as references across files. Runs once per scan, after `file`.
    fn project(&self, _project: &Project) -> Facts {
        Facts::default()
    }
}

/***
 * Let every adapter read the code: each file it handles, then the whole project.
 *
 * The result keeps the adapters' order; it decides who says what a symbol is
 * when two adapters know the same line (the first one wins).
 *
 * @flow  scan 6 "adapters read the code"
 * @in    blocks: RawBlock[] from docair-core.extract.extract
 * @calls docair-lang-rust.rust-adapter.project "reads every Rust file with tree-sitter"
 * @calls docair-lang-infra.infra-adapter.project "reads Terraform, charts and manifests as a whole"
 * @calls docair-lang-apis.api-adapter.project "reads OpenAPI and AsyncAPI descriptions"
 * @calls docair-lang-generic.generic-adapter.file "guesses what each block documents from its code line"
 * @out   facts: Facts[]
 ***/
pub fn collect(project: &Project, adapters: &[&dyn Adapter]) -> Vec<Facts> {
    adapters
        .iter()
        .map(|a| {
            let mut facts = Facts::default();
            for (file, blocks) in project.files.iter().zip(project.blocks) {
                if a.handles(file) {
                    facts.extend(a.file(file, blocks));
                }
            }
            facts.extend(a.project(project));
            facts
        })
        .collect()
}

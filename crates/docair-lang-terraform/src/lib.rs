//! The Terraform adapter (docs/ADAPTERS.md §5.3): Terraform: every `resource` is a node, what it references and `depends_on` are lines, and what it brings into existence (a cluster, a namespace, a release) is `@creates`.
//!
//! It reads `.tf` files. The references cross tools (a `helm_release` installs a chart,
//! a pod runs what a Dockerfile builds), so the reading is shared: `docair-infra`
//! reads all infrastructure code once, and this adapter reports its own files.

use docair_core::adapter::{Adapter, AdapterKind, Project};
use docair_core::facts::Facts;
use docair_core::lang::{Family, LanguageSpec};
use docair_core::model::SourceFile;
use docair_infra::InfraTool;

/***
 * The Terraform adapter: Terraform: every `resource` is a node, what it references and `depends_on` are lines, and what it brings into existence (a cluster, a namespace, a release) is `@creates`.
 ***/
pub struct TerraformAdapter;

/// This adapter, for the registry (docs/ADAPTERS.md §5.3): every `docair-lang-*` crate exports one.
pub static ADAPTER: &dyn Adapter = &TerraformAdapter;

impl Adapter for TerraformAdapter {
    fn name(&self) -> &'static str {
        "terraform"
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Infra
    }

    fn languages(&self) -> &'static [LanguageSpec] {
        // Its templates and variable files too: `templatefile("values.yaml.tftpl", …)`, `dev.tfvars`
        // (docair-4wk.46).
        &[
            LanguageSpec {
                name: "terraform",
                extensions: &["tf"],
                family: Family::Hash,
            },
            LanguageSpec {
                name: "tftpl",
                extensions: &["tftpl"],
                family: Family::Hash,
            },
            LanguageSpec {
                name: "tfvars",
                extensions: &["tfvars"],
                family: Family::Hash,
            },
        ]
    }

    fn handles(&self, file: &SourceFile) -> bool {
        matches!(file.lang.name, "terraform" | "tftpl" | "tfvars")
    }

    /***
     * The Terraform files' nodes and lines, from the shared reading of all infrastructure code.
     *
     * @calls docair-infra.facts "reads all infrastructure once, reports this tool's files"
     ***/
    fn project(&self, project: &Project) -> Facts {
        docair_infra::facts(project, InfraTool::Terraform, "terraform")
    }
}

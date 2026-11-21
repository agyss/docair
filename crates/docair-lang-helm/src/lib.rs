//! The Helm adapter (docs/ADAPTERS.md §5.3): Helm charts: each template's Kubernetes objects with `.Values` filled in from the chart's values; a chart that no Terraform installs is a release of its own.
//!
//! It reads YAML in a chart: beside a `Chart.yaml`, or in its `templates/`. The references cross tools (a `helm_release` installs a chart,
//! a pod runs what a Dockerfile builds), so the reading is shared: `docair-infra`
//! reads all infrastructure code once, and this adapter reports its own files.

use docair_core::adapter::{Adapter, AdapterKind, Project};
use docair_core::facts::Facts;
use docair_core::lang::{Family, LanguageSpec};
use docair_core::model::SourceFile;
use docair_infra::InfraTool;

/***
 * The Helm adapter: Helm charts: each template's Kubernetes objects with `.Values` filled in from the chart's values; a chart that no Terraform installs is a release of its own.
 ***/
pub struct HelmAdapter;

/// This adapter, for the registry (docs/ADAPTERS.md §5.3): every `docair-lang-*` crate exports one.
pub static ADAPTER: &dyn Adapter = &HelmAdapter;

impl Adapter for HelmAdapter {
    fn name(&self) -> &'static str {
        "helm"
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Infra
    }

    /// Helm's own templates (`_helpers.tpl`), with doc blocks as `{{/*** … ***/}}`.
    fn languages(&self) -> &'static [LanguageSpec] {
        &[LanguageSpec {
            name: "helm",
            extensions: &["tpl"],
            family: Family::CStyle,
        }]
    }

    fn handles(&self, file: &SourceFile) -> bool {
        file.lang.name == "yaml"
    }

    /***
     * The Helm files' nodes and lines, from the shared reading of all infrastructure code.
     *
     * @calls docair-infra.facts "reads all infrastructure once, reports this tool's files"
     ***/
    fn project(&self, project: &Project) -> Facts {
        docair_infra::facts(project, InfraTool::Helm, "helm")
    }
}

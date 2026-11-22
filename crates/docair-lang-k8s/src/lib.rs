//! The Kubernetes adapter (docs/ADAPTERS.md §5.3): Kubernetes manifests: each object is a node; an Ingress routes to its Services, a Service selects its workloads, a pod runs what its image builds and sets the variables its containers read.
//!
//! It reads YAML outside any chart. The references cross tools (a `helm_release` installs a chart,
//! a pod runs what a Dockerfile builds), so the reading is shared: `docair-infra`
//! reads all infrastructure code once, and this adapter reports its own files.

use docair_core::adapter::{Adapter, AdapterKind, Project};
use docair_core::facts::Facts;
use docair_core::model::SourceFile;
use docair_infra::InfraTool;

/***
 * The Kubernetes adapter: Kubernetes manifests: each object is a node; an Ingress routes to its Services, a Service selects its workloads, a pod runs what its image builds and sets the variables its containers read.
 ***/
pub struct KubernetesAdapter;

/// This adapter, for the registry (docs/ADAPTERS.md §5.3): every `docair-lang-*` crate exports one.
pub static ADAPTER: &dyn Adapter = &KubernetesAdapter;

impl Adapter for KubernetesAdapter {
    fn name(&self) -> &'static str {
        "k8s"
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Infra
    }

    fn handles(&self, file: &SourceFile) -> bool {
        file.lang.name == "yaml"
    }

    /***
     * The Kubernetes files' nodes and lines, from the shared reading of all infrastructure code.
     *
     * @calls docair-infra.facts "reads all infrastructure once, reports this tool's files"
     ***/
    fn project(&self, project: &Project) -> Facts {
        docair_infra::facts(project, InfraTool::Kubernetes, "k8s")
    }
}

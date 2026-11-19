//! Infrastructure from its code: Terraform, Helm charts and Kubernetes manifests (SPEC §6.1).
//!
//! The adapter reads the whole project first (every resource, every chart,
//! every Kubernetes object), then completes and adds nodes file by file:
//!
//! * a Terraform `resource` is a node; what it references, and `depends_on`,
//!   become edges (`@in … from tf:<address>`), and so does the cluster its
//!   `helm` or `kubernetes` provider talks to;
//! * what a resource brings into existence is `@creates`: a `helm_release`
//!   creates a release in its namespace, a `kind_cluster` (or a cloud cluster)
//!   a cluster, a `kubernetes_namespace` a namespace;
//! * a release of a chart in the project contains the chart's objects;
//! * an Ingress `@routes` to its Services, a Service `@selects` its workloads.
//!
//! Authored blocks win: a resource or object with a block keeps it, and gets
//! only the inferred relations it does not state itself.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use docair_core::adapter::{Adapter, Project};
use docair_core::facts::{
    Draw, Evidence, Facts, Inferred, Meet, Relation, Serve, Symbol, Target, Verb,
};
use docair_core::ids::slug;
use docair_core::model::{Origin, RawBlock, SourceFile};

/// Cluster resources of the providers docair knows.
const CLUSTERS: &[&str] = &[
    "kind_cluster",
    "google_container_cluster",
    "aws_eks_cluster",
    "azurerm_kubernetes_cluster",
    "digitalocean_kubernetes_cluster",
    "k3d_cluster",
];

/// A top-level Terraform block: `resource "T" "N" { … }`.
#[derive(Debug, Clone)]
struct TfBlock {
    file: String,
    /// 1-based line of the block's first line.
    line: usize,
    kind: String,
    labels: Vec<String>,
    body: Vec<String>,
}

impl TfBlock {
    fn address(&self) -> String {
        self.labels.join(".")
    }

    /// A top-level attribute's value, unquoted: `name = "traefik"` → `traefik`.
    fn attr(&self, key: &str) -> Option<String> {
        let mut depth = 0i32;
        for l in &self.body {
            let t = l.trim();
            if depth == 0
                && let Some((k, v)) = t.split_once('=')
                && k.trim() == key
            {
                return Some(v.trim().trim_end_matches(',').trim_matches('"').to_string());
            }
            depth += brace_delta(t);
        }
        None
    }
}

/// `{`, `[` open and `}`, `]` close, outside strings.
fn brace_delta(line: &str) -> i32 {
    let mut d = 0;
    let mut in_str = false;
    let mut prev = ' ';
    for c in line.chars() {
        match c {
            '"' if prev != '\\' => in_str = !in_str,
            '{' | '[' if !in_str => d += 1,
            '}' | ']' if !in_str => d -= 1,
            _ => {}
        }
        prev = c;
    }
    d
}

/// The top-level blocks of a Terraform file.
fn tf_blocks(file: &str, text: &str) -> Vec<TfBlock> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        let first = l.split_whitespace().next().unwrap_or("");
        if l.starts_with(char::is_alphabetic) && l.trim_end().ends_with('{') {
            let labels: Vec<String> = l
                .split('"')
                .skip(1)
                .step_by(2)
                .map(str::to_string)
                .collect();
            let mut depth = brace_delta(l);
            let mut body = Vec::new();
            let mut j = i + 1;
            while j < lines.len() && depth > 0 {
                depth += brace_delta(lines[j]);
                if depth > 0 {
                    body.push(lines[j].to_string());
                }
                j += 1;
            }
            out.push(TfBlock {
                file: file.to_string(),
                line: i + 1,
                kind: first.to_string(),
                labels,
                body,
            });
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// A Kubernetes object in a manifest or chart template.
#[derive(Debug, Clone)]
struct K8sObject {
    file: String,
    /// 1-based line where the object starts (its `apiVersion:`).
    line: usize,
    kind: String,
    name: String,
    namespace: Option<String>,
    labels: BTreeSet<String>,
    selector: BTreeSet<String>,
    /// Ingress: (host, path, service, port).
    routes: Vec<(String, String, String, Option<String>)>,
    /// Service: the port it forwards to.
    target_port: Option<String>,
    /// Ingress: the controller that serves it (`ingressClassName`).
    class: Option<String>,
    /// Workload: the environment variables its containers get.
    env: Vec<EnvSet>,
    /// ConfigMap, Secret: the keys of `data` and `stringData`.
    keys: Vec<String>,
    /// Workload: its containers' images, as written (`{{ .Values.server.image.repository }}:…`), filled from the chart's values once read.
    images: Vec<String>,
}

/// What a Dockerfile builds: the Rust crates `cargo build -p` builds, and the project files it copies in.
#[derive(Debug, Clone, Default)]
struct DockerBuild {
    crates: Vec<String>,
    copies: Vec<String>,
}

/// How a workload's container gets an environment variable.
#[derive(Debug, Clone, PartialEq, Eq)]
enum EnvSet {
    /// `- name: X` with `value:`: the workload sets it itself.
    Value(String),
    /// `valueFrom: configMapKeyRef` / `secretKeyRef`: from one key of a ConfigMap or Secret.
    Key {
        var: String,
        kind: String,
        name: String,
    },
    /// `envFrom: configMapRef` / `secretRef`: every key of a ConfigMap or Secret.
    All { kind: String, name: String },
}

impl K8sObject {
    fn symbol(&self) -> String {
        format!("{} {}", self.kind, self.name)
    }

    fn id(&self) -> String {
        let short = match self.kind.as_str() {
            "Deployment" => "deploy",
            "StatefulSet" => "sts",
            "DaemonSet" => "ds",
            "ReplicaSet" => "rs",
            "CronJob" => "cronjob",
            "Job" => "job",
            "Pod" => "pod",
            "Service" => "svc",
            "Ingress" => "ingress",
            "ConfigMap" => "cm",
            "Secret" => "secret",
            "NetworkPolicy" => "netpol",
            "Namespace" => "ns",
            "ServiceAccount" => "sa",
            "PersistentVolumeClaim" => "pvc",
            "HorizontalPodAutoscaler" => "hpa",
            other => return format!("k8s.{}.{}", slug(&other.to_lowercase()), slug(&self.name)),
        };
        format!("k8s.{short}.{}", slug(&self.name))
    }

    fn node_kind(&self) -> &'static str {
        match self.kind.as_str() {
            "Deployment" | "StatefulSet" | "DaemonSet" | "ReplicaSet" | "Pod" | "CronJob"
            | "Job" => "workload",
            "Service" => "endpoint",
            "Ingress" | "Gateway" | "HTTPRoute" => "gateway",
            "Namespace" => "namespace",
            _ => "config",
        }
    }

    fn is_workload(&self) -> bool {
        self.node_kind() == "workload"
    }
}

/// The values `"x"` passed to template helpers: `{{ include "sel" "docair-server" }}` → `docair-server`.
fn quoted_args(line: &str) -> Vec<String> {
    if !line.contains("{{") {
        return Vec::new();
    }
    line.split('"')
        .skip(1)
        .step_by(2)
        .skip(1)
        .map(str::to_string)
        .collect()
}

fn indent(l: &str) -> usize {
    l.len() - l.trim_start().len()
}

/// The objects of one YAML file: documents split at `---`, each with kind and name.
fn k8s_objects(file: &str, text: &str) -> Vec<K8sObject> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut start = 0;
    for k in 0..=lines.len() {
        if k == lines.len() || lines[k].trim() == "---" {
            if let Some(o) = k8s_doc(file, &lines[start..k], start) {
                out.push(o);
            }
            start = k + 1;
        }
    }
    out
}

fn k8s_doc(file: &str, lines: &[&str], offset: usize) -> Option<K8sObject> {
    let api = lines.iter().position(|l| l.starts_with("apiVersion:"))?;
    let kind = lines
        .iter()
        .find_map(|l| l.strip_prefix("kind:"))?
        .trim()
        .to_string();
    let stem = file
        .rsplit('/')
        .next()
        .unwrap_or(file)
        .split('.')
        .next()
        .unwrap_or(file)
        .to_string();
    let mut name = None;
    let mut namespace = None;
    let mut labels = BTreeSet::new();
    let mut selector = BTreeSet::new();
    let mut routes = Vec::new();
    let mut target_port = None;
    let mut class = None;
    let mut env: Vec<EnvSet> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    let mut images: Vec<String> = Vec::new();
    // A path stack of keys by indentation, to know where a line is.
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut host = String::new();
    let mut path = String::new();
    let mut svc: Option<String> = None;
    for raw in lines {
        let t = raw.trim();
        if t.is_empty()
            || t.starts_with('#')
            || t.starts_with("{{-") && t.ends_with("}}") && !t.contains(':')
        {
            // Helper lines like `{{- include "x" . | nindent 4 }}` can carry labels.
            if let Some(top) = stack.last()
                && (top.1 == "labels" || top.1 == "matchLabels" || top.1 == "selector")
            {
                for a in quoted_args(t) {
                    if top.1 == "selector"
                        && stack.iter().any(|(_, k)| k == "spec")
                        && kind == "Service"
                    {
                        selector.insert(a);
                    } else {
                        labels.insert(a);
                    }
                }
            }
            continue;
        }
        let ind = indent(raw);
        while stack.last().is_some_and(|(i, _)| *i >= ind) {
            stack.pop();
        }
        let item = t.trim_start_matches("- ");
        let Some((key, value)) = item.split_once(':') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = value.trim().trim_matches('"').to_string();
        let parents: Vec<&str> = stack.iter().map(|(_, k)| k.as_str()).collect();
        match (parents.as_slice(), key.as_str()) {
            (["metadata"], "name") => name = Some(value.clone()),
            (["metadata"], "namespace") => namespace = Some(value.clone()),
            (p, _) if p.last() == Some(&"labels") || p.last() == Some(&"matchLabels") => {
                labels.insert(format!("{key}={value}"));
                labels.insert(value.clone());
            }
            (["spec", "selector"], _) if kind == "Service" => {
                selector.insert(format!("{key}={value}"));
            }
            (p, "targetPort") if p.first() == Some(&"spec") && kind == "Service" => {
                target_port = Some(value.clone())
            }
            (["spec"], "ingressClassName") => class = Some(value.clone()),
            (p, "image") if p.contains(&"containers") => images.push(value.clone()),
            // Environment: `env: - name: X`, then `valueFrom: secretKeyRef: name: …`.
            (p, "name") if p.last() == Some(&"env") => env.push(EnvSet::Value(value.clone())),
            // `valueFrom: fieldRef` or `resourceFieldRef`: Kubernetes fills it from the pod itself
            // (the downward API), so no setting sets it.
            (p, "fieldPath" | "resource")
                if p.len() >= 2
                    && p[p.len() - 2] == "valueFrom"
                    && matches!(p[p.len() - 1], "fieldRef" | "resourceFieldRef") =>
            {
                if matches!(env.last(), Some(EnvSet::Value(_))) {
                    env.pop();
                }
            }
            (p, "name")
                if p.len() >= 2
                    && p[p.len() - 2] == "valueFrom"
                    && matches!(p[p.len() - 1], "configMapKeyRef" | "secretKeyRef") =>
            {
                let kind = if p[p.len() - 1] == "secretKeyRef" {
                    "Secret"
                } else {
                    "ConfigMap"
                };
                if let (Some(EnvSet::Value(var)), Some(name)) =
                    (env.last().cloned(), object_name(t, &value))
                {
                    *env.last_mut().expect("the variable is there") = EnvSet::Key {
                        var,
                        kind: kind.into(),
                        name,
                    };
                }
            }
            (p, "name")
                if p.len() >= 2
                    && p[p.len() - 2] == "envFrom"
                    && matches!(p[p.len() - 1], "configMapRef" | "secretRef") =>
            {
                let kind = if p[p.len() - 1] == "secretRef" {
                    "Secret"
                } else {
                    "ConfigMap"
                };
                if let Some(name) = object_name(t, &value) {
                    env.push(EnvSet::All {
                        kind: kind.into(),
                        name,
                    });
                }
            }
            (["data"] | ["stringData"], k) if kind == "ConfigMap" || kind == "Secret" => {
                keys.push(k.to_string())
            }
            (_, "kubernetes.io/ingress.class") => class = Some(value.clone()),
            // A templated host (`{{ . | quote }}`) is not known: the rule then holds for any host.
            (_, "host") if kind == "Ingress" => {
                host = if value.contains("{{") {
                    String::new()
                } else {
                    value.clone()
                }
            }
            (_, "path") if kind == "Ingress" => {
                // A new path: the one before it is done, even when its port had no number.
                if let Some(s) = svc.take() {
                    routes.push((host.clone(), path.clone(), s, None));
                }
                path = value.clone()
            }
            (p, "name") if kind == "Ingress" && p.last() == Some(&"port") => {
                if let Some(s) = svc.take() {
                    routes.push((host.clone(), path.clone(), s, Some(value.clone())));
                }
            }
            (p, "name") if kind == "Ingress" && p.last() == Some(&"service") => {
                svc = Some(value.clone())
            }
            (p, "number") if kind == "Ingress" && p.last() == Some(&"port") => {
                if let Some(s) = svc.take() {
                    routes.push((host.clone(), path.clone(), s, Some(value.clone())));
                }
            }
            (_, "serviceName") if kind == "Ingress" => svc = Some(value.clone()),
            (_, "servicePort") if kind == "Ingress" => {
                if let Some(s) = svc.take() {
                    routes.push((host.clone(), path.clone(), s, Some(value.clone())));
                }
            }
            _ => {}
        }
        if value.is_empty() || value.starts_with("{{") && !value.ends_with("}}") {
            stack.push((ind, key.clone()));
        } else if value.starts_with("{{") {
            // `selector: {{ include … }}`: the helper's args are the labels.
            if key == "selector" || key == "matchLabels" || key == "labels" {
                for a in quoted_args(t) {
                    if kind == "Service" && key == "selector" {
                        selector.insert(a);
                    } else {
                        labels.insert(a);
                    }
                }
            }
        }
        if value.is_empty() {
            continue;
        }
    }
    if let Some(s) = svc {
        routes.push((host, path, s, None));
    }
    let mut name = name?;
    if name.contains("{{") {
        name = stem;
    }
    routes.retain(|(_, _, s, _)| !s.contains("{{"));
    Some(K8sObject {
        file: file.to_string(),
        line: offset + api + 1,
        kind,
        name,
        namespace,
        labels,
        selector,
        routes,
        target_port,
        class,
        env,
        keys,
        images,
    })
}

/// An object's name as a manifest writes it: `docair-ai`, or the default of a template (`{{ .Values.x | default "docair-ai" }}`).
fn object_name(line: &str, value: &str) -> Option<String> {
    if !value.contains("{{") {
        return Some(value.to_string()).filter(|v| !v.is_empty());
    }
    let (_, after) = line.split_once("default")?;
    let name = after.split('"').nth(1)?;
    Some(name.to_string()).filter(|n| !n.is_empty())
}

/// What the adapter knows about the whole project.
#[derive(Debug, Default)]
struct Index {
    /// The system node: where created clusters go.
    system: String,
    /// Release nodes the Terraform creates.
    releases: BTreeSet<String>,
    resources: Vec<TfBlock>,
    /// Terraform addresses with an authored block → that block's `@creates` target ("" for none).
    authored: HashMap<String, String>,
    locals: HashMap<String, String>,
    /// Locals whose value reads a file: `file(…)`, `filesha1(…)`, `templatefile(…)`.
    file_locals: BTreeSet<String>,
    /// Cluster nodes the Terraform creates: authored `@creates` targets or `k8s.cluster.<name>`.
    clusters: Vec<String>,
    /// Addresses of cluster resources the helm/kubernetes providers talk to.
    provider_clusters: Vec<String>,
    /// Chart directory → release node ids that install it.
    chart_releases: BTreeMap<String, Vec<String>>,
    /// Chart directories (with a Chart.yaml) and their chart names.
    charts: BTreeMap<String, String>,
    objects: Vec<K8sObject>,
    /// Object symbols (`Deployment x`) that have an authored block.
    authored_objects: BTreeSet<String>,
    /// Chart directory → its `values.yaml`, flattened: `ingress.className` → `traefik`.
    values: BTreeMap<String, HashMap<String, String>>,
    /// Blocks inside Terraform resources: `node {`, `extra_port_mappings {`, `values = [yamlencode({`.
    subs: Vec<Sub>,
    /// The `@id` of the authored block on a code line: (file, line) → id.
    ids_at: HashMap<(String, usize), String>,
    /// Dockerfiles by path: what each builds.
    dockerfiles: HashMap<String, DockerBuild>,
    /// Image repository → the Dockerfile Terraform's `docker build -f … -t …` builds it from.
    builds: HashMap<String, String>,
    /// Rust crate name → the id of its `main`, when the crate has a `src/main.rs`.
    mains: HashMap<String, String>,
}

/// A block inside a Terraform resource, with its own attributes.
#[derive(Debug, Clone)]
struct Sub {
    file: String,
    line: usize,
    /// `node`, `extra_port_mappings`, `values`.
    kind: String,
    /// `container_port = 30080`; for `values`, every `key = value` inside (`nodePort`, `isDefaultClass`).
    attrs: HashMap<String, String>,
    /// The line of the block it sits in, when that is a block too (`node` for `extra_port_mappings`).
    parent: Option<usize>,
    /// The resource's address: `kind_cluster.docair`.
    resource: String,
}

/// The blocks inside a resource's body: `name {` and `name = [yamlencode({`, with the lines they start on.
fn tf_subs(r: &TfBlock) -> Vec<Sub> {
    let mut out: Vec<Sub> = Vec::new();
    // (index in out, depth the block opened at)
    let mut open: Vec<(usize, i32)> = Vec::new();
    let mut depth = 0;
    for (k, l) in r.body.iter().enumerate() {
        let t = l.trim();
        let line = r.line + 1 + k;
        let head = t
            .split(|c: char| c.is_whitespace() || c == '=' || c == '{')
            .next()
            .unwrap_or("");
        let named = !head.is_empty() && head.chars().all(|c| c.is_alphanumeric() || c == '_');
        let opens = named && (t == format!("{head} {{") || t.contains("yamlencode({"));
        if let Some((at, _)) = open.last()
            && let Some((key, value)) = t.split_once('=')
            && !opens
        {
            let key = key.trim().trim_matches('"');
            let value = value.trim().trim_end_matches(',').trim().trim_matches('"');
            if !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                out[*at].attrs.insert(key.to_string(), value.to_string());
            }
            // `ports = { web = { nodePort = 30080 } }`: the keys on one line, too.
            for part in t.split(['{', '}', ',']) {
                if let Some((k2, v2)) = part.split_once('=') {
                    let k2 = k2.trim();
                    if !k2.is_empty() && k2.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        out[*at]
                            .attrs
                            .entry(k2.to_string())
                            .or_insert_with(|| v2.trim().trim_matches('"').to_string());
                    }
                }
            }
        }
        if opens {
            out.push(Sub {
                file: r.file.clone(),
                line,
                kind: head.to_string(),
                attrs: HashMap::new(),
                parent: open.last().map(|(at, _)| out[*at].line),
                resource: r.address(),
            });
            open.push((out.len() - 1, depth));
        }
        depth += brace_delta(l);
        while open.last().is_some_and(|(_, d)| depth <= *d) {
            open.pop();
        }
    }
    out
}

/// A chart's `values.yaml`, flattened to dotted keys: `ingress:\n  className: traefik` → `ingress.className` → `traefik`.
fn flat_values(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    for raw in text.lines() {
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with('-') {
            continue;
        }
        let ind = indent(raw);
        while stack.last().is_some_and(|(i, _)| *i >= ind) {
            stack.pop();
        }
        let Some((key, value)) = t.split_once(':') else {
            continue;
        };
        let value = value
            .split(" #")
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches('"');
        let path: Vec<&str> = stack
            .iter()
            .map(|(_, k)| k.as_str())
            .chain([key.trim()])
            .collect();
        if value.is_empty() {
            stack.push((ind, key.trim().to_string()));
        } else {
            out.insert(path.join("."), value.to_string());
        }
    }
    out
}

/// A template's value from the chart's values: `{{ .Values.ingress.className }}` → `traefik`.
fn from_values(template: &str, values: Option<&HashMap<String, String>>) -> Option<String> {
    let inner = template
        .trim()
        .strip_prefix("{{")?
        .strip_suffix("}}")?
        .trim();
    let path = inner.strip_prefix(".Values.")?;
    if path.contains(|c: char| c.is_whitespace() || c == '|') {
        return None;
    }
    values?.get(path).cloned().filter(|v| !v.is_empty())
}

/// The block attached to a line, if any: its following line is the line or one or two before it.
fn block_at(blocks: &[RawBlock], line: usize) -> Option<&RawBlock> {
    blocks.iter().find(|b| {
        b.origin == Origin::Authored
            && b.following
                .as_ref()
                .is_some_and(|f| f.line <= line && f.line + 2 >= line)
    })
}

fn authored_creates(b: &RawBlock) -> Option<String> {
    b.tags
        .iter()
        .find(|t| t.name == "creates" && t.origin == Origin::Authored)
        .map(|t| t.value.split_whitespace().next().unwrap_or("").to_string())
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

impl Index {
    /***
     * Read every Terraform file, chart and manifest of the project once, with their authored blocks.
     ***/
    fn new(files: &[SourceFile], blocks: &[Vec<RawBlock>], system: &str) -> Index {
        let mut a = Index {
            system: system.to_string(),
            ..Index::default()
        };
        for (f, bs) in files.iter().zip(blocks) {
            if f.lang.name == "terraform" {
                for b in tf_blocks(&f.rel, &f.text) {
                    match b.kind.as_str() {
                        "resource" | "data" if b.labels.len() == 2 => {
                            if let Some(auth) = block_at(bs, b.line) {
                                a.authored.insert(
                                    b.address(),
                                    authored_creates(auth).unwrap_or_default(),
                                );
                            }
                            a.resources.push(b);
                        }
                        "locals" => {
                            // A local runs from `name = …` to the next one at the same indent.
                            let top = b
                                .body
                                .iter()
                                .find(|l| !l.trim().is_empty())
                                .map_or(0, |l| indent(l));
                            let mut current: Option<String> = None;
                            for l in &b.body {
                                let starts = indent(l) == top
                                    && l.split_once('=').is_some_and(|(k, _)| {
                                        let k = k.trim();
                                        !k.is_empty()
                                            && k.chars().all(|c| c.is_alphanumeric() || c == '_')
                                    });
                                if starts && let Some((k, v)) = l.split_once('=') {
                                    let k = k.trim().to_string();
                                    a.locals
                                        .insert(k.clone(), v.trim().trim_matches('"').to_string());
                                    current = Some(k);
                                }
                                if let Some(k) = &current
                                    && reads_file(l)
                                {
                                    a.file_locals.insert(k.clone());
                                }
                            }
                        }
                        "provider"
                            if matches!(
                                b.labels.first().map(String::as_str),
                                Some("helm" | "kubernetes")
                            ) =>
                        {
                            a.provider_clusters.extend(
                                b.body
                                    .iter()
                                    .flat_map(|l| refs_in(l))
                                    .filter(|r| {
                                        CLUSTERS.contains(&r.split('.').next().unwrap_or(""))
                                    })
                                    .map(|r| r.split('.').take(2).collect::<Vec<_>>().join(".")),
                            );
                        }
                        _ => {}
                    }
                }
            }
            if f.lang.name == "dockerfile" {
                a.dockerfiles.insert(f.rel.clone(), docker_build(&f.text));
            }
            // A crate with a `main`: what `cargo build -p` makes runs it (`<crate>.main`, the Rust adapter's id).
            if f.rel.ends_with("Cargo.toml")
                && let Some(name) = package_name(&f.text)
                && files
                    .iter()
                    .any(|x| x.rel == format!("{}src/main.rs", dir_prefix(&f.rel)))
            {
                a.mains
                    .insert(name.clone(), format!("{}.main", slug(&name)));
            }
            if f.rel.ends_with("/values.yaml") || f.rel == "values.yaml" {
                a.values
                    .insert(dir_of(&f.rel).to_string(), flat_values(&f.text));
            }
            for b in bs.iter().filter(|b| b.origin == Origin::Authored) {
                if let (Some(fl), Some(id)) = (&b.following, b.tags.iter().find(|t| t.name == "id"))
                {
                    a.ids_at
                        .insert((f.rel.clone(), fl.line), id.value.trim().to_string());
                }
            }
            if f.rel.ends_with("Chart.yaml") {
                let name = f
                    .text
                    .lines()
                    .find_map(|l| l.strip_prefix("name:"))
                    .map(|n| n.trim().trim_matches('"').to_string())
                    .unwrap_or_else(|| {
                        dir_of(&f.rel)
                            .rsplit('/')
                            .next()
                            .unwrap_or("chart")
                            .to_string()
                    });
                a.charts.insert(dir_of(&f.rel).to_string(), name);
                if let Some(b) = bs.iter().find(|b| b.origin == Origin::Authored)
                    && let Some(id) = b.tags.iter().find(|t| t.name == "id")
                {
                    a.chart_releases
                        .entry(dir_of(&f.rel).to_string())
                        .or_default()
                        .push(id.value.clone());
                }
            }
            if f.lang.name == "yaml" {
                for o in k8s_objects(&f.rel, &f.text) {
                    if block_at(bs, o.line).is_some() {
                        a.authored_objects.insert(o.symbol());
                    }
                    a.objects.push(o);
                }
            }
        }
        a.provider_clusters.sort();
        a.provider_clusters.dedup();
        a.subs = a.resources.iter().flat_map(tf_subs).collect();
        // A templated class, from the chart's values: `{{ .Values.ingress.className }}`.
        for o in &mut a.objects {
            if let Some(c) = o.class.clone().filter(|c| c.contains("{{")) {
                o.class = chart_of(&o.file).and_then(|d| from_values(&c, a.values.get(d)));
            }
            // Images, filled from the chart's values: `docair-server:dev`.
            let values = chart_of(&o.file).and_then(|d| a.values.get(d));
            o.images = o
                .images
                .iter()
                .filter_map(|i| fill_values(i, values))
                .collect();
        }
        // What Terraform builds: `docker build -f docker/server.Dockerfile -t ${local.server_image} .`.
        for r in &a.resources {
            for l in &r.body {
                for (dockerfile, tag) in docker_builds(l) {
                    let tag = expand_locals(&tag, &a.locals);
                    if let Some(repo) = image_repo(&tag) {
                        a.builds.insert(repo, dockerfile);
                    }
                }
            }
        }
        // What the clusters and releases are called, now that every file is read.
        for r in a.resources.clone() {
            let ty = r.labels[0].as_str();
            if CLUSTERS.contains(&ty) {
                a.clusters.push(a.created_id(&r).unwrap_or_default());
            }
            if ty == "helm_release"
                && let Some(id) = a.created_id(&r)
            {
                a.releases.insert(id.clone());
                if let Some(dir) = a.chart_dir(&r) {
                    a.chart_releases.entry(dir).or_default().insert(0, id);
                }
            }
        }
        a.clusters.retain(|c| !c.is_empty());
        a
    }

    /// A string attribute, with `local.x` looked up.
    fn value(&self, r: &TfBlock, key: &str) -> Option<String> {
        let v = r.attr(key)?;
        Some(match v.strip_prefix("local.") {
            Some(name) => self.locals.get(name).cloned().unwrap_or(v),
            None => v,
        })
    }

    /// The chart directory of a `helm_release` whose chart lies in the project.
    fn chart_dir(&self, r: &TfBlock) -> Option<String> {
        let chart = self.value(r, "chart")?;
        if self.value(r, "repository").is_some() {
            return None;
        }
        // `${path.module}/../../deploy/helm/docair` → `deploy/helm/docair`
        let tail: Vec<&str> = chart
            .split('/')
            .filter(|s| !s.is_empty() && !s.starts_with("${") && *s != ".." && *s != ".")
            .collect();
        let tail = tail.join("/");
        self.charts
            .keys()
            .filter(|d| !tail.is_empty() && (d.ends_with(&tail) || tail.ends_with(d.as_str())))
            .max_by_key(|d| d.len())
            .cloned()
    }

    /// The node a resource brings into existence, or `None`.
    fn created_id(&self, r: &TfBlock) -> Option<String> {
        if let Some(target) = self.authored.get(&r.address())
            && !target.is_empty()
        {
            return Some(target.clone());
        }
        let ty = r.labels[0].as_str();
        let name = self
            .value(r, "name")
            .filter(|n| !n.contains("${"))
            .unwrap_or_else(|| r.labels[1].clone());
        if ty == "helm_release" {
            Some(format!("k8s.release.{}", slug(&name)))
        } else if CLUSTERS.contains(&ty) {
            Some(format!("k8s.cluster.{}", slug(&name)))
        } else if ty.starts_with("kubernetes_namespace") {
            let n = r
                .body
                .iter()
                .find_map(|l| {
                    l.trim().strip_prefix("name").map(|v| {
                        v.trim_start_matches([' ', '='])
                            .trim_matches('"')
                            .to_string()
                    })
                })
                .unwrap_or(name);
            Some(format!("k8s.ns.{}", slug(&n)))
        } else {
            None
        }
    }

    /// What a resource uses, the cluster it talks to, and what it creates.
    fn resource_relations(&self, r: &TfBlock, authored: Option<&RawBlock>) -> Vec<Relation> {
        let known: BTreeSet<String> = self.resources.iter().map(TfBlock::address).collect();
        let me = r.address();
        let line = r.line;
        let why = "terraform reference";
        let mut uses: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut after = false;
        for l in &r.body {
            let t = l.trim();
            if t.starts_with("depends_on") {
                after = true;
            }
            for full in refs_in(t) {
                let addr: String = full.split('.').take(2).collect::<Vec<_>>().join(".");
                if addr == me || !known.contains(&addr) {
                    continue;
                }
                let what = if after {
                    "runs after".to_string()
                } else {
                    full.split('.')
                        .nth(2)
                        .map_or("uses".to_string(), |a| a.to_string())
                };
                uses.entry(addr).or_default().insert(what);
            }
            if after && t.contains(']') {
                after = false;
            }
        }
        let ty = r.labels[0].as_str();
        if ty.starts_with("helm_") || ty.starts_with("kubernetes_") {
            for c in &self.provider_clusters {
                uses.entry(c.clone())
                    .or_default()
                    .insert("cluster access".into());
            }
        }
        // A file it reads, itself or through a local: the block names the store (`file:`).
        let reads = r.body.iter().find(|l| {
            reads_file(l)
                || refs_in(l.trim()).iter().any(|f| {
                    f.strip_prefix("local.").is_some_and(|n| {
                        self.file_locals.contains(n.split('.').next().unwrap_or(n))
                    })
                })
                || self
                    .file_locals
                    .iter()
                    .any(|n| l.contains(&format!("local.{n}")))
        });
        let file_read = reads.map(|l| {
            Relation::new(
                Verb::Reads,
                Target::Ref("file:".into()),
                Evidence::new(line, format!("terraform: {}", l.trim())),
            )
        });
        let mut tags: Vec<Relation> = uses
            .into_iter()
            .map(|(addr, what)| {
                let what = what.into_iter().collect::<Vec<_>>().join(", ");
                Relation::new(
                    Verb::In,
                    Target::Ref(format!("tf:{addr}")),
                    Evidence::new(line, why),
                )
                .label(what)
            })
            .collect();
        tags.extend(file_read);
        let states_creates = authored.is_some_and(|b| authored_creates(b).is_some());
        if !states_creates && let Some(target) = self.created_id(r) {
            let how = if ty == "helm_release" {
                let chart = self.value(r, "chart").unwrap_or_default();
                let chart = chart.rsplit('/').next().unwrap_or(&chart).to_string();
                let version = self
                    .value(r, "version")
                    .map(|v| format!(" {v}"))
                    .unwrap_or_default();
                format!("helm install: chart {chart}{version}")
            } else if CLUSTERS.contains(&ty) {
                format!("{ty} creates the cluster")
            } else {
                "creates it".into()
            };
            tags.push(
                Relation::new(
                    Verb::Creates,
                    Target::Ref(target),
                    Evidence::new(line, "terraform resource type"),
                )
                .payload(Some(how)),
            );
        }
        tags
    }

    /// Nodes that a resource brings into existence: a cluster, a namespace, a release.
    fn created(&self, r: &TfBlock) -> Vec<Symbol> {
        let mut out = Vec::new();
        if self
            .authored
            .get(&r.address())
            .is_some_and(|t| !t.is_empty())
        {
            return out;
        }
        let Some(id) = self.created_id(r) else {
            return out;
        };
        let ty = r.labels[0].as_str();
        let name = id.rsplit('.').next().unwrap_or(&id).to_string();
        let cluster_parts: Vec<String> = self.clusters.clone();
        let mut push = |id: &str, kind: &str, name: &str, summary: String, part_of: Vec<String>| {
            out.push(discovered(
                &r.file,
                r.line,
                id,
                kind,
                name,
                &summary,
                part_of,
                &format!("created by {}", r.address()),
            ));
        };
        if ty == "helm_release" {
            let ns = self
                .value(r, "namespace")
                .unwrap_or_else(|| "default".into());
            let chart = self.value(r, "chart").unwrap_or_default();
            let chart = chart.rsplit('/').next().unwrap_or(&chart).to_string();
            push(
                &id,
                "release",
                &name,
                format!(
                    "The Helm release {name}: what `helm install` made of the chart {chart}, in the namespace {ns}."
                ),
                vec![format!("k8s.ns.{}", slug(&ns))],
            );
            push(
                &format!("k8s.ns.{}", slug(&ns)),
                "namespace",
                &ns,
                format!("The namespace {ns} in the cluster."),
                cluster_parts,
            );
        } else if CLUSTERS.contains(&ty) {
            push(
                &id,
                "cluster",
                &name,
                format!("The Kubernetes cluster {name}, created by Terraform ({ty})."),
                vec![self.system.clone()],
            );
        } else if ty.starts_with("kubernetes_namespace") {
            push(
                &id,
                "namespace",
                &name,
                format!("The namespace {name} in the cluster."),
                cluster_parts,
            );
        }
        out
    }

    /// What a Kubernetes object does: an Ingress routes, a Service selects.
    /// What a block inside a resource is called on the graph: its block's `@id`, else the resource.
    fn sub_target(&self, file: &str, line: usize, resource: &str) -> String {
        self.ids_at
            .get(&(file.to_string(), line))
            .cloned()
            .unwrap_or_else(|| format!("tf:{resource}"))
    }

    /***
     * The way into the cluster, from the blocks inside resources: a port mapping leads into its node, the node to the release whose NodePort it publishes, the ingress controller's values to the Ingresses of its class.
     *
     * `extra_port_mappings { container_port = 30080 }` sits in `node { }`;
     * `helm_release` values with `nodePort = 30080` take that port; a release
     * whose values make it an IngressClass (`ingressClass`, `isDefaultClass`)
     * takes the Ingresses whose `ingressClassName` is its name, and those
     * without one when it is the default. The lines go to the `@id` of the
     * block on the other end, else to the resource.
     ***/
    fn sub_relations(&self, sub: &Sub) -> Vec<Relation> {
        let call = |to: String, what: String, why: &str| {
            Relation::new(Verb::Calls, Target::Ref(to), Evidence::new(sub.line, why))
                .payload(Some(what))
        };
        match sub.kind.as_str() {
            "extra_port_mappings" => {
                let Some(parent) = sub.parent else {
                    return Vec::new();
                };
                let a = |k: &str| sub.attrs.get(k).cloned().unwrap_or_default();
                let host = match a("listen_address") {
                    l if l.is_empty() => a("host_port"),
                    l => format!("{l}:{}", a("host_port")),
                };
                vec![call(
                    self.sub_target(&sub.file, parent, &sub.resource),
                    format!(
                        "{} {host} → :{}",
                        a("protocol").to_uppercase(),
                        a("container_port")
                    ),
                    "extra_port_mappings in node",
                )]
            }
            "node" => {
                let ports: Vec<&String> = self
                    .subs
                    .iter()
                    .filter(|m| m.file == sub.file && m.parent == Some(sub.line))
                    .filter_map(|m| m.attrs.get("container_port"))
                    .collect();
                let mut out = Vec::new();
                for v in self.subs.iter().filter(|v| v.kind == "values") {
                    if let Some(port) = v.attrs.get("nodePort").filter(|p| ports.contains(p)) {
                        out.push(call(
                            self.sub_target(&v.file, v.line, &v.resource),
                            format!("NodePort {port}"),
                            "container_port = nodePort",
                        ));
                    }
                }
                out
            }
            "values" => {
                let Some(r) = self.resources.iter().find(|r| r.address() == sub.resource) else {
                    return Vec::new();
                };
                let controller = sub.attrs.contains_key("ingressClass")
                    || sub.attrs.contains_key("isDefaultClass");
                if !controller {
                    return Vec::new();
                }
                let name = self.value(r, "name").unwrap_or_default();
                let default = sub.attrs.get("isDefaultClass").is_some_and(|v| v == "true");
                self.objects
                    .iter()
                    .filter(|o| o.kind == "Ingress")
                    .filter(|o| match &o.class {
                        Some(c) => *c == name,
                        None => default,
                    })
                    .map(|o| {
                        call(
                            format!("k8s:{}", o.symbol().replacen(' ', "/", 1)),
                            format!("ingress class {name}"),
                            "ingressClassName",
                        )
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /***
     * The environment variables an object sets for the code: a ConfigMap's or Secret's keys that workloads take, a workload's own values.
     *
     * `envFrom` takes every key; `valueFrom` one key, under the variable's
     * name. The workloads are those of the same chart or folder.
     ***/
    fn env_set_by(&self, o: &K8sObject) -> Vec<(String, Evidence)> {
        let same_place = |other: &K8sObject| {
            dir_of(&other.file) == dir_of(&o.file) || chart_of(&other.file) == chart_of(&o.file)
        };
        let mut out: Vec<(String, Evidence)> = Vec::new();
        let mut add = |name: &str, why: String| {
            if !out.iter().any(|(n, _)| n == name) {
                out.push((name.to_string(), Evidence::new(o.line, why)));
            }
        };
        if o.is_workload() {
            for e in &o.env {
                if let EnvSet::Value(var) = e {
                    add(var, format!("env value in {} {}", o.kind, o.name));
                }
            }
            return out;
        }
        if o.kind != "ConfigMap" && o.kind != "Secret" {
            return out;
        }
        for w in self
            .objects
            .iter()
            .filter(|w| w.is_workload() && same_place(w))
        {
            for e in &w.env {
                match e {
                    EnvSet::Key { var, kind, name } if *kind == o.kind && *name == o.name => {
                        let r = if o.kind == "Secret" {
                            "secretKeyRef"
                        } else {
                            "configMapKeyRef"
                        };
                        add(var, format!("{r} in {} {}", w.kind, w.name));
                    }
                    EnvSet::All { kind, name } if *kind == o.kind && *name == o.name => {
                        let r = if o.kind == "Secret" {
                            "secretRef"
                        } else {
                            "configMapRef"
                        };
                        for k in &o.keys {
                            add(k, format!("envFrom {r} in {} {}", w.kind, w.name));
                        }
                    }
                    _ => {}
                }
            }
        }
        out
    }

    fn object_relations(&self, o: &K8sObject) -> Vec<Relation> {
        let mut tags = Vec::new();
        let same_place = |other: &K8sObject| {
            dir_of(&other.file) == dir_of(&o.file) || chart_of(&other.file) == chart_of(&o.file)
        };
        if o.kind == "Ingress" {
            // The controller of its class takes the requests and follows these rules.
            if let Some(class) = &o.class {
                let release = format!("k8s.release.{}", slug(class));
                if self.releases.contains(&release)
                    || self
                        .chart_releases
                        .values()
                        .flatten()
                        .any(|r| *r == release)
                {
                    tags.push(
                        Relation::new(
                            Verb::In,
                            Target::Ref(release),
                            Evidence::new(o.line, "ingressClassName"),
                        )
                        .label(format!("requests (ingress class {class})")),
                    );
                }
            }
            for (host, path, svc, port) in &o.routes {
                if let Some(s) = self
                    .objects
                    .iter()
                    .find(|x| x.kind == "Service" && &x.name == svc && same_place(x))
                {
                    let at = format!("HTTP {host}{}", if path.is_empty() { "/" } else { path });
                    let port = port
                        .as_ref()
                        .map(|p| format!(", port {p}"))
                        .unwrap_or_default();
                    tags.push(
                        Relation::new(
                            Verb::Routes,
                            Target::Ref(format!("k8s:{}", s.symbol().replacen(' ', "/", 1))),
                            Evidence::new(o.line, "Ingress rule"),
                        )
                        .payload(Some(format!("{at}{port}"))),
                    );
                }
            }
        }
        if o.kind == "Service" {
            for w in self
                .objects
                .iter()
                .filter(|x| x.is_workload() && same_place(x))
            {
                let by_selector =
                    !o.selector.is_empty() && o.selector.iter().all(|s| w.labels.contains(s));
                if by_selector || (o.selector.is_empty() && w.name == o.name) {
                    tags.push(
                        Relation::new(
                            Verb::Selects,
                            Target::Ref(format!("k8s:{}", w.symbol().replacen(' ', "/", 1))),
                            Evidence::new(o.line, "Service selector"),
                        )
                        .payload(o.target_port.as_ref().map(|p| format!("port {p}"))),
                    );
                    // The program the pods run: image → the Dockerfile that builds it → what it builds (docair-4wk.26).
                    for image in &w.images {
                        if let Some((program, why)) = self.program_of(image) {
                            tags.push(Relation::new(
                                Verb::Calls,
                                Target::Ref(program),
                                Evidence::new(
                                    o.line,
                                    format!("{} {}: image {image}, {why}", w.kind, w.name),
                                ),
                            ));
                        }
                    }
                }
            }
        }
        tags
    }
}

impl Index {
    /// What a pod with this image runs: the `main` of the crate its Dockerfile builds, else the part
    /// a file it copies in declares (nginx's config); with the reason, for the evidence.
    fn program_of(&self, image: &str) -> Option<(String, String)> {
        let repo = image_repo(image)?;
        let dockerfile = self.builds.get(&repo)?;
        let build = self.dockerfiles.get(dockerfile)?;
        if let Some((name, main)) = build
            .crates
            .iter()
            .find_map(|c| self.mains.get(c).map(|m| (c, m)))
        {
            return Some((
                main.clone(),
                format!("built by {dockerfile} (cargo build -p {name})"),
            ));
        }
        build.copies.iter().find_map(|copied| {
            self.ids_at
                .iter()
                .filter(|((file, _), _)| file == copied)
                .min_by_key(|((_, line), _)| *line)
                .map(|(_, id)| {
                    (
                        id.clone(),
                        format!("built by {dockerfile}, which copies {copied}"),
                    )
                })
        })
    }
}

/// What a Dockerfile builds: `RUN cargo build … -p <crate>` and `COPY <file> …` of project files (not `--from` a stage).
fn docker_build(text: &str) -> DockerBuild {
    let mut out = DockerBuild::default();
    for l in text.lines().map(str::trim) {
        let words: Vec<&str> = l.split_whitespace().collect();
        match words.first().map(|w| w.to_ascii_uppercase()).as_deref() {
            Some("RUN") if words.contains(&"cargo") && words.contains(&"build") => {
                for (k, w) in words.iter().enumerate() {
                    if matches!(*w, "-p" | "--package")
                        && let Some(c) = words.get(k + 1)
                    {
                        out.crates.push(c.trim_matches('"').to_string());
                    } else if let Some(c) = w.strip_prefix("--package=") {
                        out.crates.push(c.to_string());
                    }
                }
            }
            Some("COPY") | Some("ADD") if !words.iter().any(|w| w.starts_with("--from")) => {
                let args: Vec<&str> = words[1..]
                    .iter()
                    .filter(|w| !w.starts_with("--"))
                    .copied()
                    .collect();
                if args.len() >= 2 {
                    out.copies.extend(
                        args[..args.len() - 1]
                            .iter()
                            .map(|a| a.trim_start_matches("./").to_string()),
                    );
                }
            }
            _ => {}
        }
    }
    out
}

/// The `docker build` commands in a line: (Dockerfile, tag) of each, `-f`/`--file` and `-t`/`--tag`.
fn docker_builds(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let line = line
        .trim()
        .trim_start_matches("command")
        .trim()
        .trim_start_matches('=')
        .trim()
        .trim_matches('"');
    for part in line.split("&&").flat_map(|p| p.split(';')) {
        let words: Vec<&str> = part.split_whitespace().collect();
        let Some(at) = words.windows(2).position(|w| w == ["docker", "build"]) else {
            continue;
        };
        let rest = &words[at + 2..];
        let flag = |names: &[&str]| {
            rest.windows(2)
                .find(|w| names.contains(&w[0]))
                .map(|w| w[1].trim_matches('"').to_string())
        };
        let dockerfile = flag(&["-f", "--file"]).unwrap_or_else(|| "Dockerfile".to_string());
        if let Some(tag) = flag(&["-t", "--tag"]) {
            out.push((dockerfile.trim_start_matches("./").to_string(), tag));
        }
    }
    out
}

/// `${local.x}` replaced by the local's value, a few levels deep; other interpolations stay.
fn expand_locals(text: &str, locals: &HashMap<String, String>) -> String {
    let mut out = text.to_string();
    for _ in 0..4 {
        let Some(start) = out.find("${local.") else {
            break;
        };
        let Some(len) = out[start..].find('}') else {
            break;
        };
        let name = &out[start + 8..start + len];
        let Some(v) = locals.get(name) else {
            break;
        };
        out = format!("{}{v}{}", &out[..start], &out[start + len + 1..]);
    }
    out
}

/// An image's repository: `docair-server` of `docair-server:dev` or `registry/docair-server:1.2`; none while it is not known.
fn image_repo(image: &str) -> Option<String> {
    let repo = image
        .rsplit_once(':')
        .map_or(image, |(r, tag)| if tag.contains('/') { image } else { r });
    let repo = repo.rsplit('/').next().unwrap_or(repo).trim();
    (!repo.is_empty() && !repo.contains("${") && !repo.contains("{{")).then(|| repo.to_string())
}

/// A template filled from the chart's values: every `{{ .Values.x }}` in it; none when one is not known.
fn fill_values(text: &str, values: Option<&HashMap<String, String>>) -> Option<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let end = rest[start..].find("}}")? + start + 2;
        out.push_str(&rest[..start]);
        out.push_str(&from_values(&rest[start..end], values)?);
        rest = &rest[end..];
    }
    out.push_str(rest);
    Some(out)
}

/// `name = "x"` under `[package]` in a Cargo.toml.
fn package_name(toml: &str) -> Option<String> {
    let mut in_package = false;
    for l in toml.lines().map(str::trim) {
        if l.starts_with('[') {
            in_package = l == "[package]";
        } else if in_package && let Some(v) = l.strip_prefix("name") {
            return Some(
                v.trim()
                    .trim_start_matches('=')
                    .trim()
                    .trim_matches('"')
                    .to_string(),
            );
        }
    }
    None
}

/// `crates/docair-server/` of `crates/docair-server/Cargo.toml`; empty at the root.
fn dir_prefix(rel: &str) -> String {
    rel.rsplit_once('/')
        .map_or(String::new(), |(d, _)| format!("{d}/"))
}

/// Whether a line of Terraform reads a file: `file(…)`, `filesha1(…)`, `templatefile(…)`, `fileset(…)`.
fn reads_file(line: &str) -> bool {
    const READS: &[&str] = &[
        "file(",
        "filebase64(",
        "filesha1(",
        "filesha256(",
        "filemd5(",
        "templatefile(",
        "fileset(",
    ];
    READS.iter().any(|f| {
        line.match_indices(f).any(|(at, _)| {
            line[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
        })
    })
}

/// The chart directory a template belongs to: the directory above `templates/`.
fn chart_of(file: &str) -> Option<&str> {
    file.rsplit_once("/templates/").map(|(d, _)| d)
}

/// `kind_cluster.docair.kubeconfig` and the like: resource references in a line.
fn refs_in(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        let parts: Vec<&str> = cur.split('.').collect();
        if parts.len() >= 2
            && parts[0].contains('_')
            && !matches!(parts[0], "path" | "each" | "count" | "self")
            && parts.iter().all(|p| !p.is_empty())
        {
            out.push(cur.clone());
        }
        cur.clear();
    };
    let mut in_str = false;
    for c in line.chars() {
        if c == '"' {
            in_str = !in_str;
            flush(&mut cur, &mut out);
            continue;
        }
        if in_str && !line.contains("${") {
            continue;
        }
        if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' {
            cur.push(c);
        } else {
            flush(&mut cur, &mut out);
        }
    }
    flush(&mut cur, &mut out);
    out
}

#[allow(clippy::too_many_arguments)]
fn discovered(
    file: &str,
    line: usize,
    id: &str,
    kind: &str,
    name: &str,
    summary: &str,
    part_of: Vec<String>,
    why: &str,
) -> Symbol {
    Symbol {
        draw: Draw::Alone,
        name: Some(name.to_string()),
        id: Some(id.to_string()),
        kind: Some(Inferred::new(kind, why)),
        parents: part_of,
        docs: Some(summary.to_string()),
        ..Symbol::at("", file, line)
    }
}

/// The facts being collected; each symbol gets its key here.
#[derive(Default)]
struct Out {
    facts: Facts,
}

impl Out {
    fn add(&mut self, mut s: Symbol, relations: Vec<Relation>) -> String {
        s.key = format!("infra:{}", self.facts.symbols.len());
        let key = s.key.clone();
        for mut r in relations {
            r.from = key.clone();
            self.facts.relations.push(r);
        }
        self.facts.symbols.push(s);
        key
    }

    /// The environment variables the symbol `key` sets.
    fn sets(&mut self, key: &str, env: Vec<(String, Evidence)>) {
        for (name, evidence) in env {
            self.facts.serves.push(Serve {
                by: key.to_string(),
                at: Meet::Env(name),
                evidence,
            });
        }
    }
}

impl Index {
    /***
     * What authored Terraform and Kubernetes blocks get from the code: what they use, create, route and select.
     *
     * The relations hang on symbols without a name of their own; the block's
     * code line says what it documents (the generic adapter reads it).
     ***/
    fn anchors(&self, file: &SourceFile, blocks: &[RawBlock], out: &mut Out) {
        let authored = |b: &&RawBlock, from: usize, to: usize| {
            b.origin == Origin::Authored
                && b.following
                    .as_ref()
                    .is_some_and(|f| from <= f.line && f.line <= to)
        };
        if file.lang.name == "terraform" {
            for r in self.resources.iter().filter(|r| r.file == file.rel) {
                if let Some(b) = blocks.iter().find(|b| authored(b, r.line, r.line)) {
                    out.add(
                        Symbol::at("", &file.rel, r.line),
                        self.resource_relations(r, Some(b)),
                    );
                }
            }
            for sub in self.subs.iter().filter(|s| s.file == file.rel) {
                let relations = self.sub_relations(sub);
                if !relations.is_empty() && blocks.iter().any(|b| authored(&b, sub.line, sub.line))
                {
                    out.add(Symbol::at("", &file.rel, sub.line), relations);
                }
            }
        } else {
            for o in self.objects.iter().filter(|o| o.file == file.rel) {
                let from = o.line.saturating_sub(2);
                if blocks.iter().any(|b| authored(&b, from, o.line)) {
                    let key = out.add(
                        Symbol {
                            attach_from: from,
                            ..Symbol::at("", &file.rel, o.line)
                        },
                        self.object_relations(o),
                    );
                    out.sets(&key, self.env_set_by(o));
                }
            }
        }
    }

    /***
     * Nodes from infrastructure code that has no block: Terraform resources and what they create, Kubernetes objects.
     ***/
    fn discover(&self, file: &SourceFile, out: &mut Out) {
        if file.lang.name == "terraform" {
            let dir = dir_of(&file.rel);
            let group = slug(dir.rsplit('/').next().unwrap_or("terraform"));
            let group = if group.is_empty() {
                "terraform".to_string()
            } else {
                group
            };
            let first_in_dir = !self
                .resources
                .iter()
                .any(|r| dir_of(&r.file) == dir && r.file < file.rel);
            if first_in_dir && self.resources.iter().any(|r| r.file == file.rel) {
                out.add(discovered(
                    &file.rel,
                    1,
                    &group,
                    "job",
                    &group,
                    &format!("The Terraform root module in {dir}: `terraform apply` there creates what its resources describe."),
                    vec![],
                    "terraform directory",
                ), vec![]);
            }
            for r in self.resources.iter().filter(|r| r.file == file.rel) {
                let authored = self.authored.contains_key(&r.address());
                if !authored {
                    let id = format!("tf.{}.{}", slug(&r.labels[0]), slug(&r.labels[1]));
                    let mut s = discovered(
                        &r.file,
                        r.line,
                        &id,
                        "config",
                        &r.address(),
                        &format!("Terraform {} `{}`.", r.kind, r.address()),
                        vec!["terraform".into(), group.clone()],
                        "terraform resource",
                    );
                    s.display = Some(format!("{} {}", r.kind, r.address()));
                    out.add(s, self.resource_relations(r, None));
                }
                for s in self.created(r) {
                    out.add(s, vec![]);
                }
            }
        }
        if file.lang.name == "yaml" {
            let chart = chart_of(&file.rel);
            let releases: Vec<String> = chart
                .and_then(|c| self.chart_releases.get(c).cloned())
                .unwrap_or_else(|| {
                    chart
                        .and_then(|c| self.charts.get(c))
                        .map(|n| vec![format!("k8s.release.{}", slug(n))])
                        .unwrap_or_default()
                });
            for o in self.objects.iter().filter(|o| o.file == file.rel) {
                if self.authored_objects.contains(&o.symbol()) {
                    continue;
                }
                let mut part_of = releases.clone();
                if let Some(ns) = &o.namespace
                    && !ns.contains("{{")
                {
                    part_of.push(format!("k8s.ns.{}", slug(ns)));
                }
                let mut s = discovered(
                    &o.file,
                    o.line,
                    &o.id(),
                    o.node_kind(),
                    &o.symbol(),
                    &format!("Kubernetes {} `{}`.", o.kind, o.name),
                    part_of,
                    &format!("Kubernetes kind `{}`", o.kind),
                );
                s.display = Some(o.symbol());
                let key = out.add(s, self.object_relations(o));
                out.sets(&key, self.env_set_by(o));
            }
            // A chart that no Terraform installs is its own release.
            if file.rel.ends_with("Chart.yaml")
                && let Some(name) = self.charts.get(dir_of(&file.rel))
                && !self.chart_releases.contains_key(dir_of(&file.rel))
            {
                out.add(
                    discovered(
                        &file.rel,
                        1,
                        &format!("k8s.release.{}", slug(name)),
                        "release",
                        name,
                        &format!("The Helm chart {name}, installed as a release."),
                        vec![self.system.clone()],
                        "Chart.yaml",
                    ),
                    vec![],
                );
            }
        }
    }
}

/***
 * The infrastructure adapter: Terraform, Helm charts and Kubernetes manifests, read as a whole because references cross files.
 ***/
pub struct InfraAdapter;

impl Adapter for InfraAdapter {
    fn name(&self) -> &'static str {
        "infra"
    }

    fn handles(&self, file: &SourceFile) -> bool {
        file.lang.name == "terraform" || file.lang.name == "yaml"
    }

    /***
     * Read every Terraform file, chart and manifest once, then report each file's relations and the nodes found without a block.
     *
     * @calls docair-lang-infra.index.new "reads every resource, chart and object"
     * @calls docair-lang-infra.index.anchors "completes the authored blocks"
     * @calls docair-lang-infra.index.discover "finds resources and objects that have no block"
     ***/
    fn project(&self, project: &Project) -> Facts {
        let index = Index::new(project.files, project.blocks, project.system);
        let mut out = Out::default();
        for (file, blocks) in project.files.iter().zip(project.blocks) {
            if self.handles(file) {
                index.anchors(file, blocks, &mut out);
                index.discover(file, &mut out);
            }
        }
        out.facts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terraform_blocks_attributes_and_references() {
        let tf = "locals {\n  chart_dir = \"${path.module}/../charts/shop\"\n}\n\nresource \"helm_release\" \"shop\" {\n  name      = \"shop\"\n  chart     = local.chart_dir\n  namespace = \"shop\"\n  set = [{ name = \"x\", value = terraform_data.images.id }]\n  depends_on = [helm_release.ingress]\n}\n";
        let blocks = tf_blocks("t/main.tf", tf);
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            (blocks[1].kind.as_str(), blocks[1].address()),
            ("resource", "helm_release.shop".to_string())
        );
        assert_eq!(blocks[1].attr("namespace").as_deref(), Some("shop"));
        assert_eq!(blocks[1].line, 5);
        assert_eq!(
            refs_in("  set = [{ value = terraform_data.images.id }]"),
            vec!["terraform_data.images.id"]
        );
        assert_eq!(
            refs_in("  host = \"${kind_cluster.dev.endpoint}\""),
            vec!["kind_cluster.dev.endpoint"]
        );
        assert!(refs_in("chart = \"${path.module}/x\"").is_empty());
    }

    /// The way in: host port → node → the release that takes the NodePort → the Ingresses of its class.
    #[test]
    fn the_way_into_the_cluster() {
        let cluster = "resource \"kind_cluster\" \"docair\" {\n  kind_config {\n    node {\n      role = \"control-plane\"\n      extra_port_mappings {\n        container_port = 30080\n        host_port      = 8080\n        protocol       = \"TCP\"\n      }\n    }\n  }\n}\n";
        let traefik = "resource \"helm_release\" \"traefik\" {\n  name  = \"traefik\"\n  chart = \"traefik\"\n  values = [yamlencode({\n    ports = {\n      web = { nodePort = 30080 }\n    }\n    ingressClass = { enabled = true, isDefaultClass = true }\n  })]\n}\n";
        let mut resources = tf_blocks("cluster.tf", cluster);
        resources.extend(tf_blocks("ingress.tf", traefik));
        let ingress = "apiVersion: networking.k8s.io/v1\nkind: Ingress\nmetadata:\n  name: docair\nspec:\n  ingressClassName: traefik\n";
        let index = Index {
            subs: resources.iter().flat_map(tf_subs).collect(),
            objects: k8s_objects("c/templates/ingress.yaml", ingress),
            resources,
            ..Index::default()
        };
        let lines = |kind: &str| -> Vec<String> {
            let sub = index.subs.iter().find(|s| s.kind == kind).unwrap();
            index
                .sub_relations(sub)
                .iter()
                .map(|r| {
                    let Target::Ref(to) = &r.to else {
                        unreachable!()
                    };
                    format!("{to} ({})", r.payload.clone().unwrap_or_default())
                })
                .collect()
        };
        assert_eq!(
            lines("extra_port_mappings"),
            ["tf:kind_cluster.docair (TCP 8080 → :30080)"]
        );
        assert_eq!(lines("node"), ["tf:helm_release.traefik (NodePort 30080)"]);
        assert_eq!(
            lines("values"),
            ["k8s:Ingress/docair (ingress class traefik)"]
        );
    }

    /// Ports by name, and two paths in one rule: each path keeps its Service.
    #[test]
    fn ingress_paths_with_named_ports() {
        let yaml = "apiVersion: networking.k8s.io/v1\nkind: Ingress\nmetadata:\n  name: docair\nspec:\n  ingressClassName: {{ .Values.ingress.className }}\n  rules:\n    - http:\n        paths:\n          - path: /api\n            pathType: Prefix\n            backend:\n              service:\n                name: docair-server\n                port:\n                  name: http\n          - path: /\n            pathType: Prefix\n            backend:\n              service:\n                name: docair-web\n                port:\n                  name: http\n";
        let o = &k8s_objects("c/templates/ingress.yaml", yaml)[0];
        let routes: Vec<(&str, &str)> = o
            .routes
            .iter()
            .map(|(_, p, s, _)| (p.as_str(), s.as_str()))
            .collect();
        assert_eq!(routes, [("/api", "docair-server"), ("/", "docair-web")]);
        let values = flat_values("ingress:\n  className: traefik\n  host: \"\"\n");
        assert_eq!(
            from_values(o.class.as_deref().unwrap(), Some(&values)).as_deref(),
            Some("traefik")
        );
    }

    #[test]
    fn kubernetes_objects_routes_and_selectors() {
        let yaml = "apiVersion: networking.k8s.io/v1\nkind: Ingress\nmetadata:\n  name: shop\nspec:\n  rules:\n    - host: shop.local\n      http:\n        paths:\n          - path: /api\n            backend:\n              service:\n                name: api\n                port:\n                  number: 80\n---\napiVersion: v1\nkind: Service\nmetadata:\n  name: api\nspec:\n  selector:\n    app: api\n  ports:\n    - port: 80\n      targetPort: 8080\n";
        let o = k8s_objects("c/templates/x.yaml", yaml);
        assert_eq!(o.len(), 2);
        assert_eq!(
            o[0].routes,
            vec![(
                "shop.local".into(),
                "/api".into(),
                "api".into(),
                Some("80".into())
            )]
        );
        assert_eq!(o[1].line, 17);
        assert!(o[1].selector.contains("app=api"));
        assert_eq!(o[1].target_port.as_deref(), Some("8080"));
        assert_eq!(o[1].id(), "k8s.svc.api");
    }

    /// What sets a variable: a ConfigMap through `envFrom`, a Secret key through `secretKeyRef`, a workload's own `value`; nothing for one Kubernetes fills from the pod (`fieldRef`).
    #[test]
    fn the_variables_objects_set() {
        let dep = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: server\nspec:\n  template:\n    spec:\n      containers:\n        - name: server\n          ports:\n            - name: http\n              containerPort: 8080\n          envFrom:\n            - configMapRef:\n                name: server\n          env:\n            - name: DOCAIR_POD_IP\n              valueFrom:\n                fieldRef:\n                  fieldPath: status.podIP\n          {{- if .Values.ai.apiKey }}\n            - name: DOCAIR_BIND\n              value: 0.0.0.0:8080\n            - name: DOCAIR_AI_API_KEY\n              valueFrom:\n                secretKeyRef:\n                  name: {{ .Values.ai.existingSecret | default \"ai\" }}\n                  key: api-key\n          {{- end }}\n";
        let cm = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: server\ndata:\n  DOCAIR_MAX_FILES: {{ .Values.maxFiles | quote }}\n  {{- with .Values.ai.model }}\n  DOCAIR_AI_MODEL: {{ . | quote }}\n  {{- end }}\n";
        let secret = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: ai\nstringData:\n  api-key: {{ .Values.ai.apiKey | quote }}\n";
        let mut objects = k8s_objects("c/templates/deploy.yaml", dep);
        objects.extend(k8s_objects("c/templates/cm.yaml", cm));
        objects.extend(k8s_objects("c/templates/secret.yaml", secret));
        assert_eq!(
            objects[0].env,
            [
                EnvSet::All {
                    kind: "ConfigMap".into(),
                    name: "server".into()
                },
                EnvSet::Value("DOCAIR_BIND".into()),
                EnvSet::Key {
                    var: "DOCAIR_AI_API_KEY".into(),
                    kind: "Secret".into(),
                    name: "ai".into()
                },
            ]
        );
        let index = Index {
            objects,
            ..Index::default()
        };
        let names = |i: usize| -> Vec<String> {
            index
                .env_set_by(&index.objects[i])
                .into_iter()
                .map(|(n, _)| n)
                .collect()
        };
        assert_eq!(names(0), ["DOCAIR_BIND"], "the workload's own value");
        assert_eq!(
            names(1),
            ["DOCAIR_MAX_FILES", "DOCAIR_AI_MODEL"],
            "every key, envFrom"
        );
        assert_eq!(
            names(2),
            ["DOCAIR_AI_API_KEY"],
            "one key, under the variable's name"
        );
    }

    #[test]
    fn helper_selectors_match_by_their_arguments() {
        let svc = "apiVersion: v1\nkind: Service\nmetadata:\n  name: docair-server\nspec:\n  selector:\n    {{- include \"docair.selector\" \"docair-server\" | nindent 4 }}\n  ports:\n    - port: 8080\n      targetPort: http\n";
        let dep = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: docair-server\nspec:\n  template:\n    metadata:\n      labels:\n        app.kubernetes.io/name: docair-server\n";
        let s = &k8s_objects("c/templates/s.yaml", svc)[0];
        let d = &k8s_objects("c/templates/d.yaml", dep)[0];
        assert!(s.selector.contains("docair-server"), "{:?}", s.selector);
        assert!(d.labels.contains("docair-server"), "{:?}", d.labels);
    }
}

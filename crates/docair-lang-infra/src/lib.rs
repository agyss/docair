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
use docair_core::facts::{Draw, Evidence, Facts, Inferred, Relation, Symbol, Target, Verb};
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
            (_, "kubernetes.io/ingress.class") => class = Some(value.clone()),
            // A templated host (`{{ . | quote }}`) is not known: the rule then holds for any host.
            (_, "host") if kind == "Ingress" => {
                host = if value.contains("{{") {
                    String::new()
                } else {
                    value.clone()
                }
            }
            (_, "path") if kind == "Ingress" => path = value.clone(),
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
        class: class.filter(|c| !c.contains("{{")),
    })
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
                            for l in &b.body {
                                if let Some((k, v)) = l.split_once('=') {
                                    a.locals.insert(
                                        k.trim().to_string(),
                                        v.trim().trim_matches('"').to_string(),
                                    );
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
                }
            }
        }
        tags
    }
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
    fn add(&mut self, mut s: Symbol, relations: Vec<Relation>) {
        s.key = format!("infra:{}", self.facts.symbols.len());
        for mut r in relations {
            r.from = s.key.clone();
            self.facts.relations.push(r);
        }
        self.facts.symbols.push(s);
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
        } else {
            for o in self.objects.iter().filter(|o| o.file == file.rel) {
                let from = o.line.saturating_sub(2);
                if blocks.iter().any(|b| authored(&b, from, o.line)) {
                    out.add(
                        Symbol {
                            attach_from: from,
                            ..Symbol::at("", &file.rel, o.line)
                        },
                        self.object_relations(o),
                    );
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
                out.add(s, self.object_relations(o));
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

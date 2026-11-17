terraform {
  required_providers {
    kind = { source = "tehcyx/kind" }
    helm = { source = "hashicorp/helm" }
  }
}

resource "kind_cluster" "dev" {
  name = "dev"
}

provider "helm" {
  kubernetes = {
    host                   = kind_cluster.dev.endpoint
    client_certificate     = kind_cluster.dev.client_certificate
    cluster_ca_certificate = kind_cluster.dev.cluster_ca_certificate
  }
}

locals {
  chart_dir = "${path.module}/../charts/shop"
}

resource "helm_release" "ingress" {
  name             = "traefik"
  repository       = "https://traefik.github.io/charts"
  chart            = "traefik"
  version          = "42.2.0"
  namespace        = "traefik"
  create_namespace = true
}

resource "helm_release" "shop" {
  name             = "shop"
  chart            = local.chart_dir
  namespace        = "shop"
  create_namespace = true
  depends_on       = [helm_release.ingress]
}

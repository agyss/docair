terraform {
  required_providers {
    helm = { source = "hashicorp/helm" }
  }
}

resource "helm_release" "app" {
  name      = "app"
  chart     = "${path.module}/../charts/app"
  namespace = "app"
  values    = [templatefile("${path.module}/values.yaml.tftpl", { replicas = 2 })]
}

module "net" {
  source = "./modules/net"
}

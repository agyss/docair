#***
# `terraform apply` in infra/terraform: builds the images and stands up the whole cluster.
#
# One root module, run on the workstation. It creates the kind cluster,
# builds the two images and loads them into the node, installs Traefik and
# then the docair chart. `terraform destroy` removes the cluster again.
#
# @id      terraform
# @kind    job
# @part-of workstation
#***
terraform {
  required_version = ">= 1.9"

  required_providers {
    kind = {
      source  = "tehcyx/kind"
      version = "0.13.0"
    }
    helm = {
      source  = "hashicorp/helm"
      version = "3.3.0"
    }
  }
}

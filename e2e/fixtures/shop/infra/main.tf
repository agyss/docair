#***
# The infrastructure: one cluster for the API and the database.
#
# @id      infra
# @kind    host
# @part-of shop
#***
terraform {
  required_version = ">= 1.9"
}

#***
# Terraform creates the cluster.
#
# @id      tf.cluster
# @kind    config
# @part-of infra
# @creates cluster (kind create)
#***
resource "kind_cluster" "shop" {
  #***
  # The Kubernetes cluster that runs the API and the database.
  #
  # @id      cluster
  # @kind    cluster
  # @part-of infra
  #***
  name = "shop"
}

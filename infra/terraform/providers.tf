# Both Helm releases talk to the API server of the new cluster.
provider "helm" {
  kubernetes = {
    host                   = kind_cluster.docair.endpoint
    client_certificate     = kind_cluster.docair.client_certificate
    client_key             = kind_cluster.docair.client_key
    cluster_ca_certificate = kind_cluster.docair.cluster_ca_certificate
  }
}

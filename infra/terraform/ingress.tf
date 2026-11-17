#***
# Install Traefik as the cluster's ingress controller (Helm chart traefik 42.2.0).
#
# ingress-nginx was retired in March 2026; Traefik is maintained and reads
# standard Ingress objects.
#
# @id      tf.traefik
# @kind    config
# @part-of terraform
# @in      credentials: kubeconfig from tf.kind_cluster
# @creates k8s.release.traefik (helm install: chart traefik 42.2.0)
# @flow    provision 5 "Terraform installs Traefik"
#***
resource "helm_release" "traefik" {
  #***
  # Helm release traefik: the chart traefik 42.2.0 from traefik.github.io,
  # installed into the namespace traefik with the values below.
  #
  # @id      k8s.release.traefik
  # @kind    release
  # @part-of k8s.ns.traefik
  # @flow    provision 6 "Traefik's release is up: the controller listens on NodePort 30080"
  #***
  name       = "traefik"
  repository = "https://traefik.github.io/charts"
  chart      = "traefik"
  version    = "42.2.0"

  #***
  # The namespace `traefik`: everything that belongs to the ingress controller.
  #
  # @id      k8s.ns.traefik
  # @kind    namespace
  # @part-of kind
  #***
  namespace        = "traefik"
  create_namespace = true

  #***
  # Traefik, the ingress controller: takes every HTTP request that enters the
  # cluster and routes it by the Ingress rules.
  #
  # Its Service is a NodePort on 30080, so the port that kind publishes on
  # the node leads straight to it. It is the default IngressClass.
  #
  # @id      k8s.traefik
  # @kind    gateway
  # @part-of k8s.release.traefik
  # @calls   k8s.ingress.docair (match host and path)
  # @flow    open 4 "kube-proxy hands the connection to Traefik"
  # @flow    upload 6 "Traefik takes the request"
  #***
  values = [yamlencode({
    service = { spec = { type = "NodePort" } }
    ports = {
      web       = { nodePort = 30080 }
      websecure = { expose = { default = false } }
    }
    ingressClass = { enabled = true, isDefaultClass = true }
    providers    = { kubernetesIngress = { enabled = true } }
  })]
}

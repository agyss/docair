locals {
  chart_dir = "${local.repo}/deploy/helm/docair"
  # Helm does not notice changed templates of a local chart; this hash does.
  chart_hash = sha1(join("", [for f in sort(fileset(local.chart_dir, "**")) : filesha1("${local.chart_dir}/${f}")]))

  dotenv_path = "${local.repo}/.env"
  ai_api_key = sensitive(var.ai_api_key != null ? var.ai_api_key : try(
    regex("(?m)^\\s*(?:export\\s+)?DOCAIR_AI_API_KEY\\s*=\\s*[\"']?([^\"'\\s#]+)", file(local.dotenv_path))[0],
    "",
  ))
}

#***
# Install the docair chart: the two apps, their Services, the Ingress and the network policy.
#
# Runs after the images are in the node and Traefik is up. The image hash and
# a hash of the chart's files are passed as pod annotations, so rebuilt images
# and changed templates roll out new pods. The AI settings go into the
# ConfigMap, the key (from .env, never from a committed file) into the Secret
# docair-ai.
#
# @id      tf.docair
# @kind    config
# @part-of terraform
# @in      ingress class: traefik from tf.traefik
# @in      images: in containerd from tf.load_images
# @reads   dotenv -> DOCAIR_AI_API_KEY
# @creates k8s.release.docair (helm install: chart deploy/helm/docair, image tag dev, AI endpoint and key)
# @flow    provision 7 "Terraform installs the docair chart"
#***
resource "helm_release" "docair" {
  name  = "docair"
  chart = local.chart_dir

  #***
  # The namespace `docair`: holds the Helm release docair. Terraform creates
  # it together with the release (create_namespace).
  #
  # @id      k8s.ns.docair
  # @kind    namespace
  # @part-of kind
  #***
  namespace        = "docair"
  create_namespace = true

  values = [yamlencode({
    server = { image = { tag = var.image_tag }, podAnnotations = { "docair/images" = terraform_data.images.id, "docair/chart" = local.chart_hash } }
    web    = { image = { tag = var.image_tag }, podAnnotations = { "docair/images" = terraform_data.images.id, "docair/chart" = local.chart_hash } }
    ai     = { endpoint = var.ai_endpoint, model = var.ai_model }
  })]

  set_sensitive = [{ name = "ai.apiKey", value = local.ai_api_key, type = "string" }]

  depends_on = [terraform_data.load_images, helm_release.traefik]
}

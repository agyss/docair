locals {
  repo         = abspath("${path.module}/../..")
  server_image = "docair-server:${var.image_tag}"
  web_image    = "docair-web:${var.image_tag}"

  # Rebuild when the sources of an image change.
  server_files = concat(
    ["Cargo.toml", "Cargo.lock", "docker/server.Dockerfile"],
    tolist(fileset(local.repo, "crates/**")),
  )
  web_files = concat(
    ["docker/web.Dockerfile", "docker/nginx.conf", "web/package.json", "web/package-lock.json", "web/angular.json"],
    tolist(fileset(local.repo, "web/tsconfig*.json")),
    tolist(fileset(local.repo, "web/src/**")),
    tolist(fileset(local.repo, "web/public/**")),
  )
  server_sources = sha1(join("", [for f in sort(local.server_files) : filesha1("${local.repo}/${f}")]))
  web_sources    = sha1(join("", [for f in sort(local.web_files) : filesha1("${local.repo}/${f}")]))
}

#***
# Build the two images with Docker: docair-server (Rust) and docair-web (Angular + nginx).
#
# @id      tf.images
# @kind    job
# @part-of terraform
# @writes  images (docair-server:dev, docair-web:dev)
# @flow    provision 2 "Terraform creates the kind cluster and builds the images"
#***
resource "terraform_data" "images" {
  triggers_replace = [local.server_sources, local.web_sources, var.image_tag]

  provisioner "local-exec" {
    working_dir = local.repo
    command     = "docker build -f docker/server.Dockerfile -t ${local.server_image} . && docker build -f docker/web.Dockerfile -t ${local.web_image} ."
  }
}

#***
# Copy the images into the node's containerd, so pods start without a registry.
#
# This is what `kind load docker-image` does: `docker save` streams the
# images into `ctr images import` inside the node container.
#
# @id      tf.load_images
# @kind    job
# @part-of terraform
# @in      node: docair-control-plane from tf.kind_cluster
# @reads   images -> image archives
# @writes  kind-node (images in containerd)
# @flow    provision 4 "the images are imported into the node"
#***
resource "terraform_data" "load_images" {
  triggers_replace = [terraform_data.images.id, kind_cluster.docair.id]

  provisioner "local-exec" {
    command = "docker save ${local.server_image} ${local.web_image} | docker exec -i ${var.cluster_name}-control-plane ctr --namespace=k8s.io images import --all-platforms --digests --snapshotter=overlayfs -"
  }
}

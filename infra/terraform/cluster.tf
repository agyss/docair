#***
# Create the kind cluster: one Kubernetes node that runs as a Docker container.
#
# kind starts the container `docair-control-plane` from the kindest/node image,
# boots kubeadm inside it and writes a kubeconfig. Terraform hands that
# kubeconfig to the Helm provider for the two releases.
#
# @id      tf.kind_cluster
# @kind    config
# @part-of terraform
# @creates kind (docker run kindest/node: the node container docair-control-plane) -> kubeconfig
# @flow    provision 2 "Terraform creates the kind cluster and builds the images"
#***
resource "kind_cluster" "docair" {
  name           = var.cluster_name
  node_image     = var.node_image == "" ? null : var.node_image
  wait_for_ready = true

  #***
  # The Kubernetes cluster `docair`: API server, scheduler, kubelet and
  # containerd, all inside the node container.
  #
  # @id      kind
  # @kind    cluster
  # @part-of kind-net
  # @flow    provision 3 "the cluster's node container starts, the images land in Docker"
  #***
  kind_config {
    kind        = "Cluster"
    api_version = "kind.x-k8s.io/v1alpha4"

    networking {
      #***
      # The pod network 10.244.0.0/16: kindnet gives every pod its own IP from this range.
      #
      # @id      pod-network
      # @kind    network
      # @part-of kind
      #***
      pod_subnet = "10.244.0.0/16"

      #***
      # The service network 10.96.0.0/12: every Service gets a stable
      # ClusterIP from this range, and kube-proxy forwards it to the pods behind it.
      #
      # @id      service-network
      # @kind    network
      # @part-of kind
      #***
      service_subnet = "10.96.0.0/12"
    }

    #***
    # The node `docair-control-plane`: a Docker container on the network
    # `kind`, running the whole control plane and all pods.
    #
    # It carries the label ingress-ready=true. Port 30080 inside it is the
    # NodePort of Traefik.
    #
    # @id      kind-node
    # @kind    host
    # @part-of kind
    # @calls   k8s.traefik (NodePort 30080 → :8000)
    # @flow    open 3 "the node receives the request on port 30080"
    # @flow    upload 5 "the node receives it on port 30080"
    #***
    node {
      role   = "control-plane"
      labels = { "ingress-ready" = "true" }

      #***
      # 127.0.0.1:8080 on the workstation: Docker publishes the node's port 30080 here.
      #
      # This is the only way into the cluster. In the devcontainer, VS Code
      # forwards this port to the browser on your machine.
      #
      # @id      host-port
      # @kind    gateway
      # @part-of docker
      # @calls   kind-node (TCP to :30080 over the network kind)
      # @flow    open 2 "Docker forwards 127.0.0.1:8080 into the node"
      # @flow    upload 4 "the request reaches 127.0.0.1:8080"
      #***
      extra_port_mappings {
        container_port = 30080
        host_port      = var.host_port
        listen_address = "127.0.0.1"
        protocol       = "TCP"
      }
    }
  }
}

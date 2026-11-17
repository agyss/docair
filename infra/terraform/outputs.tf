output "url" {
  description = "Open this in a browser (in the devcontainer, VS Code forwards the port)."
  value       = "http://localhost:${var.host_port}"
}

output "kubeconfig" {
  description = "kubectl access: terraform output -raw kubeconfig > ~/.kube/config"
  value       = kind_cluster.docair.kubeconfig
  sensitive   = true
}

variable "cluster_name" {
  description = "Name of the kind cluster. The node container is called <name>-control-plane."
  type        = string
  default     = "docair"
}

variable "host_port" {
  description = "Port on 127.0.0.1 where the app is reachable (published port of the kind node)."
  type        = number
  default     = 8080
}

variable "node_image" {
  description = "kindest/node image. Empty means the default of the kind provider."
  type        = string
  default     = ""
}

variable "image_tag" {
  description = "Tag of the locally built docair-server and docair-web images."
  type        = string
  default     = "dev"
}

variable "ai_endpoint" {
  description = "Base URL of an OpenAI-compatible API for the web app's Ask box (docs/SPEC.md §10). Empty turns asking off."
  type        = string
  default     = "https://api.together.xyz/v1"
}

variable "ai_model" {
  description = "Model id as the endpoint names it."
  type        = string
  default     = "zai-org/GLM-5.3-Flash"
}

variable "ai_api_key" {
  description = "Key for the AI endpoint. Null: read DOCAIR_AI_API_KEY from the repository's gitignored .env."
  type        = string
  default     = null
  sensitive   = true
}

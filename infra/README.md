# infra: a kind cluster for docair, provisioned with Terraform

```
browser ──► 127.0.0.1:8080 ──► docair-control-plane:30080 ──► Traefik ──► Ingress
            (Docker publishes      (kind node container,          │  /      → Service docair-web    → nginx + Angular app
             the node's port)       network "kind")               └─ /api   → Service docair-server → docair-server (Rust)
```

`terraform -chdir=infra/terraform apply` does everything:

1. creates the kind cluster `docair` (one node, a Docker container),
2. builds the images `docair-server:dev` and `docair-web:dev`,
3. imports them into the node's containerd (no registry needed),
4. installs Traefik (Helm) as the ingress controller on NodePort 30080,
5. installs the chart `deploy/helm/docair`.

Then open <http://localhost:8080>. In the devcontainer, Docker runs inside the
container (docker-in-docker) and VS Code forwards port 8080 to your browser.
`infra/smoke-test.sh` checks the result: the frontend through the Ingress, an
upload through `/api/scan`, the path check, and that every pod is Ready.
`terraform -chdir=infra/terraform destroy` removes the cluster.

**Asking (AI).** The web app's Ask box asks a model how something works and
replays the answer on the page (docs/SPEC.md §10). Terraform configures it:
`ai_endpoint` and `ai_model` (default: Together, `zai-org/GLM-5.3-Flash`) go
into the ConfigMap, and the key goes into the Secret `docair-ai`. The key is read
from `DOCAIR_AI_API_KEY` in the repository's `.env` (gitignored, see
`.env.example`) unless you pass `-var ai_api_key=…`. Without a key the Ask box
still shows, but the endpoint refuses; set `-var ai_endpoint=` to turn it off.

## How a request gets in

| Hop | What it is | Where it is defined |
|-----|------------|---------------------|
| 127.0.0.1:8080 | Docker publishes the node container's port 30080 | `extra_port_mappings` in `terraform/cluster.tf` |
| node :30080 | NodePort of the Traefik Service; kube-proxy forwards it to the Traefik pod | `ports.web.nodePort` in `terraform/ingress.tf` |
| Traefik | ingress controller: matches the request against Ingress rules | `terraform/ingress.tf` |
| Ingress `docair` | `/api` → Service docair-server, `/` → Service docair-web | `deploy/helm/docair/templates/ingress.yaml` |
| Service | ClusterIP from 10.96.0.0/12; kube-proxy picks a pod IP from 10.244.0.0/16 | `templates/*-service.yaml`, `networking` in `cluster.tf` |
| pod :8080 | nginx (web) or docair-server; a NetworkPolicy admits only Traefik | `templates/networkpolicy.yaml` |

The parts that have no code of their own are declared here:

```docair
The machine that runs Docker: your laptop, or this devcontainer, which runs
Docker inside itself (docker-in-docker). Terraform, the docair CLI and the
browser's port forwarding all start here.

@id      workstation
@kind    host
@part-of docair
```

```docair
Docker Engine: builds the two images, runs the kind node as a container and
publishes the node's port 30080 on 127.0.0.1:8080.

@id      docker
@kind    host
@part-of workstation
```

```docair
Docker's local image store: docair-server:dev and docair-web:dev, as built by Terraform.

@id      images
@kind    store
@part-of docker
```

```docair
The Docker network `kind`, a bridge network: the kind node container is attached to it, and published ports reach the node through it.

@id      kind-net
@kind    network
@part-of docker
```

```docair
CoreDNS, the cluster DNS: names like docair-server.docair.svc.cluster.local resolve to Service IPs.

@id      coredns
@kind    endpoint
@part-of kind
```

```docair
You, at the terminal: you run `terraform apply` once and get a running cluster.

@id      developer
@kind    external
@part-of docair
@calls   tf.kind_cluster (terraform apply)
@calls   tf.images (terraform apply)
@flow    provision 1 "you run terraform apply"
```

# The ecosystem's second site: a Kubernetes cluster

Spec: `specs/saas/v4-ecosystem.md`, "One isolated node per business". The cluster is a Vultr
Kubernetes Engine cluster (`agentrix-saas-eco`, Atlanta). It runs beside `saas-node`, which keeps
its own deploy under `deploy/saas/`; nothing here touches that server.

- `edge/`: Caddy as the cluster's edge. One load balancer in front of it; Caddy routes each
  business's host to its Service and obtains the certificates itself.
- One folder per business will follow, each a StatefulSet with its own volume, secret and network
  policy.

The cluster's access file, ids and every secret stay outside this repo, in
`~/.config/agentrix-saas/k8s/` on the operator's machine.

```bash
export KUBECONFIG=~/.config/agentrix-saas/k8s/kubeconfig.yaml
kubectl apply -f deploy/k8s/edge/     # edge.yaml first, then routes.yaml
```

ingress-nginx is not used: its project archived the repository in March 2026.

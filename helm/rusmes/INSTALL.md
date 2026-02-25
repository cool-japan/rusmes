# RusMES Helm Chart Installation Guide

This guide provides step-by-step instructions for installing RusMES using Helm.

## Quick Install

### Development Environment

For quick testing in a development environment:

```bash
helm install rusmes ./helm/rusmes \
  --namespace rusmes-dev \
  --create-namespace \
  --values ./helm/rusmes/values-development.yaml
```

### Production Environment

For production deployment:

```bash
helm install rusmes ./helm/rusmes \
  --namespace rusmes \
  --create-namespace \
  --values ./helm/rusmes/values-production.yaml \
  --set postgresql.auth.password="YOUR_STRONG_PASSWORD" \
  --set config.hostname="mail.example.com"
```

## Prerequisites

Before installing, ensure you have:

1. **Kubernetes Cluster** (1.20+)
   - Minimum: 3 worker nodes with 4 CPU, 16GB RAM each
   - Recommended: 5+ worker nodes with 8 CPU, 32GB RAM each

2. **Helm** (3.0+)
   ```bash
   # Install Helm
   curl https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 | bash
   ```

3. **kubectl** configured to access your cluster
   ```bash
   kubectl cluster-info
   ```

4. **StorageClass** configured
   ```bash
   kubectl get storageclass
   ```

5. **cert-manager** (optional, for automatic TLS)
   ```bash
   kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.13.0/cert-manager.yaml
   ```

6. **Ingress Controller** (optional, for web access)
   ```bash
   kubectl apply -f https://raw.githubusercontent.com/kubernetes/ingress-nginx/main/deploy/static/provider/cloud/deploy.yaml
   ```

## Installation Steps

### Step 1: Prepare Configuration

Create a custom values file:

```bash
cat > my-values.yaml <<EOF
config:
  hostname: mail.example.com

postgresql:
  auth:
    password: "$(openssl rand -base64 32)"

backup:
  s3:
    enabled: true
    bucket: "my-backups"
    region: "us-west-2"
    accessKeyId: "YOUR_KEY"
    secretAccessKey: "YOUR_SECRET"

ingress:
  hosts:
    - host: mail.example.com
      paths:
        - path: /
          pathType: Prefix
  tls:
    - secretName: rusmes-tls
      hosts:
        - mail.example.com
EOF
```

### Step 2: Create Namespace

```bash
kubectl create namespace rusmes
```

### Step 3: Set up TLS (if using cert-manager)

Create ClusterIssuer:

```bash
cat <<EOF | kubectl apply -f -
apiVersion: cert-manager.io/v1
kind: ClusterIssuer
metadata:
  name: letsencrypt-prod
spec:
  acme:
    server: https://acme-v02.api.letsencrypt.org/directory
    email: admin@example.com
    privateKeySecretRef:
      name: letsencrypt-prod
    solvers:
    - http01:
        ingress:
          class: nginx
EOF
```

### Step 4: Install Chart

```bash
helm install rusmes ./helm/rusmes \
  --namespace rusmes \
  --values ./helm/rusmes/values-production.yaml \
  --values my-values.yaml \
  --timeout 10m
```

### Step 5: Wait for Deployment

```bash
# Watch pod status
kubectl get pods -n rusmes -w

# Wait for all pods to be ready
kubectl wait --for=condition=ready pod \
  -l app.kubernetes.io/name=rusmes \
  -n rusmes \
  --timeout=600s
```

### Step 6: Initialize Database

```bash
# Run migrations
kubectl exec -it rusmes-0 -n rusmes -- rusmes-cli migrate

# Create admin user
kubectl exec -it rusmes-0 -n rusmes -- rusmes-cli user create \
  admin@mail.example.com \
  --admin \
  --password "admin-password"
```

### Step 7: Configure DNS

Get the LoadBalancer IP:

```bash
kubectl get svc rusmes -n rusmes \
  -o jsonpath='{.status.loadBalancer.ingress[0].ip}'
```

Add DNS records:

```
# MX Record
@  IN MX 10 mail.example.com.

# A Record
mail  IN A  <LOADBALANCER_IP>

# DKIM (generate first)
default._domainkey  IN TXT "v=DKIM1; k=rsa; p=<PUBLIC_KEY>"

# SPF
@  IN TXT "v=spf1 mx ~all"

# DMARC
_dmarc  IN TXT "v=DMARC1; p=quarantine; rua=mailto:postmaster@example.com"
```

Generate DKIM key:

```bash
kubectl exec -it rusmes-0 -n rusmes -- rusmes-cli dkim generate example.com
```

### Step 8: Verify Installation

```bash
# Check deployment status
helm status rusmes -n rusmes

# Check pods
kubectl get pods -n rusmes

# Check services
kubectl get svc -n rusmes

# View logs
kubectl logs -f rusmes-0 -n rusmes

# Test SMTP
kubectl run -it --rm test-smtp --image=busybox -n rusmes -- \
  telnet rusmes 25
```

### Step 9: Access Services

#### JMAP Web Interface

```bash
# If using ingress
echo "https://$(kubectl get ingress -n rusmes rusmes-jmap -o jsonpath='{.spec.rules[0].host}')"

# If using port-forward
kubectl port-forward svc/rusmes-jmap 8080:8080 -n rusmes
# Then visit http://localhost:8080
```

#### Prometheus Metrics

```bash
kubectl port-forward svc/rusmes-metrics 9090:9090 -n rusmes
# Then visit http://localhost:9090/metrics
```

## Configuration Examples

### Minimal Configuration

```yaml
replicaCount: 1
persistence:
  size: 20Gi
postgresql:
  auth:
    password: "simple-password"
```

### Production Configuration

```yaml
replicaCount: 5
autoscaling:
  enabled: true
  minReplicas: 5
  maxReplicas: 20
persistence:
  size: 500Gi
  storageClass: "fast-ssd"
postgresql:
  architecture: replication
  primary:
    persistence:
      size: 1Ti
backup:
  enabled: true
  s3:
    enabled: true
```

### High Availability Configuration

```yaml
replicaCount: 5
podDisruptionBudget:
  enabled: true
  minAvailable: 3
affinity:
  podAntiAffinity:
    requiredDuringSchedulingIgnoredDuringExecution:
      - topologyKey: kubernetes.io/hostname
      - topologyKey: topology.kubernetes.io/zone
postgresql:
  architecture: replication
  replication:
    enabled: true
    numSynchronousReplicas: 1
  readReplicas:
    replicaCount: 2
```

## Upgrade

### Minor Version Upgrade

```bash
helm upgrade rusmes ./helm/rusmes \
  --namespace rusmes \
  --values my-values.yaml \
  --reuse-values
```

### Major Version Upgrade

```bash
# 1. Backup first
kubectl exec rusmes-0 -n rusmes -- rusmes-cli backup create

# 2. Upgrade
helm upgrade rusmes ./helm/rusmes \
  --namespace rusmes \
  --values my-values.yaml \
  --reuse-values

# 3. Run migrations
kubectl exec rusmes-0 -n rusmes -- rusmes-cli migrate
```

## Uninstall

```bash
# Standard uninstall (keeps PVCs)
helm uninstall rusmes -n rusmes

# Complete cleanup (deletes all data)
helm uninstall rusmes -n rusmes
kubectl delete pvc -l app.kubernetes.io/instance=rusmes -n rusmes
kubectl delete namespace rusmes
```

## Troubleshooting

### Pods Not Starting

```bash
kubectl describe pod rusmes-0 -n rusmes
kubectl logs rusmes-0 -n rusmes
```

### Database Connection Issues

```bash
kubectl logs rusmes-postgresql-0 -n rusmes
kubectl exec -it rusmes-0 -n rusmes -- env | grep POSTGRES
```

### TLS Certificate Issues

```bash
kubectl describe certificate -n rusmes
kubectl logs -n cert-manager deploy/cert-manager
```

### View All Resources

```bash
kubectl get all -n rusmes -l app.kubernetes.io/instance=rusmes
```

## Additional Resources

- [README.md](README.md) - Full documentation
- [values.yaml](values.yaml) - Default configuration
- [values-production.yaml](values-production.yaml) - Production example
- [values-development.yaml](values-development.yaml) - Development example
- [examples/](examples/) - More configuration examples

## Support

For issues and questions:
- GitHub Issues: https://github.com/example/rusmes/issues
- Documentation: https://rusmes.example.com/docs

# RusMES Helm Chart

Enterprise Mail Server in Rust - Production-ready Helm chart for Kubernetes deployment.

## Overview

This Helm chart deploys RusMES (Rust Mail Enterprise Server) on a Kubernetes cluster. It includes:

- Multi-protocol mail server (SMTP, IMAP, POP3, JMAP)
- PostgreSQL database (with optional high availability)
- Automatic TLS certificate management
- Horizontal Pod Autoscaling
- Prometheus metrics and Grafana dashboards
- Automated backups to S3 or local storage
- Network policies and RBAC for security
- Pod disruption budgets for high availability

## Prerequisites

- Kubernetes 1.20+
- Helm 3.0+
- PV provisioner support in the underlying infrastructure
- cert-manager (optional, for automatic TLS)
- Prometheus Operator (optional, for ServiceMonitor)
- Ingress controller (nginx recommended)

## Quick Start

### Development Installation

For development/testing with minimal resources:

```bash
helm install rusmes ./helm/rusmes \
  --namespace rusmes-dev \
  --create-namespace \
  --values ./helm/rusmes/values-development.yaml
```

### Production Installation

For production deployment with high availability:

```bash
# 1. Create namespace
kubectl create namespace rusmes

# 2. Install cert-manager for automatic TLS (if not already installed)
kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.13.0/cert-manager.yaml

# 3. Create cluster issuer for Let's Encrypt
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

# 4. Create values file with your configuration
cat > my-values.yaml <<EOF
config:
  hostname: mail.example.com

postgresql:
  auth:
    password: "CHANGE_ME_TO_STRONG_PASSWORD"

backup:
  s3:
    enabled: true
    bucket: "my-company-rusmes-backups"
    region: "us-west-2"
    accessKeyId: "YOUR_ACCESS_KEY"
    secretAccessKey: "YOUR_SECRET_KEY"

ingress:
  hosts:
    - host: mail.example.com
      paths:
        - path: /
          pathType: Prefix
  tls:
    - secretName: rusmes-jmap-tls
      hosts:
        - mail.example.com
EOF

# 5. Install the chart
helm install rusmes ./helm/rusmes \
  --namespace rusmes \
  --values ./helm/rusmes/values-production.yaml \
  --values my-values.yaml

# 6. Wait for deployment
kubectl wait --for=condition=ready pod -l app.kubernetes.io/name=rusmes -n rusmes --timeout=300s

# 7. Initialize database and create admin user
kubectl exec -it rusmes-0 -n rusmes -- rusmes-cli migrate
kubectl exec -it rusmes-0 -n rusmes -- rusmes-cli user create admin@mail.example.com --admin
```

### Installing from Helm Repository

```bash
# Add the repository (if published)
helm repo add rusmes https://charts.rusmes.example.com
helm repo update

# Install the chart
helm install rusmes rusmes/rusmes \
  --namespace rusmes \
  --create-namespace \
  --values my-values.yaml
```

## Configuration

### Key Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| **Deployment** | | |
| `replicaCount` | Number of replicas | `3` |
| `image.repository` | Image repository | `rusmes/rusmes` |
| `image.tag` | Image tag | `latest` |
| `image.pullPolicy` | Image pull policy | `IfNotPresent` |
| **Application** | | |
| `config.hostname` | Server hostname | `mail.example.com` |
| `config.maxMessageSize` | Max message size in bytes | `52428800` (50MB) |
| `config.maxRecipients` | Max recipients per message | `100` |
| `config.maxConnections` | Max concurrent connections | `1000` |
| **Protocols** | | |
| `config.smtp.enabled` | Enable SMTP server | `true` |
| `config.imap.enabled` | Enable IMAP server | `true` |
| `config.pop3.enabled` | Enable POP3 server | `true` |
| `config.jmap.enabled` | Enable JMAP server | `true` |
| **Security** | | |
| `config.security.enableDkim` | Enable DKIM signing | `true` |
| `config.security.enableSpf` | Enable SPF checking | `true` |
| `config.security.enableDmarc` | Enable DMARC | `true` |
| `config.security.antispam.enabled` | Enable antispam | `true` |
| **Database** | | |
| `postgresql.enabled` | Enable PostgreSQL subchart | `true` |
| `postgresql.auth.password` | PostgreSQL password | `changeme` |
| `postgresql.primary.persistence.size` | PostgreSQL storage size | `100Gi` |
| `externalPostgresql.host` | External PostgreSQL host | `postgres-service` |
| **Storage** | | |
| `persistence.enabled` | Enable persistent storage | `true` |
| `persistence.size` | Mail storage size | `50Gi` |
| `persistence.storageClass` | Storage class | `fast-ssd` |
| `backupPersistence.enabled` | Enable backup storage | `true` |
| `backupPersistence.size` | Backup storage size | `500Gi` |
| **Networking** | | |
| `service.type` | Service type | `LoadBalancer` |
| `ingress.enabled` | Enable ingress | `true` |
| `ingress.className` | Ingress class | `nginx` |
| **Scaling** | | |
| `autoscaling.enabled` | Enable HPA | `true` |
| `autoscaling.minReplicas` | Minimum replicas | `3` |
| `autoscaling.maxReplicas` | Maximum replicas | `10` |
| `autoscaling.targetCPUUtilizationPercentage` | Target CPU % | `70` |
| **TLS** | | |
| `tls.enabled` | Enable TLS | `true` |
| `tls.certManager.enabled` | Use cert-manager | `true` |
| `tls.certManager.issuer` | Cert-manager issuer | `letsencrypt-prod` |
| **Monitoring** | | |
| `config.metrics.enabled` | Enable metrics | `true` |
| `prometheus.enabled` | Enable Prometheus | `false` |
| `grafana.enabled` | Enable Grafana | `false` |
| **Backup** | | |
| `backup.enabled` | Enable backups | `true` |
| `backup.schedule` | Backup schedule (cron) | `0 2 * * *` |
| `backup.retention` | Days to retain backups | `30` |
| `backup.s3.enabled` | Use S3 for backups | `false` |
| **Security Policies** | | |
| `networkPolicy.enabled` | Enable network policies | `true` |
| `podDisruptionBudget.enabled` | Enable PDB | `true` |
| `rbac.create` | Create RBAC resources | `true` |
| **Resources** | | |
| `resources.requests.memory` | Memory request | `512Mi` |
| `resources.requests.cpu` | CPU request | `500m` |
| `resources.limits.memory` | Memory limit | `2Gi` |
| `resources.limits.cpu` | CPU limit | `2000m` |

### Example values.yaml

```yaml
replicaCount: 5

config:
  hostname: mail.mycompany.com
  maxMessageSize: 104857600  # 100MB

postgresql:
  enabled: true
  auth:
    password: "strong-password-here"
  primary:
    persistence:
      size: 200Gi

persistence:
  size: 100Gi
  storageClass: fast-ssd

ingress:
  enabled: true
  hosts:
    - host: jmap.mycompany.com
      paths:
        - path: /
          pathType: Prefix

autoscaling:
  enabled: true
  minReplicas: 3
  maxReplicas: 10
  targetCPUUtilizationPercentage: 70

backup:
  enabled: true
  schedule: "0 2 * * *"
  s3:
    enabled: true
    bucket: mycompany-rusmes-backups
    region: us-west-2
```

## Post-Installation Steps

### 1. Configure DNS Records

Point your domain's MX record to the LoadBalancer IP:

```bash
# Get the LoadBalancer IP
kubectl get svc rusmes -n rusmes -o jsonpath='{.status.loadBalancer.ingress[0].ip}'
```

Create the following DNS records:

```
# MX Record
@  IN MX 10 mail.example.com.

# A Record
mail  IN A  <LOADBALANCER_IP>

# SPF Record (if enabled)
@  IN TXT "v=spf1 mx ~all"

# DKIM Record (generate with rusmes-cli)
default._domainkey  IN TXT "v=DKIM1; k=rsa; p=<PUBLIC_KEY>"

# DMARC Record (if enabled)
_dmarc  IN TXT "v=DMARC1; p=quarantine; rua=mailto:postmaster@example.com"
```

### 2. Generate DKIM Keys

```bash
kubectl exec -it rusmes-0 -n rusmes -- rusmes-cli dkim generate example.com
```

### 3. Create User Accounts

```bash
# Create admin user
kubectl exec -it rusmes-0 -n rusmes -- rusmes-cli user create admin@example.com --admin

# Create regular user
kubectl exec -it rusmes-0 -n rusmes -- rusmes-cli user create user@example.com
```

### 4. Test Mail Flow

```bash
# Send test email via SMTP
kubectl run -it --rm test-smtp --image=busybox --restart=Never -n rusmes -- \
  telnet rusmes 25

# Check IMAP access
kubectl run -it --rm test-imap --image=busybox --restart=Never -n rusmes -- \
  telnet rusmes 143
```

## Upgrading

### Minor Version Upgrade

```bash
helm upgrade rusmes ./helm/rusmes \
  --namespace rusmes \
  --values my-values.yaml
```

### Major Version Upgrade

1. Backup your data first:

```bash
kubectl exec rusmes-0 -n rusmes -- rusmes-cli backup create
```

2. Upgrade the chart:

```bash
helm upgrade rusmes ./helm/rusmes \
  --namespace rusmes \
  --values my-values.yaml \
  --set postgresql.auth.password="$POSTGRES_PASSWORD"
```

3. Run database migrations:

```bash
kubectl exec rusmes-0 -n rusmes -- rusmes-cli migrate
```

### Rollback

If upgrade fails, rollback to previous version:

```bash
helm rollback rusmes -n rusmes
```

## Uninstalling

### Standard Uninstall

```bash
helm uninstall rusmes --namespace rusmes
```

### Complete Cleanup (including PVCs)

WARNING: This will delete all mail data!

```bash
# Uninstall chart
helm uninstall rusmes --namespace rusmes

# Delete PVCs
kubectl delete pvc -l app.kubernetes.io/instance=rusmes -n rusmes

# Delete namespace
kubectl delete namespace rusmes
```

## Features

- **High Availability**: Multi-replica StatefulSet with pod anti-affinity
- **Persistent Storage**: Automatic PVC management
- **Auto-scaling**: Horizontal Pod Autoscaler based on CPU/memory
- **TLS Support**: Automatic certificate management with cert-manager
- **Monitoring**: Prometheus metrics and Grafana dashboards
- **Backups**: Scheduled backups to S3 or local storage
- **Network Policies**: Secure network isolation
- **RBAC**: Role-based access control
- **Pod Disruption Budget**: Ensure availability during updates

## Monitoring

The chart includes:
- ServiceMonitor for Prometheus (if prometheus-operator is installed)
- Pre-configured Grafana dashboard
- Health check endpoints

Access metrics:

```bash
kubectl port-forward svc/my-rusmes-metrics 9090:9090 -n rusmes
```

## Backups

Enable automated backups:

```yaml
backup:
  enabled: true
  schedule: "0 2 * * *"  # Daily at 2 AM
  retention: 30
  s3:
    enabled: true
    bucket: rusmes-backups
    region: us-east-1
    accessKeyId: "AKIAIOSFODNN7EXAMPLE"
    secretAccessKey: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
```

## Security

- Non-root containers
- Read-only root filesystem
- Dropped capabilities
- Network policies
- Secret management
- RBAC enabled

## Troubleshooting

### Check Deployment Status

```bash
# Helm status
helm status rusmes -n rusmes

# Pod status
kubectl get pods -n rusmes -l app.kubernetes.io/name=rusmes

# Detailed pod info
kubectl describe pod rusmes-0 -n rusmes

# Logs
kubectl logs -f rusmes-0 -n rusmes

# Previous logs (if pod restarted)
kubectl logs rusmes-0 -n rusmes --previous
```

### Common Issues

#### Pods Not Starting

Check events and describe the pod:

```bash
kubectl get events -n rusmes --sort-by='.lastTimestamp'
kubectl describe pod rusmes-0 -n rusmes
```

Common causes:
- Insufficient resources
- PVC binding issues
- Image pull failures
- Configuration errors

#### Database Connection Issues

Check PostgreSQL status:

```bash
kubectl get pods -n rusmes -l app.kubernetes.io/name=postgresql
kubectl logs -f rusmes-postgresql-0 -n rusmes
```

Test connection from rusmes pod:

```bash
kubectl exec -it rusmes-0 -n rusmes -- sh
# Inside pod:
# psql -h rusmes-postgresql -U rusmes -d rusmes
```

#### TLS Certificate Issues

Check cert-manager logs:

```bash
kubectl logs -n cert-manager deploy/cert-manager
```

Check certificate status:

```bash
kubectl describe certificate rusmes-jmap-tls -n rusmes
kubectl describe certificaterequest -n rusmes
```

#### Performance Issues

Check resource usage:

```bash
kubectl top pods -n rusmes
kubectl top nodes
```

Check HPA status:

```bash
kubectl get hpa -n rusmes
kubectl describe hpa rusmes -n rusmes
```

#### Mail Not Being Delivered

1. Check SMTP logs:

```bash
kubectl logs rusmes-0 -n rusmes | grep smtp
```

2. Verify DNS records:

```bash
dig MX example.com
dig mail.example.com
```

3. Test SMTP connectivity:

```bash
kubectl exec -it rusmes-0 -n rusmes -- rusmes-cli test smtp
```

### Debug Configuration

Preview rendered templates:

```bash
helm template rusmes ./helm/rusmes --debug --values my-values.yaml
```

Validate chart:

```bash
helm lint ./helm/rusmes --values my-values.yaml
```

### Access Metrics

Port-forward to access Prometheus metrics:

```bash
kubectl port-forward svc/rusmes-metrics 9090:9090 -n rusmes
# Visit http://localhost:9090/metrics
```

## Advanced Configuration

### Using External PostgreSQL

To use an external PostgreSQL database:

```yaml
postgresql:
  enabled: false

externalPostgresql:
  host: "postgres.example.com"
  port: 5432
  database: "rusmes"
  username: "rusmes"
  password: "secure-password"
```

### High Availability Setup

For production HA deployment:

```yaml
replicaCount: 5

autoscaling:
  enabled: true
  minReplicas: 5
  maxReplicas: 20

postgresql:
  architecture: replication
  replication:
    enabled: true
    numSynchronousReplicas: 1
  readReplicas:
    replicaCount: 2

podDisruptionBudget:
  enabled: true
  minAvailable: 3

affinity:
  podAntiAffinity:
    requiredDuringSchedulingIgnoredDuringExecution:
      - labelSelector:
          matchExpressions:
            - key: app.kubernetes.io/name
              operator: In
              values:
                - rusmes
        topologyKey: kubernetes.io/hostname
```

### Multi-Region Deployment

Deploy across multiple regions with separate releases:

```bash
# Region 1 (us-west)
helm install rusmes-west ./helm/rusmes \
  --namespace rusmes \
  --values values-production.yaml \
  --set config.hostname=mail-west.example.com

# Region 2 (us-east)
helm install rusmes-east ./helm/rusmes \
  --namespace rusmes \
  --values values-production.yaml \
  --set config.hostname=mail-east.example.com
```

### Custom Storage Classes

Use different storage classes for different components:

```yaml
persistence:
  storageClass: "fast-nvme-ssd"
  size: 200Gi

postgresql:
  primary:
    persistence:
      storageClass: "fast-ssd"
      size: 500Gi

backupPersistence:
  storageClass: "standard-hdd"
  size: 2Ti
```

### Resource Limits by Node Type

Use node selectors and taints:

```yaml
nodeSelector:
  workload-type: mail-server
  disk: nvme-ssd

tolerations:
  - key: "mail-server"
    operator: "Equal"
    value: "true"
    effect: "NoSchedule"
```

Label nodes:

```bash
kubectl label nodes worker-1 workload-type=mail-server
kubectl label nodes worker-1 disk=nvme-ssd
kubectl taint nodes worker-1 mail-server=true:NoSchedule
```

### Monitoring Integration

#### Prometheus ServiceMonitor

Enabled automatically when prometheus-operator is detected:

```yaml
prometheus:
  enabled: true
  serviceMonitor:
    enabled: true
    interval: 30s
    scrapeTimeout: 10s
```

#### Grafana Dashboard

Import the included dashboard:

```bash
kubectl apply -f helm/rusmes/dashboards/rusmes-dashboard.json
```

Or enable auto-import:

```yaml
grafana:
  enabled: true
  dashboards:
    enabled: true
```

## Development

### Local Development with Minikube

```bash
# Start minikube
minikube start --cpus=4 --memory=8192

# Enable ingress
minikube addons enable ingress

# Install with dev values
helm install rusmes ./helm/rusmes \
  --values ./helm/rusmes/values-development.yaml

# Get service URL
minikube service rusmes -n rusmes-dev --url
```

### Local Development with Kind

```bash
# Create cluster
kind create cluster --config kind-config.yaml

# Install chart
helm install rusmes ./helm/rusmes \
  --namespace rusmes-dev \
  --create-namespace \
  --values ./helm/rusmes/values-development.yaml

# Port forward
kubectl port-forward svc/rusmes 8080:8080 -n rusmes-dev
```

### Testing the Chart

```bash
# Lint the chart
helm lint ./helm/rusmes

# Dry run installation
helm install rusmes ./helm/rusmes \
  --dry-run --debug \
  --values my-values.yaml

# Template rendering
helm template rusmes ./helm/rusmes \
  --values my-values.yaml

# Install to test cluster
helm install rusmes ./helm/rusmes \
  --namespace rusmes-test \
  --create-namespace \
  --values values-development.yaml

# Run tests
helm test rusmes -n rusmes-test

# Uninstall
helm uninstall rusmes -n rusmes-test
```

### Building and Publishing

```bash
# Update dependencies
helm dependency update ./helm/rusmes

# Package the chart
helm package ./helm/rusmes

# Create index
helm repo index .

# Push to chart repository
# (Upload to your chart repository)
```

## Support

- Documentation: https://rusmes.example.com/docs
- Issues: https://github.com/example/rusmes/issues

# RusMES Kubernetes Deployment

This directory contains production-ready Kubernetes manifests for deploying RusMES (Rust Mail Enterprise Server) on Kubernetes 1.28+.

## Overview

RusMES is a high-performance mail server written in Rust, supporting SMTP, IMAP, POP3, and JMAP protocols. This Kubernetes deployment provides:

- **High Availability**: 3 replicas by default with Pod Disruption Budget
- **Persistent Storage**: StatefulSet with PersistentVolumeClaims
- **Security**: RBAC, SecurityContext, TLS encryption
- **Monitoring**: Prometheus metrics and health checks
- **Scalability**: Horizontal scaling support

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Load Balancer                          │
│  SMTP:25/587  IMAP:143/993  POP3:110/995  JMAP:443         │
└─────────────────────────────────────────────────────────────┘
                            │
        ┌───────────────────┼───────────────────┐
        ▼                   ▼                   ▼
   ┌────────┐         ┌────────┐         ┌────────┐
   │rusmes-0│         │rusmes-1│         │rusmes-2│
   │ (Pod)  │         │ (Pod)  │         │ (Pod)  │
   └────────┘         └────────┘         └────────┘
        │                   │                   │
        └───────────────────┼───────────────────┘
                            ▼
                    ┌──────────────┐
                    │  PostgreSQL  │
                    │   (Optional) │
                    └──────────────┘
```

## Files

### Core Manifests

1. **namespace.yaml** - Dedicated namespace for RusMES
2. **rbac.yaml** - ServiceAccount, Role, and RoleBinding with minimal permissions
3. **statefulset.yaml** - StatefulSet with 3 replicas, persistent storage, health checks
4. **service.yaml** - Services for SMTP, IMAP, POP3, JMAP, and metrics
5. **configmap.yaml** - Complete rusmes.toml configuration
6. **secrets.yaml** - TLS certificates, passwords, DKIM keys (templates)
7. **ingress.yaml** - Ingress for JMAP HTTP and metrics endpoints
8. **pvc.yaml** - PersistentVolumeClaims and StorageClasses

### Legacy Files (to be removed)

- **deployment.yaml** - Replaced by statefulset.yaml (kept for backward compatibility)

## Prerequisites

Before deploying RusMES, ensure you have:

1. **Kubernetes Cluster**: Version 1.28 or higher
2. **kubectl**: Configured to access your cluster
3. **Storage Provisioner**: For persistent volumes (e.g., AWS EBS, GCE PD, local-path)
4. **Ingress Controller**: nginx-ingress or similar (for JMAP HTTP)
5. **cert-manager** (Optional): For automatic TLS certificate management
6. **Prometheus** (Optional): For metrics collection

### Install Prerequisites

```bash
# Install nginx-ingress
kubectl apply -f https://raw.githubusercontent.com/kubernetes/ingress-nginx/controller-v1.8.1/deploy/static/provider/cloud/deploy.yaml

# Install cert-manager (optional)
kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.13.0/cert-manager.yaml

# Install Prometheus (optional)
helm repo add prometheus-community https://prometheus-community.github.io/helm-charts
helm install prometheus prometheus-community/kube-prometheus-stack -n monitoring --create-namespace
```

## Quick Start

### 1. Create Namespace

```bash
kubectl apply -f namespace.yaml
```

### 2. Configure Secrets

**IMPORTANT**: Update all secrets with production values before deploying.

```bash
# Edit secrets.yaml and replace all CHANGEME placeholders
vim secrets.yaml

# Apply secrets
kubectl apply -f secrets.yaml
```

**Generate TLS Certificates:**

```bash
# Option 1: Self-signed certificates (testing only)
openssl req -x509 -newkey rsa:4096 -keyout tls.key -out tls.crt -days 365 -nodes \
  -subj "/CN=mail.example.com"

kubectl create secret tls rusmes-tls \
  --cert=tls.crt \
  --key=tls.key \
  -n rusmes

# Option 2: Let's Encrypt (production)
# See "TLS Certificates" section below
```

**Generate DKIM Keys:**

```bash
# Generate DKIM key pair
openssl genrsa -out dkim.key 2048
openssl rsa -in dkim.key -pubout -out dkim.pub

# Create secret
kubectl create secret generic rusmes-dkim \
  --from-file=dkim.key=dkim.key \
  --from-literal=dkim.selector=default \
  --from-literal=dkim.domain=example.com \
  -n rusmes

# Publish public key in DNS
# default._domainkey.example.com. IN TXT "v=DKIM1; k=rsa; p=<base64-public-key>"
```

### 3. Configure Application

Edit `configmap.yaml` to customize:

```bash
vim configmap.yaml
# Update domain, hostnames, storage backend, etc.

kubectl apply -f configmap.yaml
```

### 4. Create Storage Resources

```bash
# Apply storage classes and PVCs
kubectl apply -f pvc.yaml
```

### 5. Apply RBAC

```bash
kubectl apply -f rbac.yaml
```

### 6. Deploy RusMES

```bash
# Deploy StatefulSet
kubectl apply -f statefulset.yaml

# Deploy Services
kubectl apply -f service.yaml

# Deploy Ingress (optional)
kubectl apply -f ingress.yaml
```

### 7. Verify Deployment

```bash
# Check pod status
kubectl get pods -n rusmes -w

# Check services
kubectl get svc -n rusmes

# Check persistent volumes
kubectl get pvc -n rusmes

# View logs
kubectl logs -f rusmes-0 -n rusmes

# Check all resources
kubectl get all -n rusmes
```

## Configuration

### Domain Configuration

Update `configmap.yaml` with your domain:

```toml
domain = "example.com"
postmaster = "postmaster@example.com"

[jmap]
base_url = "https://jmap.example.com"
```

### Storage Backend

RusMES supports multiple storage backends:

**1. Filesystem (Default)**

```toml
[storage]
backend = "filesystem"
path = "/var/lib/rusmes/mail"
```

**2. PostgreSQL**

```toml
[storage]
backend = "postgres"
connection_string = "postgresql://rusmes:${POSTGRES_PASSWORD}@postgres-service:5432/rusmes"
max_connections = 100
```

**3. AmateRS Distributed Storage**

```toml
[storage]
backend = "amaters"
endpoints = ["amaters-0.amaters:8080", "amaters-1.amaters:8080", "amaters-2.amaters:8080"]
replication_factor = 3
```

### Resource Limits

Default resource limits (per pod):

```yaml
resources:
  requests:
    memory: "512Mi"
    cpu: "500m"
  limits:
    memory: "2Gi"
    cpu: "2000m"
```

Adjust based on your workload in `statefulset.yaml`.

## TLS Certificates

### Option 1: cert-manager (Recommended)

1. Install cert-manager:

```bash
kubectl apply -f https://github.com/cert-manager/cert-manager/releases/download/v1.13.0/cert-manager.yaml
```

2. Create ClusterIssuer:

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

3. Certificates will be automatically created by Ingress annotations.

### Option 2: Manual Certificates

```bash
# Generate self-signed certificate
openssl req -x509 -newkey rsa:4096 -keyout tls.key -out tls.crt -days 365 -nodes \
  -subj "/CN=mail.example.com"

# Create secret
kubectl create secret tls rusmes-tls \
  --cert=tls.crt \
  --key=tls.key \
  -n rusmes
```

## DNS Configuration

Configure the following DNS records:

```
# MX record for mail delivery
example.com.           IN MX 10 mail.example.com.

# A records for mail server
mail.example.com.      IN A  <LOAD_BALANCER_IP>
smtp.example.com.      IN A  <LOAD_BALANCER_IP>
imap.example.com.      IN A  <LOAD_BALANCER_IP>
jmap.example.com.      IN A  <LOAD_BALANCER_IP>

# SPF record
example.com.           IN TXT "v=spf1 mx a:<LOAD_BALANCER_IP> -all"

# DKIM record
default._domainkey.example.com. IN TXT "v=DKIM1; k=rsa; p=<PUBLIC_KEY>"

# DMARC record
_dmarc.example.com.    IN TXT "v=DMARC1; p=quarantine; rua=mailto:postmaster@example.com"
```

Get LoadBalancer IP:

```bash
kubectl get svc rusmes-smtp -n rusmes -o jsonpath='{.status.loadBalancer.ingress[0].ip}'
```

## Scaling

### Horizontal Scaling

Scale the StatefulSet:

```bash
# Scale to 5 replicas
kubectl scale statefulset rusmes --replicas=5 -n rusmes

# Verify scaling
kubectl get pods -n rusmes -w
```

### Vertical Scaling

Update resource limits in `statefulset.yaml`:

```yaml
resources:
  requests:
    memory: "1Gi"
    cpu: "1000m"
  limits:
    memory: "4Gi"
    cpu: "4000m"
```

Apply changes:

```bash
kubectl apply -f statefulset.yaml
```

### Storage Expansion

Expand PVC size:

```bash
kubectl patch pvc data-rusmes-0 -n rusmes -p '{"spec":{"resources":{"requests":{"storage":"100Gi"}}}}'
```

## Monitoring

### Prometheus Metrics

RusMES exposes Prometheus metrics on port 9090:

```bash
# Port-forward metrics endpoint
kubectl port-forward svc/rusmes-metrics 9090:9090 -n rusmes

# Access metrics
curl http://localhost:9090/metrics
```

### Grafana Dashboard

Import the RusMES Grafana dashboard:

```bash
# See grafana/ directory for dashboard templates
kubectl apply -f ../grafana/rusmes-dashboard.json
```

### Health Checks

Health check endpoints:

- **Liveness**: TCP socket on SMTP port (25)
- **Readiness**: HTTP GET `/health` on metrics port (9090)
- **Startup**: HTTP GET `/health` on metrics port (9090)

Check pod health:

```bash
kubectl describe pod rusmes-0 -n rusmes | grep -A 10 "Conditions:"
```

## Backup and Restore

### Backup

Create a CronJob for automated backups:

```bash
cat <<EOF | kubectl apply -f -
apiVersion: batch/v1
kind: CronJob
metadata:
  name: rusmes-backup
  namespace: rusmes
spec:
  schedule: "0 2 * * *"  # Daily at 2 AM
  jobTemplate:
    spec:
      template:
        spec:
          containers:
          - name: backup
            image: rusmes/rusmes:latest
            command: ["/usr/local/bin/rusmes-cli", "backup", "/var/lib/rusmes", "/backup"]
            volumeMounts:
            - name: data
              mountPath: /var/lib/rusmes
            - name: backup
              mountPath: /backup
          restartPolicy: OnFailure
          volumes:
          - name: data
            persistentVolumeClaim:
              claimName: data-rusmes-0
          - name: backup
            persistentVolumeClaim:
              claimName: rusmes-backup
EOF
```

### Manual Backup

```bash
# Backup specific pod data
kubectl exec rusmes-0 -n rusmes -- tar czf /tmp/backup.tar.gz /var/lib/rusmes
kubectl cp rusmes/rusmes-0:/tmp/backup.tar.gz ./backup-$(date +%Y%m%d).tar.gz
```

### Restore

```bash
# Restore from backup
kubectl cp ./backup-20260215.tar.gz rusmes/rusmes-0:/tmp/backup.tar.gz
kubectl exec rusmes-0 -n rusmes -- tar xzf /tmp/backup.tar.gz -C /
kubectl delete pod rusmes-0 -n rusmes  # Restart pod
```

## Troubleshooting

### Pod Not Starting

```bash
# Check pod status
kubectl describe pod rusmes-0 -n rusmes

# Check events
kubectl get events -n rusmes --sort-by='.lastTimestamp'

# Check logs
kubectl logs rusmes-0 -n rusmes

# Check init container logs
kubectl logs rusmes-0 -n rusmes -c init-config
kubectl logs rusmes-0 -n rusmes -c init-data-dir
```

### Connection Issues

```bash
# Test SMTP connectivity
kubectl run -it --rm debug --image=alpine --restart=Never -n rusmes -- \
  sh -c "apk add netcat-openbsd && nc -zv rusmes-smtp 25"

# Test IMAP connectivity
kubectl run -it --rm debug --image=alpine --restart=Never -n rusmes -- \
  sh -c "apk add netcat-openbsd && nc -zv rusmes-imap 143"

# Test from within cluster
kubectl exec rusmes-0 -n rusmes -- nc -zv rusmes-headless 25
```

### Performance Issues

```bash
# Check resource usage
kubectl top pods -n rusmes

# Check node resources
kubectl top nodes

# Increase resources
kubectl edit statefulset rusmes -n rusmes
```

### Storage Issues

```bash
# Check PVC status
kubectl get pvc -n rusmes

# Check PV status
kubectl get pv

# Describe PVC
kubectl describe pvc data-rusmes-0 -n rusmes

# Check disk usage
kubectl exec rusmes-0 -n rusmes -- df -h /var/lib/rusmes
```

### TLS Certificate Issues

```bash
# Check cert-manager status
kubectl get certificate -n rusmes
kubectl describe certificate rusmes-jmap-tls -n rusmes

# Check secret
kubectl get secret rusmes-tls -n rusmes
kubectl describe secret rusmes-tls -n rusmes

# Renew certificate
kubectl delete certificate rusmes-jmap-tls -n rusmes
kubectl apply -f ingress.yaml
```

### Common Issues

**Issue**: Pods in CrashLoopBackOff

```bash
# Check logs for errors
kubectl logs rusmes-0 -n rusmes --previous

# Common causes:
# - Invalid configuration in ConfigMap
# - Missing secrets
# - Insufficient resources
# - Storage issues
```

**Issue**: LoadBalancer stuck in Pending

```bash
# Check service
kubectl describe svc rusmes-smtp -n rusmes

# Common causes:
# - Cloud provider doesn't support LoadBalancer
# - Quota exceeded
# - Invalid service configuration

# Workaround: Use NodePort
kubectl patch svc rusmes-smtp -n rusmes -p '{"spec":{"type":"NodePort"}}'
```

## Security Best Practices

1. **Change all default passwords** in `secrets.yaml`
2. **Use TLS/SSL** for all connections
3. **Enable RBAC** with minimal permissions
4. **Use Pod Security Policies** or Pod Security Standards
5. **Enable network policies** to restrict traffic
6. **Regular security updates** of container images
7. **Rotate TLS certificates** regularly
8. **Enable audit logging**
9. **Use secrets management** (e.g., Sealed Secrets, Vault)
10. **Scan images** for vulnerabilities

## Maintenance

### Rolling Updates

Update container image:

```bash
kubectl set image statefulset/rusmes rusmes=rusmes/rusmes:v0.2.0 -n rusmes
```

### Rolling Restart

```bash
kubectl rollout restart statefulset rusmes -n rusmes
```

### Update Configuration

```bash
# Edit ConfigMap
kubectl edit configmap rusmes-config -n rusmes

# Restart pods to apply changes
kubectl rollout restart statefulset rusmes -n rusmes
```

## Uninstall

Remove RusMES completely:

```bash
# Delete all resources
kubectl delete -f ingress.yaml
kubectl delete -f service.yaml
kubectl delete -f statefulset.yaml
kubectl delete -f rbac.yaml
kubectl delete -f configmap.yaml
kubectl delete -f secrets.yaml
kubectl delete -f pvc.yaml

# Delete namespace
kubectl delete namespace rusmes
```

**Warning**: This will delete all mail data. Backup first!

## Support

- **Documentation**: https://github.com/yourusername/rusmes
- **Issues**: https://github.com/yourusername/rusmes/issues
- **Discussions**: https://github.com/yourusername/rusmes/discussions

## License

Apache-2.0

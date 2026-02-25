# RusMES Kubernetes Deployment - File Index

## Directory Structure (80K total, 16 files)

```
k8s/
├── namespace.yaml           (231B)   - Namespace definition
├── rbac.yaml               (1.3K)   - RBAC: ServiceAccount, Role, RoleBinding
├── configmap.yaml          (4.1K)   - rusmes.toml configuration
├── secrets.yaml            (3.5K)   - TLS, passwords, DKIM keys (templates)
├── pvc.yaml                (2.1K)   - PVCs and StorageClasses
├── statefulset.yaml        (5.6K)   - StatefulSet with 3 replicas
├── service.yaml            (3.4K)   - Services (SMTP, IMAP, POP3, JMAP, metrics)
├── ingress.yaml            (4.1K)   - Ingress for JMAP and metrics
├── kustomization.yaml      (1.2K)   - Kustomize configuration
├── deployment.yaml         (4.1K)   - Legacy deployment (use statefulset.yaml)
├── deploy.sh               (4.1K)   - Automated deployment script
├── undeploy.sh             (3.4K)   - Automated cleanup script
├── validate.sh             (9.4K)   - Deployment validation script
├── README.md               (15K)    - Comprehensive documentation
├── QUICKSTART.md           (4.2K)   - Quick reference guide
├── MANIFEST_SUMMARY.md     (11K)    - Task completion report
└── INDEX.md                (this)   - This file index
```

## Quick Navigation

### Getting Started
1. Read: [README.md](README.md) - Full deployment guide
2. Read: [QUICKSTART.md](QUICKSTART.md) - Essential commands
3. Edit: [secrets.yaml](secrets.yaml) - Update all CHANGEME values
4. Run: `./deploy.sh` - Deploy to Kubernetes
5. Run: `./validate.sh` - Validate deployment

### Manifest Files (Deploy in this order)
1. [namespace.yaml](namespace.yaml) - Create namespace first
2. [rbac.yaml](rbac.yaml) - Set up permissions
3. [configmap.yaml](configmap.yaml) - Application configuration
4. [secrets.yaml](secrets.yaml) - Sensitive data (update first!)
5. [pvc.yaml](pvc.yaml) - Storage resources
6. [statefulset.yaml](statefulset.yaml) - Main deployment
7. [service.yaml](service.yaml) - Network services
8. [ingress.yaml](ingress.yaml) - HTTP ingress (optional)

### Helper Scripts
- [deploy.sh](deploy.sh) - Automated deployment (recommended)
- [undeploy.sh](undeploy.sh) - Automated cleanup
- [validate.sh](validate.sh) - Health checks and validation

### Documentation
- [README.md](README.md) - Complete deployment guide (15K)
- [QUICKSTART.md](QUICKSTART.md) - Quick reference (4K)
- [MANIFEST_SUMMARY.md](MANIFEST_SUMMARY.md) - Technical details (11K)
- [INDEX.md](INDEX.md) - This file

### Advanced
- [kustomization.yaml](kustomization.yaml) - Kustomize overlay
- [deployment.yaml](deployment.yaml) - Legacy (use statefulset.yaml)

## Deployment Methods

### Method 1: Automated (Recommended)
```bash
./deploy.sh
```

### Method 2: Manual kubectl
```bash
kubectl apply -f namespace.yaml
kubectl apply -f rbac.yaml
kubectl apply -f configmap.yaml
kubectl apply -f secrets.yaml
kubectl apply -f pvc.yaml
kubectl apply -f statefulset.yaml
kubectl apply -f service.yaml
kubectl apply -f ingress.yaml
```

### Method 3: Kustomize
```bash
kubectl apply -k .
```

## File Descriptions

### namespace.yaml
Creates dedicated `rusmes` namespace with labels and annotations.

### rbac.yaml
Defines ServiceAccount, Role (read-only permissions), and RoleBinding for security.

### configmap.yaml
Complete rusmes.toml configuration including:
- SMTP, IMAP, POP3, JMAP settings
- Storage backend options
- Mail processing pipeline
- Security features (DKIM, SPF, DMARC)
- Logging and metrics

### secrets.yaml
Templates for sensitive data:
- rusmes-tls: TLS certificates
- rusmes-db: PostgreSQL credentials
- rusmes-admin: Administrator credentials
- rusmes-dkim: DKIM private key
- rusmes-backup-s3: S3 backup credentials

**IMPORTANT**: Update all CHANGEME placeholders before deployment!

### pvc.yaml
Storage resources:
- rusmes-backup: 500Gi NFS for backups
- postgres-data: 100Gi SSD for database
- StorageClasses: fast-ssd, nfs-storage, local-ssd

### statefulset.yaml
Main deployment with:
- 3 replicas (high availability)
- 50Gi PVC per pod
- Resource limits: 512Mi-2Gi RAM, 500m-2000m CPU
- Health checks (liveness, readiness, startup)
- Security contexts (non-root, read-only fs)
- Init containers for setup
- PodDisruptionBudget (min 2 available)

### service.yaml
Network services:
- rusmes-smtp: LoadBalancer (25, 587)
- rusmes-imap: LoadBalancer (143, 993)
- rusmes-pop3: LoadBalancer (110, 995)
- rusmes-jmap: ClusterIP (8080)
- rusmes-metrics: ClusterIP (9090)
- rusmes-headless: Headless service

### ingress.yaml
HTTP ingress for:
- JMAP API (jmap.example.com, mail.example.com)
- Metrics endpoint (metrics.example.com)
- TLS termination (cert-manager)
- Rate limiting, CORS, security headers

### kustomization.yaml
Kustomize configuration for overlay-based deployment.

### deploy.sh
Automated deployment script with:
- Prerequisites checking
- Secrets validation
- Step-by-step deployment
- Status verification
- User-friendly output

### undeploy.sh
Automated cleanup script with:
- Confirmation prompts (type DELETE)
- Graceful resource deletion
- Warning about data loss

### validate.sh
Comprehensive validation including:
- Resource existence checks
- Pod health status
- Service endpoints
- Storage binding
- Connectivity tests
- Summary statistics

## Common Tasks

### Deploy
```bash
./deploy.sh
```

### Check Status
```bash
kubectl get all -n rusmes
./validate.sh
```

### View Logs
```bash
kubectl logs -f rusmes-0 -n rusmes
```

### Scale
```bash
kubectl scale statefulset rusmes --replicas=5 -n rusmes
```

### Update Configuration
```bash
kubectl edit configmap rusmes-config -n rusmes
kubectl rollout restart statefulset rusmes -n rusmes
```

### Get LoadBalancer IPs
```bash
kubectl get svc -n rusmes
```

### Cleanup
```bash
./undeploy.sh
```

## Prerequisites

- Kubernetes 1.28+
- kubectl configured
- 50Gi+ storage per replica
- LoadBalancer support (cloud provider)
- cert-manager (optional, for TLS)
- Ingress controller (optional, for JMAP HTTP)

## Support

See [README.md](README.md) for comprehensive documentation and troubleshooting.

## License

Apache-2.0

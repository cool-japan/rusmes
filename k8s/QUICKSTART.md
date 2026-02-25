# RusMES Kubernetes Quick Start Guide

This is a quick reference for deploying and managing RusMES on Kubernetes.

## Prerequisites

- Kubernetes cluster 1.28+
- kubectl configured
- 50Gi+ available storage per replica
- LoadBalancer support (for cloud providers)

## Quick Deploy (Automated)

```bash
# 1. Edit secrets with production values
vim secrets.yaml

# 2. Deploy everything
./deploy.sh

# 3. Validate deployment
./validate.sh
```

## Quick Deploy (Manual)

```bash
# 1. Create namespace
kubectl apply -f namespace.yaml

# 2. Update and apply secrets
vim secrets.yaml
kubectl apply -f secrets.yaml

# 3. Apply resources in order
kubectl apply -f rbac.yaml
kubectl apply -f configmap.yaml
kubectl apply -f pvc.yaml
kubectl apply -f statefulset.yaml
kubectl apply -f service.yaml
kubectl apply -f ingress.yaml

# 4. Wait for pods
kubectl get pods -n rusmes -w
```

## Common Commands

### Check Status

```bash
# All resources
kubectl get all -n rusmes

# Pods only
kubectl get pods -n rusmes

# Services and LoadBalancer IPs
kubectl get svc -n rusmes

# Storage
kubectl get pvc -n rusmes
```

### View Logs

```bash
# Follow logs from first pod
kubectl logs -f rusmes-0 -n rusmes

# View all pods
kubectl logs -l app=rusmes -n rusmes --tail=50

# Previous crashed pod
kubectl logs rusmes-0 -n rusmes --previous
```

### Scale

```bash
# Scale to 5 replicas
kubectl scale statefulset rusmes --replicas=5 -n rusmes

# Check scaling progress
kubectl get pods -n rusmes -w
```

### Update Configuration

```bash
# Edit ConfigMap
kubectl edit configmap rusmes-config -n rusmes

# Restart pods to apply
kubectl rollout restart statefulset rusmes -n rusmes
```

### Troubleshooting

```bash
# Describe pod
kubectl describe pod rusmes-0 -n rusmes

# Check events
kubectl get events -n rusmes --sort-by='.lastTimestamp'

# Check resource usage
kubectl top pods -n rusmes

# Execute shell in pod
kubectl exec -it rusmes-0 -n rusmes -- /bin/sh

# Port forward for testing
kubectl port-forward rusmes-0 -n rusmes 8080:8080
```

## Get LoadBalancer IPs

```bash
# SMTP
kubectl get svc rusmes-smtp -n rusmes -o jsonpath='{.status.loadBalancer.ingress[0].ip}'

# IMAP
kubectl get svc rusmes-imap -n rusmes -o jsonpath='{.status.loadBalancer.ingress[0].ip}'

# POP3
kubectl get svc rusmes-pop3 -n rusmes -o jsonpath='{.status.loadBalancer.ingress[0].ip}'
```

## Test Connectivity

```bash
# Test SMTP
telnet <SMTP_IP> 25

# Test IMAP
telnet <IMAP_IP> 143

# Test JMAP (via kubectl port-forward)
kubectl port-forward svc/rusmes-jmap -n rusmes 8080:8080
curl http://localhost:8080/.well-known/jmap

# Test metrics
kubectl port-forward svc/rusmes-metrics -n rusmes 9090:9090
curl http://localhost:9090/metrics
```

## DNS Configuration

After getting LoadBalancer IPs, configure DNS:

```
# Replace <IP> with actual LoadBalancer IP
mail.example.com.      IN A  <IP>
smtp.example.com.      IN A  <IP>
imap.example.com.      IN A  <IP>
jmap.example.com.      IN A  <IP>

# MX record
example.com.           IN MX 10 mail.example.com.
```

## Backup

```bash
# Manual backup of pod data
kubectl exec rusmes-0 -n rusmes -- tar czf /tmp/backup.tar.gz /var/lib/rusmes
kubectl cp rusmes/rusmes-0:/tmp/backup.tar.gz ./backup-$(date +%Y%m%d).tar.gz
```

## Restore

```bash
# Restore from backup
kubectl cp ./backup-20260215.tar.gz rusmes/rusmes-0:/tmp/backup.tar.gz
kubectl exec rusmes-0 -n rusmes -- tar xzf /tmp/backup.tar.gz -C /
kubectl delete pod rusmes-0 -n rusmes
```

## Uninstall

```bash
# Automated (prompts for confirmation)
./undeploy.sh

# Manual
kubectl delete -f ingress.yaml
kubectl delete -f service.yaml
kubectl delete -f statefulset.yaml
kubectl delete -f configmap.yaml
kubectl delete -f rbac.yaml
kubectl delete -f secrets.yaml
kubectl delete -f pvc.yaml
kubectl delete namespace rusmes
```

## Important Files

- **namespace.yaml** - Namespace definition
- **rbac.yaml** - RBAC permissions
- **configmap.yaml** - rusmes.toml configuration
- **secrets.yaml** - TLS, passwords, DKIM keys
- **statefulset.yaml** - Main deployment
- **service.yaml** - Network services
- **ingress.yaml** - HTTP ingress for JMAP
- **pvc.yaml** - Storage configuration

## Scripts

- **deploy.sh** - Automated deployment
- **undeploy.sh** - Automated cleanup
- **validate.sh** - Validation checks

## Support

See README.md for comprehensive documentation.

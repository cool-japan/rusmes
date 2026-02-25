# Changelog

All notable changes to the RusMES Helm chart will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-02-15

### Added
- Initial Helm chart release
- StatefulSet deployment with configurable replicas
- PostgreSQL subchart integration (Bitnami)
- Prometheus subchart integration (optional)
- Grafana subchart integration (optional)
- Comprehensive ConfigMap for rusmes.toml generation
- Secrets management for database credentials and TLS
- PersistentVolumeClaims for mail data and backups
- Services for SMTP, IMAP, POP3, JMAP, and metrics
- Ingress for JMAP web interface
- HorizontalPodAutoscaler for automatic scaling
- PodDisruptionBudget for high availability
- NetworkPolicy for security isolation
- RBAC with Role and RoleBinding
- ServiceAccount with configurable annotations
- TLS support with cert-manager integration
- ACME configuration for automatic certificate renewal
- Backup configuration with S3 support
- Production-ready values.yaml with sensible defaults
- Development values (values-development.yaml)
- Production example values (values-production.yaml)
- Example configurations in examples/ directory
- Comprehensive README.md with installation guide
- Detailed INSTALL.md with step-by-step instructions
- NOTES.txt with post-installation instructions
- Validation script (validate.sh)
- .helmignore for excluding unnecessary files
- Support for external PostgreSQL database
- Session affinity for stateful connections
- Health checks (liveness, readiness, startup probes)
- Anti-affinity rules for pod distribution
- Security contexts (non-root, read-only filesystem)
- Monitoring integration with Prometheus ServiceMonitor

### Configuration Options
- Configurable replicas (default: 3)
- Configurable resources (CPU, memory)
- Configurable persistence (size, storageClass)
- Configurable autoscaling (min/max replicas, CPU/memory targets)
- Configurable security features (DKIM, SPF, DMARC)
- Configurable mail protocols (SMTP, IMAP, POP3, JMAP)
- Configurable logging (level, format)
- Configurable backup schedule and retention
- Configurable network policies
- Configurable node selectors and tolerations
- Configurable affinity rules

### Dependencies
- PostgreSQL: ^13.0.0 (Bitnami)
- Prometheus: ^25.0.0 (Prometheus Community)
- Grafana: ^8.0.0 (Grafana Labs)

### Documentation
- Complete README with all configuration parameters
- Installation guide with prerequisites and steps
- Troubleshooting guide with common issues
- Examples for production and development deployments
- Upgrade and rollback instructions
- Uninstall instructions with PVC cleanup

### Files
- Chart.yaml - Chart metadata
- values.yaml - Default configuration
- values-production.yaml - Production configuration example
- values-development.yaml - Development configuration example
- templates/_helpers.tpl - Template helpers
- templates/NOTES.txt - Post-install notes
- templates/configmap.yaml - Configuration management
- templates/secrets.yaml - Secrets management
- templates/statefulset.yaml - Main deployment
- templates/service.yaml - Network services
- templates/ingress.yaml - Ingress configuration
- templates/hpa.yaml - Autoscaling configuration
- templates/pvc.yaml - Persistent volume claims
- templates/pdb.yaml - Pod disruption budget
- templates/rbac.yaml - RBAC configuration
- templates/serviceaccount.yaml - Service account
- templates/networkpolicy.yaml - Network policies
- .helmignore - Exclude patterns
- README.md - Main documentation
- INSTALL.md - Installation guide
- CHANGELOG.md - Version history
- validate.sh - Validation script
- examples/production.yaml - Production example
- examples/minimal.yaml - Minimal example

### Notes
- Tested with Kubernetes 1.20+
- Tested with Helm 3.0+
- Requires PV provisioner
- Optional cert-manager for automatic TLS
- Optional ingress controller for web access

## [Unreleased]

### Planned
- Helm tests for automated validation
- Support for custom init containers
- Support for custom sidecar containers
- Support for custom volumes
- Grafana dashboard JSON files
- AlertManager integration
- Datadog integration
- Splunk integration
- External secrets operator integration
- Vault secrets integration
- Multi-tenancy support
- Horizontal scaling for database
- Redis cache integration
- Elasticsearch integration for search
- Advanced networking with service mesh
- GitOps examples (ArgoCD, Flux)
- Kustomize overlays
- Terraform module
- CloudFormation template
- ARM template for Azure

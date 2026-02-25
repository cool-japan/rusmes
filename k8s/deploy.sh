#!/bin/bash
set -e

# RusMES Kubernetes Deployment Script
# This script deploys RusMES mail server to Kubernetes

NAMESPACE="rusmes"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Print colored message
print_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if kubectl is installed
check_kubectl() {
    if ! command -v kubectl &> /dev/null; then
        print_error "kubectl not found. Please install kubectl first."
        exit 1
    fi
    print_info "kubectl found: $(kubectl version --client --short 2>/dev/null || kubectl version --client)"
}

# Check cluster connectivity
check_cluster() {
    if ! kubectl cluster-info &> /dev/null; then
        print_error "Cannot connect to Kubernetes cluster. Please configure kubectl first."
        exit 1
    fi
    print_info "Connected to cluster: $(kubectl config current-context)"
}

# Create namespace
create_namespace() {
    print_info "Creating namespace: $NAMESPACE"
    kubectl apply -f "$SCRIPT_DIR/namespace.yaml"
}

# Check secrets
check_secrets() {
    print_warn "IMPORTANT: Make sure you have updated secrets.yaml with production values!"
    print_warn "Current secrets contain CHANGEME placeholders."
    echo ""
    read -p "Have you updated secrets.yaml? (yes/no): " answer
    if [ "$answer" != "yes" ]; then
        print_error "Please update secrets.yaml before deploying."
        exit 1
    fi
}

# Deploy secrets
deploy_secrets() {
    print_info "Deploying secrets..."
    kubectl apply -f "$SCRIPT_DIR/secrets.yaml"
}

# Deploy RBAC
deploy_rbac() {
    print_info "Deploying RBAC (ServiceAccount, Role, RoleBinding)..."
    kubectl apply -f "$SCRIPT_DIR/rbac.yaml"
}

# Deploy ConfigMap
deploy_configmap() {
    print_info "Deploying ConfigMap..."
    kubectl apply -f "$SCRIPT_DIR/configmap.yaml"
}

# Deploy Storage
deploy_storage() {
    print_info "Deploying PersistentVolumeClaims and StorageClasses..."
    kubectl apply -f "$SCRIPT_DIR/pvc.yaml"
}

# Deploy StatefulSet
deploy_statefulset() {
    print_info "Deploying StatefulSet..."
    kubectl apply -f "$SCRIPT_DIR/statefulset.yaml"
}

# Deploy Services
deploy_services() {
    print_info "Deploying Services..."
    kubectl apply -f "$SCRIPT_DIR/service.yaml"
}

# Deploy Ingress
deploy_ingress() {
    print_info "Deploying Ingress..."
    kubectl apply -f "$SCRIPT_DIR/ingress.yaml"
}

# Wait for pods
wait_for_pods() {
    print_info "Waiting for pods to be ready..."
    kubectl wait --for=condition=ready pod -l app=rusmes -n "$NAMESPACE" --timeout=300s || {
        print_warn "Pods took longer than expected to start. Check status with: kubectl get pods -n $NAMESPACE"
    }
}

# Show status
show_status() {
    echo ""
    print_info "=== Deployment Status ==="
    echo ""
    echo "Pods:"
    kubectl get pods -n "$NAMESPACE"
    echo ""
    echo "Services:"
    kubectl get svc -n "$NAMESPACE"
    echo ""
    echo "PersistentVolumeClaims:"
    kubectl get pvc -n "$NAMESPACE"
    echo ""
    print_info "=== Next Steps ==="
    echo ""
    echo "1. Check pod logs:"
    echo "   kubectl logs -f rusmes-0 -n $NAMESPACE"
    echo ""
    echo "2. Get LoadBalancer IPs:"
    echo "   kubectl get svc -n $NAMESPACE"
    echo ""
    echo "3. Configure DNS records (see README.md)"
    echo ""
    echo "4. Test SMTP connectivity:"
    echo "   telnet <SMTP_IP> 25"
    echo ""
}

# Main deployment
main() {
    echo ""
    print_info "=========================================="
    print_info "  RusMES Kubernetes Deployment Script"
    print_info "=========================================="
    echo ""

    check_kubectl
    check_cluster
    check_secrets

    echo ""
    print_info "Starting deployment to namespace: $NAMESPACE"
    echo ""

    create_namespace
    deploy_secrets
    deploy_rbac
    deploy_configmap
    deploy_storage
    deploy_statefulset
    deploy_services
    deploy_ingress

    echo ""
    wait_for_pods
    show_status

    echo ""
    print_info "Deployment completed successfully!"
    echo ""
}

# Run main function
main

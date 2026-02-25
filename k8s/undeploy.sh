#!/bin/bash
set -e

# RusMES Kubernetes Undeployment Script
# This script removes RusMES mail server from Kubernetes

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

# Confirm deletion
confirm_deletion() {
    echo ""
    print_warn "=========================================="
    print_warn "  WARNING: This will DELETE all RusMES data!"
    print_warn "=========================================="
    echo ""
    print_warn "This action will:"
    echo "  - Delete all pods and services"
    echo "  - Delete all persistent volumes (mail data will be lost)"
    echo "  - Delete all secrets and configurations"
    echo "  - Delete the entire namespace: $NAMESPACE"
    echo ""
    print_warn "Make sure you have backed up all important data!"
    echo ""

    read -p "Are you sure you want to continue? Type 'DELETE' to confirm: " answer
    if [ "$answer" != "DELETE" ]; then
        print_info "Undeployment cancelled."
        exit 0
    fi
}

# Delete resources
delete_resources() {
    print_info "Deleting Ingress..."
    kubectl delete -f "$SCRIPT_DIR/ingress.yaml" --ignore-not-found=true

    print_info "Deleting Services..."
    kubectl delete -f "$SCRIPT_DIR/service.yaml" --ignore-not-found=true

    print_info "Deleting StatefulSet..."
    kubectl delete -f "$SCRIPT_DIR/statefulset.yaml" --ignore-not-found=true

    print_info "Waiting for pods to terminate..."
    kubectl wait --for=delete pod -l app=rusmes -n "$NAMESPACE" --timeout=120s || true

    print_info "Deleting ConfigMap..."
    kubectl delete -f "$SCRIPT_DIR/configmap.yaml" --ignore-not-found=true

    print_info "Deleting RBAC..."
    kubectl delete -f "$SCRIPT_DIR/rbac.yaml" --ignore-not-found=true

    print_info "Deleting Secrets..."
    kubectl delete -f "$SCRIPT_DIR/secrets.yaml" --ignore-not-found=true

    print_info "Deleting PersistentVolumeClaims and StorageClasses..."
    kubectl delete -f "$SCRIPT_DIR/pvc.yaml" --ignore-not-found=true

    print_info "Deleting Namespace..."
    kubectl delete -f "$SCRIPT_DIR/namespace.yaml" --ignore-not-found=true
}

# Show final status
show_status() {
    echo ""
    print_info "Checking for remaining resources..."

    if kubectl get namespace "$NAMESPACE" &> /dev/null; then
        print_warn "Namespace $NAMESPACE still exists (may take time to fully terminate)"
        kubectl get all -n "$NAMESPACE" || true
    else
        print_info "Namespace $NAMESPACE has been deleted"
    fi

    echo ""
    print_info "Checking for remaining PersistentVolumes..."
    kubectl get pv | grep rusmes || print_info "No RusMES PersistentVolumes found"
}

# Main undeployment
main() {
    echo ""
    print_info "=========================================="
    print_info "  RusMES Kubernetes Undeployment Script"
    print_info "=========================================="

    confirm_deletion

    echo ""
    print_info "Starting undeployment from namespace: $NAMESPACE"
    echo ""

    delete_resources
    show_status

    echo ""
    print_info "Undeployment completed successfully!"
    print_info "All RusMES resources have been removed."
    echo ""
}

# Run main function
main

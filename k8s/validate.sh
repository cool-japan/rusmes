#!/bin/bash

# RusMES Kubernetes Validation Script
# This script validates the RusMES deployment

NAMESPACE="rusmes"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Counters
PASSED=0
FAILED=0
WARNINGS=0

# Print colored message
print_pass() {
    echo -e "${GREEN}[PASS]${NC} $1"
    ((PASSED++))
}

print_fail() {
    echo -e "${RED}[FAIL]${NC} $1"
    ((FAILED++))
}

print_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
    ((WARNINGS++))
}

print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_section() {
    echo ""
    echo "=========================================="
    echo "$1"
    echo "=========================================="
}

# Check if namespace exists
check_namespace() {
    print_section "Checking Namespace"
    if kubectl get namespace "$NAMESPACE" &> /dev/null; then
        print_pass "Namespace '$NAMESPACE' exists"
    else
        print_fail "Namespace '$NAMESPACE' not found"
        return 1
    fi
}

# Check RBAC resources
check_rbac() {
    print_section "Checking RBAC"

    if kubectl get serviceaccount rusmes -n "$NAMESPACE" &> /dev/null; then
        print_pass "ServiceAccount 'rusmes' exists"
    else
        print_fail "ServiceAccount 'rusmes' not found"
    fi

    if kubectl get role rusmes -n "$NAMESPACE" &> /dev/null; then
        print_pass "Role 'rusmes' exists"
    else
        print_fail "Role 'rusmes' not found"
    fi

    if kubectl get rolebinding rusmes -n "$NAMESPACE" &> /dev/null; then
        print_pass "RoleBinding 'rusmes' exists"
    else
        print_fail "RoleBinding 'rusmes' not found"
    fi
}

# Check ConfigMap
check_configmap() {
    print_section "Checking ConfigMap"

    if kubectl get configmap rusmes-config -n "$NAMESPACE" &> /dev/null; then
        print_pass "ConfigMap 'rusmes-config' exists"

        # Check if rusmes.toml is present
        if kubectl get configmap rusmes-config -n "$NAMESPACE" -o jsonpath='{.data.rusmes\.toml}' | grep -q "domain"; then
            print_pass "ConfigMap contains rusmes.toml configuration"
        else
            print_fail "ConfigMap missing rusmes.toml data"
        fi
    else
        print_fail "ConfigMap 'rusmes-config' not found"
    fi
}

# Check Secrets
check_secrets() {
    print_section "Checking Secrets"

    secrets=("rusmes-tls" "rusmes-db" "rusmes-admin" "rusmes-dkim")

    for secret in "${secrets[@]}"; do
        if kubectl get secret "$secret" -n "$NAMESPACE" &> /dev/null; then
            print_pass "Secret '$secret' exists"

            # Check if secret contains CHANGEME (security risk)
            if kubectl get secret "$secret" -n "$NAMESPACE" -o yaml | grep -q "CHANGEME"; then
                print_warn "Secret '$secret' contains CHANGEME placeholder - update for production!"
            fi
        else
            print_fail "Secret '$secret' not found"
        fi
    done
}

# Check StatefulSet
check_statefulset() {
    print_section "Checking StatefulSet"

    if kubectl get statefulset rusmes -n "$NAMESPACE" &> /dev/null; then
        print_pass "StatefulSet 'rusmes' exists"

        # Check replicas
        desired=$(kubectl get statefulset rusmes -n "$NAMESPACE" -o jsonpath='{.spec.replicas}')
        ready=$(kubectl get statefulset rusmes -n "$NAMESPACE" -o jsonpath='{.status.readyReplicas}')

        print_info "Desired replicas: $desired, Ready replicas: ${ready:-0}"

        if [ "$desired" == "${ready:-0}" ]; then
            print_pass "All replicas are ready"
        else
            print_warn "Not all replicas are ready ($ready/$desired)"
        fi
    else
        print_fail "StatefulSet 'rusmes' not found"
    fi
}

# Check Pods
check_pods() {
    print_section "Checking Pods"

    pods=$(kubectl get pods -n "$NAMESPACE" -l app=rusmes --no-headers 2>/dev/null | wc -l)

    if [ "$pods" -gt 0 ]; then
        print_pass "Found $pods RusMES pod(s)"

        # Check pod status
        kubectl get pods -n "$NAMESPACE" -l app=rusmes --no-headers | while read -r line; do
            pod_name=$(echo "$line" | awk '{print $1}')
            pod_status=$(echo "$line" | awk '{print $3}')

            if [ "$pod_status" == "Running" ]; then
                print_pass "Pod '$pod_name' is Running"
            else
                print_warn "Pod '$pod_name' status: $pod_status"
            fi
        done
    else
        print_fail "No RusMES pods found"
    fi
}

# Check Services
check_services() {
    print_section "Checking Services"

    services=("rusmes-smtp" "rusmes-imap" "rusmes-pop3" "rusmes-jmap" "rusmes-metrics" "rusmes-headless")

    for service in "${services[@]}"; do
        if kubectl get service "$service" -n "$NAMESPACE" &> /dev/null; then
            print_pass "Service '$service' exists"

            # Check LoadBalancer services
            svc_type=$(kubectl get service "$service" -n "$NAMESPACE" -o jsonpath='{.spec.type}')
            if [ "$svc_type" == "LoadBalancer" ]; then
                external_ip=$(kubectl get service "$service" -n "$NAMESPACE" -o jsonpath='{.status.loadBalancer.ingress[0].ip}')
                if [ -n "$external_ip" ]; then
                    print_info "  External IP: $external_ip"
                else
                    print_warn "  LoadBalancer pending (no external IP yet)"
                fi
            fi
        else
            print_fail "Service '$service' not found"
        fi
    done
}

# Check PVCs
check_pvcs() {
    print_section "Checking PersistentVolumeClaims"

    pvcs=$(kubectl get pvc -n "$NAMESPACE" --no-headers 2>/dev/null | wc -l)

    if [ "$pvcs" -gt 0 ]; then
        print_pass "Found $pvcs PersistentVolumeClaim(s)"

        kubectl get pvc -n "$NAMESPACE" --no-headers | while read -r line; do
            pvc_name=$(echo "$line" | awk '{print $1}')
            pvc_status=$(echo "$line" | awk '{print $2}')
            pvc_size=$(echo "$line" | awk '{print $4}')

            if [ "$pvc_status" == "Bound" ]; then
                print_pass "PVC '$pvc_name' is Bound ($pvc_size)"
            else
                print_warn "PVC '$pvc_name' status: $pvc_status"
            fi
        done
    else
        print_warn "No PersistentVolumeClaims found"
    fi
}

# Check Ingress
check_ingress() {
    print_section "Checking Ingress"

    ingresses=("rusmes-jmap" "rusmes-metrics")

    for ingress in "${ingresses[@]}"; do
        if kubectl get ingress "$ingress" -n "$NAMESPACE" &> /dev/null; then
            print_pass "Ingress '$ingress' exists"

            # Check hosts
            hosts=$(kubectl get ingress "$ingress" -n "$NAMESPACE" -o jsonpath='{.spec.rules[*].host}')
            print_info "  Hosts: $hosts"
        else
            print_warn "Ingress '$ingress' not found (optional)"
        fi
    done
}

# Check PodDisruptionBudget
check_pdb() {
    print_section "Checking PodDisruptionBudget"

    if kubectl get pdb rusmes -n "$NAMESPACE" &> /dev/null; then
        print_pass "PodDisruptionBudget 'rusmes' exists"
    else
        print_warn "PodDisruptionBudget 'rusmes' not found (optional)"
    fi
}

# Test connectivity
test_connectivity() {
    print_section "Testing Connectivity"

    # Get first running pod
    pod=$(kubectl get pods -n "$NAMESPACE" -l app=rusmes --field-selector=status.phase=Running -o jsonpath='{.items[0].metadata.name}' 2>/dev/null)

    if [ -n "$pod" ]; then
        print_info "Testing connectivity from pod: $pod"

        # Test SMTP port
        if kubectl exec "$pod" -n "$NAMESPACE" -- nc -zv localhost 25 &> /dev/null; then
            print_pass "SMTP port (25) is listening"
        else
            print_fail "SMTP port (25) is not accessible"
        fi

        # Test IMAP port
        if kubectl exec "$pod" -n "$NAMESPACE" -- nc -zv localhost 143 &> /dev/null; then
            print_pass "IMAP port (143) is listening"
        else
            print_fail "IMAP port (143) is not accessible"
        fi

        # Test JMAP port
        if kubectl exec "$pod" -n "$NAMESPACE" -- nc -zv localhost 8080 &> /dev/null; then
            print_pass "JMAP port (8080) is listening"
        else
            print_fail "JMAP port (8080) is not accessible"
        fi

        # Test metrics endpoint
        if kubectl exec "$pod" -n "$NAMESPACE" -- wget -q -O- http://localhost:9090/metrics | head -n 1 &> /dev/null; then
            print_pass "Metrics endpoint is accessible"
        else
            print_fail "Metrics endpoint is not accessible"
        fi
    else
        print_warn "No running pods found, skipping connectivity tests"
    fi
}

# Show summary
show_summary() {
    print_section "Validation Summary"

    echo ""
    echo "Total checks:"
    echo -e "  ${GREEN}Passed:${NC}   $PASSED"
    echo -e "  ${RED}Failed:${NC}   $FAILED"
    echo -e "  ${YELLOW}Warnings:${NC} $WARNINGS"
    echo ""

    if [ "$FAILED" -eq 0 ]; then
        print_pass "All critical checks passed!"
        if [ "$WARNINGS" -gt 0 ]; then
            print_warn "There are $WARNINGS warning(s) to address"
        fi
        return 0
    else
        print_fail "Validation failed with $FAILED error(s)"
        return 1
    fi
}

# Main validation
main() {
    echo ""
    echo "=========================================="
    echo "  RusMES Kubernetes Validation"
    echo "=========================================="

    check_namespace || exit 1
    check_rbac
    check_configmap
    check_secrets
    check_statefulset
    check_pods
    check_services
    check_pvcs
    check_ingress
    check_pdb
    test_connectivity
    show_summary
}

# Run main function
main

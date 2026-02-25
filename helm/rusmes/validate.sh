#!/bin/bash
# Helm Chart Validation Script

set -e

CHART_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHART_NAME="rusmes"

echo "========================================="
echo "RusMES Helm Chart Validation"
echo "========================================="
echo ""

# Check if helm is installed
if ! command -v helm &> /dev/null; then
    echo "ERROR: helm is not installed"
    exit 1
fi

echo "Helm version:"
helm version --short
echo ""

# Check if kubectl is available (optional)
if command -v kubectl &> /dev/null; then
    echo "kubectl version:"
    kubectl version --client --short 2>/dev/null || kubectl version --client
    echo ""
fi

# 1. Lint the chart
echo "========================================="
echo "1. Linting Chart"
echo "========================================="
helm lint "$CHART_DIR"
echo "✓ Chart lint passed"
echo ""

# 2. Lint with production values
echo "========================================="
echo "2. Linting with Production Values"
echo "========================================="
if [ -f "$CHART_DIR/values-production.yaml" ]; then
    helm lint "$CHART_DIR" --values "$CHART_DIR/values-production.yaml"
    echo "✓ Production values lint passed"
else
    echo "⚠ Production values file not found, skipping"
fi
echo ""

# 3. Lint with development values
echo "========================================="
echo "3. Linting with Development Values"
echo "========================================="
if [ -f "$CHART_DIR/values-development.yaml" ]; then
    helm lint "$CHART_DIR" --values "$CHART_DIR/values-development.yaml"
    echo "✓ Development values lint passed"
else
    echo "⚠ Development values file not found, skipping"
fi
echo ""

# 4. Template rendering test
echo "========================================="
echo "4. Template Rendering Test"
echo "========================================="
helm template test "$CHART_DIR" > /dev/null
echo "✓ Template rendering passed"
echo ""

# 5. Template with production values
echo "========================================="
echo "5. Template with Production Values"
echo "========================================="
if [ -f "$CHART_DIR/values-production.yaml" ]; then
    helm template test "$CHART_DIR" --values "$CHART_DIR/values-production.yaml" > /dev/null
    echo "✓ Production template rendering passed"
else
    echo "⚠ Production values file not found, skipping"
fi
echo ""

# 6. Dry run installation
echo "========================================="
echo "6. Dry Run Installation"
echo "========================================="
helm install test "$CHART_DIR" --dry-run --debug > /dev/null 2>&1
echo "✓ Dry run installation passed"
echo ""

# 7. Check for required files
echo "========================================="
echo "7. Checking Required Files"
echo "========================================="

REQUIRED_FILES=(
    "Chart.yaml"
    "values.yaml"
    ".helmignore"
    "README.md"
    "templates/_helpers.tpl"
    "templates/NOTES.txt"
    "templates/configmap.yaml"
    "templates/secrets.yaml"
    "templates/statefulset.yaml"
    "templates/service.yaml"
    "templates/ingress.yaml"
    "templates/hpa.yaml"
    "templates/pvc.yaml"
    "templates/rbac.yaml"
    "templates/serviceaccount.yaml"
    "templates/pdb.yaml"
    "templates/networkpolicy.yaml"
)

for file in "${REQUIRED_FILES[@]}"; do
    if [ -f "$CHART_DIR/$file" ]; then
        echo "✓ $file exists"
    else
        echo "✗ $file is missing"
        exit 1
    fi
done
echo ""

# 8. Validate Chart.yaml
echo "========================================="
echo "8. Validating Chart.yaml"
echo "========================================="

if grep -q "^name: $CHART_NAME" "$CHART_DIR/Chart.yaml"; then
    echo "✓ Chart name is correct"
else
    echo "✗ Chart name is incorrect"
    exit 1
fi

if grep -q "^version:" "$CHART_DIR/Chart.yaml"; then
    echo "✓ Chart version is set"
else
    echo "✗ Chart version is missing"
    exit 1
fi

if grep -q "^appVersion:" "$CHART_DIR/Chart.yaml"; then
    echo "✓ App version is set"
else
    echo "✗ App version is missing"
    exit 1
fi
echo ""

# 9. Check template syntax
echo "========================================="
echo "9. Checking Template Syntax"
echo "========================================="

for template in "$CHART_DIR/templates"/*.yaml; do
    if [ -f "$template" ]; then
        filename=$(basename "$template")
        # Try to render the template
        if helm template test "$CHART_DIR" --show-only "templates/$filename" > /dev/null 2>&1; then
            echo "✓ $filename syntax is valid"
        else
            echo "✗ $filename has syntax errors"
            exit 1
        fi
    fi
done
echo ""

# 10. Validate YAML syntax
echo "========================================="
echo "10. Validating YAML Syntax"
echo "========================================="

if command -v yamllint &> /dev/null; then
    yamllint -c - "$CHART_DIR" <<EOF
extends: default
rules:
  line-length:
    max: 120
    level: warning
  document-start: disable
  truthy:
    allowed-values: ['true', 'false', 'on', 'off']
EOF
    echo "✓ YAML syntax validation passed"
else
    echo "⚠ yamllint not installed, skipping YAML validation"
fi
echo ""

# 11. Package the chart
echo "========================================="
echo "11. Packaging Chart"
echo "========================================="

PACKAGE_DIR=$(mktemp -d)
helm package "$CHART_DIR" -d "$PACKAGE_DIR" > /dev/null
echo "✓ Chart packaged successfully"
echo "Package location: $PACKAGE_DIR/${CHART_NAME}-*.tgz"
rm -rf "$PACKAGE_DIR"
echo ""

# 12. Summary
echo "========================================="
echo "Validation Summary"
echo "========================================="
echo "✓ All validation checks passed"
echo ""
echo "Chart is ready for deployment!"
echo ""
echo "Next steps:"
echo "  1. helm install $CHART_NAME $CHART_DIR"
echo "  2. helm package $CHART_DIR"
echo "  3. Upload to chart repository"
echo ""

#!/bin/bash
# Verify fuzz testing setup for rusmes

set -e

FUZZ_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$FUZZ_DIR"

echo "========================================="
echo "Rusmes Fuzz Testing Setup Verification"
echo "========================================="
echo ""

# Check for nightly toolchain
echo "Checking Rust nightly toolchain..."
if ! rustup toolchain list | grep -q nightly; then
    echo "✗ Nightly toolchain not found"
    echo "  Install with: rustup install nightly"
    exit 1
fi
echo "✓ Nightly toolchain found"
echo ""

# Check for cargo-fuzz
echo "Checking cargo-fuzz installation..."
if ! command -v cargo-fuzz &> /dev/null; then
    echo "✗ cargo-fuzz not found"
    echo "  Install with: cargo install cargo-fuzz"
    exit 1
fi
echo "✓ cargo-fuzz found: $(cargo fuzz --version)"
echo ""

# List fuzz targets
echo "Listing fuzz targets..."
TARGETS=$(cargo +nightly fuzz list)
echo "$TARGETS"
echo ""

TARGET_COUNT=$(echo "$TARGETS" | wc -l)
EXPECTED_COUNT=6

if [ "$TARGET_COUNT" -ne "$EXPECTED_COUNT" ]; then
    echo "✗ Expected $EXPECTED_COUNT targets, found $TARGET_COUNT"
    exit 1
fi
echo "✓ All $EXPECTED_COUNT fuzz targets found"
echo ""

# Check corpus directories
echo "Checking corpus directories..."
CORPUS_OK=true
for target in $TARGETS; do
    if [ ! -d "corpus/$target" ]; then
        echo "✗ Corpus directory missing: corpus/$target"
        CORPUS_OK=false
    else
        CORPUS_FILES=$(ls -1 "corpus/$target" 2>/dev/null | wc -l)
        echo "  ✓ corpus/$target ($CORPUS_FILES seed files)"
    fi
done

if [ "$CORPUS_OK" = false ]; then
    exit 1
fi
echo ""

# Test compilation
echo "Testing compilation of all fuzz targets..."
if ! cargo +nightly check --quiet 2>&1 | grep -E "(error|warning)" | head -20; then
    echo "✓ All fuzz targets compile successfully"
else
    echo "Note: Some warnings detected (see above)"
fi
echo ""

# Quick smoke test (1 iteration each)
echo "Running smoke tests (1 iteration each)..."
SMOKE_FAILED=false
for target in $TARGETS; do
    echo -n "  Testing $target... "
    if timeout 30 cargo +nightly fuzz run "$target" -- -runs=1 -max_total_time=5 >/dev/null 2>&1; then
        echo "✓"
    else
        echo "✗ FAILED"
        SMOKE_FAILED=true
    fi
done
echo ""

if [ "$SMOKE_FAILED" = true ]; then
    echo "✗ Some smoke tests failed"
    echo "  This may indicate a bug in the fuzz target or parser"
    exit 1
fi

echo "========================================="
echo "Setup Verification Complete"
echo "========================================="
echo ""
echo "All checks passed! You can now run:"
echo "  ./run_all_fuzz_tests.sh          # Run all tests for 1 hour each"
echo "  cargo +nightly fuzz run <target> # Run a specific target"
echo ""
echo "Available targets:"
for target in $TARGETS; do
    echo "  - $target"
done
echo ""
echo "For more information, see README.md"

#!/bin/bash
# Run all fuzz tests for rusmes parsers
# Usage: ./run_all_fuzz_tests.sh [time_in_seconds]

set -e

FUZZ_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$FUZZ_DIR"

# Default to 1 hour (3600 seconds) if not specified
TIME_LIMIT="${1:-3600}"

echo "========================================="
echo "Rusmes Fuzz Testing Suite"
echo "========================================="
echo "Time limit per target: ${TIME_LIMIT} seconds"
echo "========================================="
echo ""

TARGETS=(
    fuzz_smtp_parser
    fuzz_imap_parser
    fuzz_mime_parser
    fuzz_email_address
    fuzz_sieve_parser
    fuzz_jmap_json
)

FAILED_TARGETS=()
PASSED_TARGETS=()

for target in "${TARGETS[@]}"; do
    echo "========================================="
    echo "Fuzzing: $target"
    echo "========================================="

    if cargo +nightly fuzz run "$target" -- -max_total_time="$TIME_LIMIT"; then
        echo "✓ $target completed successfully"
        PASSED_TARGETS+=("$target")
    else
        echo "✗ CRASH FOUND in $target!"
        echo "  Artifacts saved to artifacts/$target/"
        FAILED_TARGETS+=("$target")
    fi

    echo ""
done

echo "========================================="
echo "Fuzz Testing Summary"
echo "========================================="
echo "Passed: ${#PASSED_TARGETS[@]}/${#TARGETS[@]}"
echo "Failed: ${#FAILED_TARGETS[@]}/${#TARGETS[@]}"
echo ""

if [ ${#PASSED_TARGETS[@]} -gt 0 ]; then
    echo "Passed targets:"
    for target in "${PASSED_TARGETS[@]}"; do
        echo "  ✓ $target"
    done
    echo ""
fi

if [ ${#FAILED_TARGETS[@]} -gt 0 ]; then
    echo "Failed targets (crashes found):"
    for target in "${FAILED_TARGETS[@]}"; do
        echo "  ✗ $target"
    done
    echo ""
    echo "To reproduce crashes:"
    for target in "${FAILED_TARGETS[@]}"; do
        echo "  cargo +nightly fuzz run $target artifacts/$target/crash-*"
    done
    exit 1
fi

echo "========================================="
echo "All fuzz targets passed!"
echo "========================================="

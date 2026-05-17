#!/usr/bin/env bash
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

# Dry-run mode must report bootstrap_turbo step without executing it.
output=$(bash "$SCRIPT_DIR/install.sh" --dry-run --skill autospec --harness claude 2>&1 || true)

case "$output" in
    *bootstrap_turbo*) ;;
    *) echo "FAIL: bootstrap_turbo step not reported in dry-run output"; echo "----"; echo "$output"; exit 1 ;;
esac

case "$output" in
    *tobihagemann/turbo*) ;;
    *) echo "FAIL: turbo repo not mentioned in dry-run output"; exit 1 ;;
esac

# Dry-run must not actually clone into a marker location.
if [ -d "$HOME/.turbo/repo-test-marker" ]; then
    echo "FAIL: dry-run made filesystem changes at marker location"
    exit 1
fi

echo "PASS"

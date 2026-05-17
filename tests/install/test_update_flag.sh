#!/usr/bin/env bash
# Verifies --update triggers both autospec pull and turbo pull steps.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

output=$(bash "$SCRIPT_DIR/install.sh" --update --dry-run --skill autospec --harness claude 2>&1 || true)

case "$output" in
    *pull_autospec*) ;;
    *) echo "FAIL: --update did not invoke pull_autospec"; echo "$output"; exit 1 ;;
esac

case "$output" in
    *"would git pull --ff-only"*) ;;
    *) echo "FAIL: pull_autospec dry-run message missing"; exit 1 ;;
esac

case "$output" in
    *bootstrap_turbo*) ;;
    *) echo "FAIL: --update did not invoke bootstrap_turbo"; exit 1 ;;
esac

# Without --update, pull_autospec must be a no-op (returns silently).
output_no_update=$(bash "$SCRIPT_DIR/install.sh" --dry-run --skill autospec --harness claude 2>&1 || true)
case "$output_no_update" in
    *pull_autospec*) echo "FAIL: pull_autospec ran without --update"; exit 1 ;;
esac

echo "PASS"

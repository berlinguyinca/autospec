#!/usr/bin/env bash
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

output=$(bash "$SCRIPT_DIR/install.sh" --dry-run --skill autospec --harness claude 2>&1 || true)

for expected in \
    "ensure_system_tools" \
    "bootstrap_superpowers" \
    "bootstrap_oh_my_codex" \
    "bootstrap_oh_my_opencode" \
    "bootstrap_oh_my_claude"
do
    case "$output" in
        *"$expected"*) ;;
        *) echo "FAIL: $expected step not reported in dry-run output"; echo "----"; echo "$output"; exit 1 ;;
    esac
done

case "$output" in
    *"obra/superpowers"*) ;;
    *) echo "FAIL: superpowers repo not mentioned in dry-run output"; exit 1 ;;
esac

echo "PASS"

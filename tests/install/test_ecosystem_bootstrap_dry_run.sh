#!/usr/bin/env bash
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

output=$(bash "$SCRIPT_DIR/install.sh" --dry-run --skill autospec --harness claude 2>&1 || true)

for expected in "ensure_system_tools" "bootstrap_superpowers" "obra/superpowers"; do
    case "$output" in
        *"$expected"*) ;;
        *) echo "FAIL: $expected not reported in dry-run output"; echo "$output"; exit 1 ;;
    esac
done

# The oh-my-* peers are no longer installed or set up by the installer.
if printf '%s' "$output" | grep -q -E 'bootstrap_oh_my|oh-my-(codex|opencode|claude)|(omx|omc) setup'; then
    echo "FAIL: dry-run output still mentions an oh-my peer"; exit 1
fi

echo "PASS"

#!/usr/bin/env bash
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

output=$(bash "$SCRIPT_DIR/install.sh" --dry-run --skill autospec --harness claude 2>&1 || true)

for expected in \
    "ensure_system_tools" \
    "bootstrap_superpowers"
do
    case "$output" in
        *"$expected"*) ;;
        *) echo "FAIL: $expected step not reported in dry-run output"; echo "----"; echo "$output"; exit 1 ;;
    esac
done

# The oh-my-* peers are no longer installed or set up by the installer.
for forbidden in \
    "bootstrap_oh_my" \
    "oh-my-codex" \
    "oh-my-opencode" \
    "oh-my-claude" \
    "omx setup" \
    "omc setup"
do
    case "$output" in
        *"$forbidden"*) echo "FAIL: dry-run output still mentions $forbidden"; echo "----"; echo "$output"; exit 1 ;;
    esac
done

case "$output" in
    *"obra/superpowers"*) ;;
    *) echo "FAIL: superpowers repo not mentioned in dry-run output"; exit 1 ;;
esac

echo "PASS"

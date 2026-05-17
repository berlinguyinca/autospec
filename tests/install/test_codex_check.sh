#!/usr/bin/env bash
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"

# Force codex absence via PATH manipulation.
output=$(PATH="/usr/bin:/bin" bash "$SCRIPT_DIR/install.sh" --dry-run --skill autospec --harness claude 2>&1 || true)

case "$output" in
    *check_codex*) ;;
    *) echo "FAIL: check_codex step not reported"; echo "----"; echo "$output"; exit 1 ;;
esac

case "$output" in
    *"codex CLI NOT found"*) ;;
    *) echo "FAIL: codex-absence message missing"; exit 1 ;;
esac

case "$output" in
    *"peer-review will skip gracefully"*) ;;
    *) echo "FAIL: graceful-skip note missing"; exit 1 ;;
esac

echo "PASS"

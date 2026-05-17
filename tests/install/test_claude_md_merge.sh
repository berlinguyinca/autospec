#!/usr/bin/env bash
# Verifies that the canonical CLAUDE.md block content can be idempotently merged
# via merge_marked_block. Tests the *content* + *helper* contract; the install.sh
# call site is exercised by test_install_dry_run.sh.
set -eu
SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=../../scripts/lib/install-helpers.sh
. "$SCRIPT_DIR/scripts/lib/install-helpers.sh"

block_file="$SCRIPT_DIR/scripts/lib/claude-md-block.txt"
[ -f "$block_file" ] || { echo "FAIL: $block_file missing"; exit 1; }

target=$(mktemp)
trap 'rm -f "$target"' EXIT
echo "# Existing content" > "$target"

content="$(cat "$block_file")"
merge_marked_block "$target" "autospec-block" "$content"

grep -q "<!-- autospec-block -->" "$target" || { echo "FAIL: marker missing"; exit 1; }
grep -q "Existing content" "$target" || { echo "FAIL: pre-existing content lost"; exit 1; }
grep -q "autospec + turbo integration" "$target" || { echo "FAIL: block content missing"; exit 1; }

# Idempotency
merge_marked_block "$target" "autospec-block" "$content"
count=$(grep -c "<!-- autospec-block -->" "$target")
[ "$count" -eq 1 ] || { echo "FAIL: marker duplicated (count=$count)"; exit 1; }

# In-place replacement
merge_marked_block "$target" "autospec-block" "updated content marker"
grep -q "updated content marker" "$target" || { echo "FAIL: content not replaced"; exit 1; }
grep -q "autospec + turbo integration" "$target" && { echo "FAIL: old content not removed"; exit 1; }

echo "PASS"

#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$SCRIPT_DIR/scripts/lib/install-helpers.sh"

# Test: merge_marked_block writes a fenced block into a target file idempotently
tmpfile=$(mktemp)
echo "existing content" > "$tmpfile"

merge_marked_block "$tmpfile" "autospec-block" "test content"
grep -q "<!-- autospec-block -->" "$tmpfile" || { echo "FAIL: marker not added"; exit 1; }
grep -q "test content" "$tmpfile" || { echo "FAIL: content not added"; exit 1; }

# Re-running with same content must not duplicate
merge_marked_block "$tmpfile" "autospec-block" "test content"
count=$(grep -c "<!-- autospec-block -->" "$tmpfile")
[[ "$count" -eq 1 ]] || { echo "FAIL: marker duplicated (count=$count)"; exit 1; }

# Re-running with different content must replace in place
merge_marked_block "$tmpfile" "autospec-block" "updated content"
grep -q "updated content" "$tmpfile" || { echo "FAIL: content not updated"; exit 1; }
! grep -q "test content" "$tmpfile" || { echo "FAIL: old content not removed"; exit 1; }

rm "$tmpfile"

# Test: incomplete-block input gets repaired (no closing marker synthesized and new content inserted)
tmpfile2=$(mktemp)
cat > "$tmpfile2" <<'EOF'
header line
<!-- testmarker -->
old content (no closing tag)
EOF
merge_marked_block "$tmpfile2" "testmarker" "fresh content"
grep -q "fresh content" "$tmpfile2" || { echo "FAIL: new content not inserted into incomplete block"; exit 1; }
grep -q "<!-- /testmarker -->" "$tmpfile2" || { echo "FAIL: closing marker not synthesized"; exit 1; }
grep -q "header line" "$tmpfile2" || { echo "FAIL: content before block lost after incomplete-block repair"; exit 1; }
rm "$tmpfile2"

echo "PASS"

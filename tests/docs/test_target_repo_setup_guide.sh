#!/usr/bin/env bash
# Verifies docs/target-repo-setup.md (Change γ from
# docs/specs/2026-05-17-cross-session-ci-rot-design.md) exists and contains
# the required content elements: a reference to issue #307, the 4
# migration-replay convention strings verbatim, the verification one-liner
# (required_status_checks predicate), and a Verification block.

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
DOC="$SCRIPT_DIR/docs/target-repo-setup.md"
README="$SCRIPT_DIR/README.md"

fail() { echo "FAIL: $*" >&2; exit 1; }

[ -f "$DOC" ] || fail "doc missing at $DOC"

# Required content elements.
grep -qF "issue #307" "$DOC" || fail "doc must reference issue #307"
grep -qF "make migrate-test" "$DOC"      || fail "doc must include 'make migrate-test' verbatim"
grep -qF "npm run migrate:test" "$DOC"   || fail "doc must include 'npm run migrate:test' verbatim"
grep -qF "bin/migrate-test" "$DOC"       || fail "doc must include 'bin/migrate-test' verbatim"
grep -qF "pytest tests/migrations" "$DOC" || fail "doc must include 'pytest tests/migrations' verbatim"
grep -qF "required_status_checks" "$DOC" || fail "doc must include 'required_status_checks' for the verification block"

# Verification one-liner must include the gh api branch-protection call.
grep -qF "gh api repos/" "$DOC" \
    || fail "doc must include the gh api branch-protection one-liner"

# Four numbered sections per spec § Change γ outline.
for heading in \
    "Required branch protection on" \
    "Migration-replay test convention" \
    "Why this matters" \
    "Verification"; do
    grep -qF "$heading" "$DOC" || fail "doc missing section: '$heading'"
done

# README link.
[ -f "$README" ] || fail "README.md missing"
grep -qF "target-repo-setup" "$README" \
    || fail "README must link to docs/target-repo-setup.md"

# The README link must appear inside the Install section (i.e. after the
# '## Install' heading and before the next '## ' heading).
install_line=$(grep -n "^## Install" "$README" | head -1 | cut -d: -f1)
[ -n "$install_line" ] || fail "README missing '## Install' heading"
next_h2_line=$(awk -v start="$install_line" 'NR>start && /^## / {print NR; exit}' "$README")
link_line=$(grep -n "target-repo-setup" "$README" | head -1 | cut -d: -f1)
[ -n "$link_line" ] || fail "README must link to docs/target-repo-setup.md"
[ "$link_line" -gt "$install_line" ] || fail "README link must appear after '## Install'"
if [ -n "$next_h2_line" ]; then
    [ "$link_line" -lt "$next_h2_line" ] \
        || fail "README link must appear inside the Install section (before next '## ' heading)"
fi

echo "PASS"

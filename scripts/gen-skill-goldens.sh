#!/usr/bin/env bash
# gen-skill-goldens.sh — regenerate the block-expansion golden sha256 snapshots.
#
# validate.sh's check_block_expansion gate expands every markered trio member
# through scripts/expand-skill-blocks.sh and compares its sha256 against a
# committed golden at tests/fixtures/skill-goldens/<skill>.<suffix>.sha256
# (suffix one of: SKILL.md | codex.prompt.md | opencode.agent.md). This script
# regenerates those goldens with the exact same expander + hash one-liner:
#
#   expand-skill-blocks.sh <member> | shasum -a 256 | cut -d' ' -f1 \
#     > tests/fixtures/skill-goldens/<skill>.<suffix>.sha256
#
# A golden is written for SKILL.md whenever the skill has one, and for the
# codex/opencode mirrors only when they carry an autospec-block marker (matching
# check_block_expansion's "markered member requires a golden" rule). Writes are
# idempotent: a golden whose content already matches is left untouched, so a
# no-op run produces no diff.
#
# Usage:
#   gen-skill-goldens.sh                 # regenerate goldens for every skill
#   gen-skill-goldens.sh <skill>...      # only the named skills (bare names)
#   gen-skill-goldens.sh -h | --help
#
# Exit codes:
#   0  success
#   2  usage error / missing expander
#   3  a named skill does not exist
#
# Conventions: set -u (no -e — explicit checks); if/then/fi for one-sided
# conditionals; no RETURN traps (repo bash 3.2 gotchas).
set -u

PROG="$(basename "$0")"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
EXPANDER="$REPO_ROOT/scripts/expand-skill-blocks.sh"
GOLDEN_DIR="$REPO_ROOT/tests/fixtures/skill-goldens"

usage() {
    cat <<EOF
$PROG — regenerate block-expansion golden sha256 snapshots.

Usage:
  $PROG                 Regenerate goldens for every skill under skills/.
  $PROG <skill>...      Regenerate goldens only for the named skills.
  $PROG -h | --help     Show this help.

For each markered trio member it writes
  tests/fixtures/skill-goldens/<skill>.<suffix>.sha256
matching validate.sh's check_block_expansion. SKILL.md always gets a golden;
codex/opencode mirrors get one when they carry an autospec-block marker, or when
a golden already exists for them.
Idempotent: an already-current golden is left untouched.
EOF
}

die() {  # die <code> <message>
    code="$1"; shift
    printf '%s: %s\n' "$PROG" "$*" >&2
    exit "$code"
}

# Whether a file carries an autospec-block marker (so check_block_expansion
# requires a golden for it).
has_marker() {  # has_marker <file>
    grep -q '<!-- autospec-block:' "$1" 2>/dev/null
}

# Regenerate one golden if needed. Echoes a one-line status. The hash one-liner
# is byte-for-byte the one check_block_expansion runs: shasum -a 256 | cut.
gen_one() {  # gen_one <skill> <src-file> <suffix>
    skill="$1"; src="$2"; suffix="$3"
    [ -f "$src" ] || return 0
    golden="$GOLDEN_DIR/${skill}.${suffix}.sha256"
    new_hash="$(bash "$EXPANDER" "$src" | shasum -a 256 | cut -d' ' -f1)"
    if [ -f "$golden" ]; then
        old_hash="$(tr -d '[:space:]' < "$golden")"
        if [ "$old_hash" = "$new_hash" ]; then
            return 0  # already current — no write, no diff
        fi
    fi
    printf '%s\n' "$new_hash" > "$golden"
    printf '%s: wrote %s\n' "$PROG" "$golden"
}

# Regenerate a mirror's golden when the gate would compare one: either the member
# carries a marker, or a golden already exists for it.
gen_mirror() {  # gen_mirror <skill> <src-file> <suffix>
    skill="$1"; src="$2"; suffix="$3"
    [ -f "$src" ] || return 0
    if has_marker "$src" || [ -f "$GOLDEN_DIR/${skill}.${suffix}.sha256" ]; then
        gen_one "$skill" "$src" "$suffix"
    fi
}

# Regenerate every applicable golden for one skill directory.
gen_skill() {  # gen_skill <skill-dir>
    dir="${1%/}"
    skill="$(basename "$dir")"
    # SKILL.md: always gets a golden when present (check_block_expansion fails
    # closed on a missing SKILL.md golden).
    gen_one "$skill" "$dir/SKILL.md" "SKILL.md"
    # codex/opencode mirrors: a markered member needs a golden, and so does any
    # member that already has one. check_block_expansion compares every golden it
    # finds, marker or not, so refreshing only the markered ones left goldens the
    # gate still checks and this tool would never update — stale and silent until
    # some unrelated change tripped them.
    gen_mirror "$skill" "$dir/codex/prompt.md" "codex.prompt.md"
    gen_mirror "$skill" "$dir/opencode/agent.md" "opencode.agent.md"
}

main() {
    names=""
    while [ $# -gt 0 ]; do
        case "$1" in
            -h|--help) usage; exit 0 ;;
            --) shift; break ;;
            -*) die 2 "unknown option: $1" ;;
            *)  names="${names}${names:+ }$1"; shift ;;
        esac
    done
    for arg in "$@"; do
        names="${names}${names:+ }$arg"
    done

    [ -f "$EXPANDER" ] || die 2 "expander missing: $EXPANDER"
    [ -d "$GOLDEN_DIR" ] || die 2 "golden dir missing: $GOLDEN_DIR"

    if [ -n "$names" ]; then
        for name in $names; do
            dir="$REPO_ROOT/skills/$name"
            [ -d "$dir" ] || die 3 "no such skill: skills/$name"
            gen_skill "$dir"
        done
    else
        for dir in "$REPO_ROOT"/skills/*/; do
            [ -d "$dir" ] || continue
            [ -f "$dir/SKILL.md" ] || continue
            gen_skill "$dir"
        done
    fi
}

main "$@"

#!/usr/bin/env bash
# tests/install/test_skill_reference_shipping.sh
#
# check_reference_pointer_integrity proves each installer DECLARES every file
# under references/. It cannot prove the installer SHIPS them: delete the
# install_reference_files call and that gate, every cargo test, and validate all
# stay green. This test closes that gap by installing into a throwaway HOME and
# looking for the files on disk.
#
# It drives the per-skill installer rather than the top-level install.sh, which is
# what the top-level delegates to anyway, and skips the runtime bootstrap the
# larger script performs.
set -eu

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TEST_HOME="$(mktemp -d -t autospec-ref-ship.XXXXXX)"
OUT="$(mktemp -t autospec-ref-ship-out.XXXXXX)"
cleanup() { rm -rf "$TEST_HOME" "$OUT"; }
trap cleanup EXIT INT TERM

fail() {
    echo "FAIL: $*"
    cat "$OUT"
    exit 1
}

# Every skill that owns a references/ directory, so a skill that grows one later
# is covered without editing this list.
for skill_dir in "$REPO_ROOT"/skills/*/; do
    skill="$(basename "$skill_dir")"
    [ -d "$skill_dir/references" ] || continue

    # Hidden entries are placeholders (.gitkeep) and are deliberately not shipped.
    files="$(cd "$skill_dir/references" && find . -type f -not -path '*/.*' | sed 's|^\./||' | sort)"
    [ -n "$files" ] || continue

    rm -rf "$TEST_HOME"
    mkdir -p "$TEST_HOME"
    HOME="$TEST_HOME" bash "$skill_dir/install.sh" --harness all >"$OUT" 2>&1 \
        || fail "$skill installer exited non-zero"

    for rel in $files; do
        # The harness-neutral root is the only copy OpenCode can reach.
        [ -f "$TEST_HOME/.autospec/skills/$skill/references/$rel" ] \
            || fail "$skill: references/$rel missing from the harness-neutral root"
        [ -f "$TEST_HOME/.claude/skills/$skill/references/$rel" ] \
            || fail "$skill: references/$rel missing beside the installed Claude SKILL.md"
        [ -f "$TEST_HOME/.codex/skills/$skill/references/$rel" ] \
            || fail "$skill: references/$rel missing beside the installed Codex SKILL.md"
    done
    echo "ok: $skill shipped $(printf '%s\n' "$files" | wc -l | tr -d ' ') reference file(s) to 3 destinations"
done

# Negative control: a skill with no references/ must not gain the directory, or
# the assertions above would pass for a script that blindly mkdir -p's.
rm -rf "$TEST_HOME"
mkdir -p "$TEST_HOME"
HOME="$TEST_HOME" bash "$REPO_ROOT/skills/autospec-stop/install.sh" --harness claude >"$OUT" 2>&1 \
    || fail "autospec-stop installer exited non-zero"
if [ -d "$TEST_HOME/.claude/skills/autospec-stop/references" ]; then
    fail "autospec-stop has no references/ in the repo but one was installed"
fi
echo "ok: a skill without references/ installs none"

echo "PASS: skill reference shipping"

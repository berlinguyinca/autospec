#!/usr/bin/env bats
# tests/route-decide-wiring.bats — guards issue #3179.
#
# scripts/route-decide.sh is documented in README.md, docs/USER_MANUAL.md, and
# docs/CONFIG_REFERENCE.md as the routing decision point. On the pre-fix tree,
# nothing in scripts/*.sh actually calls it — the only two references outside
# route-decide.sh itself (scripts/calibrate-profile.sh, scripts/verify-voter-
# vendor.sh) are prose comments, not invocations — while the docs claim it
# "makes the call". Either fact alone would be fine; both together are a
# documented-but-dead entry point (a correctness bug per AGENTS.md, not a docs
# nit).
#
# This test passes iff EITHER:
#   (a) a real executable caller exists under scripts/ (a non-comment line,
#       outside route-decide.sh itself, that invokes the script), OR
#   (b) every one of the three docs explicitly declares the script advisory.
# It must NOT pass on a bare `grep -rn route-decide scripts/` (that matches the
# comment-only references above) and must NOT pass on a doc file that merely
# mentions the script's name without the word "advisory".

REPO_ROOT="${BATS_TEST_DIRNAME}/.."

# Count files under scripts/ (excluding route-decide.sh itself) that reference
# route-decide.sh on a line that is NOT a comment. A pure comment mention (the
# counter-team's explicit "a comment counts as a caller" challenge) must not
# count as an executable caller.
count_executable_callers() {
    local count=0
    local f
    while IFS= read -r f; do
        [ "$(basename "$f")" = "route-decide.sh" ] && continue
        if grep -v '^[[:space:]]*#' "$f" | grep -q 'route-decide\.sh'; then
            count=$((count + 1))
        fi
    done < <(find "$REPO_ROOT/scripts" -maxdepth 1 -type f -name '*.sh')
    printf '%s' "$count"
}

# A doc declares the script advisory when it mentions route-decide.sh and the
# literal word "advisory" together in the same file.
doc_declares_advisory() {
    local doc="$1"
    grep -qi 'route-decide\.sh' "$doc" && grep -qi 'advisory' "$doc"
}

@test "route-decide.sh has an executable caller, or docs declare it advisory (primary AC guard)" {
    # This is the disjunction the acceptance criteria require. It must NOT be
    # satisfied by the comment-only references in scripts/calibrate-profile.sh
    # or scripts/verify-voter-vendor.sh (the counter-team's explicit "a comment
    # mentioning a script counts as a caller" challenge) — count_executable_callers
    # strips comment lines before counting.
    caller_count="$(count_executable_callers)"
    if [ "$caller_count" != "0" ]; then
        return 0
    fi
    for doc in "$REPO_ROOT/README.md" "$REPO_ROOT/docs/USER_MANUAL.md" "$REPO_ROOT/docs/CONFIG_REFERENCE.md"; do
        doc_declares_advisory "$doc" || {
            echo "no executable caller AND no advisory declaration in: $doc" >&2
            return 1
        }
    done
}

@test "docs/CONFIG_REFERENCE.md no longer claims route-decide.sh 'makes the call' without qualification" {
    caller_count="$(count_executable_callers)"
    if [ "$caller_count" != "0" ]; then
        skip "an executable caller exists; this doc-wording check does not apply"
    fi
    # The exact prior wording asserted wiring as fact. Once the script is
    # advisory, that phrasing must not survive unqualified in CONFIG_REFERENCE.md.
    run grep -n "route-decide.sh\` makes the call\." "$REPO_ROOT/docs/CONFIG_REFERENCE.md"
    [ "$status" -ne 0 ]
}

@test "no change to the decision logic inside scripts/route-decide.sh (out-of-scope guard)" {
    # route-decide.sh must remain executable and its documented CLI contract
    # (labels flag, --explain) must still be present verbatim.
    [ -x "$REPO_ROOT/scripts/route-decide.sh" ]
    grep -q -- '--labels' "$REPO_ROOT/scripts/route-decide.sh"
    grep -q -- '--explain' "$REPO_ROOT/scripts/route-decide.sh"
}

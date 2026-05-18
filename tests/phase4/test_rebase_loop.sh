#!/usr/bin/env bash
# Verifies the Phase 4 implementer prompts contain a rebase-and-retest
# pre-merge gate (change α from docs/specs/2026-05-17-cross-session-ci-rot-design.md).
#
# Strategy: this is a prompt-text test (the gate is executed by an LLM
# following the documented shell). We assert:
#   1. Both v2-flow prompt and legacy SKILL.md prompt contain the gate
#      keywords (mergeStateStatus, gh pr update-branch, BEHIND, DIRTY,
#      CLEAN, AUTOSPEC_REBASE_MAX_ATTEMPTS).
#   2. The gate is documented before gh pr merge --admin --squash.
#   3. The exact shell block extracted from the v2-flow prompt, when
#      executed under a fake `gh` shim, exhibits the documented control
#      flow for three scenarios:
#         (a) BEHIND on attempt 1 → CLEAN on attempt 2 → reaches merge.
#         (b) DIRTY → exits non-zero, posts an issue comment, no merge.
#         (c) BEHIND on three consecutive attempts → escalation comment,
#             non-zero exit, no merge.
#
# Test design follows tests/phase4/test_prompt_structure.sh conventions.

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
V2_PROMPT="$SCRIPT_DIR/skills/autospec-run/prompts/phase4-implementer.md"
LEGACY_PROMPT="$SCRIPT_DIR/skills/autospec-run/SKILL.md"

fail() { echo "FAIL: $*" >&2; exit 1; }

[ -f "$V2_PROMPT" ] || fail "v2-flow prompt missing at $V2_PROMPT"
[ -f "$LEGACY_PROMPT" ] || fail "legacy prompt missing at $LEGACY_PROMPT"

# --- 1. Keyword presence in both prompts -----------------------------------

for prompt in "$V2_PROMPT" "$LEGACY_PROMPT"; do
    name="$(basename "$(dirname "$prompt")")/$(basename "$prompt")"
    for kw in "mergeStateStatus" "gh pr update-branch" "BEHIND" "DIRTY" "CLEAN" "AUTOSPEC_REBASE_MAX_ATTEMPTS"; do
        grep -qF "$kw" "$prompt" || fail "$name: missing rebase-gate keyword '$kw'"
    done
done

# --- 2. Gate must appear before gh pr merge --admin --squash ---------------

for prompt in "$V2_PROMPT" "$LEGACY_PROMPT"; do
    name="$(basename "$(dirname "$prompt")")/$(basename "$prompt")"
    # Find the first gate occurrence and the last admin-squash merge invocation
    # (the prose may mention `gh pr merge --admin --squash` before the gate to
    # introduce it; the executable merge call lives after the gate block).
    gate_line=$(grep -n "mergeStateStatus" "$prompt" | head -1 | cut -d: -f1)
    merge_line=$(grep -n "gh pr merge .*--admin --squash" "$prompt" | tail -1 | cut -d: -f1)
    [ -n "$gate_line" ] || fail "$name: no gate line found"
    [ -n "$merge_line" ] || fail "$name: no admin-squash merge line found"
    [ "$gate_line" -lt "$merge_line" ] || fail "$name: gate must appear before admin-squash merge (gate=$gate_line, merge=$merge_line)"
done

# --- 3. Execute the gate shell block under a fake gh shim ------------------
# Extract the first fenced ```bash block in the v2-flow prompt that contains
# 'mergeStateStatus' AND a while loop — that's the gate.

extract_gate_block() {
    awk '
        /^```bash[[:space:]]*$/   { in_fence=1; buf=""; next }
        /^```[[:space:]]*$/ && in_fence {
            if (buf ~ /mergeStateStatus/ && buf ~ /while/) { print buf; exit }
            in_fence=0; buf=""; next
        }
        in_fence { buf = buf $0 "\n" }
    ' "$V2_PROMPT"
}

GATE_BLOCK="$(extract_gate_block)"
[ -n "$GATE_BLOCK" ] || fail "could not extract rebase-gate shell block from $V2_PROMPT"

# Make a tmp dir for shims and state files.
TMPDIR_GATE="$(mktemp -d)"
trap 'rm -rf "$TMPDIR_GATE"' EXIT

# Scenario runner: writes a fake `gh` to PATH, expands <PR>/<issue>
# placeholders to concrete values, and executes the gate. The fake gh emits
# a deterministic mergeStateStatus per call based on a counter, records
# every call to a log, and stubs the auxiliary commands the gate uses.
run_scenario() {
    sequence="$1"   # space-separated list, e.g. "BEHIND CLEAN"
    expect_exit="$2"
    expect_merge_count="$3"
    expect_comment_substr="$4"

    workdir="$TMPDIR_GATE/$5"
    mkdir -p "$workdir/bin"
    : > "$workdir/gh.log"
    : > "$workdir/comments.log"
    printf '%s\n' $sequence > "$workdir/state-sequence"

    cat > "$workdir/bin/gh" <<'GHSHIM'
#!/usr/bin/env bash
# fake gh — services these subcommands needed by the rebase gate:
#   gh pr view <PR> --json mergeStateStatus --jq .mergeStateStatus
#   gh pr view <PR> --json statusCheckRollup --jq ...
#   gh pr update-branch <PR>
#   gh pr merge <PR> --admin --squash --delete-branch
#   gh issue comment <N> --body ...
log="$WORKDIR/gh.log"
printf '%s\n' "gh $*" >> "$log"

if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    # Differentiate the mergeStateStatus query from the rollup query
    case "$*" in
        *mergeStateStatus*)
            seqfile="$WORKDIR/state-sequence"
            idxfile="$WORKDIR/state-index"
            idx=$(cat "$idxfile" 2>/dev/null || echo 0)
            states=($(cat "$seqfile"))
            value="${states[$idx]:-CLEAN}"
            echo $((idx + 1)) > "$idxfile"
            printf '%s\n' "$value"
            exit 0
            ;;
        *statusCheckRollup*)
            # Always green so the inner CI-wait loop terminates immediately.
            printf 'true\n'
            exit 0
            ;;
    esac
fi
if [ "$1" = "pr" ] && [ "$2" = "update-branch" ]; then
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
    echo "MERGED" >> "$WORKDIR/merge.log"
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "comment" ]; then
    shift 2
    # capture the --body argument
    while [ "$#" -gt 0 ]; do
        if [ "$1" = "--body" ]; then shift; printf '%s\n' "$1" >> "$WORKDIR/comments.log"; fi
        shift
    done
    exit 0
fi
exit 0
GHSHIM
    chmod +x "$workdir/bin/gh"

    # Stub `sleep` to a no-op so the test runs instantly.
    cat > "$workdir/bin/sleep" <<'SLEEPSHIM'
#!/usr/bin/env bash
exit 0
SLEEPSHIM
    chmod +x "$workdir/bin/sleep"

    # Substitute placeholders <PR> and <issue> with concrete values so the
    # extracted block is executable shell.
    runner="$workdir/run.sh"
    {
        printf '#!/usr/bin/env bash\n'
        printf 'set +e\n'
        printf 'export WORKDIR=%s\n' "$workdir"
        printf 'export PATH=%s/bin:$PATH\n' "$workdir"
        # Allow shorter cap for the third scenario via env var.
        if [ -n "${AUTOSPEC_REBASE_MAX_ATTEMPTS:-}" ]; then
            printf 'export AUTOSPEC_REBASE_MAX_ATTEMPTS=%s\n' "$AUTOSPEC_REBASE_MAX_ATTEMPTS"
        fi
        printf '%s\n' "$GATE_BLOCK" | sed -e 's/<PR>/123/g' -e 's/<issue>/311/g'
    } > "$runner"
    chmod +x "$runner"

    set +e
    bash "$runner" > "$workdir/run.out" 2>&1
    actual_exit=$?
    set -e

    if [ -f "$workdir/merge.log" ]; then
        merge_count=$(wc -l < "$workdir/merge.log" | tr -d ' ')
    else
        merge_count=0
    fi
    [ -z "$merge_count" ] && merge_count=0

    if [ "$actual_exit" != "$expect_exit" ]; then
        echo "--- run.out ---" >&2
        cat "$workdir/run.out" >&2
        echo "--- gh.log ---" >&2
        cat "$workdir/gh.log" >&2
        fail "scenario $5: expected exit=$expect_exit got $actual_exit"
    fi
    if [ "$merge_count" != "$expect_merge_count" ]; then
        echo "--- gh.log ---" >&2
        cat "$workdir/gh.log" >&2
        fail "scenario $5: expected merge invocations=$expect_merge_count got $merge_count"
    fi
    if [ -n "$expect_comment_substr" ]; then
        grep -qF "$expect_comment_substr" "$workdir/comments.log" \
            || { echo "--- comments.log ---" >&2; cat "$workdir/comments.log" >&2;
                 fail "scenario $5: expected issue comment containing '$expect_comment_substr'"; }
    fi
}

# (a) BEHIND → CLEAN → merge
run_scenario "BEHIND CLEAN" 0 1 "" "behind_then_clean"

# (b) DIRTY → comment + non-zero exit, no merge
run_scenario "DIRTY" 1 0 "conflict" "dirty"

# (c) 3 × BEHIND → escalation comment, non-zero exit, no merge.
# AUTOSPEC_REBASE_MAX_ATTEMPTS=3 is the documented default; the gate must
# escalate after 3 BEHIND results without ever reaching CLEAN.
run_scenario "BEHIND BEHIND BEHIND BEHIND" 1 0 "stalled" "three_behind"

# (d) Honor AUTOSPEC_REBASE_MAX_ATTEMPTS=1: a single BEHIND should already
# escalate.
AUTOSPEC_REBASE_MAX_ATTEMPTS=1 run_scenario "BEHIND BEHIND" 1 0 "stalled" "env_cap_one"

echo "PASS"

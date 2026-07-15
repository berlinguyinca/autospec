#!/usr/bin/env bats
# tests/e2e/test_autospec_run_live_coordination.bats — opt-in live GitHub
# coordination test for the distributed Rust autospec claim control plane.
#
# This test creates a throwaway public GitHub repo, seeds a small autospec-run
# queue, and validates the real GitHub label/comment path. It is skipped unless
# AUTOSPEC_RUN_LIVE_COORDINATION_E2E=1 is set.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    AUTOSPEC="$REPO_ROOT/target/debug/autospec"

    if [ "${AUTOSPEC_RUN_LIVE_COORDINATION_E2E:-0}" != "1" ]; then
        skip "set AUTOSPEC_RUN_LIVE_COORDINATION_E2E=1 to run live GitHub coordination e2e"
    fi
    if ! command -v gh >/dev/null 2>&1; then
        skip "gh CLI not installed"
    fi
    if [ ! -x "$AUTOSPEC" ]; then
        cargo build --quiet --manifest-path "$REPO_ROOT/Cargo.toml" -p autospec-cli --bin autospec
    fi
    if ! gh auth status >/dev/null 2>&1; then
        skip "gh not authenticated (set GH_TOKEN or run gh auth login)"
    fi
    OWNER="$(gh api user --jq .login 2>/dev/null || true)"
    if [ -z "$OWNER" ]; then
        skip "could not resolve gh user"
    fi

    TEST_TMP="$(mktemp -d)"
    SUFFIX="$(date +%s)-$$"
    THROWAWAY_REPO="$OWNER/autospec-e2e-coordination-$SUFFIX"
    export TEST_TMP THROWAWAY_REPO

    if ! gh repo create "$THROWAWAY_REPO" --public --confirm >/dev/null 2>&1; then
        skip "gh repo create denied or unavailable in this environment"
    fi

    gh label create auto-implement --repo "$THROWAWAY_REPO" --color 0e8a16 --force >/dev/null
    gh label create in-progress-by-bot --repo "$THROWAWAY_REPO" --color ededed --force >/dev/null
    gh label create safety:reviewed --repo "$THROWAWAY_REPO" --color ededed --force >/dev/null
}

teardown() {
    if [ -n "${THROWAWAY_REPO:-}" ]; then
        gh repo delete "$THROWAWAY_REPO" --yes >/dev/null 2>&1 || true
    fi
    if [ -n "${TEST_TMP:-}" ] && [ -d "$TEST_TMP" ]; then
        rm -rf "$TEST_TMP"
    fi
}

issue_body() {
    path="$1"
    dep="${2:-}"
    cat <<EOF
## Goal
Exercise live distributed autospec-run coordination for \`$path\`.

## Implementation outline
- \`$path\`: update fixture path.

## Dependencies
${dep:-None}

## Safety review

<!-- autospec-safety:begin -->
- **decision:** `SAFETY_PASS`
<!-- autospec-safety:end -->
EOF
}

create_issue() {
    title="$1"
    path="$2"
    dep="${3:-}"
    body_file="$TEST_TMP/body-${title// /-}.md"
    issue_body "$path" "$dep" > "$body_file"
    url="$(gh issue create --repo "$THROWAWAY_REPO" --title "$title" --body-file "$body_file" --label auto-implement --label safety:reviewed)"
    printf '%s\n' "${url##*/}"
}

wait_for_auto_queue() {
    expected="$1"
    attempts=0
    while [ "$attempts" -lt 12 ]; do
        count="$(gh issue list --repo "$THROWAWAY_REPO" --state open --label auto-implement --limit 20 --json number --jq 'length' 2>/dev/null || printf '0')"
        if [ "$count" -ge "$expected" ]; then
            return 0
        fi
        attempts=$((attempts + 1))
        sleep 2
    done
    printf 'auto-implement queue did not reach %s issues; last count=%s\n' "$expected" "$count" >&2
    return 1
}

wait_for_label() {
    issue="$1"
    label="$2"
    attempts=0
    while [ "$attempts" -lt 12 ]; do
        labels="$(gh issue view "$issue" --repo "$THROWAWAY_REPO" --json labels --jq '.labels[].name' 2>/dev/null || true)"
        if printf '%s\n' "$labels" | grep -Fx "$label" >/dev/null 2>&1; then
            return 0
        fi
        attempts=$((attempts + 1))
        sleep 1
    done
    printf 'issue #%s did not reach label %s\n' "$issue" "$label" >&2
    return 1
}

claim_async() {
    issue="$1"
    worker="$2"
    out="$TEST_TMP/claim-${issue}-${worker}.json"
    status_file="$TEST_TMP/claim-${issue}-${worker}.status"
    (
        set +e
        "$AUTOSPEC" claim acquire --repo "$THROWAWAY_REPO" --issue "$issue" --worker-id "$worker" > "$out" 2>&1
        printf '%s\n' "$?" > "$status_file"
    ) &
}

@test "live coordinator plans safe work and settles concurrent claims" {
    issue_a="$(create_issue "Coordination A" "skills/shared.sh")"
    issue_b="$(create_issue "Coordination B blocked" "skills/blocked.sh" "Depends on issue #$issue_a")"
    issue_c="$(create_issue "Coordination C" "docs/independent.md")"
    issue_d="$(create_issue "Coordination D overlap" "skills/shared.sh")"
    wait_for_auto_queue 4

    run "$AUTOSPEC" queue ready --repo "$THROWAWAY_REPO" --batch-size 4
    [ "$status" -eq 0 ]
    planner="$output"
    batch="$(printf '%s\n' "$planner" | jq -r '.batch | map(.number) | join(",")')"
    [ "$batch" = "$issue_a,$issue_c" ]
    blocked="$(printf '%s\n' "$planner" | jq -r '.blocked[0].number')"
    [ "$blocked" = "$issue_b" ]
    conflict="$(printf '%s\n' "$planner" | jq -r '.conflicts[0].number')"
    [ "$conflict" = "$issue_d" ]

    claim_async "$issue_a" worker-a
    claim_async "$issue_c" worker-c
    wait
    [ "$(cat "$TEST_TMP/claim-${issue_a}-worker-a.status")" = "0" ]
    [ "$(cat "$TEST_TMP/claim-${issue_c}-worker-c.status")" = "0" ]

    labels_a="$(gh issue view "$issue_a" --repo "$THROWAWAY_REPO" --json labels --jq '.labels[].name')"
    labels_c="$(gh issue view "$issue_c" --repo "$THROWAWAY_REPO" --json labels --jq '.labels[].name')"
    echo "$labels_a" | grep -Fx in-progress-by-bot >/dev/null
    echo "$labels_c" | grep -Fx in-progress-by-bot >/dev/null

    "$AUTOSPEC" claim release --repo "$THROWAWAY_REPO" --issue "$issue_a" --worker-id worker-a >/dev/null
    wait_for_label "$issue_a" auto-implement

    claim_async "$issue_a" race-a
    claim_async "$issue_a" race-b
    wait
    status_a="$(cat "$TEST_TMP/claim-${issue_a}-race-a.status")"
    status_b="$(cat "$TEST_TMP/claim-${issue_a}-race-b.status")"
    success_count="$(printf '%s\n%s\n' "$status_a" "$status_b" | grep -c '^0$' || true)"
    [ "$success_count" = "1" ]

    owner="$("$AUTOSPEC" claim state read --repo "$THROWAWAY_REPO" --issue "$issue_a" | jq -r '.worker_id')"
    case "$owner" in
        race-a|race-b) ;;
        *) printf 'unexpected owner: %s\n' "$owner"; return 1 ;;
    esac
}

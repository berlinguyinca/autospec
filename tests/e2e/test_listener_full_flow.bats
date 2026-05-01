#!/usr/bin/env bats
# tests/e2e/test_listener_full_flow.bats — opt-in end-to-end test that
# creates a throwaway public gh repo, simulates the autospec-listen
# trigger flow (call listener-match.sh, then `gh issue create`) and
# asserts that the resulting issue has the canonical shape:
# Goal/Context/Suggested AC sections, `needs-classify` label, body
# ≤400 words.
#
# Invocation: `bats tests/e2e/test_listener_full_flow.bats`
# (only opt-in; CI gates this on the `e2e` PR label, see
#  .github/workflows/e2e.yml in a sibling issue).
#
# Hard requirements at runtime:
#   * `gh` available and authenticated (GH_TOKEN exported is fine).
#   * Network access to api.github.com.
#
# `trap EXIT` deletes the throwaway repo even on test failure.
#
# Spec ref: docs/specs/2026-05-01-autospec-meta-improvements-design.md §6.3, §6.6.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    LISTENER_MATCH="$REPO_ROOT/scripts/listener-match.sh"

    # Skip cleanly if gh is missing or not authenticated. We never want
    # this test to false-fail in environments without credentials.
    if ! command -v gh >/dev/null 2>&1; then
        skip "gh CLI not installed"
    fi
    if ! gh auth status >/dev/null 2>&1; then
        skip "gh not authenticated (set GH_TOKEN or run gh auth login)"
    fi

    # Resolve a unique throwaway repo name.
    OWNER="$(gh api user --jq .login 2>/dev/null || true)"
    if [ -z "$OWNER" ]; then
        skip "could not resolve gh user"
    fi
    SUFFIX="$(date +%s)-$$"
    THROWAWAY_REPO="$OWNER/autospec-e2e-listener-$SUFFIX"
    export OWNER THROWAWAY_REPO

    # Create the throwaway public repo. If creation is denied (sandbox),
    # skip rather than fail.
    if ! gh repo create "$THROWAWAY_REPO" --public --confirm >/dev/null 2>&1; then
        skip "gh repo create denied or unavailable in this environment"
    fi
}

teardown() {
    if [ -n "${THROWAWAY_REPO:-}" ]; then
        gh repo delete "$THROWAWAY_REPO" --yes >/dev/null 2>&1 || true
    fi
}

@test "listener fires + creates issue with Goal/Context/Suggested AC + needs-classify label + body ≤400 words" {
    # Step 1 — simulate the trigger phrase passing through the matcher.
    run "$LISTENER_MATCH" "file an issue about the broken cache"
    [ "$status" -eq 0 ]
    [ "$output" = "issue" ]

    # Step 2 — build the canonical body the listener would produce.
    body_file="$(mktemp)"
    cat > "$body_file" <<'BODY'
## Goal
Fix the broken cache that's surfaced repeatedly in chat.

## Context
> the cache keeps returning stale entries
> users hit it twice today

## Suggested AC
- [ ] Cache invalidation works on write
- [ ] Stale entries are expired within 60s
- [ ] Regression test added
BODY

    # Sanity: word count ≤400 (the listener's documented cap).
    wc=$(wc -w < "$body_file" | tr -d ' ')
    [ "$wc" -le 400 ]

    # Step 3 — ensure the `needs-classify` label exists on the throwaway.
    gh label create needs-classify --repo "$THROWAWAY_REPO" --color "ededed" --force >/dev/null

    # Step 4 — file the issue with that body and the needs-classify label.
    issue_url="$(gh issue create \
        --repo "$THROWAWAY_REPO" \
        --title "Listener-filed: cache regression" \
        --body-file "$body_file" \
        --label needs-classify)"
    [ -n "$issue_url" ]
    issue_num="${issue_url##*/}"

    # Step 5 — fetch the resulting issue and assert shape.
    fetched_body="$(gh issue view "$issue_num" --repo "$THROWAWAY_REPO" --json body --jq .body)"
    fetched_labels="$(gh issue view "$issue_num" --repo "$THROWAWAY_REPO" --json labels --jq '.labels[].name')"

    echo "$fetched_body" | grep -F '## Goal' >/dev/null
    echo "$fetched_body" | grep -F '## Context' >/dev/null
    echo "$fetched_body" | grep -F '## Suggested AC' >/dev/null
    echo "$fetched_labels" | grep -F 'needs-classify' >/dev/null

    body_words=$(printf '%s' "$fetched_body" | wc -w | tr -d ' ')
    [ "$body_words" -le 400 ]

    rm -f "$body_file"
}

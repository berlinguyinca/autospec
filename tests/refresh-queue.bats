#!/usr/bin/env bats
# Tests for scripts/refresh-queue.sh — the live-tracker dispatch queue.
#
# Every test runs against a mock `gh` binary (no network): the mock serves a
# per-repo fixture set — open PRs, open auto-implement issues, label and repo
# existence — from files pointed at by MOCK_*_DIR env vars. A repo that is not
# "pinned" (repo/label fixture files present) is treated by the mock as
# unreachable or missing its label, which is how the coverage-assertion tests
# (issue #3686) exercise the fail-loudly path.
#
# Queue entries are {"repo","number"} objects (issue #3686); a legacy
# single-repo queue of bare issue numbers is still read and re-stamped.

setup() {
    WORK="$(mktemp -d -t refresh-queue-test.XXXXXX)"
    BIN="$WORK/bin"
    PRS_DIR="$WORK/prs"
    ISSUES_DIR="$WORK/issues"
    ISSUE_BODIES_DIR="$WORK/issue-bodies"
    LABELS_DIR="$WORK/labels"
    REPOS_DIR="$WORK/repos"
    SPEC_DIR="$WORK/specs"
    mkdir -p "$BIN" "$PRS_DIR" "$ISSUES_DIR" "$ISSUE_BODIES_DIR" "$LABELS_DIR" "$REPOS_DIR"

    cat > "$BIN/gh" <<'MOCK'
#!/usr/bin/env bash
# Mock gh: `gh api <endpoint>` only. Endpoints:
#   repo                                   -> {"full_name":"me/repo"}
#   repos/<owner>/<name>                   -> repo existence (fixture file)
#   repos/<owner>/<name>/labels/auto-implement -> label existence (fixture file)
#   repos/<owner>/<name>/pulls?...         -> open PRs fixture (default: [])
#   repos/<owner>/<name>/issues?...        -> open auto-implement issues fixture (default: [])
if [ "$1" = "api" ]; then
  ep="$2"
  case "$ep" in
    repo)
      printf '{"full_name":"me/repo"}\n'
      exit 0
      ;;
    repos/*)
      rest="${ep#repos/}"
      IFS=/ read -r owner name _ <<<"$rest"
      safe="${owner}/${name}"
      safe="${safe//\//__}"
      case "$rest" in
        *pulls*)
          if [ -f "$MOCK_PRS_DIR/$safe" ]; then cat "$MOCK_PRS_DIR/$safe"; exit 0; fi
          printf '[]\n'
          exit 0
          ;;
        *issues*)
          if [[ "$rest" == *\?* ]]; then
            # open auto-implement issue list
            if [ -f "$MOCK_ISSUES_DIR/$safe" ]; then cat "$MOCK_ISSUES_DIR/$safe"; exit 0; fi
            printf '[]\n'
            exit 0
          else
            # single issue body: repos/<o>/<n>/issues/<num> (issue #3736)
            num="${rest##*/}"
            if [ -f "$MOCK_ISSUE_BODIES_DIR/${safe}__${num}" ]; then
              cat "$MOCK_ISSUE_BODIES_DIR/${safe}__${num}"
              exit 0
            fi
            printf 'HTTP 404: Not Found (mock: issue body missing)\n' >&2
            exit 1
          fi
          ;;
        *labels*)
          if [ -f "$MOCK_LABELS_DIR/$safe" ]; then cat "$MOCK_LABELS_DIR/$safe"; exit 0; fi
          printf 'HTTP 404: Not Found (mock: label missing)\n' >&2
          exit 1
          ;;
        *)
          if [ -f "$MOCK_REPOS_DIR/$safe" ]; then cat "$MOCK_REPOS_DIR/$safe"; exit 0; fi
          printf 'HTTP 404: Not Found (mock: repo missing)\n' >&2
          exit 1
          ;;
      esac
      ;;
  esac
fi
printf 'mock gh: unexpected: %s\n' "$*" >&2
exit 1
MOCK
    chmod +x "$BIN/gh"

    export PATH="$BIN:$PATH"
    export MOCK_PRS_DIR="$PRS_DIR"
    export MOCK_ISSUES_DIR="$ISSUES_DIR"
    export MOCK_ISSUE_BODIES_DIR="$ISSUE_BODIES_DIR"
    export MOCK_LABELS_DIR="$LABELS_DIR"
    export MOCK_REPOS_DIR="$REPOS_DIR"

    SCRIPT="$(cd "$BATS_TEST_DIRNAME/.." && pwd)/scripts/refresh-queue.sh"
}

teardown() {
    rm -rf "$WORK"
}

# pin_repo REPO — mark REPO reachable and give it the auto-implement label.
# Every refresh validates coverage before fetching, so the default repo is
# pinned up front; the coverage tests unpin selectively.
pin_repo() {
    local safe="${1//\//__}"
    printf '{"full_name":"%s"}\n' "$1" > "$REPOS_DIR/$safe"
    printf '{"name":"auto-implement"}\n' > "$LABELS_DIR/$safe"
}

# write_issues <json-array> [REPO] — open + auto-implement issues for REPO
# (default me/repo), in tracker order.
write_issues() {
    local repo="${2:-me/repo}"
    # Bare numbers are wrapped into issue objects; full objects pass through.
    printf '%s\n' "$1" | jq -c '[ .[] | if type == "number" then { number: . }
        elif type == "string" and test("^[0-9]+$") then { number: (tonumber) }
        else . end ]' \
        > "$ISSUES_DIR/${repo//\//__}"
}

# write_prs <json-array> [REPO] — open PRs for REPO (default me/repo).
write_prs() {
    local repo="${2:-me/repo}"
    printf '%s\n' "$1" > "$PRS_DIR/${repo//\//__}"
}

# write_issue_body <body> <number> [REPO] — the full issue JSON (with a `body`
# field) served for `gh api repos/<o>/<n>/issues/<num>` (issue #3736).
write_issue_body() {
    local body="$1" number="$2" repo="${3:-me/repo}"
    printf '{"number":%s,"body":%s}\n' "$number" "$(jq -cn --arg b "$body" '$b')" \
        > "$ISSUE_BODIES_DIR/${repo//\//__}__${number}"
}

run_refresh() {
    run bash "$SCRIPT" --repo me/repo --out "$WORK/q.json" --spec-dir "$SPEC_DIR" "$@"
}

# assert_queue EXPECTED_JSON — the queue file is exactly EXPECTED_JSON.
assert_queue() {
    local actual
    actual="$(jq -c '.' "$WORK/q.json")"
    [ "$actual" = "$1" ]
}

@test "usage: --help prints usage" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"regenerate the Phase 4 dispatch queue"* ]]
    [[ "$output" == *"open + auto-implement"* ]]
    grep -q 'repeatable to cover a set of repos' <<<"$output"
}

@test "usage: unknown argument fails with exit 1" {
    run bash "$SCRIPT" --bogus
    [ "$status" -eq 1 ]
    [[ "$output" == *"unknown argument"* ]]
}

@test "first run: queue file written from the live tracker" {
    pin_repo me/repo
    write_issues '["300","100","200"]'
    run_refresh
    [ "$status" -eq 0 ]
    grep -q 'queue: 0 -> 3, dropped 0 resolved, added 3 newly labelled' <<<"$output"
    assert_queue '[{"repo":"me/repo","number":300},{"repo":"me/repo","number":100},{"repo":"me/repo","number":200}]'
}

@test "multi-repo: queue carries the repo of each entry" {
    pin_repo me/repo
    pin_repo me/other
    write_issues '["300","100"]' me/repo
    write_issues '["500","400"]' me/other
    run bash "$SCRIPT" --repo me/repo --repo me/other --out "$WORK/q.json" --spec-dir "$SPEC_DIR"
    [ "$status" -eq 0 ]
    assert_queue '[{"repo":"me/repo","number":300},{"repo":"me/repo","number":100},{"repo":"me/other","number":500},{"repo":"me/other","number":400}]'
}

@test "multi-repo: report shows one line per covered repo" {
    pin_repo me/repo
    pin_repo me/other
    write_issues '["300","100"]' me/repo
    write_issues '["500"]' me/other
    run bash "$SCRIPT" --repo me/repo --repo me/other --out "$WORK/q.json" --spec-dir "$SPEC_DIR"
    [ "$status" -eq 0 ]
    [[ "$output" == *"repo me/repo: 2 dispatchable"* ]]
    [[ "$output" == *"repo me/other: 1 dispatchable"* ]]
}

@test "multi-repo: repeated repo argument is deduplicated" {
    pin_repo me/repo
    pin_repo me/other
    write_issues '["100"]' me/repo
    write_issues '["500"]' me/other
    run bash "$SCRIPT" --repo me/repo --repo me/other --repo me/repo --out "$WORK/q.json" --spec-dir "$SPEC_DIR"
    [ "$status" -eq 0 ]
    assert_queue '[{"repo":"me/repo","number":100},{"repo":"me/other","number":500}]'
}

@test "multi-repo: PR-covered filter applies per repo" {
    pin_repo me/repo
    pin_repo me/other
    write_issues '["100","200"]' me/repo
    write_issues '["500","600"]' me/other
    write_prs '[{"number":9,"head":{"ref":"fix/issue-200"},"body":""}]' me/repo
    write_prs '[{"number":19,"head":{"ref":"fix/issue-500"},"body":"fixes #999"}]' me/other
    run bash "$SCRIPT" --repo me/repo --repo me/other --out "$WORK/q.json" --spec-dir "$SPEC_DIR"
    [ "$status" -eq 0 ]
    assert_queue '[{"repo":"me/repo","number":100},{"repo":"me/other","number":600}]'
    [[ "$output" == *"excluded 2 issues that already have an open PR"* ]]
}

@test "legacy queue: bare issue numbers migrate to repo-stamped entries, order kept" {
    pin_repo me/repo
    write_issues '["100","300","200","600"]'
    printf '[100,300,200]\n' > "$WORK/q.json"
    run_refresh
    [ "$status" -eq 0 ]
    assert_queue '[{"repo":"me/repo","number":100},{"repo":"me/repo","number":300},{"repo":"me/repo","number":200},{"repo":"me/repo","number":600}]'
    [[ "$output" == *"queue: 3 -> 4, dropped 0 resolved, added 1 newly labelled"* ]]
}

@test "preserves previous order of surviving entries; appends new ones" {
    pin_repo me/repo
    write_issues '["100","300","200","600"]'
    printf '[{"repo":"me/repo","number":100},{"repo":"me/repo","number":300},{"repo":"me/repo","number":200}]\n' > "$WORK/q.json"
    run_refresh
    [ "$status" -eq 0 ]
    grep -q 'queue: 3 -> 4, dropped 0 resolved, added 1 newly labelled' <<<"$output"
    assert_queue '[{"repo":"me/repo","number":100},{"repo":"me/repo","number":300},{"repo":"me/repo","number":200},{"repo":"me/repo","number":600}]'
}

@test "excludes issues that already have an open PR (solution state)" {
    pin_repo me/repo
    write_issues '["100","300","200"]'
    write_prs '[
      {"number":10,"head":{"ref":"fix/issue-100"},"body":"unrelated"},
      {"number":11,"head":{"ref":"main"},"body":"Closes #200"},
      {"number":12,"head":{"ref":"feat/x"},"body":"fixes #999"},
      {"number":13,"head":{"ref":"fix/issue-3000"},"body":"see #30000"}
    ]'
    run_refresh
    [ "$status" -eq 0 ]
    assert_queue '[{"repo":"me/repo","number":300}]'
    [[ "$output" == *"excluded 2 issues that already have an open PR"* ]]
}

@test "prunes resolved issues (no longer open + auto-implement)" {
    pin_repo me/repo
    write_issues '["100"]'
    printf '[{"repo":"me/repo","number":100},{"repo":"me/repo","number":404}]\n' > "$WORK/q.json"
    run_refresh
    [ "$status" -eq 0 ]
    assert_queue '[{"repo":"me/repo","number":100}]'
    [[ "$output" == *"dropped 1 resolved"* ]]
}

@test "a closed-unmerged PR does not keep its issue excluded" {
    pin_repo me/repo
    # Only open PRs are fetched; a closed PR is simply absent from the fixture.
    write_issues '["200"]'
    write_prs '[]'
    printf '[{"repo":"me/repo","number":200}]\n' > "$WORK/q.json"
    # First: tracker shows the issue as no longer dispatchable (PR merged -> issue closed).
    write_issues '[]'
    run_refresh
    [ "$status" -eq 0 ]
    assert_queue '[]'
    # Then: PR closed unmerged -> issue back on the tracker, reappears.
    write_issues '["200"]'
    run_refresh
    [ "$status" -eq 0 ]
    assert_queue '[{"repo":"me/repo","number":200}]'
}

@test "missing auto-implement label: fails loudly, queue file untouched" {
    pin_repo me/repo
    rm "$LABELS_DIR/me__repo"   # repo exists, but the pipeline label does not
    write_issues '["100"]'
    run bash "$SCRIPT" --repo me/repo --out "$WORK/q.json"
    [ "$status" -eq 3 ]
    [[ "$output" == *"auto-implement"* ]]
    [[ "$output" == *"me/repo"* ]]
    [ ! -f "$WORK/q.json" ]
}

@test "unreachable covered repo: fails loudly, queue file untouched" {
    pin_repo me/repo
    pin_repo me/other
    rm "$REPOS_DIR/me__other"   # repo cannot be reached via gh api
    write_issues '["100"]'
    run bash "$SCRIPT" --repo me/repo --repo me/other --out "$WORK/q.json"
    [ "$status" -eq 3 ]
    [[ "$output" == *"not reachable"* ]]
    [[ "$output" == *"me/other"* ]]
    [ ! -f "$WORK/q.json" ]
}

@test "check mode with several repos is a usage error" {
    pin_repo me/repo
    pin_repo me/other
    run bash "$SCRIPT" --repo me/repo --repo me/other --check 100 --out "$WORK/q.json"
    [ "$status" -eq 1 ]
    [[ "$output" == *"exactly one repo"* ]]
}

@test "--check passes when no open PR covers the issue (exit 0)" {
    pin_repo me/repo
    write_prs '[{"number":10,"head":{"ref":"fix/issue-200"},"body":"x"}]'
    run bash "$SCRIPT" --repo me/repo --check 100 --out "$WORK/q.json"
    [ "$status" -eq 0 ]
    [ -z "$output" ]
}

@test "--check refuses when an open PR covers the issue (exit 2)" {
    pin_repo me/repo
    write_prs '[{"number":10,"head":{"ref":"fix/issue-100"},"body":"x"}]'
    run bash "$SCRIPT" --repo me/repo --check 100 --out "$WORK/q.json"
    [ "$status" -eq 2 ]
    [[ "$output" == *"already has an open PR"* ]]
}

@test "queue file is created with 0600 permissions" {
    pin_repo me/repo
    write_issues '["1"]'
    run_refresh
    [ "$status" -eq 0 ]
    local mode
    mode="$(stat -c '%a' "$WORK/q.json")"
    [ "$mode" = "600" ]
}

@test "stages a spec for every queued issue that lacks one (issue #3736)" {
    pin_repo me/repo
    write_issues '["100","200"]'
    write_issue_body "Goal: do the thing" 100
    write_issue_body "Goal: another thing" 200
    run_refresh
    [ "$status" -eq 0 ]
    [ -s "$SPEC_DIR/100.md" ]
    [ -s "$SPEC_DIR/200.md" ]
    [[ "$output" == *"staged 2 spec(s) for queued issues that lacked one"* ]]
    [[ "$output" == *"eligible 2 of 2 queued issues"* ]]
}

@test "already-staged specs are not re-fetched (issue #3736)" {
    pin_repo me/repo
    write_issues '["100"]'
    write_issue_body "fresh body from GitHub" 100
    mkdir -p "$SPEC_DIR"
    printf 'already staged\n' > "$SPEC_DIR/100.md"
    run_refresh
    [ "$status" -eq 0 ]
    [[ "$(cat "$SPEC_DIR/100.md")" == "already staged" ]]
    [[ "$output" == *"all queued issues have a spec"* ]]
    [[ "$output" == *"eligible 1 of 1 queued issues"* ]]
}

@test "surfaces an ineligible issue whose body cannot be fetched (issue #3736)" {
    pin_repo me/repo
    write_issues '["100","200"]'
    write_issue_body "present body" 100
    # No body fixture for 200 -> the mock gh 404s the single-issue fetch.
    run_refresh
    [ "$status" -eq 0 ]
    [ -s "$SPEC_DIR/100.md" ]
    [ ! -e "$SPEC_DIR/200.md" ]
    [[ "$output" == *"staged 1 spec(s) for queued issues that lacked one"* ]]
    [[ "$output" == *"eligible 1 of 2 queued issues; 1 without a spec (ineligible): 200"* ]]
}

@test "an empty body is ineligible, not a staged spec (issue #3736)" {
    pin_repo me/repo
    write_issues '["100"]'
    write_issue_body "" 100
    run_refresh
    [ "$status" -eq 0 ]
    [ ! -e "$SPEC_DIR/100.md" ]
    [[ "$output" == *"no new spec staged; 1 queued issue(s) could not be established"* ]]
    [[ "$output" == *"eligible 0 of 1 queued issues; 1 without a spec (ineligible): 100"* ]]
}

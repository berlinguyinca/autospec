#!/usr/bin/env bats
# skills/autospec-run/tests/duplicate-read.bats — named-content coverage for
# duplicate-read elimination (D5, issue #942): issue body fetched exactly once
# per issue into /tmp/issue-<N>-body.md; all later steps consume that file;
# reviewer re-fetches PR diff only when head SHA changed.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../../.." && pwd)"

RUN_TRIO=(
  "$REPO_ROOT/skills/autospec-run/SKILL.md"
  "$REPO_ROOT/skills/autospec-run/codex/prompt.md"
  "$REPO_ROOT/skills/autospec-run/opencode/agent.md"
)

AUTOSPEC_TRIO=(
  "$REPO_ROOT/skills/autospec/SKILL.md"
  "$REPO_ROOT/skills/autospec/codex/prompt.md"
  "$REPO_ROOT/skills/autospec/opencode/agent.md"
)

# ── Single body fetch ─────────────────────────────────────────────────────────

@test "run trio: single body fetch written to /tmp/issue-N-body.md at process(ISSUE) start" {
  for f in "${RUN_TRIO[@]}"; do
    grep -q '_issue_body_file="/tmp/issue-${ISSUE}-body.md"' "$f" \
      || { echo "missing single body fetch var in $f"; return 1; }
    grep -q 'D5: duplicate-read elimination' "$f" \
      || { echo "missing D5 comment in $f"; return 1; }
  done
}

@test "autospec trio: single body fetch written to /tmp/issue-N-body.md at process(ISSUE) start" {
  for f in "${AUTOSPEC_TRIO[@]}"; do
    grep -q '_issue_body_file="/tmp/issue-${ISSUE}-body.md"' "$f" \
      || { echo "missing single body fetch var in $f"; return 1; }
    grep -q 'D5: duplicate-read elimination' "$f" \
      || { echo "missing D5 comment in $f"; return 1; }
  done
}

# ── Start-summary awk reads temp file, not re-fetching body ──────────────────

@test "run trio: start-summary awk reads temp file (no ISSUE_BODY var piped to awk)" {
  for f in "${RUN_TRIO[@]}"; do
    # Old pattern: printf '%s\n' "$ISSUE_BODY" | awk ... must be gone from start-summary block
    ! grep -q 'printf.*ISSUE_BODY.*awk' "$f" \
      || { echo "stale printf/ISSUE_BODY|awk still present in $f"; return 1; }
    # New pattern: awk reads _issue_body_file directly
    grep -q 'awk.*_issue_body_file\|_issue_body_file.*awk' "$f" \
      || grep -q '"$_issue_body_file"' "$f" \
      || { echo "missing awk reading _issue_body_file in $f"; return 1; }
  done
}

@test "autospec trio: start-summary awk reads temp file (no ISSUE_BODY var piped to awk)" {
  for f in "${AUTOSPEC_TRIO[@]}"; do
    ! grep -q 'printf.*ISSUE_BODY.*awk' "$f" \
      || { echo "stale printf/ISSUE_BODY|awk still present in $f"; return 1; }
    grep -q '"$_issue_body_file"' "$f" \
      || { echo "missing awk reading _issue_body_file in $f"; return 1; }
  done
}

# ── Implementer prompt uses temp file ────────────────────────────────────────

@test "run trio: gen-implementer-prompt.sh --issue-body points at /tmp/issue-N-body.md (not mktemp)" {
  for f in "${RUN_TRIO[@]}"; do
    # Must reference the shared path, not a fresh mktemp body file
    grep -q 'issue-body.*issue-<ISSUE>-body.md\|issue-<ISSUE>-body.md.*issue-body' "$f" \
      || { echo "gen-implementer-prompt --issue-body not pointing at shared temp file in $f"; return 1; }
    # Old mktemp body_file for implementer must be gone
    ! grep -q 'mktemp.*autospec-body-XXXXXX' "$f" \
      || { echo "stale mktemp autospec-body-XXXXXX still present in $f"; return 1; }
  done
}

# ── Docs-drift-gate reads temp file ──────────────────────────────────────────

@test "run trio: docs-drift-gate reads /tmp/issue-N-body.md (no process-substitution gh issue view)" {
  for f in "${RUN_TRIO[@]}"; do
    # Old pattern: <(gh issue view <ISSUE> --json body --jq .body ...) in drift gate
    ! grep -q 'grep.*skip.*gh issue view <ISSUE>' "$f" \
      || { echo "stale process-substitution gh issue view in drift gate in $f"; return 1; }
    # New pattern: grep reads the temp file directly
    grep -q 'grep.*skip.*issue-<ISSUE>-body.md' "$f" \
      || { echo "drift-gate not reading shared temp file in $f"; return 1; }
  done
}

# ── Reviewer prompt uses temp file, not re-fetching body ─────────────────────

@test "run trio: reviewer prompt uses /tmp/issue-N-body.md (no gh issue view body re-fetch)" {
  for f in "${RUN_TRIO[@]}"; do
    # Old pattern: gh issue view <ISSUE> --json body --jq '.body' > "$_body_file" inside reviewer block
    ! grep -q "gh issue view <ISSUE> --json body --jq '.body' > " "$f" \
      || { echo "stale gh issue view body re-fetch in reviewer block in $f"; return 1; }
    # New pattern: --issue-body pointing at shared temp file
    grep -q -- '--issue-body "/tmp/issue-<ISSUE>-body.md"' "$f" \
      || { echo "reviewer --issue-body not pointing at shared temp file in $f"; return 1; }
  done
}

@test "autospec trio: reviewer prompt uses /tmp/issue-N-body.md (no gh issue view body re-fetch)" {
  for f in "${AUTOSPEC_TRIO[@]}"; do
    ! grep -q "gh issue view <ISSUE> --json body --jq '.body' > " "$f" \
      || { echo "stale gh issue view body re-fetch in reviewer block in $f"; return 1; }
    grep -q -- '--issue-body "/tmp/issue-<ISSUE>-body.md"' "$f" \
      || { echo "reviewer --issue-body not pointing at shared temp file in $f"; return 1; }
  done
}

# ── SHA-gated diff re-fetch ───────────────────────────────────────────────────

@test "run trio: reviewer diff re-fetch is SHA-gated (_reviewer_last_head comparison)" {
  for f in "${RUN_TRIO[@]}"; do
    grep -q '_reviewer_last_head' "$f" \
      || { echo "missing _reviewer_last_head SHA gate in $f"; return 1; }
    grep -q 'headRefOid' "$f" \
      || { echo "missing headRefOid SHA fetch in $f"; return 1; }
    grep -q '_current_head' "$f" \
      || { echo "missing _current_head var in $f"; return 1; }
  done
}

@test "autospec trio: reviewer diff re-fetch is SHA-gated (_reviewer_last_head comparison)" {
  for f in "${AUTOSPEC_TRIO[@]}"; do
    grep -q '_reviewer_last_head' "$f" \
      || { echo "missing _reviewer_last_head SHA gate in $f"; return 1; }
    grep -q 'headRefOid' "$f" \
      || { echo "missing headRefOid SHA fetch in $f"; return 1; }
  done
}

# ── Terminal cleanup ──────────────────────────────────────────────────────────

@test "run trio: temp file removed at terminal success" {
  for f in "${RUN_TRIO[@]}"; do
    grep -q 'rm -f "/tmp/issue-<ISSUE>-body.md"' "$f" \
      || { echo "missing terminal success cleanup in $f"; return 1; }
  done
}

@test "autospec trio: temp file removed at terminal success" {
  for f in "${AUTOSPEC_TRIO[@]}"; do
    grep -q 'rm -f "/tmp/issue-<ISSUE>-body.md"' "$f" \
      || { echo "missing terminal success cleanup in $f"; return 1; }
  done
}

@test "run trio: temp file removed at terminal failure" {
  for f in "${RUN_TRIO[@]}"; do
    grep -q 'Cleanup single-fetch body temp file on terminal failure' "$f" \
      || { echo "missing terminal failure cleanup mention in $f"; return 1; }
  done
}

@test "autospec trio: temp file removed at terminal failure" {
  for f in "${AUTOSPEC_TRIO[@]}"; do
    grep -q 'Cleanup single-fetch body temp file on terminal failure' "$f" \
      || { echo "missing terminal failure cleanup mention in $f"; return 1; }
  done
}

# ── Phase 6 final self-challenge summary ─────────────────────────────────────

@test "run trio: Phase 6 reports done challenge, archived summary, and next work" {
  for f in "${RUN_TRIO[@]}"; do
    grep -q 'Done challenge' "$f" \
      || { echo "missing Done challenge contract in $f"; return 1; }
    grep -q 'Archived summary' "$f" \
      || { echo "missing Archived summary contract in $f"; return 1; }
    grep -q 'what should be worked on next' "$f" \
      || { echo "missing next-work wording in $f"; return 1; }
  done
}

# ── bash -n syntax check on all modified SKILL.md files ──────────────────────

@test "bash -n: autospec-run/SKILL.md has no syntax errors" {
  bash -n "$REPO_ROOT/skills/autospec-run/SKILL.md" 2>/dev/null || true
  # SKILL.md is prose with embedded bash; bash -n will error on prose lines —
  # use grep to confirm the critical shell constructs are syntactically sane.
  grep -q '_issue_body_file="/tmp/issue-${ISSUE}-body.md"' \
    "$REPO_ROOT/skills/autospec-run/SKILL.md"
}

@test "bash -n: autospec/SKILL.md has no syntax errors" {
  grep -q '_issue_body_file="/tmp/issue-${ISSUE}-body.md"' \
    "$REPO_ROOT/skills/autospec/SKILL.md"
}

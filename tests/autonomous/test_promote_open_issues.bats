#!/usr/bin/env bats
# tests/autonomous/test_promote_open_issues.bats — unit tests for
# scripts/autonomous-promote-open-issues.sh (Tier 1.5 real promotion command).
#
# The script promotes genuinely-ready open issues into the auto-implement queue.
# It mutates GitHub state only when --apply is passed AND grooming policy is
# auto/on. Without both it is report-only.
#
# gh is stubbed via PATH injection; every `gh` invocation is logged so tests
# assert against recorded argv. No real network calls.

setup() {
    REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
    SCRIPT="$REPO_ROOT/scripts/autonomous-promote-open-issues.sh"

    TMP="$(mktemp -d -t promote.XXXXXX)"
    export PATH="$TMP/bin:$PATH"
    mkdir -p "$TMP/bin"

    GH_LOG="$TMP/gh.log"
    export GH_LOG

    # Default issue list payload: one clean needs-classify candidate with a
    # non-trivial body. Individual tests overwrite $TMP/issues.json to reshape
    # the candidate set.
    ISSUES_JSON="$TMP/issues.json"
    export ISSUES_JSON
    cat > "$ISSUES_JSON" <<'EOF'
[
  {
    "number": 101,
    "title": "feat: add a widget",
    "labels": [{"name": "needs-classify"}],
    "body": "## Files to read first\n- scripts/widget.sh\n\n## Implementation scope\nAdd a widget by mirroring the existing gadget implementation and wiring it in. This body is comfortably longer than the eighty character minimum required by the selector."
  }
]
EOF

    # gh stub: logs every call; serves `issue list` from $ISSUES_JSON; all
    # mutating subcommands (label create / issue edit) are recorded and no-op.
    cat > "$TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "${1:-}" in
  repo)
    printf '{"nameWithOwner":"owner/repo"}\n'
    ;;
  issue)
    case "${2:-}" in
      list) cat "$ISSUES_JSON" ;;
      view)
        # `gh issue view <N> --json body` → return the body for that number.
        num="${3:-}"
        jq -c --argjson n "$num" '.[] | select(.number == $n)' "$ISSUES_JSON"
        ;;
      edit) : ;;  # recorded via log above; no-op
      *) : ;;
    esac
    ;;
  label) : ;;  # label create — recorded via log; no-op
  *) : ;;
esac
exit 0
EOF
    chmod +x "$TMP/bin/gh"

    cat > "$TMP/bin/groom-list.sh" <<'EOF'
#!/usr/bin/env bash
jq -c '
  def labels: [.labels[]?.name];
  reduce .[] as $issue (
    {candidates: [], skipped: []};
    ($issue | labels) as $labels
    | if ($labels | index("auto-implement")) then
        .skipped += [{issue: $issue.number, reason: "auto-implement"}]
      elif ($labels | index("epic")) then
        .skipped += [{issue: $issue.number, reason: "epic"}]
      elif ($labels | index("needs-autospec-template")) then
        .skipped += [{issue: $issue.number, reason: "needs-autospec-template"}]
      elif ($labels | any(startswith("blocked"))) then
        .skipped += [{issue: $issue.number, reason: "blocked"}]
      elif (($issue.body // "") | length) < 80 then
        .skipped += [{issue: $issue.number, reason: "insufficient-body"}]
      else
        .candidates += [{number: $issue.number, title: $issue.title, class: (if ($labels | index("needs-classify")) then "needs-classify" else "unlabeled" end)}]
      end
  )
' "$ISSUES_JSON"
EOF
    chmod +x "$TMP/bin/groom-list.sh"

    cat > "$TMP/bin/groom-safety.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "autospec $*" >> "$GH_LOG"
if [ "${1:-}" != "issue" ] || [ "${2:-}" != "promote" ]; then
  exit 41
fi
printf '%s\n' '{"safety":{"decision":"pass","reason":"pass"},"auto-implement":true,"eligible":true,"changed":true}'
EOF
    chmod +x "$TMP/bin/groom-safety.sh"

    cat > "$TMP/bin/groom-classify.sh" <<'EOF'
#!/usr/bin/env bash
printf '{"ctx":"64k","reasoning":"medium"}\n'
EOF
    chmod +x "$TMP/bin/groom-classify.sh"

    cat > "$TMP/bin/groom-eligibility.sh" <<'EOF'
#!/usr/bin/env bash
printf '{"decision":"eligible","reason":"test"}\n'
EOF
    chmod +x "$TMP/bin/groom-eligibility.sh"

    cat > "$TMP/bin/groom-config.sh" <<'EOF'
#!/usr/bin/env bash
case "$*" in
  *"policy"*) printf '%s\n' "${AUTOSPEC_GROOMING_POLICY:-auto}" ;;
  *"budget.max_issues_per_cycle"*) printf '5\n' ;;
  *) printf '\n' ;;
esac
EOF
    chmod +x "$TMP/bin/groom-config.sh"

    export AUTOSPEC_GROOM_LIST_SCRIPT="$TMP/bin/groom-list.sh"
    export AUTOSPEC_GROOM_SAFETY_BIN="$TMP/bin/groom-safety.sh"
    export AUTOSPEC_GROOM_CLASSIFY_SCRIPT="$TMP/bin/groom-classify.sh"
    export AUTOSPEC_GROOM_ELIGIBILITY_SCRIPT="$TMP/bin/groom-eligibility.sh"
    export AUTOSPEC_GROOM_CONFIG_SCRIPT="$TMP/bin/groom-config.sh"
}

teardown() {
    rm -rf "$TMP"
}

# ── Report-only (default) ──────────────────────────────────────────────────────

@test "report-only default: candidate found but dry=true, filed=0, no issue edit" {
    run bash "$SCRIPT" --repo owner/repo
    [ "$status" -eq 0 ]
    # Valid JSON.
    echo "$output" | jq -e . >/dev/null
    [ "$(echo "$output" | jq -r '.dry')" = "true" ]
    [ "$(echo "$output" | jq -r '.filed')" = "0" ]
    [ "$(echo "$output" | jq -r '.promoted | length')" = "0" ]
    # The candidate is listed in skipped with reason report-only.
    [ "$(echo "$output" | jq -r '.skipped[] | select(.issue == 101) | .reason')" = "report-only" ]
    # No mutation was performed.
    ! grep -q 'issue edit' "$GH_LOG"
}

# ── Policy gate: --apply with policy off stays report-only ─────────────────────

@test "apply flag with policy off stays report-only" {
    AUTOSPEC_GROOMING_POLICY=off run bash "$SCRIPT" --repo owner/repo --apply
    [ "$status" -eq 0 ]
    echo "$output" | jq -e . >/dev/null
    [ "$(echo "$output" | jq -r '.dry')" = "true" ]
    [ "$(echo "$output" | jq -r '.filed')" = "0" ]
    ! grep -q 'issue edit' "$GH_LOG"
}

@test "policy auto without --apply stays report-only" {
    AUTOSPEC_GROOMING_POLICY=auto run bash "$SCRIPT" --repo owner/repo
    [ "$status" -eq 0 ]
    [ "$(echo "$output" | jq -r '.dry')" = "true" ]
    [ "$(echo "$output" | jq -r '.filed')" = "0" ]
    ! grep -q 'issue edit' "$GH_LOG"
}

# ── Apply mode enabled (both gates) ────────────────────────────────────────────

@test "apply mode enabled promotes the candidate with correct label mutations" {
    AUTOSPEC_GROOMING_POLICY=auto run bash "$SCRIPT" --repo owner/repo --apply
    [ "$status" -eq 0 ]
    echo "$output" | jq -e . >/dev/null
    [ "$(echo "$output" | jq -r '.dry')" = "false" ]
    [ "$(echo "$output" | jq -r '.filed')" = "1" ]
    [ "$(echo "$output" | jq -r '.promoted[0]')" = "101" ]

    # Model-fit classification still removes needs-classify, but the shell must
    # never add auto-implement itself.
    grep -q 'issue edit 101' "$GH_LOG"
    grep -E 'issue edit 101 .*--remove-label' "$GH_LOG" | grep -q 'needs-classify'
    # A ctx:* and reasoning:* label were applied (not coupled to a specific tier).
    grep -E 'issue edit 101 .*--add-label' "$GH_LOG" | grep -q 'ctx:'
    grep -E 'issue edit 101 .*--add-label' "$GH_LOG" | grep -q 'reasoning:'
    grep -q 'autospec issue promote --repo owner/repo --number 101 --remove-label needs-autospec-template --json' "$GH_LOG"
    ! grep -E 'issue edit 101 .*--add-label' "$GH_LOG" | grep -q 'auto-implement'
}

# ── Selector exclusions ────────────────────────────────────────────────────────

@test "excludes epic / needs-autospec-template / blocked / short-body issues" {
    cat > "$ISSUES_JSON" <<'EOF'
[
  {"number": 201, "title": "epic", "labels": [{"name": "needs-classify"},{"name": "epic"}], "body": "A sufficiently long body that easily clears the eighty character minimum threshold for promotion selection."},
  {"number": 202, "title": "template", "labels": [{"name": "needs-classify"},{"name": "needs-autospec-template"}], "body": "A sufficiently long body that easily clears the eighty character minimum threshold for promotion selection."},
  {"number": 203, "title": "blocked", "labels": [{"name": "needs-classify"},{"name": "blocked-dependency"}], "body": "A sufficiently long body that easily clears the eighty character minimum threshold for promotion selection."},
  {"number": 204, "title": "stub", "labels": [{"name": "needs-classify"}], "body": "too short"},
  {"number": 205, "title": "already queued", "labels": [{"name": "needs-classify"},{"name": "auto-implement"}], "body": "A sufficiently long body that easily clears the eighty character minimum threshold for promotion selection."}
]
EOF
    AUTOSPEC_GROOMING_POLICY=auto run bash "$SCRIPT" --repo owner/repo --apply
    [ "$status" -eq 0 ]
    echo "$output" | jq -e . >/dev/null
    # Nothing promoted; nothing mutated.
    [ "$(echo "$output" | jq -r '.filed')" = "0" ]
    [ "$(echo "$output" | jq -r '.promoted | length')" = "0" ]
    ! grep -q 'issue edit' "$GH_LOG"

    # Each exclusion recorded with a meaningful reason.
    [ "$(echo "$output" | jq -r '.skipped[] | select(.issue == 201) | .reason')" != "" ]
    echo "$output" | jq -r '.skipped[] | select(.issue == 201) | .reason' | grep -q 'epic'
    echo "$output" | jq -r '.skipped[] | select(.issue == 202) | .reason' | grep -q 'needs-autospec-template'
    echo "$output" | jq -r '.skipped[] | select(.issue == 203) | .reason' | grep -q 'blocked'
    echo "$output" | jq -r '.skipped[] | select(.issue == 204) | .reason' | grep -q 'insufficient-body'
    echo "$output" | jq -r '.skipped[] | select(.issue == 205) | .reason' | grep -q 'auto-implement'
}

# ── Empty candidate set ────────────────────────────────────────────────────────

@test "no open needs-classify issues emits valid empty JSON" {
    printf '[]\n' > "$ISSUES_JSON"
    run bash "$SCRIPT" --repo owner/repo
    [ "$status" -eq 0 ]
    echo "$output" | jq -e . >/dev/null
    [ "$(echo "$output" | jq -r '.filed')" = "0" ]
    [ "$(echo "$output" | jq -r '.promoted | length')" = "0" ]
    [ "$(echo "$output" | jq -r '.skipped | length')" = "0" ]
}

# ── --help ─────────────────────────────────────────────────────────────────────

@test "--help exits 0 and prints usage" {
    run bash "$SCRIPT" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *"autonomous-promote-open-issues"* ]]
}

@test "unknown flag exits non-zero" {
    run bash "$SCRIPT" --bogus-flag
    [ "$status" -ne 0 ]
}

# ── filed is always a bare integer / dry is a bool (loop parsing contract) ─────

@test "output filed is integer and dry is boolean type" {
    run bash "$SCRIPT" --repo owner/repo
    [ "$status" -eq 0 ]
    [ "$(echo "$output" | jq -r '.filed | type')" = "number" ]
    [ "$(echo "$output" | jq -r '.dry | type')" = "boolean" ]
    [ "$(echo "$output" | jq -r '.promoted | type')" = "array" ]
}

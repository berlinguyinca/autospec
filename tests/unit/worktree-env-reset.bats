#!/usr/bin/env bats
# Unit regression for issue #1841: the premerge gate must not prefer stale
# AUTOSPEC_REPO_DIR state when running from an issue worktree.

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/autonomous-premerge-gate.sh"

setup() {
  TMP="$(mktemp -d)"
  PARENT_REPO="$TMP/parent"
  ACTIVE_WT="$(mktemp -d /tmp/wt-env-reset-unit.XXXXXX)"
  mkdir -p "$TMP/bin" "$PARENT_REPO/.git" "$PARENT_REPO/.autospec" "$ACTIVE_WT/.autospec"
  export PATH="$TMP/bin:$PATH"

  cat > "$TMP/bin/autospec-qa" <<'SH'
#!/usr/bin/env bash
printf 'autospec-qa: all checks passed\n'
exit 0
SH
  chmod +x "$TMP/bin/autospec-qa"

  cat > "$TMP/bin/autospec-secaudit" <<'SH'
#!/usr/bin/env bash
printf 'autospec-secaudit: all checks passed\n'
exit 0
SH
  chmod +x "$TMP/bin/autospec-secaudit"

  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
exit 0
SH
  chmod +x "$TMP/bin/gh"

  cat > "$TMP/bin/notify.sh" <<'SH'
#!/usr/bin/env bash
exit 0
SH
  chmod +x "$TMP/bin/notify.sh"

  printf '{"verdict":"FAIL","findings":[{"release_blocking":true}]}' > "$PARENT_REPO/.autospec/qa-verdict.json"
  printf '{"verdict":"PASS","findings":[]}' > "$ACTIVE_WT/.autospec/qa-verdict.json"
}

teardown() {
  rm -rf "$TMP" "$ACTIVE_WT"
}

@test "premerge gate uses active worktree verdict over stale AUTOSPEC_REPO_DIR" {
  export AUTOSPEC_REPO_DIR="$PARENT_REPO"
  export AUTOSPEC_QA_PRESENT_OVERRIDE=true
  export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

  run bash -c "cd '$ACTIVE_WT' && bash '$SCRIPT' --pr-branch feat/unit-env-reset --notify-sh '$TMP/bin/notify.sh'"

  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q "repo-state: ignoring stale AUTOSPEC_REPO_DIR=.*parent; using active worktree .*wt-env-reset-unit"
  printf '%s\n' "$output" | grep -q '^merge-ok$'
}

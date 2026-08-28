#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
HELPER="$REPO_ROOT/skills/autospec-shared/scripts/project-sync-issue.sh"

setup() {
  TMP="$(mktemp -d)"
  mkdir -p "$TMP/bin"
  cat > "$TMP/bin/autospec" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$AUTOSPEC_CALLS"
if [ -n "${AUTOSPEC_SYNC_FAIL:-}" ]; then
  echo "sync denied" >&2
  exit 9
fi
SH
  chmod +x "$TMP/bin/autospec"
  export AUTOSPEC_CALLS="$TMP/autospec.calls"
  export PATH="$TMP/bin:$PATH"
}

teardown() {
  rm -rf "$TMP"
  unset AUTOSPEC_CALLS AUTOSPEC_SYNC_FAIL AUTOSPEC_DRY_RUN
}

@test "shared issue projection invokes the managed sync boundary" {
  run bash "$HELPER" "https://github.com/acme/widgets/issues/42" "$TMP/repo"
  [ "$status" -eq 0 ]
  [ "$output" = "" ]
  grep -Fxq "project sync --repo-dir $TMP/repo --issue-url https://github.com/acme/widgets/issues/42" "$AUTOSPEC_CALLS"
}

@test "shared issue projection skips dry-run and degrades on sync failure" {
  run env AUTOSPEC_DRY_RUN=1 bash "$HELPER" "https://github.com/acme/widgets/issues/42" "$TMP/repo"
  [ "$status" -eq 0 ]
  [ ! -e "$AUTOSPEC_CALLS" ]

  run env AUTOSPEC_SYNC_FAIL=1 bash "$HELPER" "https://github.com/acme/widgets/issues/42" "$TMP/repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"WARNING: managed Project sync failed"* ]]
  [ "$(wc -l < "$AUTOSPEC_CALLS" | tr -d ' ')" -eq 1 ]
}

@test "all product issue publishers use the shared projection boundary" {
  publishers=(
    scripts/autospec-explore.sh
    scripts/autonomous-self-improvement.sh
    scripts/autospec-gap-miner.sh
    scripts/qa-finding-to-issue.sh
    scripts/qa-brute-force-sweep.sh
    skills/autospec-shared/scripts/autospec-self-issue.sh
    skills/autospec-shared/scripts/doc-freshness-tier.sh
    skills/autospec-shared/scripts/gap-remediation-loop.sh
    skills/autospec-shared/scripts/grow-define-file-issues.sh
    skills/autospec-shared/scripts/repo-quality-audit.sh
    crates/autospec-cli/src/commands/autonomous/tier2_publisher.rs
  )
  for publisher in "${publishers[@]}"; do
    grep -q "project-sync-issue\|project_sync_issue" "$REPO_ROOT/$publisher" || {
      echo "missing managed Project sync at $publisher" >&2
      return 1
    }
  done
  ! grep -q "project-sync-issue" "$REPO_ROOT/scripts/project-board-control-mirror.sh"
}

@test "define split and classify require sync after successful non-dry-run mutation" {
  for skill in autospec-define autospec-split autospec-classify; do
    body="$REPO_ROOT/skills/$skill/SKILL.md"
    grep -Fq 'autospec project sync --repo-dir "$PWD" --issue-url "$ISSUE_URL"' "$body"
    grep -q 'dry-run' "$body"
    grep -q 'WARNING.*Project sync' "$body"
  done
}

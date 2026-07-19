#!/usr/bin/env bash
if [ -z "${BATS_VERSION:-}" ]; then
  exec bats "$0" "$@"
fi

REPO_ROOT="${BATS_TEST_DIRNAME}/../.."
SCRIPT="$REPO_ROOT/scripts/autospec-control-plane.sh"

@test "control-plane UI scaffold smoke includes polling progress shell" {
  output="$(bash "$SCRIPT" bootstrap --dry-run --observatory-repo autospec-observatory)"
  grep -Fq -- "--- autospec-observatory/apps/web/src/App.tsx ---" <<< "$output"
  grep -Fq -- "Live Fleet" <<< "$output"
  grep -Fq -- "Run Progress" <<< "$output"
  grep -Fq -- "poll_after_ms" <<< "$output"
  grep -Fq -- "10000" <<< "$output"
}

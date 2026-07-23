#!/usr/bin/env bats

setup() { script="$BATS_TEST_DIRNAME/../scripts/explore-research-cycle.sh"; }

@test "org sweep exposes report and ledger output paths" {
  run grep -E 'org-audits/\$ORG|report\.json|report\.md|ledger\.jsonl' "$script"
  [ "$status" -eq 0 ]
}

@test "org sweep loads prior ledger and records stale recheck" {
  run grep -E 'AUTOSPEC_EXPLORE_ORG_PRIOR_LEDGER|ORG_RECHECK|ORG_MAX_AGE|stale_recheck' "$script"
  [ "$status" -eq 0 ]
}

@test "org option is parsed independently from repository output" {
  run grep -E -- '--org\)|--org-max-age\)' "$script"
  [ "$status" -eq 0 ]
}

#!/usr/bin/env bats

LIB="${BATS_TEST_DIRNAME}/../../scripts/gap-json-lib.sh"

@test "historical gap without attribution remains valid" {
  run bash -c ". '$LIB'; gap_validate_object '{\"gap_id\":\"G1\",\"dimension\":\"correctness\",\"severity\":\"low\",\"file\":\"a\",\"line\":1,\"title\":\"t\",\"body\":\"b\",\"dedupe_key\":\"k\"}'"
  [ "$status" -eq 0 ]
}

@test "complete attributed gap validates and partial attribution fails closed" {
  complete='{"gap_id":"G1","dimension":"correctness","severity":"high","file":"a","line":1,"title":"t","body":"b","dedupe_key":"k","attribution_status":"attributed","originating_pr":1,"originating_commit":"abc","review_receipt_digest":"sha256:r","reviewer_harness":"codex","reviewer_reasoning":"high","provider_diversified":true,"review_risk":"integration"}'
  run bash -c ". '$LIB'; gap_validate_object '$complete'"
  [ "$status" -eq 0 ]
  partial="$(printf '%s' "$complete" | jq -c 'del(.review_receipt_digest)')"
  run bash -c ". '$LIB'; gap_validate_object '$partial'"
  [ "$status" -eq 1 ]
}

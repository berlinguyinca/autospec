#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SCRIPT="$REPO_ROOT/scripts/autonomous-self-improvement.sh"
  TMP="$(mktemp -d -t review-escape-learning.XXXXXX)"
  WORK="$TMP/repo"
  mkdir -p "$WORK/.autospec" "$WORK/scripts" "$TMP/bin"
  : > "$WORK/scripts/autospec-run-events.sh"
  git -C "$WORK" init -q
  git -C "$WORK" config user.email test@example.com
  git -C "$WORK" config user.name Test
  git -C "$WORK" add scripts/autospec-run-events.sh
  git -C "$WORK" commit -qm base
  printf 'policy-v2\n' > "$WORK/scripts/policy.txt"
  git -C "$WORK" add scripts/policy.txt
  git -C "$WORK" commit -qm 'policy change'
  CHANGE_COMMIT="$(git -C "$WORK" rev-parse HEAD)"
  OUTCOMES="$WORK/.autospec/review-outcomes.jsonl"
  GAPS="$WORK/.autospec/gaps.json"
  LEARNING="$WORK/.autospec/review-learning.jsonl"
  LIFECYCLE="$WORK/.autospec/review-policy-lifecycle.jsonl"
}

teardown() { rm -rf "$TMP"; }

seed_high_escape() {
  cat > "$OUTCOMES" <<'JSONL'
{"schema":1,"outcome_digest":"sha256:o1","pr":123,"commit":"0123456789abcdef0123456789abcdef01234567","review_receipt_digest":"sha256:r1","reviewer_harness":"codex","reviewer_reasoning":"high","provider_diversified":true,"review_risk":"integration","first_pass_lgtm":true,"escaped_high_severity":1,"escaped_total":1,"review_cost":100,"phase55_run":"run-1"}
JSONL
  cat > "$GAPS" <<'JSON'
[{"gap_id":"G1","dimension":"integration-wiring","severity":"high","file":"scripts/consumer.sh","line":7,"title":"producer artifact was never consumed","body":"The producer emitted state that the named consumer ignored.","dedupe_key":"missing-consumer-wiring","attribution_status":"attributed","originating_pr":123,"originating_commit":"0123456789abcdef0123456789abcdef01234567","review_receipt_digest":"sha256:r1","reviewer_harness":"codex","reviewer_reasoning":"high","provider_diversified":true,"review_risk":"integration","failed_invariant":"every emitted artifact has a named consumer","named_consumer":"scripts/consumer.sh"}]
JSON
}

run_candidates() {
  run bash "$SCRIPT" candidates --repo-root "$WORK" --review-outcomes "$OUTCOMES" \
    --gaps "$GAPS" --learning-ledger "$LEARNING" --lifecycle-ledger "$LIFECYCLE"
}

write_clean_outcomes() {
  local count="$1" key="$2" cost="${3:-100}" diversified="${4:-true}"
  : > "$OUTCOMES"
  local i=1
  while [ "$i" -le "$count" ]; do
    printf '{"schema":1,"outcome_digest":"sha256:clean-%d","experiment_dedupe_key":"%s","experiment_commit":"%s","pr":%d,"commit":"%s","review_receipt_digest":"sha256:r%d","reviewer_harness":"codex","reviewer_reasoning":"high","provider_diversified":%s,"review_risk":"integration","first_pass_lgtm":true,"escaped_high_severity":0,"escaped_total":0,"review_cost":%d,"phase55_run":"canary"}\n' \
      "$i" "$key" "$CHANGE_COMMIT" "$i" "$CHANGE_COMMIT" "$i" "$diversified" "$cost" >> "$OUTCOMES"
    i=$((i + 1))
  done
}

write_experiment_proof() {
  local candidate_file="$1"
  local candidate_digest
  candidate_digest="sha256:$(jq -cS . "$candidate_file" | tr -d '\n' | sha256sum | awk '{print $1}')"
  cat > "$TMP/experiment-proof.json" <<JSON
{"schema":1,"dedupe_key":"$(jq -r .dedupe_key "$candidate_file")","candidate_digest":"$candidate_digest","change_commit":"$CHANGE_COMMIT","targeted_validation":{"status":"pass","commit":"$CHANGE_COMMIT","recipe":"git-diff-check"},"full_validation":{"status":"pass","commit":"$CHANGE_COMMIT","recipe":"git-diff-check"},"protected_boundaries":{"status":"pass","commit":"$CHANGE_COMMIT","changed":false},"rollback":{"status":"ready","commit":"$CHANGE_COMMIT","prior_policy_digest":"$(jq -r .rollback.prior_policy_digest "$candidate_file")","command":"git revert --no-edit $CHANGE_COMMIT"}}
JSON
}

@test "high attributed escape creates exactly one strengthening candidate with eight questions and the hypothesis contract" {
  seed_high_escape
  run_candidates
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q '"workstream": "review-policy"'
  [ "$(printf '%s\n' "$output" | jq -s 'map(select(.workstream == "review-policy")) | length')" -eq 1 ]
  candidate="$(printf '%s\n' "$output" | jq -c 'select(.workstream == "review-policy")')"
  printf '%s' "$candidate" | jq -e '
    .change_class == "strengthening" and .frequency == 1 and
    (.questions | length == 8) and
    (.evidence | length > 0) and (.failed_invariant | length > 0) and
    (.named_consumer | length > 0) and
    (.falsifier.command | contains("advisor-observe.sh --outcomes")) and
    (.falsifier.command | contains("escaped_high_rate")) and
    (.files | length > 0 and length <= 3) and (.dedupe_key | length > 0) and
    (.before_after.before.escaped_high_rate >= 0) and
    (.before_after.after.escaped_high_rate == 0) and
    .sample_floor == 20 and .max_cost_regression >= 0 and
    (.rollback.prior_policy_digest | startswith("sha256:"))' >/dev/null
  jq -s -e 'map(.state) == ["candidate"]' "$LIFECYCLE" >/dev/null
}

@test "repeated attributed escape increases frequency without duplicate candidates" {
  seed_high_escape
  run_candidates
  [ "$status" -eq 0 ]
  run_candidates
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" | grep -q '"frequency": 2'
  [ "$(printf '%s\n' "$output" | jq -s 'map(select(.workstream == "review-policy")) | length')" -eq 1 ]
  printf '%s\n' "$output" | jq -e 'select(.workstream == "review-policy" and .frequency == 2)' >/dev/null
  jq -s -e 'map(select(.state == "candidate")) | length == 1 and .[0].frequency == 1' "$LIFECYCLE" >/dev/null
  jq -s -e 'map(select(.event == "frequency_observed")) | length == 1 and .[0].frequency == 2' "$LEARNING" >/dev/null
}

@test "no attributed evidence creates no review-policy candidate" {
  : > "$OUTCOMES"
  printf '[]\n' > "$GAPS"
  run_candidates
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  ! printf '%s\n' "$output" | grep -q .
  [ ! -s "$LIFECYCLE" ]
}

@test "a superseding clean correction removes the prior high escape from candidate evidence" {
  seed_high_escape
  cat >> "$OUTCOMES" <<'JSONL'
{"schema":1,"outcome_digest":"sha256:o1-corrected","supersedes_outcome_digest":"sha256:o1","pr":123,"commit":"0123456789abcdef0123456789abcdef01234567","review_receipt_digest":"sha256:r1","reviewer_harness":"codex","reviewer_reasoning":"high","provider_diversified":true,"review_risk":"integration","first_pass_lgtm":true,"escaped_high_severity":0,"escaped_total":0,"review_cost":100,"phase55_run":"run-1-corrected"}
JSONL
  run_candidates
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  [ ! -s "$LIFECYCLE" ]
}

@test "weakening candidate remains report-only and never files an issue" {
  seed_high_escape
  jq '.[0].proposed_change_class = "weakening"' "$GAPS" > "$GAPS.tmp"
  mv "$GAPS.tmp" "$GAPS"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
SH
  chmod +x "$TMP/bin/gh"
  export PATH="$TMP/bin:$PATH" GH_LOG="$TMP/gh.log" AUTOSPEC_SELF_IMPROVEMENT_APPLY=1
  run bash "$SCRIPT" apply --repo-root "$WORK" --repo org/repo --apply \
    --review-outcomes "$OUTCOMES" --gaps "$GAPS" --learning-ledger "$LEARNING" \
    --lifecycle-ledger "$LIFECYCLE"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.filed == 0 and .report_only == 1' >/dev/null
  [ ! -f "$TMP/gh.log" ]
}

@test "below-floor canary is held and records candidate shadow canary held append-only states" {
  seed_high_escape
  run_candidates
  candidate="$(printf '%s\n' "$output" | jq -c 'select(.workstream == "review-policy")')"
  key="$(printf '%s' "$candidate" | jq -r '.dedupe_key')"
  printf '%s' "$candidate" > "$TMP/candidate.json"
  write_experiment_proof "$TMP/candidate.json"
  write_clean_outcomes 19 "$key"
  run bash "$SCRIPT" evaluate --repo-root "$WORK" --candidate "$TMP/candidate.json" \
    --review-outcomes "$OUTCOMES" --lifecycle-ledger "$LIFECYCLE" \
    --rollback-digest "$(printf '%s' "$candidate" | jq -r '.rollback.prior_policy_digest')" \
    --experiment-proof "$TMP/experiment-proof.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.state == "held" and .reason == "sample_floor"' >/dev/null
  jq -s -e 'map(.state) | index("candidate") != null and index("shadow") != null and
    index("canary") != null and index("held") != null' "$LIFECYCLE" >/dev/null
}

@test "canary regression records a prior-policy rollback" {
  seed_high_escape
  run_candidates
  candidate="$(printf '%s\n' "$output" | jq -c 'select(.workstream == "review-policy")')"
  key="$(printf '%s' "$candidate" | jq -r '.dedupe_key')"
  printf '%s' "$candidate" > "$TMP/candidate.json"
  write_experiment_proof "$TMP/candidate.json"
  write_clean_outcomes 3 "$key"
  jq -c '.escaped_high_severity = 1 | .escaped_total = 1' "$OUTCOMES" > "$OUTCOMES.tmp"
  mv "$OUTCOMES.tmp" "$OUTCOMES"
  rollback="$(printf '%s' "$candidate" | jq -r '.rollback.prior_policy_digest')"
  run bash "$SCRIPT" evaluate --repo-root "$WORK" --candidate "$TMP/candidate.json" \
    --review-outcomes "$OUTCOMES" --lifecycle-ledger "$LIFECYCLE" --rollback-digest "$rollback" \
    --experiment-proof "$TMP/experiment-proof.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.state == "rollback_required"' >/dev/null
  git -C "$WORK" revert --no-edit "$CHANGE_COMMIT" >/dev/null
  rollback_commit="$(git -C "$WORK" rev-parse HEAD)"
  cat > "$TMP/rollback-proof.json" <<JSON
{"schema":1,"status":"executed","change_commit":"$CHANGE_COMMIT","rollback_commit":"$rollback_commit","prior_policy_digest":"$rollback"}
JSON
  run bash "$SCRIPT" evaluate --repo-root "$WORK" --candidate "$TMP/candidate.json" \
    --review-outcomes "$OUTCOMES" --lifecycle-ledger "$LIFECYCLE" --rollback-digest "$rollback" \
    --experiment-proof "$TMP/experiment-proof.json" --rollback-proof "$TMP/rollback-proof.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e --arg rollback "$rollback" '.state == "rolled_back" and .rollback.prior_policy_digest == $rollback' >/dev/null
  jq -s -e '.[-1].state == "rolled_back"' "$LIFECYCLE" >/dev/null
}

@test "the next clean attributed outcome identifies and promotes the successful experiment" {
  seed_high_escape
  run_candidates
  candidate="$(printf '%s\n' "$output" | jq -c 'select(.workstream == "review-policy")')"
  key="$(printf '%s' "$candidate" | jq -r '.dedupe_key')"
  printf '%s' "$candidate" > "$TMP/candidate.json"
  write_experiment_proof "$TMP/candidate.json"
  write_clean_outcomes 20 "$key" 100 true
  rollback="$(printf '%s' "$candidate" | jq -r '.rollback.prior_policy_digest')"
  run bash "$SCRIPT" evaluate --repo-root "$WORK" --candidate "$TMP/candidate.json" \
    --review-outcomes "$OUTCOMES" --lifecycle-ledger "$LIFECYCLE" --rollback-digest "$rollback" \
    --experiment-proof "$TMP/experiment-proof.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.state == "promoted" and
    .successful_outcome_digest == "sha256:clean-20" and .provider_diverse_review == true' >/dev/null
  jq -s -e '.[-1].state == "promoted" and .[-1].successful_outcome_digest == "sha256:clean-20"' "$LIFECYCLE" >/dev/null
}

@test "unattributed canary rows cannot satisfy the promotion sample floor" {
  seed_high_escape
  run_candidates
  candidate="$(printf '%s\n' "$output" | jq -c 'select(.workstream == "review-policy")')"
  key="$(printf '%s' "$candidate" | jq -r '.dedupe_key')"
  printf '%s' "$candidate" > "$TMP/candidate.json"
  write_experiment_proof "$TMP/candidate.json"
  write_clean_outcomes 20 "$key" 100 true
  jq -c 'del(.review_receipt_digest)' "$OUTCOMES" > "$OUTCOMES.tmp"
  mv "$OUTCOMES.tmp" "$OUTCOMES"
  rollback="$(printf '%s' "$candidate" | jq -r '.rollback.prior_policy_digest')"
  run bash "$SCRIPT" evaluate --repo-root "$WORK" --candidate "$TMP/candidate.json" \
    --review-outcomes "$OUTCOMES" --lifecycle-ledger "$LIFECYCLE" --rollback-digest "$rollback" \
    --experiment-proof "$TMP/experiment-proof.json"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.state == "held" and .reason == "sample_floor" and .samples == 0' >/dev/null
}

@test "promotion is held when exact-commit validation proof is absent" {
  seed_high_escape
  run_candidates
  candidate="$(printf '%s\n' "$output" | jq -c 'select(.workstream == "review-policy")')"
  key="$(printf '%s' "$candidate" | jq -r '.dedupe_key')"
  printf '%s' "$candidate" > "$TMP/candidate.json"
  write_clean_outcomes 20 "$key" 100 true
  run bash "$SCRIPT" evaluate --repo-root "$WORK" --candidate "$TMP/candidate.json" \
    --review-outcomes "$OUTCOMES" --lifecycle-ledger "$LIFECYCLE" \
    --rollback-digest "$(printf '%s' "$candidate" | jq -r '.rollback.prior_policy_digest')"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.state == "held" and .reason == "experiment_proof_required"' >/dev/null
}

@test "self-asserted passing validation cannot promote when reproduction fails" {
  seed_high_escape
  run_candidates
  candidate="$(printf '%s\n' "$output" | jq -c 'select(.workstream == "review-policy")')"
  key="$(printf '%s' "$candidate" | jq -r '.dedupe_key')"
  printf '%s' "$candidate" > "$TMP/candidate.json"
  write_experiment_proof "$TMP/candidate.json"
  jq '.targeted_validation.recipe = "unknown-recipe"' \
    "$TMP/experiment-proof.json" > "$TMP/bad-proof.json"
  write_clean_outcomes 20 "$key" 100 true

  run bash "$SCRIPT" evaluate --repo-root "$WORK" --candidate "$TMP/candidate.json" \
    --review-outcomes "$OUTCOMES" --lifecycle-ledger "$LIFECYCLE" \
    --experiment-proof "$TMP/bad-proof.json" \
    --rollback-digest "$(printf '%s' "$candidate" | jq -r '.rollback.prior_policy_digest')"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.state == "held" and .reason == "experiment_proof_required"' >/dev/null
}

@test "proof-supplied commands are rejected without execution" {
  seed_high_escape
  run_candidates
  candidate="$(printf '%s\n' "$output" | jq -c 'select(.workstream == "review-policy")')"
  key="$(printf '%s' "$candidate" | jq -r '.dedupe_key')"
  printf '%s' "$candidate" > "$TMP/candidate.json"
  write_experiment_proof "$TMP/candidate.json"
  cat > "$TMP/bin/cargo" <<'SH'
#!/usr/bin/env bash
touch "$COMMAND_EXECUTED"
exit 0
SH
  chmod +x "$TMP/bin/cargo"
  export PATH="$TMP/bin:$PATH" COMMAND_EXECUTED="$TMP/command-executed"
  jq '.targeted_validation = {status:"pass", commit:.change_commit, argv:["cargo","publish"]}' \
    "$TMP/experiment-proof.json" > "$TMP/command-proof.json"
  write_clean_outcomes 20 "$key" 100 true

  run bash "$SCRIPT" evaluate --repo-root "$WORK" --candidate "$TMP/candidate.json" \
    --review-outcomes "$OUTCOMES" --lifecycle-ledger "$LIFECYCLE" \
    --experiment-proof "$TMP/command-proof.json" \
    --rollback-digest "$(printf '%s' "$candidate" | jq -r '.rollback.prior_policy_digest')"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.state == "held" and .reason == "experiment_proof_required"' >/dev/null
  [ ! -e "$COMMAND_EXECUTED" ]
}

@test "generic files cannot conceal protected authority changes" {
  seed_high_escape
  run_candidates
  candidate="$(printf '%s\n' "$output" | jq -c 'select(.workstream == "review-policy")')"
  key="$(printf '%s' "$candidate" | jq -r '.dedupe_key')"
  mkdir -p "$WORK/crates/autospec-cli/src/commands"
  printf 'const AUTO_MERGE: bool = true;\n' > "$WORK/crates/autospec-cli/src/commands/autonomous.rs"
  git -C "$WORK" add crates/autospec-cli/src/commands/autonomous.rs
  git -C "$WORK" commit -qm 'change merge authority in a generic file'
  CHANGE_COMMIT="$(git -C "$WORK" rev-parse HEAD)"
  printf '%s' "$candidate" > "$TMP/candidate.json"
  write_experiment_proof "$TMP/candidate.json"
  write_clean_outcomes 20 "$key" 100 true

  run bash "$SCRIPT" evaluate --repo-root "$WORK" --candidate "$TMP/candidate.json" \
    --review-outcomes "$OUTCOMES" --lifecycle-ledger "$LIFECYCLE" \
    --experiment-proof "$TMP/experiment-proof.json" \
    --rollback-digest "$(printf '%s' "$candidate" | jq -r '.rollback.prior_policy_digest')"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.state == "held" and .reason == "experiment_proof_required"' >/dev/null
}

@test "advance consumes repo-scoped evidence and promotes without operator input" {
  seed_high_escape
  run_candidates
  candidate="$(printf '%s\n' "$output" | jq -c 'select(.workstream == "review-policy")')"
  key="$(printf '%s' "$candidate" | jq -r '.dedupe_key')"
  printf '%s' "$candidate" > "$TMP/candidate.json"
  write_experiment_proof "$TMP/candidate.json"
  mkdir -p "$WORK/.autospec/self-improvement-evidence"
  cp "$TMP/experiment-proof.json" "$WORK/.autospec/self-improvement-evidence/experiment.json"
  write_clean_outcomes 20 "$key" 100 true

  run bash "$SCRIPT" advance --repo-root "$WORK" --review-outcomes "$OUTCOMES" \
    --lifecycle-ledger "$LIFECYCLE"
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '.advanced == 1 and .promoted == 1 and .held == 0' >/dev/null
  jq -s -e '.[-1].state == "promoted" and .[-1].falsifier_passed == true' "$LIFECYCLE" >/dev/null
}

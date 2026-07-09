#!/usr/bin/env bats
# discovery-source-proposal.bats — LLM source-proposal + probation-tier admission.
# No mocks, no live LLM/network: the "LLM proposal" is a fixture JSON file fed
# directly to the script; the deterministic parts under test are the parse
# retry loop, the forbidden-class gate, the max_new_sources_per_round cap, and
# probation.txt read/write/dedup.

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
SP="$REPO_ROOT/skills/autospec-shared/scripts/discovery-source-proposal.sh"

setup() {
  TMP="$(mktemp -d)"
  export AUTOSPEC_DISCOVERY_PROBATION="$TMP/probation.txt"
  CFG="$TMP/autospec.yml"
}

teardown() {
  rm -rf "$TMP"
  unset AUTOSPEC_DISCOVERY_PROBATION
}

cfg_with_cap() {
  cat > "$CFG" <<EOF
discovery:
  enabled: true
  max_new_sources_per_round: $1
EOF
}

@test "script exists and is bash -n clean" {
  [ -f "$SP" ]
  run bash -n "$SP"
  [ "$status" -eq 0 ]
}

@test "valid proposal admits a new source to probation.txt" {
  cfg_with_cap 3
  echo '{"sources":["lobste.rs"]}' > "$TMP/p.json"
  run bash "$SP" "$TMP/p.json" "$CFG"
  [ "$status" -eq 0 ]
  [ -f "$AUTOSPEC_DISCOVERY_PROBATION" ]
  run cat "$AUTOSPEC_DISCOVERY_PROBATION"
  [[ "$output" == *"lobste.rs"* ]]
}

@test "forbidden-class proposal is not admitted to probation" {
  cfg_with_cap 3
  echo '{"sources":["pastebin"]}' > "$TMP/p.json"
  run bash "$SP" "$TMP/p.json" "$CFG"
  [ "$status" -eq 0 ]
  [ ! -f "$AUTOSPEC_DISCOVERY_PROBATION" ] || ! grep -qFx "pastebin" "$AUTOSPEC_DISCOVERY_PROBATION"
}

@test "admissions never exceed max_new_sources_per_round" {
  cfg_with_cap 1
  echo '{"sources":["forum-a","forum-b","forum-c"]}' > "$TMP/p.json"
  run bash "$SP" "$TMP/p.json" "$CFG"
  [ "$status" -eq 0 ]
  [ "$(wc -l < "$AUTOSPEC_DISCOVERY_PROBATION" | tr -d ' ')" -eq 1 ]
}

@test "malformed proposal JSON exhausts retry and exits non-zero" {
  cfg_with_cap 3
  printf 'not json' > "$TMP/p.json"
  run bash "$SP" "$TMP/p.json" "$CFG"
  [ "$status" -ne 0 ]
  [ ! -f "$AUTOSPEC_DISCOVERY_PROBATION" ]
}

@test "malformed proposal JSON logs a directive per attempt" {
  cfg_with_cap 3
  printf 'not json' > "$TMP/p.json"
  run bash "$SP" "$TMP/p.json" "$CFG"
  [ "$status" -ne 0 ]
  [[ "$output" == *"attempt 5"* ]]
}

@test "duplicate source already on probation is not re-added" {
  cfg_with_cap 3
  echo "forum-a" > "$AUTOSPEC_DISCOVERY_PROBATION"
  echo '{"sources":["forum-a"]}' > "$TMP/p.json"
  run bash "$SP" "$TMP/p.json" "$CFG"
  [ "$status" -eq 0 ]
  [ "$(wc -l < "$AUTOSPEC_DISCOVERY_PROBATION" | tr -d ' ')" -eq 1 ]
}

@test "duplicate names within the same proposal are only admitted once" {
  cfg_with_cap 3
  echo '{"sources":["forum-a","forum-a"]}' > "$TMP/p.json"
  run bash "$SP" "$TMP/p.json" "$CFG"
  [ "$status" -eq 0 ]
  [ "$(wc -l < "$AUTOSPEC_DISCOVERY_PROBATION" | tr -d ' ')" -eq 1 ]
}

@test "probation.txt stays one name per line, no weight encoding" {
  cfg_with_cap 3
  echo '{"sources":["forum-a"]}' > "$TMP/p.json"
  run bash "$SP" "$TMP/p.json" "$CFG"
  [ "$status" -eq 0 ]
  run cat "$AUTOSPEC_DISCOVERY_PROBATION"
  [ "$output" = "forum-a" ]
}

@test "cap of 0 admits nothing" {
  cfg_with_cap 0
  echo '{"sources":["forum-a"]}' > "$TMP/p.json"
  run bash "$SP" "$TMP/p.json" "$CFG"
  [ "$status" -eq 0 ]
  [ ! -f "$AUTOSPEC_DISCOVERY_PROBATION" ] || [ "$(wc -l < "$AUTOSPEC_DISCOVERY_PROBATION" | tr -d ' ')" -eq 0 ]
}

@test "empty sources array admits nothing and exits 0" {
  cfg_with_cap 3
  echo '{"sources":[]}' > "$TMP/p.json"
  run bash "$SP" "$TMP/p.json" "$CFG"
  [ "$status" -eq 0 ]
}

@test "missing proposal file exhausts retry and exits non-zero" {
  cfg_with_cap 3
  run bash "$SP" "$TMP/does-not-exist.json" "$CFG"
  [ "$status" -ne 0 ]
}

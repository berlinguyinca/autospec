#!/usr/bin/env bats

setup() {
  ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  CYCLE="$ROOT/scripts/explore-research-cycle.sh"
  TMP="$(mktemp -d -t explore-timeout.XXXXXX)"
  mkdir -p "$TMP/repo" "$TMP/research"
  git -C "$TMP/repo" init -q
}

teardown() {
  rm -rf "$TMP"
}

@test "timed out researcher writes health evidence and cannot report clean dry" {
  cat > "$TMP/research/slow.sh" <<'EOF'
#!/usr/bin/env bash
sleep 10
printf '{"source":"slow","proposals":[]}\n'
EOF
  chmod +x "$TMP/research/slow.sh"

  run env AUTOSPEC_REPO_ROOT="$TMP/repo" \
    AUTOSPEC_RESEARCH_DIR="$TMP/research" \
    AUTOSPEC_RESEARCHER_TIMEOUT_SECS=1 \
    bash "$CYCLE" --research-sources slow --specialists-mode off --out "$TMP/result.json"

  [ "$status" -ne 0 ]
  jq -e '.researcher_health.selected == 1' "$TMP/result.json"
  jq -e '.researcher_health.succeeded == 0' "$TMP/result.json"
  jq -e '.researcher_health.failures == [{"source":"slow","reason":"timeout","exit_code":124}]' \
    "$TMP/result.json"
}

@test "missing configured researcher writes health evidence and cannot report clean dry" {
  run env AUTOSPEC_REPO_ROOT="$TMP/repo" \
    AUTOSPEC_RESEARCH_DIR="$TMP/research" \
    bash "$CYCLE" --research-sources absent --specialists-mode off --out "$TMP/result.json"

  [ "$status" -ne 0 ]
  jq -e '.researcher_health.failures[0].reason == "missing_script"' "$TMP/result.json"
}

@test "completed zero-proposal researcher remains a healthy dry input" {
  cat > "$TMP/research/empty.sh" <<'EOF'
#!/usr/bin/env bash
printf '{"source":"empty","proposals":[]}\n'
EOF
  chmod +x "$TMP/research/empty.sh"

  run env AUTOSPEC_REPO_ROOT="$TMP/repo" \
    AUTOSPEC_RESEARCH_DIR="$TMP/research" \
    bash "$CYCLE" --research-sources empty --specialists-mode off --out "$TMP/result.json"

  [ "$status" -eq 0 ]
  jq -e '.researcher_health == {"selected":1,"succeeded":1,"failures":[]}' "$TMP/result.json"
  jq -e '.proposals_total == 0 and (.proposals | length) == 0' "$TMP/result.json"
}

@test "researcher object without a proposals array is invalid evidence" {
  cat > "$TMP/research/malformed.sh" <<'EOF'
#!/usr/bin/env bash
printf '{}\n'
EOF
  chmod +x "$TMP/research/malformed.sh"

  run env AUTOSPEC_REPO_ROOT="$TMP/repo" \
    AUTOSPEC_RESEARCH_DIR="$TMP/research" \
    bash "$CYCLE" --research-sources malformed --specialists-mode off --out "$TMP/result.json"

  [ "$status" -ne 0 ]
  jq -e '.researcher_health.failures[0].reason == "invalid_output"' "$TMP/result.json"
}

@test "researcher proposals must be an array" {
  cat > "$TMP/research/malformed.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' '{"source":"malformed","proposals":"not-an-array"}'
EOF
  chmod +x "$TMP/research/malformed.sh"

  run env AUTOSPEC_REPO_ROOT="$TMP/repo" \
    AUTOSPEC_RESEARCH_DIR="$TMP/research" \
    bash "$CYCLE" --research-sources malformed --specialists-mode off --out "$TMP/result.json"

  [ "$status" -ne 0 ]
  jq -e '.researcher_health.failures[0].reason == "invalid_output"' "$TMP/result.json"
}

@test "nonnumeric researcher exit code cannot erase failure evidence" {
  cat > "$TMP/research/broken.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' '{"source":"broken","proposals":[],"error":"researcher_failed","exit_code":"abc"}'
EOF
  chmod +x "$TMP/research/broken.sh"

  run env AUTOSPEC_REPO_ROOT="$TMP/repo" \
    AUTOSPEC_RESEARCH_DIR="$TMP/research" \
    bash "$CYCLE" --research-sources broken --specialists-mode off --out "$TMP/result.json"

  [ "$status" -ne 0 ]
  jq -e '.researcher_health.failures[0] == {"source":"broken","reason":"invalid_output","exit_code":1}' \
    "$TMP/result.json"
}

@test "finalize rejects missing researcher health evidence" {
  printf '%s\n' '{"proposals":[]}' > "$TMP/dedup.json"
  printf '%s\n' '[]' > "$TMP/verdicts.json"

  run env AUTOSPEC_REPO_ROOT="$TMP/repo" \
    AUTOSPEC_RESEARCH_DIR="$TMP/research" \
    AUTOSPEC_EXPLORE_VERIFY_VERDICTS="$TMP/verdicts.json" \
    bash "$CYCLE" --stage finalize --deduped-in "$TMP/dedup.json" --out "$TMP/result.json"

  [ "$status" -ne 0 ]
  jq -e '.researcher_health.failures[0].reason == "invalid_output"' "$TMP/result.json"
}

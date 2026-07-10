#!/usr/bin/env bats
# Coverage for scripts/list-groomable.sh — deterministic candidate selection + dedup.

setup() {
  TMP="$(mktemp -d)"; mkdir -p "$TMP/bin"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
# stub: `gh issue list ...` → fixture; `gh` anything else → empty
case "$*" in
  *"issue list"*"--state open"*) cat "$GH_ISSUES_FIXTURE" ;;
  *"issue list"*"--state closed"*) printf '%s' "${GH_CLOSED_FIXTURE:-[]}" ;;
  *) printf '' ;;
esac
SH
  chmod +x "$TMP/bin/gh"; export PATH="$TMP/bin:$PATH"
  SCRIPT="${BATS_TEST_DIRNAME}/../../scripts/list-groomable.sh"
}
teardown() { rm -rf "$TMP"; }

@test "selects needs-classify, needs-template, and unlabeled; excludes auto-implement/hold" {
  export GH_ISSUES_FIXTURE="$TMP/open.json"
  cat > "$GH_ISSUES_FIXTURE" <<'JSON'
[{"number":10,"title":"fix: x","body":"aaaaaaaaaaaaaaaaaaaa","labels":[{"name":"needs-classify"}]},
 {"number":11,"title":"feat: y","body":"bbbbbbbbbbbbbbbbbbbb","labels":[{"name":"needs-autospec-template"}]},
 {"number":12,"title":"raw","body":"cccccccccccccccccccc","labels":[]},
 {"number":13,"title":"done","body":"dddddddddddddddddddd","labels":[{"name":"auto-implement"}]},
 {"number":14,"title":"held","body":"eeeeeeeeeeeeeeeeeeee","labels":[{"name":"hold:needs-human"}]}]
JSON
  run bash "$SCRIPT" --repo o/r --budget 10
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.candidates[].number] == [10,11,12]'
  echo "$output" | jq -e '.candidates[] | select(.number==11).class == "needs-template"'
}

@test "excludes a no-auto-labeled issue (spec guardrail)" {
  export GH_ISSUES_FIXTURE="$TMP/open.json"
  cat > "$GH_ISSUES_FIXTURE" <<'JSON'
[{"number":50,"title":"fix: keep","body":"aaaaaaaaaaaaaaaaaaaa","labels":[{"name":"needs-classify"}]},
 {"number":51,"title":"fix: skip","body":"bbbbbbbbbbbbbbbbbbbb","labels":[{"name":"needs-classify"},{"name":"no-auto"}]}]
JSON
  run bash "$SCRIPT" --repo o/r --budget 10
  [ "$status" -eq 0 ]
  echo "$output" | jq -e '[.candidates[].number] == [50]'
  echo "$output" | jq -e '.skipped[] | select(.number==51).reason == "excluded-label"'
}

@test "budget caps candidate count, oldest-first" {
  export GH_ISSUES_FIXTURE="$TMP/open.json"
  cat > "$GH_ISSUES_FIXTURE" <<'JSON'
[{"number":30,"title":"a","body":"aaaaaaaaaaaaaaaaaaaa","labels":[{"name":"needs-classify"}]},
 {"number":20,"title":"b","body":"bbbbbbbbbbbbbbbbbbbb","labels":[{"name":"needs-classify"}]},
 {"number":25,"title":"c","body":"cccccccccccccccccccc","labels":[{"name":"needs-classify"}]}]
JSON
  run bash "$SCRIPT" --repo o/r --budget 2
  echo "$output" | jq -e '[.candidates[].number] == [20,25]'
}

@test "dedups open candidate against closed no-op whose body has a newline in first 200 chars" {
  export GH_ISSUES_FIXTURE="$TMP/open.json"
  # Open candidate 40 shares title + first-200-body-chars with a closed no-op.
  # The shared body contains a literal newline within the first 200 chars —
  # this is the regression case: a newline-naive closed-hash builder splits the
  # basis and never matches, so this candidate would wrongly survive dedup.
  cat > "$GH_ISSUES_FIXTURE" <<'JSON'
[{"number":40,"title":"noop finding","body":"line one\nline two padding padding padding","labels":[{"name":"needs-classify"}]},
 {"number":41,"title":"real work","body":"zzzzzzzzzzzzzzzzzzzz","labels":[{"name":"needs-classify"}]}]
JSON
  export GH_CLOSED_FIXTURE='[{"number":9,"title":"noop finding","body":"line one\nline two padding padding padding"}]'
  run bash "$SCRIPT" --repo o/r --budget 10
  [ "$status" -eq 0 ]
  # 40 must be dropped as a closed-no-op dup; 41 must survive.
  echo "$output" | jq -e '[.candidates[].number] == [41]'
  echo "$output" | jq -e '.skipped[] | select(.number==40).reason == "dup-of-closed-noop"'
}

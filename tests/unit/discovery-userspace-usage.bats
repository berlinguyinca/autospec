#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
US="$REPO_ROOT/skills/autospec-shared/scripts/discovery-userspace-usage.sh"
TL="$REPO_ROOT/skills/autospec-shared/scripts/trend-ledger.sh"
VS="$REPO_ROOT/skills/autospec-shared/scripts/validate-trend-signal.sh"

setup() {
  TMP="$(mktemp -d)"
  export AUTOSPEC_TREND_LEDGER="$TMP/ledger.jsonl"
  export AUTOSPEC_USERSPACE_HISTORY_DIR="$TMP/history"
  mkdir -p "$AUTOSPEC_USERSPACE_HISTORY_DIR"
  CFG="$TMP/cfg.yml"
}

teardown() {
  rm -rf "$TMP"
  unset AUTOSPEC_TREND_LEDGER AUTOSPEC_USERSPACE_HISTORY_DIR
}

no_opt_out_cfg() {
  cat > "$CFG" <<'YML'
discovery:
  userspace:
    opt_out: false
YML
}

opt_out_cfg() {
  cat > "$CFG" <<'YML'
discovery:
  userspace:
    opt_out: true
YML
}

# A fixture transcript line: mimics a Claude Code session JSONL entry with a
# tool_use Bash command. Contains a raw secret-looking token to prove it never
# survives into the ledger.
transcript_line() {
  local cmd="$1"
  printf '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"%s"}}]}}\n' "$cmd"
}

write_repeated_fixture() {
  local dir="$AUTOSPEC_USERSPACE_HISTORY_DIR/proj-a"
  mkdir -p "$dir"
  {
    transcript_line "git status --short --branch RAWSECRETTOKEN12345"
    transcript_line "git status --short --branch RAWSECRETTOKEN12345"
    transcript_line "git status --short --branch RAWSECRETTOKEN12345"
    transcript_line "ls -la"
  } > "$dir/session1.jsonl"
}

@test "script exists and is bash -n clean" {
  [ -f "$US" ]
  run bash -n "$US"
  [ "$status" -eq 0 ]
}

@test "opt_out true exits 0 and writes nothing to the ledger" {
  opt_out_cfg
  write_repeated_fixture
  run bash "$US" "$CFG"
  [ "$status" -eq 0 ]
  [ ! -f "$AUTOSPEC_TREND_LEDGER" ]
}

@test "opt_out true never reads the history dir" {
  opt_out_cfg
  # History dir intentionally absent — a read attempt would fail/error if it tried.
  rm -rf "$AUTOSPEC_USERSPACE_HISTORY_DIR"
  run bash "$US" "$CFG"
  [ "$status" -eq 0 ]
  [ ! -f "$AUTOSPEC_TREND_LEDGER" ]
}

@test "repeated action in fixture produces a userspace-usage signal" {
  no_opt_out_cfg
  write_repeated_fixture
  run bash "$US" "$CFG"
  [ "$status" -eq 0 ]
  [ -f "$AUTOSPEC_TREND_LEDGER" ]
  run bash "$TL" --show --source userspace-usage --json
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq 'length')" -ge 1 ]
  echo "$output" | jq -e '.[0].source == "userspace-usage"' >/dev/null
}

@test "sanitized_excerpt never contains the raw fixture line verbatim" {
  no_opt_out_cfg
  write_repeated_fixture
  bash "$US" "$CFG"
  run bash "$TL" --show --source userspace-usage --json
  [ "$status" -eq 0 ]
  excerpt="$(echo "$output" | jq -r '.[0].sanitized_excerpt')"
  [[ "$excerpt" != *"RAWSECRETTOKEN12345"* ]]
  [[ "$excerpt" != *"git status --short --branch RAWSECRETTOKEN12345"* ]]
}

@test "no signal below the recurrence threshold (single-occurrence commands ignored)" {
  no_opt_out_cfg
  local_dir="$AUTOSPEC_USERSPACE_HISTORY_DIR/proj-b"
  mkdir -p "$local_dir"
  transcript_line "uniqcmd --once" > "$local_dir/session1.jsonl"
  bash "$US" "$CFG"
  if [ -f "$AUTOSPEC_TREND_LEDGER" ]; then
    run bash "$TL" --show --json
    [ "$(echo "$output" | jq '[.[] | select(.norm_key | contains("uniqcmd"))] | length')" -eq 0 ]
  fi
}

@test "recurrence bumps on a second run over the same recurring pattern" {
  no_opt_out_cfg
  write_repeated_fixture
  bash "$US" "$CFG"
  run bash "$TL" --show --source userspace-usage --json
  first_recurrence="$(echo "$output" | jq '.[0].recurrence')"

  bash "$US" "$CFG"
  run bash "$TL" --show --source userspace-usage --json
  second_recurrence="$(echo "$output" | jq '.[0].recurrence')"

  [ "$second_recurrence" -gt "$first_recurrence" ]
}

@test "emitted records pass validate-trend-signal.sh" {
  no_opt_out_cfg
  write_repeated_fixture
  bash "$US" "$CFG"
  while IFS= read -r line; do
    [ -z "$line" ] && continue
    tmp="$(mktemp)"
    printf '%s' "$line" > "$tmp"
    run "$VS" "$tmp"
    [ "$status" -eq 0 ]
    rm -f "$tmp"
  done < "$AUTOSPEC_TREND_LEDGER"
}

@test "missing history dir (not opted out) exits 0 with no crash" {
  no_opt_out_cfg
  rm -rf "$AUTOSPEC_USERSPACE_HISTORY_DIR"
  run bash "$US" "$CFG"
  [ "$status" -eq 0 ]
}

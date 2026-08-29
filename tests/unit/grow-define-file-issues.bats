#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
F="$REPO_ROOT/skills/autospec-shared/scripts/grow-define-file-issues.sh"
LG="$REPO_ROOT/skills/autospec-shared/scripts/growth-ledger.sh"

setup() {
  TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"
  export AUTOSPEC_BIN="$REPO_ROOT/tests/fixtures/autospec-project-sync-stub.sh"
  # gh mock: echoes a fake issue URL; records args; can be forced to fail
  mkdir -p "$TMP/bin"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
echo "$@" >> "$GH_LOG"
if [ -n "${GH_FAIL:-}" ]; then echo "gh: create failed" >&2; exit 1; fi
n="$(cat "$GH_COUNTER" 2>/dev/null || echo 100)"; n=$((n+1)); echo "$n" > "$GH_COUNTER"
echo "https://github.com/acme/site/issues/$n"
SH
  chmod +x "$TMP/bin/gh"
  export GH_LOG="$TMP/gh.log"; export GH_COUNTER="$TMP/gh.counter"
  export PATH="$TMP/bin:$PATH"
  echo '{"product":{"name":"Acme"},"site":{"repo_path":"."}}' > "$TMP/cfg.json"
}
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER GH_LOG GH_COUNTER GH_FAIL AUTOSPEC_BIN; }

ranked() {
  : > "$TMP/r.jsonl"
  echo '{"lens":"keyword-gap","channel":"content","kind":"artifact","title":"Add vs page","norm_title":"add vs page","roi":5,"effort":"small","severity":5,"confidence":0.9,"rank_score":0.9}' >> "$TMP/r.jsonl"
  echo '{"lens":"community","channel":"outreach","kind":"outbound","title":"Show HN post","norm_title":"show hn post","roi":4,"effort":"small","severity":4,"confidence":0.8,"rank_score":0.6}' >> "$TMP/r.jsonl"
  echo "$TMP/r.jsonl"
}

@test "script exists and is bash -n clean" {
  [ -f "$F" ]; run bash -n "$F"; [ "$status" -eq 0 ]
}

@test "creates an issue per candidate and appends one pending ledger line each" {
  run bash "$F" "$(ranked)" "$TMP/cfg.json"
  [ "$status" -eq 0 ]
  # two gh invocations
  [ "$(grep -c 'issue create' "$GH_LOG")" -eq 2 ]
  # artifact labels
  grep -q 'growth:artifact' "$GH_LOG"
  grep -q 'auto-implement' "$GH_LOG"
  # outbound labels
  grep -q 'growth:outbound' "$GH_LOG"
  grep -q 'growth/needs-draft' "$GH_LOG"
  # two pending ledger lines
  [ "$(grep -c '"outcome":"pending"' "$GROWTH_LEDGER")" -eq 2 ]
}

@test "gh failure files no ledger line" {
  export GH_FAIL=1
  run bash "$F" "$(ranked)" "$TMP/cfg.json"
  # script continues (logs + skips); ledger stays empty
  [ ! -f "$GROWTH_LEDGER" ] || [ "$(wc -l < "$GROWTH_LEDGER" | tr -d ' ')" -eq 0 ]
}

@test "missing input fails" {
  run bash "$F" "$TMP/nope.jsonl" "$TMP/cfg.json"
  [ "$status" -ne 0 ]
}

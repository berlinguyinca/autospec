#!/usr/bin/env bats

REPO_ROOT="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)"
S="$REPO_ROOT/skills/autospec-shared/scripts"

setup() {
  TMP="$(mktemp -d)"; export GROWTH_LEDGER="$TMP/ledger.jsonl"
  export AUTOSPEC_BIN="$REPO_ROOT/tests/fixtures/autospec-project-sync-stub.sh"
  mkdir -p "$TMP/bin"
  cat > "$TMP/bin/gh" <<'SH'
#!/usr/bin/env bash
echo "$@" >> "$GH_LOG"
n="$(cat "$GH_COUNTER" 2>/dev/null || echo 200)"; n=$((n+1)); echo "$n" > "$GH_COUNTER"
echo "https://github.com/acme/site/issues/$n"
SH
  chmod +x "$TMP/bin/gh"; export GH_LOG="$TMP/gh.log"; export GH_COUNTER="$TMP/gh.counter"
  export PATH="$TMP/bin:$PATH"
  echo '{"product":{"name":"Acme"},"site":{"repo_path":"."},"grow":{"max_issues_per_cycle":5}}' > "$TMP/cfg.json"
}
teardown() { rm -rf "$TMP"; unset GROWTH_LEDGER GH_LOG GH_COUNTER AUTOSPEC_BIN; }

@test "end-to-end: candidates -> pipeline -> file issues, deterministic" {
  # simulated lens outputs (what G1 subagents would produce)
  : > "$TMP/c.jsonl"
  echo '{"lens":"keyword-gap","channel":"content","kind":"artifact","title":"Add vs page","norm_title":"add vs page","roi":5,"effort":"small","severity":5,"confidence":0.9}' >> "$TMP/c.jsonl"
  echo '{"lens":"community","channel":"outreach","kind":"outbound","title":"Show HN","norm_title":"show hn","roi":4,"effort":"small","severity":3,"confidence":0.7}' >> "$TMP/c.jsonl"
  echo '{"lens":"directory","channel":"directories","kind":"outbound","title":"spammy","norm_title":"spammy","roi":2,"effort":"small","severity":2,"confidence":0.5}' >> "$TMP/c.jsonl"
  # simulated verify verdicts (what G2 subagents would produce): spammy refuted
  : > "$TMP/v.jsonl"
  echo '{"norm_title":"add vs page","real":true,"reason":"ok"}' >> "$TMP/v.jsonl"
  echo '{"norm_title":"show hn","real":true,"reason":"on-topic"}' >> "$TMP/v.jsonl"
  echo '{"norm_title":"spammy","real":false,"reason":"drive-by"}' >> "$TMP/v.jsonl"

  bash "$S/grow-define-pipeline.sh" "$TMP/c.jsonl" "$TMP/v.jsonl" "$TMP/cfg.json" > "$TMP/ranked.jsonl"
  # spammy refuted -> not in ranked; 2 survivors
  [ "$(grep -c '"norm_title"' "$TMP/ranked.jsonl")" -eq 2 ]
  ! grep -q '"norm_title":"spammy"' "$TMP/ranked.jsonl"
  # refuted recorded in ledger
  grep -q '"outcome":"refuted"' "$GROWTH_LEDGER"

  bash "$S/grow-define-file-issues.sh" "$TMP/ranked.jsonl" "$TMP/cfg.json" > "$TMP/nums.txt"
  # two issues filed, correct label routing
  [ "$(grep -c 'issue create' "$GH_LOG")" -eq 2 ]
  grep -q 'growth:artifact' "$GH_LOG"
  grep -q 'growth:outbound' "$GH_LOG"
  # two pending ledger lines
  [ "$(grep -c '"outcome":"pending"' "$GROWTH_LEDGER")" -eq 2 ]
}

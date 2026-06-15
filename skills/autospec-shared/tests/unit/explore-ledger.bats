#!/usr/bin/env bats
# explore-ledger.bats — tests for explore-ledger.sh append-only outcome ledger.

LEDGER_SH="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/explore-ledger.sh"

setup() {
    TEST_TMP="$(mktemp -d)"
    export AUTOSPEC_EXPLORE_LEDGER="$TEST_TMP/l.jsonl"
    BASH_BIN="$(command -v bash)"
}
teardown() {
    rm -rf "$TEST_TMP"
}

# A complete, valid record (caller supplies ts).
valid_record() {
    # $1=round $2=source $3=title $4=complexity $5=confidence $6=issue $7=pr $8=outcome
    printf '{"round":%s,"source":"%s","title":"%s","norm_title":"%s","complexity":"%s","confidence":%s,"issue":%s,"pr":%s,"outcome":"%s","reason":"","ts":"2026-06-14T00:00:00Z"}' \
        "${1:-1}" "${2:-spec-vs-code}" "${3:-Add X}" "${3:-add-x}" "${4:-small}" "${5:-0.8}" "${6:-101}" "${7:-0}" "${8:-pending}"
}

# ── --append ──────────────────────────────────────────────────────────────────

@test "append a valid record -> exit 0, one line, round-trips through jq" {
    rec="$(valid_record 1 spec-vs-code 'Add X' small 0.8 101 0 pending)"
    run bash "$LEDGER_SH" --append "$rec"
    [ "$status" -eq 0 ]
    [ -f "$AUTOSPEC_EXPLORE_LEDGER" ]
    lines=$(wc -l < "$AUTOSPEC_EXPLORE_LEDGER")
    [ "$lines" -eq 1 ]
    run jq -e '.issue == 101 and .source == "spec-vs-code"' "$AUTOSPEC_EXPLORE_LEDGER"
    [ "$status" -eq 0 ]
}

@test "append rejects a bad outcome value -> exit 1, nothing appended" {
    rec="$(valid_record 1 spec-vs-code 'Add X' small 0.8 101 0 not_a_real_outcome)"
    run bash "$LEDGER_SH" --append "$rec"
    [ "$status" -eq 1 ]
    [ ! -f "$AUTOSPEC_EXPLORE_LEDGER" ] || [ "$(wc -l < "$AUTOSPEC_EXPLORE_LEDGER")" -eq 0 ]
}

@test "append rejects a record missing a required key -> exit 1" {
    rec='{"round":1,"source":"spec-vs-code","title":"Add X","norm_title":"add-x","complexity":"small","confidence":0.8,"issue":101,"pr":0,"outcome":"pending"}'
    run bash "$LEDGER_SH" --append "$rec"
    [ "$status" -eq 1 ]
}

@test "append rejects wrong type for confidence -> exit 1" {
    rec='{"round":1,"source":"spec-vs-code","title":"Add X","norm_title":"add-x","complexity":"small","confidence":"high","issue":101,"pr":0,"outcome":"pending","reason":"","ts":"2026-06-14T00:00:00Z"}'
    run bash "$LEDGER_SH" --append "$rec"
    [ "$status" -eq 1 ]
}

@test "append creates the parent dir if missing" {
    export AUTOSPEC_EXPLORE_LEDGER="$TEST_TMP/nested/deep/l.jsonl"
    rec="$(valid_record 1 spec-vs-code 'Add X' small 0.8 101 0 pending)"
    run bash "$LEDGER_SH" --append "$rec"
    [ "$status" -eq 0 ]
    [ -f "$AUTOSPEC_EXPLORE_LEDGER" ]
}

# ── --update-outcome ───────────────────────────────────────────────────────────

@test "update-outcome appends a new line; show reflects latest; stats counts once" {
    bash "$LEDGER_SH" --append "$(valid_record 1 spec-vs-code 'Add X' small 0.8 101 0 pending)"
    run bash "$LEDGER_SH" --update-outcome 101 merged_clean "shipped fine"
    [ "$status" -eq 0 ]
    # Two physical lines (append-only audit trail).
    [ "$(wc -l < "$AUTOSPEC_EXPLORE_LEDGER")" -eq 2 ]
    # show (latest-per-issue) reflects the new outcome, only one row for issue 101.
    run bash "$LEDGER_SH" --show --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '[.[] | select(.issue==101)] | length == 1'
    echo "$output" | jq -e '.[] | select(.issue==101) | .outcome == "merged_clean"'
    # stats counts it once: filed 1, merged_clean 1 (not double).
    run bash "$LEDGER_SH" --stats --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.["spec-vs-code"].filed == 1'
    echo "$output" | jq -e '.["spec-vs-code"].merged_clean == 1'
}

@test "update-outcome for an unknown issue -> exit 2" {
    bash "$LEDGER_SH" --append "$(valid_record 1 spec-vs-code 'Add X' small 0.8 101 0 pending)"
    run bash "$LEDGER_SH" --update-outcome 999 merged_clean
    [ "$status" -eq 2 ]
}

@test "update-outcome rejects an invalid outcome value -> exit 1" {
    bash "$LEDGER_SH" --append "$(valid_record 1 spec-vs-code 'Add X' small 0.8 101 0 pending)"
    run bash "$LEDGER_SH" --update-outcome 101 bogus_outcome
    [ "$status" -eq 1 ]
}

# ── --stats ─────────────────────────────────────────────────────────────────────

@test "stats --json aggregates correctly across sources" {
    bash "$LEDGER_SH" --append "$(valid_record 1 spec-vs-code 'A' small 0.8 101 0 pending)"
    bash "$LEDGER_SH" --append "$(valid_record 1 spec-vs-code 'B' medium 0.7 102 0 pending)"
    bash "$LEDGER_SH" --append "$(valid_record 1 open-issues 'C' large 0.6 103 0 pending)"
    bash "$LEDGER_SH" --update-outcome 101 merged_clean
    bash "$LEDGER_SH" --update-outcome 102 qa_failed
    run bash "$LEDGER_SH" --stats --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.["spec-vs-code"].filed == 2'
    echo "$output" | jq -e '.["spec-vs-code"].merged_clean == 1'
    echo "$output" | jq -e '.["spec-vs-code"].failed == 1'
    echo "$output" | jq -e '.["spec-vs-code"].pending == 0'
    echo "$output" | jq -e '.["open-issues"].filed == 1'
    echo "$output" | jq -e '.["open-issues"].pending == 1'
}

@test "stats --json: failed bucket includes reverted and abandoned" {
    bash "$LEDGER_SH" --append "$(valid_record 1 internet 'A' small 0.8 201 0 reverted)"
    bash "$LEDGER_SH" --append "$(valid_record 1 internet 'B' small 0.8 202 0 abandoned)"
    bash "$LEDGER_SH" --append "$(valid_record 1 internet 'C' small 0.8 203 0 qa_failed)"
    run bash "$LEDGER_SH" --stats --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e '.["internet"].failed == 3'
}

@test "stats default (human table) exits 0 and mentions source" {
    bash "$LEDGER_SH" --append "$(valid_record 1 spec-vs-code 'A' small 0.8 101 0 merged_clean)"
    run bash "$LEDGER_SH" --stats
    [ "$status" -eq 0 ]
    [[ "$output" == *spec-vs-code* ]]
}

# ── --show ───────────────────────────────────────────────────────────────────────

@test "show --source filters to one source" {
    bash "$LEDGER_SH" --append "$(valid_record 1 spec-vs-code 'A' small 0.8 101 0 pending)"
    bash "$LEDGER_SH" --append "$(valid_record 1 open-issues 'B' small 0.8 102 0 pending)"
    run bash "$LEDGER_SH" --show --source spec-vs-code --json
    [ "$status" -eq 0 ]
    echo "$output" | jq -e 'length == 1'
    echo "$output" | jq -e '.[0].source == "spec-vs-code"'
}

# ── --validate ────────────────────────────────────────────────────────────────────

@test "validate passes on a clean ledger" {
    bash "$LEDGER_SH" --append "$(valid_record 1 spec-vs-code 'A' small 0.8 101 0 pending)"
    bash "$LEDGER_SH" --append "$(valid_record 1 open-issues 'B' small 0.8 102 0 merged_clean)"
    run bash "$LEDGER_SH" --validate
    [ "$status" -eq 0 ]
}

@test "validate fails (exit 1) on a corrupted line and reports the line number" {
    bash "$LEDGER_SH" --append "$(valid_record 1 spec-vs-code 'A' small 0.8 101 0 pending)"
    printf '%s\n' '{"round":1,"source":"x","title":"bad"}' >> "$AUTOSPEC_EXPLORE_LEDGER"
    run bash "$LEDGER_SH" --validate
    [ "$status" -eq 1 ]
    [[ "$output" == *2* ]]
}

@test "validate accepts a given file argument" {
    f="$TEST_TMP/other.jsonl"
    printf '%s\n' "$(valid_record 1 spec-vs-code 'A' small 0.8 101 0 pending)" > "$f"
    run bash "$LEDGER_SH" --validate "$f"
    [ "$status" -eq 0 ]
}

# ── jq-missing fail-closed ─────────────────────────────────────────────────────────

@test "jq missing -> exit 2 (fail-closed)" {
    EMPTY="$TEST_TMP/empty_path"
    mkdir -p "$EMPTY"
    run env PATH="$EMPTY" "$BASH_BIN" "$LEDGER_SH" --stats
    [ "$status" -eq 2 ]
}

# ── help ───────────────────────────────────────────────────────────────────────────

@test "help exits 0 and prints usage" {
    run bash "$LEDGER_SH" --help
    [ "$status" -eq 0 ]
    [[ "$output" == *explore-ledger.sh* ]]
}

# ── --rebuild ─────────────────────────────────────────────────────────────────────
#
# Stub gh on PATH. The stub dispatches on the subcommand ("issue list",
# "issue view", "pr list") and the presence of an issue number. Fixture:
#   201 merged PR #301              -> merged_clean, pr 301
#   202 OPEN, open PR #302          -> pending,      pr 302
#   203 OPEN, no PR, old            -> stalled,      pr 0
#   204 CLOSED, no merged PR        -> abandoned,    pr 0
#   205 NO marker                   -> skipped (skipped_no_marker)

setup_rebuild_gh_stub() {
    mkdir -p "$TEST_TMP/bin"
    cat > "$TEST_TMP/bin/gh" <<'EOF'
#!/usr/bin/env bash
# Dispatch on subcommand. $1=issue|pr, $2=list|view.
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
  cat <<'JSON'
[
  {"number":201,"title":"feat: Add retry to fetch","state":"CLOSED","closedAt":"2026-06-01T00:00:00Z","body":"Some body\n<!-- explore-ledger source=spec-vs-code complexity=small confidence=0.9 round=1 -->\nmore"},
  {"number":202,"title":"Improve internet research","state":"OPEN","closedAt":null,"body":"<!-- explore-ledger source=internet complexity=large confidence=0.4 round=1 -->"},
  {"number":203,"title":"Refactor codebase signals","state":"OPEN","closedAt":null,"createdAt":"2020-01-01T00:00:00Z","body":"<!-- explore-ledger source=codebase-signals complexity=medium confidence=0.6 round=2 -->"},
  {"number":204,"title":"Close out open issues scan","state":"CLOSED","closedAt":"2026-06-01T00:00:00Z","body":"<!-- explore-ledger source=open-issues complexity=small confidence=0.5 round=2 -->"},
  {"number":205,"title":"No marker here","state":"OPEN","closedAt":null,"body":"plain body, nothing structured"}
]
JSON
  exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
  # $3 is the issue number.
  case "$3" in
    201) echo '{"number":201,"state":"CLOSED","closedAt":"2026-06-01T00:00:00Z","createdAt":"2026-05-01T00:00:00Z"}'; exit 0 ;;
    202) echo '{"number":202,"state":"OPEN","closedAt":null,"createdAt":"2026-06-14T00:00:00Z"}'; exit 0 ;;
    203) echo '{"number":203,"state":"OPEN","closedAt":null,"createdAt":"2020-01-01T00:00:00Z"}'; exit 0 ;;
    204) echo '{"number":204,"state":"CLOSED","closedAt":"2026-06-01T00:00:00Z","createdAt":"2026-05-01T00:00:00Z"}'; exit 0 ;;
    *) echo '{}'; exit 0 ;;
  esac
fi
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  # Find which issue number is in the args (after --search).
  case "$*" in
    *"201"*) echo '[{"number":301,"state":"MERGED","mergedAt":"2026-06-02T00:00:00Z","baseRefName":"explore-sandbox"}]'; exit 0 ;;
    *"202"*) echo '[{"number":302,"state":"OPEN","mergedAt":null,"baseRefName":"explore-sandbox"}]'; exit 0 ;;
    *) echo '[]'; exit 0 ;;
  esac
fi
exit 0
EOF
    chmod +x "$TEST_TMP/bin/gh"
    export PATH="$TEST_TMP/bin:$PATH"
}

@test "rebuild: produces 4 valid records (205 skipped), all pass --validate" {
    setup_rebuild_gh_stub
    run bash "$LEDGER_SH" --rebuild
    [ "$status" -eq 0 ]
    [ -f "$AUTOSPEC_EXPLORE_LEDGER" ]
    [ "$(wc -l < "$AUTOSPEC_EXPLORE_LEDGER")" -eq 4 ]
    run bash "$LEDGER_SH" --validate
    [ "$status" -eq 0 ]
}

@test "rebuild: derives outcomes from PR + issue state" {
    setup_rebuild_gh_stub
    run bash "$LEDGER_SH" --rebuild
    [ "$status" -eq 0 ]
    run jq -e 'select(.issue==201) | .outcome=="merged_clean" and .pr==301' "$AUTOSPEC_EXPLORE_LEDGER"
    [ "$status" -eq 0 ]
    run jq -e 'select(.issue==202) | .outcome=="pending" and .pr==302' "$AUTOSPEC_EXPLORE_LEDGER"
    [ "$status" -eq 0 ]
    run jq -e 'select(.issue==203) | .outcome=="stalled" and .pr==0' "$AUTOSPEC_EXPLORE_LEDGER"
    [ "$status" -eq 0 ]
    run jq -e 'select(.issue==204) | .outcome=="abandoned" and .pr==0' "$AUTOSPEC_EXPLORE_LEDGER"
    [ "$status" -eq 0 ]
}

@test "rebuild: norm_title strips conventional prefix and normalizes" {
    setup_rebuild_gh_stub
    run bash "$LEDGER_SH" --rebuild
    [ "$status" -eq 0 ]
    run jq -r 'select(.issue==201) | .norm_title' "$AUTOSPEC_EXPLORE_LEDGER"
    [ "$status" -eq 0 ]
    [ "$output" = "add retry to fetch" ]
}

@test "rebuild: with a live ledger writes <ledger>.rebuilt and does NOT modify it" {
    setup_rebuild_gh_stub
    bash "$LEDGER_SH" --append "$(valid_record 1 spec-vs-code 'Existing' small 0.8 999 0 pending)"
    before="$(cat "$AUTOSPEC_EXPLORE_LEDGER")"
    run bash "$LEDGER_SH" --rebuild
    [ "$status" -eq 0 ]
    [ -f "$AUTOSPEC_EXPLORE_LEDGER.rebuilt" ]
    [ "$(cat "$AUTOSPEC_EXPLORE_LEDGER")" = "$before" ]
    [ "$(wc -l < "$AUTOSPEC_EXPLORE_LEDGER.rebuilt")" -eq 4 ]
    [[ "$output" == *".rebuilt"* ]]
}

@test "rebuild: summary line counts are correct" {
    setup_rebuild_gh_stub
    run bash "$LEDGER_SH" --rebuild
    [ "$status" -eq 0 ]
    [[ "$output" == *"issues=5"* ]]
    [[ "$output" == *"parsed=4"* ]]
    [[ "$output" == *"skipped_no_marker=1"* ]]
    [[ "$output" == *"merged_clean=1"* ]]
    [[ "$output" == *"pending=1"* ]]
    [[ "$output" == *"stalled=1"* ]]
    [[ "$output" == *"abandoned=1"* ]]
}

@test "rebuild: gh missing -> exit 2 (fail-closed)" {
    EMPTY="$TEST_TMP/empty_path2"
    mkdir -p "$EMPTY"
    # jq present via a symlink, gh absent.
    ln -s "$(command -v jq)" "$EMPTY/jq"
    run env PATH="$EMPTY" "$BASH_BIN" "$LEDGER_SH" --rebuild
    [ "$status" -eq 2 ]
}

@test "rebuild: jq missing -> exit 2 (fail-closed)" {
    EMPTY="$TEST_TMP/empty_path3"
    mkdir -p "$EMPTY"
    run env PATH="$EMPTY" "$BASH_BIN" "$LEDGER_SH" --rebuild
    [ "$status" -eq 2 ]
}

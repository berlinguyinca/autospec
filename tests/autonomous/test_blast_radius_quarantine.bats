#!/usr/bin/env bats
# tests/autonomous/test_blast_radius_quarantine.bats — issue #1545
# Blast-radius classifier + asynchronous human-review quarantine queue.

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
GUARDRAILS="$REPO_ROOT/scripts/autonomous-guardrails.sh"
PREMERGE="$REPO_ROOT/scripts/autonomous-premerge-gate.sh"
PRIORITIZE="$REPO_ROOT/scripts/autonomous-prioritize.sh"

setup() {
    TMP="$(mktemp -d -t blast_radius_quarantine.XXXXXX)"
    export PATH="$TMP/bin:$PATH"
    mkdir -p "$TMP/bin"
    GH_LOG="$TMP/gh.log"
    QA_LOG="$TMP/qa.log"
    export GH_LOG QA_LOG

    cat > "$TMP/bin/gh" <<'STUB'
#!/usr/bin/env bash
printf 'gh %s\n' "$*" >> "$GH_LOG"
exit 0
STUB
    chmod +x "$TMP/bin/gh"

    cat > "$TMP/bin/autospec-qa" <<'STUB'
#!/usr/bin/env bash
printf 'qa-called\n' >> "$QA_LOG"
printf 'autospec-qa: all checks passed\n'
exit 0
STUB
    chmod +x "$TMP/bin/autospec-qa"

    cat > "$TMP/bin/autospec-secaudit" <<'STUB'
#!/usr/bin/env bash
printf 'autospec-secaudit: all checks passed\n'
exit 0
STUB
    chmod +x "$TMP/bin/autospec-secaudit"

    cat > "$TMP/fenced-surfaces.yml" <<'YAML'
fenced_surfaces:
  - id: trading-money-risk
    severity: fenced
    reason: trading system money and risk engine
    paths:
      - trading-system/money/**
      - trading-system/risk/**
      - trading-system/execution/**
  - id: public-contracts
    severity: high
    reason: public API contract
    paths:
      - schemas/**
YAML
}

teardown() {
    rm -rf "$TMP"
}

@test "blast-radius classifier emits low label for reversible non-fenced paths" {
    changed="$TMP/changed-low.txt"
    printf 'docs/runbooks/OPERATIONS.md\nREADME.md\n' > "$changed"

    run bash "$GUARDRAILS" blast-radius \
        --changed-files "$changed" \
        --fenced-surfaces "$TMP/fenced-surfaces.yml" \
        --json

    [ "$status" -eq 0 ]
    result="$TMP/low.json"
    printf '%s' "$output" > "$result"
    jq -e '.decision == "allow"' "$result"
    jq -e '.label == "blast:low"' "$result"
    jq -e '.reversibility == "reversible"' "$result"
    jq -e '.fenced == false and (.fenced_matches | length) == 0' "$result"
}

@test "blast-radius classifier labels fenced trading money/risk path from registry" {
    changed="$TMP/changed-fenced.txt"
    printf 'trading-system/risk/limit_engine.py\ndocs/runbooks/OPERATIONS.md\n' > "$changed"

    run bash "$GUARDRAILS" blast-radius \
        --changed-files "$changed" \
        --fenced-surfaces "$TMP/fenced-surfaces.yml" \
        --json

    [ "$status" -eq 1 ]
    result="$TMP/fenced.json"
    printf '%s' "$output" > "$result"
    jq -e '.decision == "quarantine"' "$result"
    jq -e '.label == "blast:fenced"' "$result"
    jq -e '.fenced == true' "$result"
    jq -e '.fenced_matches[0].surface == "trading-money-risk"' "$result"
    jq -e '.fenced_matches[0].path == "trading-system/risk/limit_engine.py"' "$result"
}

@test "premerge quarantines fenced-surface changes without running QA or emitting merge-ok" {
    changed="$TMP/changed-fenced.txt"
    quarantine="$TMP/quarantine/pr-1545.json"
    printf 'trading-system/money/settlement.py\n' > "$changed"
    export AUTOSPEC_QA_PRESENT_OVERRIDE=true
    export AUTOSPEC_SECAUDIT_PRESENT_OVERRIDE=true

    run bash "$PREMERGE" \
        --pr-branch feat/risky \
        --repo berlinguyinca/autospec \
        --pr 1545 \
        --changed-files "$changed" \
        --fenced-surfaces "$TMP/fenced-surfaces.yml" \
        --quarantine-out "$quarantine"

    [ "$status" -eq 0 ]
    printf '%s\n' "$output" | grep -q '^quarantine fenced_surface$'
    ! printf '%s\n' "$output" | grep -q '^merge-ok$'
    [ ! -s "$QA_LOG" ]
    grep -q 'autospec:needs-human' "$GH_LOG"
    [ -f "$quarantine" ]
    jq -e '.schema == "autospec.autonomous.quarantine.v1"' "$quarantine"
    jq -e '.repo == "berlinguyinca/autospec" and .pr == 1545' "$quarantine"
    jq -e '.classification.label == "blast:fenced"' "$quarantine"
    jq -e '.queue == "human-review"' "$quarantine"
}

@test "priority scorer skips quarantined fenced candidate and selects next runnable item" {
    candidates="$TMP/candidates.jsonl"
    cat > "$candidates" <<'JSONL'
{"id":"fenced-1","title":"change risk engine","severity":10,"value":10,"confidence":1,"reversibility":1,"effort":1,"blast_radius":1,"fenced":true,"files":["trading-system/risk/limit_engine.py"]}
{"id":"safe-2","title":"refresh docs","severity":3,"value":3,"confidence":1,"reversibility":1,"effort":1,"blast_radius":1,"files":["docs/runbooks/OPERATIONS.md"]}
JSONL

    run bash "$PRIORITIZE" score --candidates "$candidates" --value-floor 1 --human-gate-blast-radius 4

    [ "$status" -eq 0 ]
    result="$TMP/priority.json"
    printf '%s' "$output" > "$result"
    jq -e '.decision == "run"' "$result"
    jq -e '.top.id == "safe-2"' "$result"
    jq -e '[.considered_and_skipped[] | select(.id == "fenced-1" and .reason == "human_gate")] | length == 1' "$result"
}

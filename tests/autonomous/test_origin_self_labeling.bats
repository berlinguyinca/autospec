#!/usr/bin/env bats
# tests/autonomous/test_origin_self_labeling.bats — issue #1743.
# Tier 2/3 self-improvement and resilience follow-up filing sites stamp
# `origin:self` at creation so the provenance resolver (scripts/autonomous-provenance.sh,
# merged separately) can attribute these issues without inspecting the author.

bats_require_minimum_version 1.5.0

REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
SELF_IMPROVEMENT="$REPO_ROOT/scripts/autonomous-self-improvement.sh"
RESILIENCE="$REPO_ROOT/scripts/autonomous-resilience.sh"

setup() {
    TMP="$(mktemp -d -t origin_self_labeling.XXXXXX)"
    STUB_DIR="$TMP/bin"
    mkdir -p "$STUB_DIR" "$TMP/repo/crates/autospec-cli/src/commands" "$TMP/repo/docs/reports"
    export PATH="$STUB_DIR:$PATH"
    export GH_LOG="$TMP/gh.log"
}

teardown() {
    rm -rf "$TMP"
}

seed_repo_gaps() {
    cat > "$TMP/repo/crates/autospec-cli/src/commands/run.rs" <<'RS'
pub fn run(_args: &[String]) -> Result<(), String> {
    super::not_implemented("run")
}
RS
    cat > "$TMP/repo/docs/reports/spec-state-reconciliation-2026-07-08.md" <<'MD'
# Spec State Reconciliation Report

## Remaining Risks

- Issue backlog hygiene is now the main spec-to-execution bottleneck.
- RAG has solid local gates, but production-scale corpus and citation evidence still need ongoing proof.
MD
}

@test "self-improvement apply files issues with origin:self label" {
    seed_repo_gaps
    cat > "$STUB_DIR/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"label create"*) exit 0 ;;
  *"issue create"*) printf 'https://github.com/berlinguyinca/autospec/issues/999\n'; exit 0 ;;
  *"repo view"*) printf 'berlinguyinca/autospec\n'; exit 0 ;;
esac
exit 0
SH
    chmod +x "$STUB_DIR/gh"
    export AUTOSPEC_SELF_IMPROVEMENT_APPLY=1

    run bash "$SELF_IMPROVEMENT" apply --repo-root "$TMP/repo" --repo berlinguyinca/autospec --apply --limit 1

    [ "$status" -eq 0 ]
    grep -q 'label create origin:self' "$GH_LOG"
    grep -qE 'issue create.*--label origin:self' "$GH_LOG"
}

@test "self-improvement label creation failure does not abort filing" {
    seed_repo_gaps
    cat > "$STUB_DIR/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *"label create"*) exit 1 ;;
  *"issue create"*) printf 'https://github.com/berlinguyinca/autospec/issues/999\n'; exit 0 ;;
  *"repo view"*) printf 'berlinguyinca/autospec\n'; exit 0 ;;
esac
exit 0
SH
    chmod +x "$STUB_DIR/gh"
    export AUTOSPEC_SELF_IMPROVEMENT_APPLY=1

    run bash "$SELF_IMPROVEMENT" apply --repo-root "$TMP/repo" --repo berlinguyinca/autospec --apply --limit 1

    [ "$status" -eq 0 ]
    [ "$(printf '%s' "$output" | jq -r '.filed')" = "1" ]
}

@test "resilience post-merge rollback follow-up issue carries origin:self label" {
    provenance="$TMP/provenance.json"
    cat > "$provenance" <<'JSON'
{
  "schema": "autospec.autonomous.merge_provenance.v1",
  "repo": "berlinguyinca/autospec",
  "pr": 1546,
  "workstream": "autonomous-guardrails",
  "verifier_lane": "guardian",
  "rollback_handle": "rollback-ref-1546",
  "gate_evidence": {"suite":"validate","result":"pass"}
}
JSON
    cat > "$STUB_DIR/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *'commits/main/status'*) printf '{"state":"failure","statuses":[{"state":"failure","description":"validate failed"}]}\n'; exit 0 ;;
  *"label create"*) exit 0 ;;
  issue\ create*) printf 'https://github.com/berlinguyinca/autospec/issues/2001\n'; exit 0 ;;
  *) printf '{}\n'; exit 0 ;;
esac
SH
    chmod +x "$STUB_DIR/gh"
    export AUTOSPEC_GH_CMD="$STUB_DIR/gh"
    export AUTOSPEC_MERGE_AUDIT_LOG="$TMP/audit/merge-audit.jsonl"

    run bash "$RESILIENCE" post-merge-health --repo berlinguyinca/autospec --provenance "$provenance"

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q 'FOLLOWUP_ISSUE:'
    grep -q 'label create origin:self' "$GH_LOG"
    tr '\n' ' ' < "$GH_LOG" | grep -qE 'issue create.*--label origin:self'
}

@test "resilience label creation failure does not abort follow-up filing" {
    provenance="$TMP/provenance.json"
    cat > "$provenance" <<'JSON'
{
  "schema": "autospec.autonomous.merge_provenance.v1",
  "repo": "berlinguyinca/autospec",
  "pr": 1547,
  "workstream": "autonomous-guardrails",
  "verifier_lane": "guardian",
  "rollback_handle": "rollback-ref-1547",
  "gate_evidence": {"suite":"validate","result":"pass"}
}
JSON
    cat > "$STUB_DIR/gh" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$GH_LOG"
case "$*" in
  *'commits/main/status'*) printf '{"state":"failure","statuses":[{"state":"failure","description":"validate failed"}]}\n'; exit 0 ;;
  *"label create"*) exit 1 ;;
  issue\ create*) printf 'https://github.com/berlinguyinca/autospec/issues/2002\n'; exit 0 ;;
  *) printf '{}\n'; exit 0 ;;
esac
SH
    chmod +x "$STUB_DIR/gh"
    export AUTOSPEC_GH_CMD="$STUB_DIR/gh"
    export AUTOSPEC_MERGE_AUDIT_LOG="$TMP/audit/merge-audit.jsonl"

    run bash "$RESILIENCE" post-merge-health --repo berlinguyinca/autospec --provenance "$provenance"

    [ "$status" -eq 1 ]
    printf '%s\n' "$output" | grep -q 'FOLLOWUP_ISSUE:https://github.com/berlinguyinca/autospec/issues/2002'
}

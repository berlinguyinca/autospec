#!/usr/bin/env bats

setup() {
    REPO_ROOT="$(git rev-parse --show-toplevel)"
    SCRIPT="$REPO_ROOT/skills/autospec-run/scripts/claim-issue.sh"
    FIXTURE_DIR="$(mktemp -d)"
    MOCK_BIN="$FIXTURE_DIR/bin"
    mkdir -p "$MOCK_BIN"
    : > "$FIXTURE_DIR/edit.log"

    cat > "$MOCK_BIN/gh" <<MOCKEOF
#!/usr/bin/env bash
set -eu
FIXTURE_DIR="$FIXTURE_DIR"
sub="\${1:-} \${2:-}"
case "\$sub" in
  "issue view")
    if printf '%s\n' "\$@" | grep -q -- "--jq"; then
      jq -r '.labels[].name' "\$FIXTURE_DIR/issue.json"
    else
      cat "\$FIXTURE_DIR/issue.json"
    fi
    ;;
  "label create")
    exit 0
    ;;
  "issue edit")
    printf '%s\n' "\$*" >> "\$FIXTURE_DIR/edit.log"
    exit 1
    ;;
  "repo view")
    printf 'test/repo\n'
    ;;
  *)
    printf '[]\n'
    ;;
esac
MOCKEOF
    chmod +x "$MOCK_BIN/gh"
}

teardown() {
    rm -rf "$FIXTURE_DIR"
}

safe_block() {
    cat <<'EOF'
## Safety review

<!-- autospec-safety:begin -->
- **decision:** `SAFETY_PASS`
<!-- autospec-safety:end -->

EOF
}

write_issue() {
    body="$1"
    labels_json="$2"
    jq -n --arg body "$body" --argjson labels "$labels_json" \
      '{labels:$labels, body:$body}' > "$FIXTURE_DIR/issue.json"
}

run_claim() {
    PATH="$MOCK_BIN:$PATH" AUTOSPEC_CLAIM_CONFIRM_READS=1 AUTOSPEC_CLAIM_SETTLE_SECONDS=0 \
      bash "$SCRIPT" --issue 700 --repo test/repo --worker-id worker-a --branch feat/test
}

@test "claim refuses security-quarantined issue before label mutation" {
    write_issue "$(safe_block)" '[{"name":"auto-implement"},{"name":"safety:reviewed"},{"name":"security:quarantined"}]'

    run run_claim

    [ "$status" -eq 2 ]
    [ "$(echo "$output" | jq -r '.reason')" = "safety_gate_failed" ]
    [ ! -s "$FIXTURE_DIR/edit.log" ]
}

@test "claim refuses issue missing safety:reviewed before label mutation" {
    write_issue "$(safe_block)" '[{"name":"auto-implement"}]'

    run run_claim

    [ "$status" -eq 2 ]
    [ "$(echo "$output" | jq -r '.reason')" = "safety_gate_failed" ]
    [ ! -s "$FIXTURE_DIR/edit.log" ]
}

@test "claim refuses issue missing safety markers before label mutation" {
    body="$(cat <<'EOF'
## Safety review

- **decision:** `SAFETY_PASS`
EOF
)"
    write_issue "$body" '[{"name":"auto-implement"},{"name":"safety:reviewed"}]'

    run run_claim

    [ "$status" -eq 2 ]
    [ "$(echo "$output" | jq -r '.reason')" = "safety_gate_failed" ]
    [ ! -s "$FIXTURE_DIR/edit.log" ]
}

@test "claim refuses non-pass safety block before label mutation" {
    body="$(cat <<'EOF'
## Safety review

- **decision:** `SAFETY_PASS`

<!-- autospec-safety:begin -->
- **decision:** `SAFETY_BLOCK`
<!-- autospec-safety:end -->
EOF
)"
    write_issue "$body" '[{"name":"auto-implement"},{"name":"safety:reviewed"}]'

    run run_claim

    [ "$status" -eq 2 ]
    [ "$(echo "$output" | jq -r '.reason')" = "safety_gate_failed" ]
    [ ! -s "$FIXTURE_DIR/edit.log" ]
}

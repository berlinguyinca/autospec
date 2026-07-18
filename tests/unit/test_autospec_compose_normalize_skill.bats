#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SKILL_DIR="$REPO_ROOT/skills/autospec-compose-normalize"
  SKILL="$SKILL_DIR/SKILL.md"
  WORKFLOW_GUARD="$SKILL_DIR/scripts/workflow-guard.sh"
}

write_lookup_documents() {
  local root="$1" fingerprint="$2"
  mkdir -p "$root"
  jq -n --arg fingerprint "$fingerprint" '
    [range(0; 201) | {
      number: (. + 1), state: "CLOSED", url: ("https://example.test/issues/" + ((. + 1) | tostring)),
      body: ("<!-- autospec-compose-fingerprint: generic-" + (. | tostring) + " -->")
    }] + [{
      number: 9001, state: "OPEN", url: "https://example.test/issues/9001",
      body: ("<!-- autospec-compose-fingerprint: " + $fingerprint + " -->")
    }]
  ' > "$root/issues.json"
  jq -n --arg fingerprint "$fingerprint" '[{
    number: 9002, state: "CLOSED", mergedAt: null, url: "https://example.test/pulls/9002",
    body: ("<!-- autospec-compose-fingerprint: " + $fingerprint + " -->")
  }]' > "$root/pulls.json"
}

@test "compose normalizer ships a derived three-harness skill" {
  [ -f "$SKILL" ]
  [ -f "$SKILL_DIR/codex/prompt.md" ]
  [ -f "$SKILL_DIR/opencode/agent.md" ]
  run bash "$REPO_ROOT/scripts/derive-trio.sh" "$SKILL_DIR" --check
  [ "$status" -eq 0 ]
}

@test "skill delegates checks and writes exclusively to the Rust normalizer" {
  run grep -F 'autospec runtime env normalize-compose --repo "$PWD" --check' "$SKILL"
  [ "$status" -eq 0 ]
  run grep -F 'autospec runtime env normalize-compose --repo "$PWD" --apply --fingerprint "$FINGERPRINT"' "$SKILL"
  [ "$status" -eq 0 ]
  run grep -E 'apply_patch|cat .*compose|yq .* -i|mapfile|(^|[[:space:]])jq[[:space:]]' "$SKILL"
  [ "$status" -ne 0 ]
}

@test "workflow consumes the versioned JSON contract and fails closed" {
  grep -q 'schema_version' "$SKILL"
  grep -q 'remaining_diagnostics' "$SKILL"
  grep -q 'NORMALIZE_COMPOSE_NOT_FOUND' "$SKILL"
  grep -q 'NORMALIZE_STALE_SOURCE' "$SKILL"
  grep -q '64 lowercase hexadecimal' "$SKILL"
}

@test "one fingerprint reuses one migration issue worktree and pull request" {
  grep -q '<!-- autospec-compose-fingerprint: SHA256 -->' "$SKILL"
  grep -q 'matching open issue or pull request' "$SKILL"
  grep -q 'matching merged pull request' "$SKILL"
  grep -q 'exactly one migration issue, branch, worktree, and pull request' "$SKILL"
  grep -q 'gh issue reopen <number>' "$SKILL"
  grep -q 'gh pr reopen <number>' "$SKILL"
  grep -q 'terminal recovery blocker' "$SKILL"
  grep -q 'needs-classify' "$SKILL"
  grep -q 'scripts/lint-issue.sh' "$SKILL"
  grep -q 'gh issue create --title' "$SKILL"
  grep -q '/autospec-classify --issues' "$SKILL"
  grep -q 'gh pr create' "$SKILL"
}

@test "claims cover every YAML input and selected manifest atomically" {
  grep -q 'SELECTED_MANIFEST' "$SKILL"
  grep -q 'input_paths' "$SKILL"
  grep -q 'manifest_path' "$SKILL"
  grep -Fq 'or "\t" in value' "$SKILL"
  run grep -q 'find .*\\*.yml.*\\*.yaml' "$SKILL"
  [ "$status" -ne 0 ]
  grep -q 'claim acquire.*COMPOSE_CLAIM_SESSION.*"$@"' "$SKILL"
  grep -q 'all-or-nothing' "$SKILL"
  grep -q 'claim verify.*COMPOSE_CLAIM_SESSION.*"$@"' "$SKILL"
  grep -q 'code_health:compose_claim_not_persisted' "$WORKFLOW_GUARD"
  grep -q 'claim refresh.*COMPOSE_CLAIM_SESSION.*"$@"' "$SKILL"
  grep -q 'claim release.*COMPOSE_CLAIM_SESSION.*"$@"' "$SKILL"
  grep -q 'every terminal path' "$SKILL"
}

@test "managed runs migrate while direct sessions refuse isolation" {
  grep -q 'Autospec-managed invocation' "$SKILL"
  grep -q 'Direct unmanaged invocation' "$SKILL"
  grep -q 'print the matching issue and pull-request URLs' "$SKILL"
  grep -q 'exit before.*runtime env up' "$SKILL"
}

@test "migration uses normal worktree CI review merge and cleanup gates" {
  grep -q 'worktree-guard.sh.*create' "$SKILL"
  grep -q 'docker compose.*config' "$SKILL"
  grep -q 'gh pr checks' "$SKILL"
  grep -q 'LGTM' "$SKILL"
  grep -q 'gh pr merge' "$SKILL"
  grep -q 'origin/main' "$SKILL"
}

@test "installer rejects an environment without normalize-compose" {
  [ -x "$SKILL_DIR/install.sh" ]
  tmp="$BATS_TEST_TMPDIR/install"
  mkdir -p "$tmp/home"

  run env HOME="$tmp/home" PATH=/usr/bin:/bin \
    "$SKILL_DIR/install.sh" --harness codex

  [ "$status" -ne 0 ]
  [[ "$output" == *"normalize-compose"* ]]
  [[ "$output" == *"bootstrap.sh"* ]]
  [ ! -e "$tmp/home/.codex/skills/autospec-compose-normalize/SKILL.md" ]
}

@test "installer accepts the exact normalize-compose capability for all harnesses" {
  tmp="$BATS_TEST_TMPDIR/install-all"
  mkdir -p "$tmp/home"
  if [ ! -x "$REPO_ROOT/target/debug/autospec" ]; then
    run cargo build -q -p autospec-cli --manifest-path "$REPO_ROOT/Cargo.toml"
    [ "$status" -eq 0 ]
  fi

  run env HOME="$tmp/home" PATH="$REPO_ROOT/target/debug:/usr/bin:/bin" \
    "$SKILL_DIR/install.sh" --harness all

  [ "$status" -eq 0 ]
  [ -f "$tmp/home/.claude/skills/autospec-compose-normalize/SKILL.md" ]
  [ -f "$tmp/home/.config/opencode/agent/autospec-compose-normalize.md" ]
  [ -f "$tmp/home/.codex/prompts/autospec-compose-normalize.md" ]
  [ -f "$tmp/home/.codex/skills/autospec-compose-normalize/SKILL.md" ]
  [ -x "$tmp/home/.autospec/scripts/claim-guard.sh" ]
  [ -x "$tmp/home/.autospec/scripts/lint-issue.sh" ]
  [ -x "$tmp/home/.autospec/scripts/worktree-guard.sh" ]
  [ -x "$tmp/home/.autospec/scripts/autospec-compose-normalize-guard.sh" ]
}

@test "lookup searches the exact fingerprint instead of a bounded generic migration page" {
  tmp="$BATS_TEST_TMPDIR/search"
  TEST_FINGERPRINT="$(printf 'a%.0s' {1..64})"
  write_lookup_documents "$tmp" "$TEST_FINGERPRINT"

  run "$WORKFLOW_GUARD" select "$TEST_FINGERPRINT" "$tmp/issues.json" "$tmp/pulls.json"

  [ "$status" -eq 0 ]
  [[ "$output" == *"https://example.test/issues/9001"* ]]
  [[ "$output" == *$'pr\tCLOSED\t9002\thttps://example.test/pulls/9002'* ]]
  [ "$(printf '%s\n' "$output" | wc -l)" -eq 2 ]
  run "$WORKFLOW_GUARD" search-query "$TEST_FINGERPRINT"
  [ "$status" -eq 0 ]
  [ "$output" = "$TEST_FINGERPRINT in:body" ]
  grep -Fq -- '--search "$query"' "$WORKFLOW_GUARD"
}

@test "direct unmanaged guard prints matching URLs and never starts runtime provisioning" {
  tmp="$BATS_TEST_TMPDIR/direct"
  TEST_FINGERPRINT="$(printf 'b%.0s' {1..64})"
  write_lookup_documents "$tmp" "$TEST_FINGERPRINT"

  run "$WORKFLOW_GUARD" direct-refuse "$TEST_FINGERPRINT" "$tmp/issues.json" "$tmp/pulls.json"

  [ "$status" -eq 3 ]
  [[ "$output" == *"https://example.test/issues/9001"* ]]
  [[ "$output" == *"https://example.test/pulls/9002"* ]]
  run grep -F 'runtime env up' "$WORKFLOW_GUARD"
  [ "$status" -ne 0 ]
}

@test "fallback claim session tokens are random and never PID-derived" {
  run "$WORKFLOW_GUARD" new-session-token
  [ "$status" -eq 0 ]
  first="$output"
  run "$WORKFLOW_GUARD" new-session-token
  [ "$status" -eq 0 ]
  second="$output"

  [[ "$first" =~ ^compose-normalize-[0-9a-f]{48}$ ]]
  [[ "$second" =~ ^compose-normalize-[0-9a-f]{48}$ ]]
  [ "$first" != "$second" ]
}

@test "claim lifecycle stays strict with a stable owner when caller mode is off" {
  state="$BATS_TEST_TMPDIR/claim-state"
  token="compose-normalize-test-session"
  common=(env AUTOSPEC_CLAIM_GUARD=off AUTOSPEC_STATE_DIR="$state" \
    AUTOSPEC_REPO=example/autospec AUTOSPEC_CLAIM_GUARD_SH="$REPO_ROOT/scripts/claim-guard.sh")
  targets=(compose.yaml .autospec/runtime.yml)

  run "${common[@]}" "$WORKFLOW_GUARD" claim acquire "$token" "${targets[@]}"
  [ "$status" -eq 0 ]
  run "${common[@]}" "$WORKFLOW_GUARD" claim verify "$token" "${targets[@]}"
  [ "$status" -eq 0 ]
  run "${common[@]}" "$WORKFLOW_GUARD" claim refresh "$token" "${targets[@]}"
  [ "$status" -eq 0 ]
  run "${common[@]}" "$WORKFLOW_GUARD" claim release "$token" "${targets[@]}"
  [ "$status" -eq 0 ]
  run find "$state/edit-claims" -type f -name '*.json' -print
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  run find "$state/edit-claims" -type d -name '*.lock' -print
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "release accepts an immediate claim handoff to a different owner" {
  state="$BATS_TEST_TMPDIR/handoff-state"
  common=(env AUTOSPEC_CLAIM_GUARD=off AUTOSPEC_STATE_DIR="$state" \
    AUTOSPEC_REPO=example/autospec AUTOSPEC_CLAIM_GUARD_SH="$REPO_ROOT/scripts/claim-guard.sh")

  run "${common[@]}" "$WORKFLOW_GUARD" claim acquire owner-a compose.yaml
  [ "$status" -eq 0 ]
  run "${common[@]}" "$WORKFLOW_GUARD" claim release owner-a compose.yaml
  [ "$status" -eq 0 ]
  run "${common[@]}" "$WORKFLOW_GUARD" claim acquire owner-b compose.yaml
  [ "$status" -eq 0 ]

  run "${common[@]}" "$WORKFLOW_GUARD" claim release owner-a compose.yaml
  [ "$status" -eq 0 ]
  run "${common[@]}" "$WORKFLOW_GUARD" claim verify owner-b compose.yaml
  [ "$status" -eq 0 ]
}

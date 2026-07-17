#!/usr/bin/env bats

setup() {
  REPO_ROOT="$(cd "$BATS_TEST_DIRNAME/../.." && pwd)"
  SKILL_DIR="$REPO_ROOT/skills/autospec-compose-normalize"
  SKILL="$SKILL_DIR/SKILL.md"
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
  grep -q 'claim-guard.sh.*acquire.*"$@"' "$SKILL"
  grep -q 'all-or-nothing' "$SKILL"
  grep -q 'claim-guard.sh.*status' "$SKILL"
  grep -q 'code_health:compose_claim_not_persisted' "$SKILL"
  grep -q 'claim-guard.sh.*refresh' "$SKILL"
  grep -q 'claim-guard.sh.*release.*"$@"' "$SKILL"
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

@test "installer rejects an autospec binary without normalize-compose" {
  [ -x "$SKILL_DIR/install.sh" ]
  tmp="$BATS_TEST_TMPDIR/install"
  mkdir -p "$tmp/bin" "$tmp/home"
  cat > "$tmp/bin/autospec" <<'EOF'
#!/usr/bin/env sh
echo 'runtime env: up status down normalize-compose experimental'
EOF
  chmod +x "$tmp/bin/autospec"

  run env HOME="$tmp/home" PATH="$tmp/bin:$PATH" \
    "$SKILL_DIR/install.sh" --harness codex

  [ "$status" -ne 0 ]
  [[ "$output" == *"normalize-compose"* ]]
  [[ "$output" == *"bootstrap.sh"* ]]
  [ ! -e "$tmp/home/.codex/skills/autospec-compose-normalize/SKILL.md" ]
}

@test "installer accepts the exact normalize-compose capability for all harnesses" {
  tmp="$BATS_TEST_TMPDIR/install-all"
  mkdir -p "$tmp/bin" "$tmp/home"
  cat > "$tmp/bin/autospec" <<'EOF'
#!/usr/bin/env sh
echo 'normalize-compose --repo PATH --check|--apply --fingerprint SHA256'
EOF
  chmod +x "$tmp/bin/autospec"

  run env HOME="$tmp/home" PATH="$tmp/bin:$PATH" \
    "$SKILL_DIR/install.sh" --harness all

  [ "$status" -eq 0 ]
  [ -f "$tmp/home/.claude/skills/autospec-compose-normalize/SKILL.md" ]
  [ -f "$tmp/home/.config/opencode/agent/autospec-compose-normalize.md" ]
  [ -f "$tmp/home/.codex/prompts/autospec-compose-normalize.md" ]
  [ -f "$tmp/home/.codex/skills/autospec-compose-normalize/SKILL.md" ]
  [ -x "$tmp/home/.autospec/scripts/claim-guard.sh" ]
  [ -x "$tmp/home/.autospec/scripts/lint-issue.sh" ]
  [ -x "$tmp/home/.autospec/scripts/worktree-guard.sh" ]
}
